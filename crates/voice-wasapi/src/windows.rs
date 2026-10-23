//! Enumeration, format checks, and stream lifetime run on the engine's control thread.
use crate::rt::{DesktopCapture, INPUT_QUEUE, InputCapture, VoiceOutput};
use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, DeviceId, ErrorKind, Host, HostId, SampleFormat, Stream, StreamConfig,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use voice_core::{
    AudioConfig, AudioShared, DesktopShared, DeviceInfo, DeviceKind, SAMPLE_RATE, SampleQueue,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// Owns physical capture and local stereo playback. No virtual endpoint is created.
pub struct AudioHandle {
    input: Option<Stream>,
    output: Option<Stream>,
    running: Arc<AtomicBool>,
    shared: Arc<AudioShared>,
}

impl AudioHandle {
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.shared.metrics.active.store(false, Ordering::Release);
        self.shared.metrics.output_peak.store(0, Ordering::Relaxed);
        self.shared
            .metrics
            .algorithmic_latency_frames
            .store(0, Ordering::Relaxed);
        // Joining backend threads and freeing prepared callback state happens here,
        // never inside a PCM callback. Stop capture before destroying playback.
        drop(self.input.take());
        drop(self.output.take());
        self.shared.metrics.output_peak.store(0, Ordering::Relaxed);
        self.shared
            .metrics
            .algorithmic_latency_frames
            .store(0, Ordering::Relaxed);
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Owns one explicit render-endpoint loopback stream, separate from the voice path.
pub struct DesktopCaptureHandle {
    target: String,
    stream: Option<Stream>,
    running: Arc<AtomicBool>,
    shared: Arc<DesktopShared>,
}

impl DesktopCaptureHandle {
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.shared.active.store(false, Ordering::Release);
        self.shared.peak.store(0, Ordering::Relaxed);
        drop(self.stream.take());
        self.shared.peak.store(0, Ordering::Relaxed);
    }
}

impl Drop for DesktopCaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn host() -> Result<Host> {
    cpal::host_from_id(HostId::Wasapi).context("initialize Windows WASAPI")
}

/// Stable endpoint IDs are returned as names; the friendly labels are descriptions.
pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let host = host()?;
    let mut devices = Vec::new();
    fn append(
        endpoints: impl Iterator<Item = Device>,
        kind: DeviceKind,
        devices: &mut Vec<DeviceInfo>,
    ) {
        for device in endpoints {
            let Ok(id) = device.id() else {
                continue;
            };
            let name = id.to_string();
            // CPAL Device::Display assumes description succeeds and can panic
            // when an endpoint disappears during enumeration. Keep this fallible.
            let Ok(description) = device.description() else {
                continue;
            };
            let description = if description.name().trim().is_empty() {
                name.clone()
            } else {
                description.name().to_owned()
            };
            if !eligible(&name, &description) {
                continue;
            }
            devices.push(DeviceInfo {
                id: (devices.len() + 1) as u32,
                name,
                description,
                kind: kind.clone(),
            });
        }
    }
    append(host.input_devices()?, DeviceKind::Input, &mut devices);
    append(host.output_devices()?, DeviceKind::Output, &mut devices);
    devices.sort_by(|a, b| a.description.cmp(&b.description));
    Ok(devices)
}

fn eligible(name: &str, description: &str) -> bool {
    !name.starts_with("voice-app.")
        && !description.starts_with("voice-app.")
        && description != "Voice App Virtual Microphone"
}

