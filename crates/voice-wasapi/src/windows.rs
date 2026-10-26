//! Enumeration, format checks, and stream lifetime run on the engine's control thread.
use crate::identity::{
    CableEndpoint, OwnedEndpoint, OwnedIdentity, adapter_interface_path, cable_targets,
    is_owned_adapter_path, is_vbcable_adapter, overlaps_physical_route, owned_identity,
    selected_status, vbcable_identity, verified_pair,
};
use crate::rt::{
    DesktopCapture, INPUT_QUEUE, InputCapture, LocalMonitor, MONITOR_QUEUE, VoiceOutput,
};
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
    VirtualMicrophoneStatus, VirtualMicrophoneTarget,
};

use windows::{
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_Get_DevNode_PropertyW, CM_Get_Device_Interface_PropertyW,
                CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
            },
            Properties::{
                DEVPKEY_Device_HardwareIds, DEVPKEY_Device_InstanceId, DEVPKEY_Device_Service,
                DEVPROP_TYPE_STRING, DEVPROP_TYPE_STRING_LIST, DEVPROPTYPE,
            },
        },
        Foundation::DEVPROPKEY,
        Media::Audio::IDeviceTopology,
        System::Com::{CLSCTX_ALL, CoTaskMemFree},
    },
    core::PCWSTR,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// Owns capture, local playback and an optional explicitly selected installed-driver route.
/// This user-mode adapter never creates or installs a virtual endpoint.
pub struct AudioHandle {
    input: Option<Stream>,
    output: Option<Stream>,
    virtual_output: Option<Stream>,
    running: Arc<AtomicBool>,
    shared: Arc<AudioShared>,
}

