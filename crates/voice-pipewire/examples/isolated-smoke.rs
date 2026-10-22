//! Hardware-free signal-path tests. Run only via scripts/test-pipewire.sh.
mod support;

use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use voice_core::{AudioConfig, AudioShared, DspSettings, EffectKind};

fn main() -> Result<()> {
    ensure!(
        std::env::var("VOICE_APP_ISOLATED_TEST").as_deref() == Ok("1"),
        "run scripts/test-pipewire.sh; this test must not connect to user audio"
    );
    let fixture = PathBuf::from(std::env::var("VOICE_APP_TEST_DIR")?);
    let input_id = voice_pipewire::list_devices()?
        .iter()
        .find(|d| d.name == "voice-test.input")
        .context("fixture source missing")?
        .id;
    let shared = Arc::new(AudioShared::default());
    shared.bypass.store(true, Ordering::Relaxed);
    let config = AudioConfig {
        input_device: Some("voice-test.input".into()),
        output_device: Some("voice-test.output".into()),
        ..Default::default()
    };
    let mut handle = voice_pipewire::start(config.clone(), shared.clone())?;
    let mic_source = port("-o", "voice-test.input:", None)?;
    link(&mic_source, &port("-i", "voice-app.capture:", None)?)?;
    for channel in ["FL", "FR"] {
        link(
            &port(
                "-o",
                "voice-app.monitor:",
                Some(&format!("output_{channel}")),
            )?,
            &port(
                "-i",
                "voice-test.output:",
                Some(&format!("playback_{channel}")),
            )?,
        )?;
    }
    let virtual_source = port("-o", "voice-app.virtual-microphone:", None)?;
    let monitor_left = port("-o", "voice-test.output:", Some("monitor_FL"))?;
    let monitor_right = port("-o", "voice-test.output:", Some("monitor_FR"))?;

    // One stereo capture receives input reference on L and processed virtual mic on R.
    // Their samples share a callback/clock: the impulse offset measures only software.
    let mut impulse = vec![0_f32; 48_000];
    impulse[12_000] = 0.5;
    let stereo = record_stereo(
        &fixture,
        "impulse",
        &mic_source,
        &virtual_source,
        96_000,
        || {
            play(
                &fixture,
                "impulse-input",
                &impulse,
                &port("-i", "voice-test.input:", None)?,
            )
        },
    )?;
    let left = stereo
        .iter()
        .position(|f| f[0].abs() > 0.45)
        .context("reference impulse absent")?;
    let right = stereo
        .iter()
        .position(|f| f[1].abs() > 0.45)
        .context("virtual microphone impulse absent")?;
    ensure!(
        right >= left && right - left <= 2048,
        "unexpected software impulse offset: reference={left}, virtual={right}"
    );
    let delay = right - left;

    // Verify a DSP setting on the exported source against the same input reference.
    shared.bypass.store(false, Ordering::Relaxed);
    shared.dsp.store(DspSettings {
        gain_db: -6.,
        ..Default::default()
    });
    let dsp_signal = sine(48_000, 440., 0.4);
    let stereo = record_stereo(
        &fixture,
        "dsp",
        &mic_source,
        &virtual_source,
        96_000,
        || {
            play(
                &fixture,
                "dsp-input",
                &dsp_signal,
                &port("-i", "voice-test.input:", None)?,
            )
        },
    )?;
    let energy = stereo.iter().fold([0_f64; 2], |mut energy, frame| {
        energy[0] += f64::from(frame[0]).powi(2);
        energy[1] += f64::from(frame[1]).powi(2);
        energy
    });
    let gain_ratio = (energy[1] / energy[0]).sqrt();
    ensure!(
        (0.45..0.55).contains(&gain_ratio),
        "-6dB DSP gain did not reach the source: ratio={gain_ratio}"
    );

    // Reordering must affect actual exported PCM, not only the command/preset JSON.
    // Distortion before attenuation is observably quieter than distortion after it.
    let gain_first = DspSettings {
        gain_db: -12.,
        drive: 0.8,
        ..Default::default()
    };
    let mut drive_first = gain_first;
    let gain_index = drive_first
        .effect_order
        .0
        .iter()
        .position(|kind| *kind == EffectKind::Gain)
        .unwrap();
    let drive_index = drive_first
        .effect_order
        .0
        .iter()
        .position(|kind| *kind == EffectKind::Distortion)
        .unwrap();
    drive_first.effect_order.0.swap(gain_index, drive_index);
    let mut order_gains = [0_f64; 2];
    for (index, settings) in [gain_first, drive_first].into_iter().enumerate() {
        shared.dsp.store(settings);
        let name = format!("effect-order-{index}");
        let stereo = record_stereo(
            &fixture,
            &name,
            &mic_source,
            &virtual_source,
            96_000,
            || {
                play(
                    &fixture,
                    &format!("{name}-input"),
                    &dsp_signal,
                    &port("-i", "voice-test.input:", None)?,
                )
            },
        )?;
        let energy = stereo.iter().fold([0_f64; 2], |mut energy, frame| {
            energy[0] += f64::from(frame[0]).powi(2);
            energy[1] += f64::from(frame[1]).powi(2);
            energy
        });
        order_gains[index] = (energy[1] / energy[0]).sqrt();
    }
    ensure!(
        order_gains
            .iter()
            .all(|gain| gain.is_finite() && *gain > 0.01)
            && order_gains[0] > order_gains[1] * 1.5,
        "effect order did not change exported audio: gain-first={}, distortion-first={}",
        order_gains[0],
        order_gains[1]
    );

    shared.bypass.store(true, Ordering::Relaxed);
    shared.monitor_enabled.store(true, Ordering::Relaxed);
    shared
        .monitor_gain
        .store(0.5_f32.to_bits(), Ordering::Relaxed);
    let stereo = record_stereo(
        &fixture,
        "monitor",
        &mic_source,
        &monitor_left,
        96_000,
        || {
            play(
                &fixture,
                "monitor-input",
                &dsp_signal,
                &port("-i", "voice-test.input:", None)?,
            )
        },
    )?;
    let energy = stereo.iter().fold([0_f64; 2], |mut energy, frame| {
        energy[0] += f64::from(frame[0]).powi(2);
        energy[1] += f64::from(frame[1]).powi(2);
        energy
    });
    let monitor_ratio = (energy[1] / energy[0]).sqrt();
    ensure!(
        (0.45..0.55).contains(&monitor_ratio),
        "voice monitoring gain failed: ratio={monitor_ratio}"
    );

    // Capture the stereo sink's monitor ports, not the app's own output. A missing
    // right-hand route must fail even when mono DSP and the virtual mic still work.
    let monitored = record_stereo(
        &fixture,
        "monitor-stereo",
        &monitor_left,
        &monitor_right,
        96_000,
        || {
            play(
                &fixture,
                "monitor-stereo-input",
                &dsp_signal,
                &port("-i", "voice-test.input:", None)?,
            )
        },
    )?;
    let monitor_peaks = assert_dual_mono("voice monitoring", &monitored, 0.15)?;

    // Record both headphone channels and the virtual mic concurrently: preview
    // reaches both ears with voice monitoring off, but never the virtual mic.
    shared.muted.store(true, Ordering::Relaxed);
    shared.monitor_enabled.store(false, Ordering::Relaxed);
    let preview = sine(12_000, 660., 0.25);
    let mut preview_virtual = Recorder::new(&fixture, "preview-virtual", 1, 48_000)?;
    link(
        &virtual_source,
        &port("-i", "voice-test.preview-virtual:", None)?,
    )?;
    let stereo = record_stereo(
        &fixture,
        "preview",
        &monitor_left,
        &monitor_right,
        48_000,
        || {
            ensure!(
                shared.preview.push(&preview) == preview.len(),
                "preview queue unexpectedly full"
            );
            Ok(None)
        },
    )?;
    let preview_peaks = assert_dual_mono("preview", &stereo, 0.2)?;
    let virtual_peak = preview_virtual
        .finish()?
        .iter()
        .fold(0_f32, |peak, value| peak.max(value.abs()));
    ensure!(
        virtual_peak < 1e-5,
        "preview leaked into virtual microphone: peak={virtual_peak}"
    );

    // Soundboard remains in the virtual mix when the mic is muted; the internal recording
    // continues even when no external consumer is attached.
    let mut consumer = Recorder::new(&fixture, "soundboard", 1, 48_000)?;
    link(
        &virtual_source,
        &port("-i", "voice-test.soundboard:", None)?,
    )?;
    // This test is the sole recording consumer, standing in for the engine worker.
    shared.recording.discard();
    let started = Instant::now();
    let mut recorded = [0.; 4096];
    let mut samples = 0_usize;
    let mut peak = 0_f32;
    let board = sine(512, 330., 0.25);
    while started.elapsed() < Duration::from_secs(2) {
        shared.soundboard.push(&board);
        let n = shared.recording.pop(&mut recorded);
        samples += n;
        peak = recorded[..n].iter().fold(peak, |p, v| p.max(v.abs()));
        std::thread::sleep(Duration::from_millis(5));
    }
    let raw = consumer.finish()?;
    let board_peak = raw.iter().fold(0_f32, |peak, v| peak.max(v.abs()));
    ensure!(
        samples > 48_000 && peak > 0.2 && board_peak > 0.2,
        "soundboard/recording path failed"
    );
    let metrics = shared.metrics.snapshot();
    ensure!(
        metrics.active && metrics.non_finite_samples == 0,
        "backend stopped or emitted invalid PCM"
    );
    ensure!(
        nodes()?.contains(voice_core::VIRTUAL_NODE_NAME),
        "virtual source not published"
    );
    handle.stop();
    ensure!(
        !shared.metrics.snapshot().active,
        "stop did not clear active state"
    );
    ensure!(
        !nodes()?.contains(voice_core::VIRTUAL_NODE_NAME),
        "temporary source survived stop"
    );
    // A frozen server cannot prevent the separate local control loop from accepting stop.
    let mut frozen_handle = voice_pipewire::start(config.clone(), shared.clone())?;
    let frozen = FrozenDaemon::new()?;
    let started = Instant::now();
    frozen_handle.stop();
    let stop_elapsed = started.elapsed();
    drop(frozen);
    ensure!(
        stop_elapsed < Duration::from_secs(2),
        "stop stalled behind frozen server: {stop_elapsed:?}"
    );
    let frozen = FrozenDaemon::new()?;
    let started = Instant::now();
    let unavailable = voice_pipewire::list_devices();
    let timeout_elapsed = started.elapsed();
    drop(frozen);
    ensure!(
        unavailable.is_err() && timeout_elapsed < Duration::from_secs(5),
        "enumeration did not time out on frozen server: {timeout_elapsed:?}"
    );
    let mut stalled = voice_pipewire::start(config.clone(), shared.clone())?;
    let frozen = FrozenDaemon::new()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while shared.metrics.snapshot().active && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let detected_stall = !shared.metrics.snapshot().active;
    drop(frozen);
    stalled.stop();
    ensure!(
        detected_stall,
        "watchdog did not detect frozen audio scheduling"
    );
    let mut restarted = voice_pipewire::start(config, shared.clone())?;
    ensure!(
        Command::new("pw-cli")
            .args(["destroy", &input_id.to_string()])
            .status()?
            .success(),
        "could not remove isolated fixture source"
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    while shared.metrics.snapshot().active && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    ensure!(
        !shared.metrics.snapshot().active,
        "device removal did not invalidate backend"
    );
    restarted.stop();
    println!(
        "PASS: isolated capture -> DSP -> virtual source; stereo sink monitor/preview and virtual isolation; soundboard; recording; cleanup; frozen-server stop/timeout; restart/device-removal"
    );
    println!(
        "software impulse offset: {delay} samples ({:.3} ms at 48kHz); -6dB RMS ratio: {gain_ratio:.4}; monitor RMS ratio: {monitor_ratio:.4}; frames: {}",
        delay as f64 / 48.,
        metrics.frames
    );
    println!(
        "stop with frozen private server: {stop_elapsed:?}; enumeration timeout: {timeout_elapsed:?}"
    );
    println!("stereo sink peaks: voice={monitor_peaks:?}; preview={preview_peaks:?}");
    println!(
        "effect-order RMS ratios: gain-first={:.4}; distortion-first={:.4}",
        order_gains[0], order_gains[1]
    );
    Ok(())
}

fn sine(n: usize, hz: f32, amplitude: f32) -> Vec<f32> {
    (0..n)
        .map(|i| (i as f32 * hz * std::f32::consts::TAU / 48_000.).sin() * amplitude)
        .collect()
}
fn peaks(frames: &[[f32; 2]]) -> [f32; 2] {
    frames.iter().fold([0_f32; 2], |p, f| {
        [p[0].max(f[0].abs()), p[1].max(f[1].abs())]
    })
}
fn assert_dual_mono(name: &str, frames: &[[f32; 2]], minimum_peak: f32) -> Result<[f32; 2]> {
    ensure!(
        frames.iter().flatten().all(|sample| sample.is_finite()),
        "{name} produced non-finite stereo output"
    );
    let peaks = peaks(frames);
    ensure!(
        peaks.iter().all(|peak| *peak > minimum_peak),
        "{name} missing in a headphone channel: peaks={peaks:?}"
    );
    let difference = frames
        .iter()
        .fold(0_f32, |peak, frame| peak.max((frame[0] - frame[1]).abs()));
    ensure!(
        difference < 1e-6,
        "{name} differs between headphone channels: max difference={difference}"
    );
    Ok(peaks)
}
fn nodes() -> Result<String> {
    Ok(String::from_utf8(
        Command::new("pw-cli").args(["ls", "Node"]).output()?.stdout,
    )?)
}
fn port(direction: &str, prefix: &str, suffix: Option<&str>) -> Result<String> {
    for _ in 0..100 {
        let ports = Command::new("pw-link").arg(direction).output()?;
        if let Some(port) = String::from_utf8_lossy(&ports.stdout)
            .lines()
            .find(|p| p.starts_with(prefix) && suffix.is_none_or(|end| p.ends_with(end)))
        {
            return Ok(port.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("missing isolated port: {prefix} {suffix:?}")
}
fn link(source: &str, sink: &str) -> Result<()> {
    ensure!(
        Command::new("pw-link")
            .args([source, sink])
            .status()?
            .success(),
        "cannot link {source} -> {sink}"
    );
    Ok(())
}
fn record_stereo<F>(
    fixture: &Path,
    name: &str,
    left: &str,
    right: &str,
    frames: u32,
    trigger: F,
) -> Result<Vec<[f32; 2]>>
where
    F: FnOnce() -> Result<Option<ChildGuard>>,
{
    let mut recorder = Recorder::new(fixture, name, 2, frames)?;
    let prefix = format!("voice-test.{name}:");
    link(left, &port("-i", &prefix, Some("input_FL"))?)?;
    link(right, &port("-i", &prefix, Some("input_FR"))?)?;
    let _producer = trigger()?;
    let raw = recorder.finish()?;
    Ok(raw.as_chunks::<2>().0.to_vec())
}
fn play(fixture: &Path, name: &str, samples: &[f32], sink: &str) -> Result<Option<ChildGuard>> {
    let path = fixture.join(format!("{name}.raw"));
    let mut raw = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        raw.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(&path, raw)?;
    let properties = format!(
        "node.name=voice-test.{name} adapter.auto-port-config={{mode=dsp position=preserve}}"
    );
    let child = ChildGuard(
        support::raw_stdio_command("pw-play")?
            .args([
                "--format",
                "f32",
                "--channels",
                "1",
                "--rate",
                "48000",
                "--target",
                "0",
                "--latency",
                "256",
                "--properties",
                &properties,
            ])
            .arg("-")
            .stdin(File::open(&path)?)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    link(&port("-o", &format!("voice-test.{name}:"), None)?, sink)?;
    Ok(Some(child))
}
struct Recorder {
    child: ChildGuard,
    path: PathBuf,
    frames: u32,
    channels: u32,
}
impl Recorder {
    fn new(fixture: &Path, name: &str, channels: u32, frames: u32) -> Result<Self> {
        let path = fixture.join(format!("{name}.raw"));
        let properties = format!(
            "node.name=voice-test.{name} adapter.auto-port-config={{mode=dsp position=preserve}}"
        );
        let child = ChildGuard(
            support::raw_stdio_command("pw-record")?
                .args([
                    "--format",
                    "f32",
                    "--channels",
                    &channels.to_string(),
                    "--rate",
                    "48000",
                    "--target",
                    "0",
                    "--latency",
                    "256",
                    "--properties",
                    &properties,
                ])
                .arg("-")
                .stdout(File::create(&path)?)
                .stderr(Stdio::inherit())
                .spawn()?,
        );
        Ok(Self {
            child,
            path,
            frames,
            channels,
        })
    }
    fn finish(&mut self) -> Result<Vec<f32>> {
        let required_samples = self.frames as usize * self.channels as usize;
        let required_bytes = required_samples * 4;
        let deadline = Instant::now() + Duration::from_secs(4);
        while self.child.0.try_wait()?.is_none()
            && std::fs::metadata(&self.path)?.len() < required_bytes as u64
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.child.0.try_wait()?.is_none() {
            self.child.0.kill()?;
        }
        let status = self.child.0.wait()?;
        let bytes = std::fs::read(&self.path)?;
        ensure!(
            bytes.len() >= required_bytes,
            "short isolated recording: {} bytes, status {:?}",
            bytes.len(),
            status
        );
        ensure!(
            bytes.len() % (self.channels as usize * 4) == 0,
            "partial PCM frame in isolated recording: {} bytes",
            bytes.len()
        );
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .take(required_samples)
            .map(|b| f32::from_le_bytes(*b))
            .collect())
    }
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct FrozenDaemon(String);
impl FrozenDaemon {
    fn new() -> Result<Self> {
        let pid = std::env::var("VOICE_APP_TEST_DAEMON_PID")?;
        ensure!(
            Command::new("kill")
                .args(["-STOP", &pid])
                .status()?
                .success(),
            "could not suspend isolated daemon"
        );
        Ok(Self(pid))
    }
}
impl Drop for FrozenDaemon {
    fn drop(&mut self) {
        let _ = Command::new("kill").args(["-CONT", &self.0]).status();
    }
}