fn select_device(
    host: &Host,
    devices: &[DeviceInfo],
    requested: Option<&str>,
    kind: DeviceKind,
) -> Result<String> {
    let eligible = |device: &&DeviceInfo| device.kind == kind;
    let selected = if let Some(requested) = requested {
        let numeric = requested.parse::<u32>().ok();
        devices
            .iter()
            .filter(eligible)
            .find(|device| device.name == requested || numeric == Some(device.id))
    } else {
        // Resolve defaults to a pinned stable endpoint before opening a stream. We do
        // not let CPAL's default-device auto-rerouting substitute another microphone.
        let default = match kind {
            DeviceKind::Input => host.default_input_device(),
            DeviceKind::Output => host.default_output_device(),
        }
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
        default
            .as_deref()
            .and_then(|name| devices.iter().filter(eligible).find(|d| d.name == name))
            .or_else(|| {
                devices
                    .iter()
                    .filter(eligible)
                    .min_by(|a, b| a.name.cmp(&b.name))
            })
    };
    selected.map(|device| device.name.clone()).ok_or_else(|| {
        anyhow!("selected Windows {kind:?} endpoint is unavailable; no route was changed")
    })
}

/// Resolve automatic physical routes without changing Windows defaults.
pub fn resolve_config(config: &AudioConfig) -> Result<AudioConfig> {
    config.validate().map_err(anyhow::Error::msg)?;
    let host = host()?;
    let devices = list_devices()?;
    let mut config = config.clone();
    config.input_device = Some(select_device(
        &host,
        &devices,
        config.input_device.as_deref(),
        DeviceKind::Input,
    )?);
    config.output_device = Some(select_device(
        &host,
        &devices,
        config.output_device.as_deref(),
        DeviceKind::Output,
    )?);
    Ok(config)
}

fn endpoint(host: &Host, id: &str) -> Result<Device> {
    let id = id
        .parse::<DeviceId>()
        .context("invalid Windows endpoint ID")?;
    host.device_by_id(&id)
        .ok_or_else(|| anyhow!("Windows audio endpoint disconnected"))
}

fn native_config(device: &Device, input: bool, quantum: u32) -> Result<StreamConfig> {
    let description = device
        .description()
        .context("read Windows endpoint description (it may have disconnected)")?;
    let label = description.name();
    let native = if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .context("read the Windows endpoint mix format")?;
    // CPAL advertises additional output rates using WASAPI automatic conversion.
    // Require the actual mix format instead: no hidden rate conversion is claimed.
    if native.sample_rate() != SAMPLE_RATE || native.sample_format() != SampleFormat::F32 {
        bail!(
            "Windows preview requires a native 48000 Hz F32 endpoint; {} uses {} Hz {:?}. Select 48 kHz in Windows sound settings or choose another device",
            label,
            native.sample_rate(),
            native.sample_format()
        );
    }
    if (input && !(1..=2).contains(&native.channels())) || (!input && native.channels() != 2) {
        bail!(
            "Windows preview requires a mono/stereo microphone and stereo output; {} has {} channels",
            label,
            native.channels()
        );
    }
    let mut config = native.config();
    // On WASAPI this is a requested buffer duration; actual callback periods are
    // controlled by the endpoint. RT code handles arbitrary bounded chunks.
    config.buffer_size = BufferSize::Fixed(quantum);
    Ok(config)
}

fn fail_audio(running: &AtomicBool, shared: &AudioShared) {
    running.store(false, Ordering::Release);
    shared.metrics.active.store(false, Ordering::Release);
    shared.metrics.output_peak.store(0, Ordering::Relaxed);
    shared
        .metrics
        .algorithmic_latency_frames
        .store(0, Ordering::Relaxed);
}

fn handle_audio_error(kind: ErrorKind, monitor: bool, running: &AtomicBool, shared: &AudioShared) {
    if kind == ErrorKind::Xrun {
        // CPAL reports capture discontinuities as Xrun and keeps the stream alive.
        // Report the gap without finalizing a recording or replacing the device.
        let counter = if monitor {
            &shared.metrics.monitor_overruns
        } else {
            &shared.metrics.input_underruns
        };
        counter.fetch_add(1, Ordering::Relaxed);
    } else {
        fail_audio(running, shared);
    }
}