impl AudioHandle {
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        self.shared.metrics.active.store(false, Ordering::Release);
        self.shared
            .metrics
            .virtual_microphone_active
            .store(false, Ordering::Release);
        self.shared.metrics.output_peak.store(0, Ordering::Relaxed);
        self.shared
            .metrics
            .algorithmic_latency_frames
            .store(0, Ordering::Relaxed);
        // Joining backend threads and freeing prepared callback state happens here,
        // never inside a PCM callback. Stop capture before destroying playback.
        drop(self.input.take());
        drop(self.virtual_output.take());
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

/// The connected adapter topology is independent of the editable endpoint label.
/// COM/device queries and returned-string cleanup run only on the control thread.
fn connected_adapter_path(device: &Device) -> Result<String> {
    let device = match device.as_inner() {
        cpal::platform::DeviceInner::Wasapi(device) => device,
        #[allow(unreachable_patterns)]
        _ => bail!("endpoint does not belong to WASAPI"),
    };
    let device = device
        .immdevice()
        .context("Windows endpoint disconnected")?;
    // Microsoft documents endpoint topologies as one connector, connected to the
    // adapter topology. Its MMDevice ID can wrap the native interface path; the
    // inventory accepts only the exact observed form before querying native PnP.
    unsafe {
        let topology: IDeviceTopology = device
            .Activate(CLSCTX_ALL, None)
            .context("read endpoint adapter topology")?;
        if topology.GetConnectorCount()? != 1 {
            bail!("unexpected Windows endpoint topology");
        }
        let path = topology.GetConnector(0)?.GetDeviceIdConnectedTo()?;
        if path.0.is_null() {
            bail!("Windows endpoint has no connected adapter identity");
        }
        let result = path.to_string();
        CoTaskMemFree(Some(path.0.cast()));
        result.context("invalid connected Windows adapter identity")
    }
}

// These bounded property buffers are temporary control-thread stack storage;
// they are never captured by an audio callback or allocated per PCM block.
const PNP_PROPERTY_WORDS: usize = 4096;

struct NativeAdapter {
    instance: String,
    hardware_ids: Vec<String>,
    service: String,
}

fn property_words(words: &[u16], bytes: u32) -> Result<&[u16]> {
    let bytes = usize::try_from(bytes)?;
    if bytes == 0 || !bytes.is_multiple_of(2) || bytes > std::mem::size_of_val(words) {
        bail!("Windows adapter property has an invalid bounded UTF-16 size");
    }
    Ok(&words[..bytes / 2])
}

fn property_string(words: &[u16]) -> Result<String> {
    let Some((&0, value)) = words.split_last() else {
        bail!("Windows adapter property is not terminated");
    };
    if value.is_empty() || value.contains(&0) {
        bail!("Windows adapter property contains an empty or embedded string");
    }
    String::from_utf16(value).context("Windows adapter property is invalid UTF-16")
}

fn devnode_property(devnode: u32, key: &DEVPROPKEY, expected: DEVPROPTYPE) -> Result<Vec<u16>> {
    let mut words = [0_u16; PNP_PROPERTY_WORDS];
    let mut bytes = u32::try_from(std::mem::size_of_val(&words))?;
    let mut property_type = DEVPROPTYPE::default();
    // ConfigManager reads only configured native devices and properties. No
    // registry mutation, SetupAPI installer or system-default change is involved.
    let result = unsafe {
        CM_Get_DevNode_PropertyW(
            devnode,
            key,
            &mut property_type,
            Some(words.as_mut_ptr().cast()),
            &mut bytes,
            0,
        )
    };
    if result != CR_SUCCESS || property_type != expected {
        bail!("Windows adapter PnP property is unavailable or has an unexpected type");
    }
    Ok(property_words(&words, bytes)?.to_vec())
}

fn native_adapter(adapter_path: &str) -> Result<NativeAdapter> {
    let adapter_path = adapter_interface_path(adapter_path)
        .context("Windows connected adapter interface is invalid or unsupported")?;
    let path: Vec<u16> = adapter_path.encode_utf16().chain(Some(0)).collect();
    let mut words = [0_u16; PNP_PROPERTY_WORDS];
    let mut bytes = u32::try_from(std::mem::size_of_val(&words))?;
    let mut property_type = DEVPROPTYPE::default();
    // Interface paths can contain ROOT#MEDIA; never infer the installed driver's
    // hardware ID or pair identity from that generic path or an endpoint label.
    let result = unsafe {
        CM_Get_Device_Interface_PropertyW(
            PCWSTR(path.as_ptr()),
            &DEVPKEY_Device_InstanceId,
            &mut property_type,
            Some(words.as_mut_ptr().cast()),
            &mut bytes,
            0,
        )
    };
    if result != CR_SUCCESS || property_type != DEVPROP_TYPE_STRING {
        bail!("Windows connected adapter PnP instance is unavailable");
    }
    let instance = property_string(property_words(&words, bytes)?)?;
    let instance_wide: Vec<u16> = instance.encode_utf16().chain(Some(0)).collect();
    let mut devnode = 0;
    let result = unsafe {
        CM_Locate_DevNodeW(
            &mut devnode,
            PCWSTR(instance_wide.as_ptr()),
            CM_LOCATE_DEVNODE_NORMAL,
        )
    };
    if result != CR_SUCCESS {
        bail!("Windows connected adapter disappeared");
    }
    let ids = devnode_property(
        devnode,
        &DEVPKEY_Device_HardwareIds,
        DEVPROP_TYPE_STRING_LIST,
    )?;
    if !ids.ends_with(&[0, 0]) {
        bail!("Windows adapter hardware IDs are not a terminated string list");
    }
    let hardware_ids = ids[..ids.len() - 1]
        .split_inclusive(|word| *word == 0)
        .map(property_string)
        .collect::<Result<Vec<_>>>()?;
    let service = property_string(&devnode_property(
        devnode,
        &DEVPKEY_Device_Service,
        DEVPROP_TYPE_STRING,
    )?)?;
    Ok(NativeAdapter {
        instance,
        hardware_ids,
        service,
    })
}

struct Inventory {
    physical: Vec<DeviceInfo>,
    owned: Vec<OwnedEndpoint>,
    cables: Vec<CableEndpoint>,
}

fn inventory(host: &Host) -> Result<Inventory> {
    let mut inventory = Inventory {
        physical: Vec::new(),
        owned: Vec::new(),
        cables: Vec::new(),
    };
    fn append(
        endpoints: impl Iterator<Item = Device>,
        kind: DeviceKind,
        inventory: &mut Inventory,
    ) {
        for device in endpoints {
            let Ok(id) = device.id() else { continue };
            let name = id.to_string();
            // Missing native identity fails closed before offering physical/default
            // routes: a renamed virtual capture must never leak into the voice path.
            let Ok(adapter_id) = connected_adapter_path(&device) else {
                continue;
            };
            let Some(adapter) = adapter_interface_path(&adapter_id) else {
                continue;
            };
            if is_owned_adapter_path(adapter) {
                let identity = owned_identity(adapter).unwrap_or(OwnedIdentity {
                    instance: String::new(),
                    role: None,
                });
                inventory.owned.push(OwnedEndpoint {
                    id: name,
                    identity,
                    kind: kind.clone(),
                });
                continue;
            }
            let Ok(native) = native_adapter(adapter) else {
                continue;
            };
            if is_vbcable_adapter(&native.hardware_ids, &native.service) {
                // Quarantine every recognized base VB-CABLE endpoint, including
                // legacy/unknown references, malformed metadata and 16-channel
                // outputs. Only a complete profile may become a publishable target.
                let identity = vbcable_identity(
                    adapter,
                    &native.instance,
                    &native.hardware_ids,
                    &native.service,
                )
                .unwrap_or(OwnedIdentity {
                    instance: String::new(),
                    role: None,
                });
                let description = device
                    .description()
                    .ok()
                    .map(|description| description.name().to_owned())
                    .filter(|description| !description.trim().is_empty())
                    .unwrap_or_else(|| name.clone());
                let supported_format =
                    virtual_native_config(&device, kind == DeviceKind::Input, 256).is_ok();
                inventory.cables.push(CableEndpoint {
                    id: name,
                    description,
                    identity,
                    kind: kind.clone(),
                    supported_format,
                });
                continue;
            }
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
            inventory.physical.push(DeviceInfo {
                id: (inventory.physical.len() + 1) as u32,
                name,
                description,
                kind: kind.clone(),
            });
        }
    }
    append(host.input_devices()?, DeviceKind::Input, &mut inventory);
    append(host.output_devices()?, DeviceKind::Output, &mut inventory);
    inventory
        .physical
        .sort_by(|a, b| a.description.cmp(&b.description));
    Ok(inventory)
}

/// Stable endpoint IDs are names; recognized virtual adapters never appear as
/// physical microphone/headphones/default/desktop-loopback routes.
pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    Ok(inventory(&host()?)?.physical)
}