fn handle_desktop_error(kind: ErrorKind, running: &AtomicBool, shared: &DesktopShared) {
    if kind == ErrorKind::Xrun {
        shared.overruns.fetch_add(1, Ordering::Relaxed);
    } else {
        running.store(false, Ordering::Release);
        shared.active.store(false, Ordering::Release);
        shared.peak.store(0, Ordering::Relaxed);
    }
}

/// Capture, process, record, and optionally monitor the physical microphone.
/// Monitoring is controlled by the existing atomics; this does not publish a virtual mic.
pub fn start(config: AudioConfig, shared: Arc<AudioShared>) -> Result<AudioHandle> {
    let config = resolve_config(&config)?;
    let host = host()?;
    let input = endpoint(
        &host,
        config
            .input_device
            .as_deref()
            .context("resolved input missing")?,
    )?;
    let output = endpoint(
        &host,
        config
            .output_device
            .as_deref()
            .context("resolved output missing")?,
    )?;
    let input_config = native_config(&input, true, config.quantum)?;
    let output_config = native_config(&output, false, config.quantum)?;
    let channels = usize::from(input_config.channels);
    shared.metrics.active.store(false, Ordering::Release);
    shared.soundboard.discard();
    shared.preview.discard();
    let queue = Arc::new(SampleQueue::new(INPUT_QUEUE));
    let running = Arc::new(AtomicBool::new(true));
    let mut capture = InputCapture::new(shared.clone(), queue.clone());
    let mut processor = VoiceOutput::new(shared.clone(), queue, SAMPLE_RATE);
    let capture_gate = running.clone();
    let capture_error_gate = running.clone();
    let capture_error_shared = shared.clone();
    let input = input
        .build_input_stream::<f32, _, _>(
            input_config,
            move |samples, _| {
                if capture_gate.load(Ordering::Acquire) {
                    capture.process(samples, channels);
                }
            },
            move |error| {
                handle_audio_error(
                    error.kind(),
                    false,
                    &capture_error_gate,
                    &capture_error_shared,
                )
            },
            Some(CONNECT_TIMEOUT),
        )
        .context("open native 48 kHz microphone capture")?;
    let output_gate = running.clone();
    let output_error_gate = running.clone();
    let output_error_shared = shared.clone();
    let output = output
        .build_output_stream::<f32, _, _>(
            output_config,
            move |samples, _| processor.render_if_running(samples, &output_gate),
            move |error| {
                handle_audio_error(error.kind(), true, &output_error_gate, &output_error_shared)
            },
            Some(CONNECT_TIMEOUT),
        )
        .context("open native 48 kHz stereo local playback")?;
    let handle = AudioHandle {
        input: Some(input),
        output: Some(output),
        running,
        shared,
    };
    handle
        .input
        .as_ref()
        .context("input stream missing")?
        .play()?;
    handle
        .output
        .as_ref()
        .context("output stream missing")?
        .play()?;
    if !handle.running.load(Ordering::Acquire) {
        bail!("Windows audio failed during startup; output remains silent");
    }
    handle.shared.metrics.active.store(true, Ordering::Release);
    // Never overwrite an error concurrently delivered between the readiness checks.
    if !handle.running.load(Ordering::Acquire) {
        handle.shared.metrics.active.store(false, Ordering::Release);
        bail!("Windows audio failed during startup; output remains silent");
    }
    Ok(handle)
}

/// WASAPI loopback captures only the explicitly selected render endpoint. It is
/// intentionally not connected to the voice input or its processed output queue.
pub fn start_desktop(target: String, shared: Arc<DesktopShared>) -> Result<DesktopCaptureHandle> {
    if target.trim().is_empty() || target.starts_with("voice-app.") {
        bail!("select an explicit Windows desktop output outside voice-app");
    }
    let host = host()?;
    let target = select_device(&host, &list_devices()?, Some(&target), DeviceKind::Output)?;
    let output = endpoint(&host, &target)?;
    let config = native_config(&output, false, 256)?;
    shared.active.store(false, Ordering::Release);
    shared.frames.store(0, Ordering::Relaxed);
    shared.overruns.store(0, Ordering::Relaxed);
    shared.non_finite_samples.store(0, Ordering::Relaxed);
    shared.peak.store(0, Ordering::Relaxed);
    let running = Arc::new(AtomicBool::new(true));
    let mut capture = DesktopCapture::new(shared.clone());
    let gate = running.clone();
    let capture_shared = shared.clone();
    let error_gate = running.clone();
    let error_shared = shared.clone();
    // CPAL 0.18.2 enables AUDCLNT_STREAMFLAGS_LOOPBACK when build_input_stream
    // is called on a WASAPI render device. Opening a default capture is not involved.
    let stream = output
        .build_input_stream::<f32, _, _>(
            config,
            move |samples, _| {
                if gate.load(Ordering::Acquire) {
                    capture.process(samples);
                }
                if !gate.load(Ordering::Acquire) {
                    capture_shared.peak.store(0, Ordering::Relaxed);
                }
            },
            move |error| handle_desktop_error(error.kind(), &error_gate, &error_shared),
            Some(CONNECT_TIMEOUT),
        )
        .context("open explicit native 48 kHz stereo WASAPI loopback")?;
    let handle = DesktopCaptureHandle {
        target,
        stream: Some(stream),
        running,
        shared,
    };
    handle
        .stream
        .as_ref()
        .context("desktop stream missing")?
        .play()?;
    if !handle.running.load(Ordering::Acquire) {
        bail!("Windows desktop capture failed during startup");
    }
    handle.shared.active.store(true, Ordering::Release);
    if !handle.running.load(Ordering::Acquire) {
        handle.shared.active.store(false, Ordering::Release);
        bail!("Windows desktop capture failed during startup");
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_xruns_increment_counters_without_stopping_voice() {
        let running = AtomicBool::new(true);
        let shared = AudioShared::default();
        shared.metrics.active.store(true, Ordering::Release);
        handle_audio_error(ErrorKind::Xrun, false, &running, &shared);
        handle_audio_error(ErrorKind::Xrun, true, &running, &shared);
        assert!(running.load(Ordering::Acquire));
        assert!(shared.metrics.active.load(Ordering::Acquire));
        assert_eq!(shared.metrics.input_underruns.load(Ordering::Relaxed), 1);
        assert_eq!(shared.metrics.monitor_overruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn fatal_voice_error_disables_callbacks_and_clears_levels() {
        let running = AtomicBool::new(true);
        let shared = AudioShared::default();
        shared.metrics.active.store(true, Ordering::Release);
        shared
            .metrics
            .output_peak
            .store(0.5_f32.to_bits(), Ordering::Relaxed);
        shared
            .metrics
            .algorithmic_latency_frames
            .store(512, Ordering::Relaxed);
        handle_audio_error(ErrorKind::DeviceNotAvailable, false, &running, &shared);
        assert!(!running.load(Ordering::Acquire));
        assert!(!shared.metrics.active.load(Ordering::Acquire));
        assert_eq!(shared.metrics.output_peak.load(Ordering::Relaxed), 0);
        assert_eq!(
            shared
                .metrics
                .algorithmic_latency_frames
                .load(Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn desktop_xrun_keeps_capture_active_but_fatal_error_stops_it() {
        let running = AtomicBool::new(true);
        let shared = DesktopShared::default();
        shared.active.store(true, Ordering::Release);
        handle_desktop_error(ErrorKind::Xrun, &running, &shared);
        assert!(running.load(Ordering::Acquire));
        assert!(shared.active.load(Ordering::Acquire));
        assert_eq!(shared.overruns.load(Ordering::Relaxed), 1);
        handle_desktop_error(ErrorKind::StreamInvalidated, &running, &shared);
        assert!(!running.load(Ordering::Acquire));
        assert!(!shared.active.load(Ordering::Acquire));
        assert_eq!(shared.peak.load(Ordering::Relaxed), 0);
    }
}