fn eligible(name: &str, description: &str) -> bool {
    !name.starts_with("voice-app.")
        && !description.starts_with("voice-app.")
        && description != "Voice App Virtual Microphone"
        && description != "Voice App Virtual Input"
}

fn pair_status(
    host: &Host,
    inventory: &Inventory,
    selected: Option<&str>,
) -> VirtualMicrophoneStatus {
    let mut targets = cable_targets(&inventory.cables);
    // Preserve compatibility with a separately installed old first-party pair.
    // Its driver development/signing is not part of the external cable route.
    if let Some((render, capture)) = verified_pair(&inventory.owned) {
        let pair = endpoint(host, &render.id)
            .and_then(|device| virtual_native_config(&device, false, 256))
            .and_then(|_| endpoint(host, &capture.id))
            .and_then(|device| virtual_native_config(&device, true, 256));
        if pair.is_ok() {
            targets.push(VirtualMicrophoneTarget {
                provider: "voice_app".into(),
                render_device: render.id.clone(),
                capture_device: capture.id.clone(),
                render_description: "Voice App Virtual Input".into(),
                capture_description: "Voice App Virtual Microphone".into(),
            });
        }
    }
    selected_status(targets, selected)
}

/// Discover installed cable targets without opening streams or enabling a route.
/// A saved ID is never replaced; multiple targets require an explicit choice.
pub fn virtual_microphone_status_for(selected: Option<&str>) -> Result<VirtualMicrophoneStatus> {
    let host = host()?;
    Ok(pair_status(&host, &inventory(&host)?, selected))
}

pub fn virtual_microphone_status() -> Result<VirtualMicrophoneStatus> {
    virtual_microphone_status_for(None)
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
    let inventory = inventory(&host)?;
    let devices = &inventory.physical;
    if let Some(requested) = config.virtual_output_device.as_deref() {
        let pair = pair_status(&host, &inventory, Some(requested));
        if !pair.available || pair.render_device.as_deref() != Some(requested) {
            bail!(
                "selected virtual output is unavailable or unsupported; select an exact detected VB-CABLE target, with both endpoints set to 48000 Hz stereo. No route was substituted"
            );
        }
    }
    let mut config = config.clone();
    config.input_device = Some(select_device(
        &host,
        devices,
        config.input_device.as_deref(),
        DeviceKind::Input,
    )?);
    config.output_device = Some(select_device(
        &host,
        devices,
        config.output_device.as_deref(),
        DeviceKind::Output,
    )?);
    if let Some(requested) = config.virtual_output_device.as_deref() {
        let target = pair_status(&host, &inventory, Some(requested))
            .targets
            .into_iter()
            .find(|target| target.render_device == requested)
            .context("selected virtual target disappeared")?;
        if overlaps_physical_route(
            &target,
            config
                .input_device
                .as_deref()
                .context("resolved input missing")?,
            config
                .output_device
                .as_deref()
                .context("resolved headphones missing")?,
        ) {
            bail!("virtual cable endpoints cannot also be the physical microphone or headphones");
        }
    }
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

fn virtual_native_config(device: &Device, input: bool, quantum: u32) -> Result<StreamConfig> {
    let config = native_config(device, input, quantum)?;
    if config.channels != 2 {
        bail!("Virtual cable endpoints require a native 48000 Hz stereo F32 shared mix");
    }
    Ok(config)
}

fn fail_audio(running: &AtomicBool, shared: &AudioShared) {
    running.store(false, Ordering::Release);
    shared.metrics.active.store(false, Ordering::Release);
    shared
        .metrics
        .virtual_microphone_active
        .store(false, Ordering::Release);
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

fn handle_virtual_error(kind: ErrorKind, running: &AtomicBool, shared: &AudioShared) {
    if kind == ErrorKind::Xrun {
        shared
            .metrics
            .virtual_microphone_overruns
            .fetch_add(1, Ordering::Relaxed);
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
/// The explicitly selected virtual render endpoint clocks the final mix.
/// Monitoring and previews stay on a separate prepared queue and physical output.
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
    let virtual_device = config
        .virtual_output_device
        .as_deref()
        .map(|id| endpoint(&host, id))
        .transpose()?;
    let virtual_config = virtual_device
        .as_ref()
        .map(|device| virtual_native_config(device, false, config.quantum))
        .transpose()?;
    let channels = usize::from(input_config.channels);
    shared.metrics.active.store(false, Ordering::Release);
    shared
        .metrics
        .virtual_microphone_active
        .store(false, Ordering::Release);
    shared.soundboard.discard();
    shared.preview.discard();
    let queue = Arc::new(SampleQueue::new(INPUT_QUEUE));
    let running = Arc::new(AtomicBool::new(true));
    let mut capture = InputCapture::new(shared.clone(), queue.clone());
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
    let (output, virtual_output) = if let Some(virtual_device) = virtual_device {
        let monitor_queue = Arc::new(SampleQueue::new(MONITOR_QUEUE));
        let mut processor =
            VoiceOutput::publishing(shared.clone(), queue, monitor_queue.clone(), SAMPLE_RATE);
        let mut monitor = LocalMonitor::new(shared.clone(), monitor_queue);
        let virtual_gate = running.clone();
        let virtual_error_gate = running.clone();
        let virtual_error_shared = shared.clone();
        let virtual_output = virtual_device
            .build_output_stream::<f32, _, _>(
                virtual_config.context("virtual output format missing")?,
                move |samples, _| processor.render_if_running(samples, &virtual_gate),
                move |error| {
                    handle_virtual_error(error.kind(), &virtual_error_gate, &virtual_error_shared)
                },
                Some(CONNECT_TIMEOUT),
            )
            .context("open explicit external virtual output")?;
        let output = output
            .build_output_stream::<f32, _, _>(
                output_config,
                move |samples, _| monitor.render_if_running(samples, &output_gate),
                move |error| {
                    handle_audio_error(error.kind(), true, &output_error_gate, &output_error_shared)
                },
                Some(CONNECT_TIMEOUT),
            )
            .context("open native 48 kHz stereo local monitor")?;
        (output, Some(virtual_output))
    } else {
        let mut processor = VoiceOutput::new(shared.clone(), queue, SAMPLE_RATE);
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
        (output, None)
    };
    let handle = AudioHandle {
        input: Some(input),
        output: Some(output),
        virtual_output,
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
    if let Some(virtual_output) = &handle.virtual_output {
        virtual_output.play()?;
    }
    if !handle.running.load(Ordering::Acquire) {
        bail!("Windows audio failed during startup; output remains silent");
    }
    handle.shared.metrics.active.store(true, Ordering::Release);
    handle
        .shared
        .metrics
        .virtual_microphone_active
        .store(handle.virtual_output.is_some(), Ordering::Release);
    // Never overwrite an error concurrently delivered between the readiness checks.
    if !handle.running.load(Ordering::Acquire) {
        fail_audio(&handle.running, &handle.shared);
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
    fn bounded_native_pnp_properties_reject_bad_sizes_terminators_and_utf16() {
        assert_eq!(property_words(&[65, 0], 4).unwrap(), &[65, 0]);
        for bytes in [0, 1, 3, 6] {
            assert!(property_words(&[65, 0], bytes).is_err());
        }
        assert_eq!(property_string(&[65, 0]).unwrap(), "A");
        for words in [
            &[][..],
            &[0][..],
            &[65][..],
            &[65, 0, 66, 0][..],
            &[0xd800, 0][..],
        ] {
            assert!(property_string(words).is_err());
        }
    }

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

    #[test]
    fn virtual_xrun_has_its_own_counter_and_fatal_failure_stops_both_outputs() {
        let running = AtomicBool::new(true);
        let shared = AudioShared::default();
        shared.metrics.active.store(true, Ordering::Release);
        shared
            .metrics
            .virtual_microphone_active
            .store(true, Ordering::Release);
        handle_virtual_error(ErrorKind::Xrun, &running, &shared);
        assert!(running.load(Ordering::Acquire));
        assert!(
            shared
                .metrics
                .virtual_microphone_active
                .load(Ordering::Acquire)
        );
        assert_eq!(
            shared
                .metrics
                .virtual_microphone_overruns
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(shared.metrics.monitor_overruns.load(Ordering::Relaxed), 0);
        handle_virtual_error(ErrorKind::DeviceNotAvailable, &running, &shared);
        assert!(!running.load(Ordering::Acquire));
        assert!(!shared.metrics.active.load(Ordering::Acquire));
        assert!(
            !shared
                .metrics
                .virtual_microphone_active
                .load(Ordering::Acquire)
        );
    }
}
