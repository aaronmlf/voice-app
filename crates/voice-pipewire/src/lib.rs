//! Linux routing adapter. Only the process closures touch PCM on the realtime thread.
use anyhow::{Context, Result, anyhow, bail};
use pipewire as pw;
use pw::{properties::properties, spa};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, atomic::Ordering, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use voice_core::{AudioConfig, AudioShared, DeviceInfo, DeviceKind, SampleQueue};

mod desktop;
pub use desktop::{DesktopCaptureHandle, start_desktop};

const MAX_BLOCK: usize = 8192;
const QUEUE_SIZE: usize = 8192;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const MONO_POSITION: [u32; 1] = [spa::sys::SPA_AUDIO_CHANNEL_MONO];
const MONITOR_POSITION: [u32; 2] = [
    spa::sys::SPA_AUDIO_CHANNEL_FL,
    spa::sys::SPA_AUDIO_CHANNEL_FR,
];
const MONITOR_STRIDE: usize = MONITOR_POSITION.len() * size_of::<f32>();

/// Owns the PipeWire thread and its temporary nodes. Dropping it stops audio and removes them.
pub struct AudioHandle {
    stop: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<AudioShared>,
}
impl AudioHandle {
    pub fn stop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.shared.metrics.active.store(false, Ordering::Release);
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

/// Enumerate usable source/sink nodes. Application-owned nodes are intentionally omitted.
/// Enumeration is bounded, including when a server accepts a socket but never responds.
pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    Ok(device_snapshot()?.devices)
}

/// Resolve automatic routes against the desktop defaults without changing those defaults.
/// An explicitly selected device must still exist; it is never silently replaced.
pub fn resolve_config(config: &AudioConfig) -> Result<AudioConfig> {
    config.validate().map_err(anyhow::Error::msg)?;
    let snapshot = device_snapshot()?;
    let mut resolved = config.clone();
    resolved.input_device = Some(select_device(
        &snapshot.devices,
        config.input_device.as_deref(),
        snapshot.default_input.as_deref(),
        DeviceKind::Input,
    )?);
    resolved.output_device = Some(select_device(
        &snapshot.devices,
        config.output_device.as_deref(),
        snapshot.default_output.as_deref(),
        DeviceKind::Output,
    )?);
    Ok(resolved)
}

#[derive(Default)]
struct DeviceSnapshot {
    devices: Vec<DeviceInfo>,
    default_input: Option<String>,
    default_output: Option<String>,
}

struct BoundMetadata {
    // Remove the hook before destroying the proxy to which it was attached.
    _listener: pw::metadata::MetadataListener,
    _metadata: pw::metadata::Metadata,
}

fn metadata_device_name(value: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(value?).ok()?;
    value
        .get("name")?
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn device_snapshot() -> Result<DeviceSnapshot> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context
        .connect_rc(None)
        .context("connect to the user's PipeWire server")?;
    let registry = core.get_registry_rc()?;
    let snapshot = Rc::new(RefCell::new(DeviceSnapshot::default()));
    let found = snapshot.clone();
    let bound_metadata = Rc::new(RefCell::new(Vec::<BoundMetadata>::new()));
    let retained = bound_metadata.clone();
    let bind_registry = registry.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            let Some(props) = global.props.as_ref() else {
                return;
            };
            if global.type_ == pw::types::ObjectType::Metadata {
                if props.get("metadata.name") == Some("default")
                    && let Ok(metadata) = bind_registry.bind::<pw::metadata::Metadata, _>(global)
                {
                    let defaults = found.clone();
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _, value| {
                            if subject != pw::core::PW_ID_CORE {
                                return 0;
                            }
                            let mut snapshot = defaults.borrow_mut();
                            match key {
                                Some("default.audio.source") => {
                                    snapshot.default_input = metadata_device_name(value);
                                }
                                Some("default.audio.sink") => {
                                    snapshot.default_output = metadata_device_name(value);
                                }
                                None => {
                                    snapshot.default_input = None;
                                    snapshot.default_output = None;
                                }
                                _ => {}
                            }
                            0
                        })
                        .register();
                    retained.borrow_mut().push(BoundMetadata {
                        _listener: listener,
                        _metadata: metadata,
                    });
                }
                return;
            }
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(name) = props.get("node.name") else {
                return;
            };
            let kind = match props.get("media.class") {
                Some("Audio/Source" | "Audio/Source/Virtual") => DeviceKind::Input,
                Some("Audio/Sink") => DeviceKind::Output,
                _ => return,
            };
            if !eligible_device(name, &kind)
                || (kind == DeviceKind::Input
                    && (props.get("device.class") == Some("monitor")
                        || props.get("stream.monitor") == Some("true")))
            {
                return;
            }
            found.borrow_mut().devices.push(DeviceInfo {
                id: global.id,
                name: name.into(),
                description: props.get("node.description").unwrap_or(name).into(),
                kind,
            });
        })
        .global_remove({
            let snapshot = snapshot.clone();
            move |id| {
                snapshot
                    .borrow_mut()
                    .devices
                    .retain(|device| device.id != id)
            }
        })
        .register();
    let finished = Rc::new(Cell::new(false));
    let error = Rc::new(RefCell::new(None::<String>));
    let pending = Rc::new(Cell::new(core.sync(0)?));
    let pending_done = pending.clone();
    let done = finished.clone();
    let quit = mainloop.clone();
    let error_slot = error.clone();
    let error_quit = mainloop.clone();
    let _core = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending_done.get() {
                done.set(true);
                quit.quit();
            }
        })
        .error(move |id, _, _, message| {
            if id == pw::core::PW_ID_CORE {
                *error_slot.borrow_mut() = Some(message.into());
                error_quit.quit();
            }
        })
        .register();
    let quit = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| quit.quit());
    timer
        .update_timer(Some(CONNECT_TIMEOUT), None)
        .into_result()?;
    mainloop.run();
    if let Some(error) = error.borrow_mut().take() {
        bail!("PipeWire: {error}");
    }
    if !finished.get() {
        bail!("PipeWire device enumeration timed out");
    }
    if !bound_metadata.borrow().is_empty() {
        // Bind requests were issued while handling the first registry burst. A second
        // roundtrip ensures their initial property events arrive before reading defaults.
        // Keep the original timer: both phases share one bounded connection deadline.
        finished.set(false);
        pending.set(core.sync(1)?);
        mainloop.run();
        if let Some(error) = error.borrow_mut().take() {
            bail!("PipeWire: {error}");
        }
        if !finished.get() {
            bail!("PipeWire device metadata timed out");
        }
    }
    let mut result = std::mem::take(&mut *snapshot.borrow_mut());
    result.devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

/// Starts capture, processed virtual microphone and a distinct local monitoring stream.
/// Automatic routes prefer desktop defaults and are resolved to explicit node names before
/// creating streams, so the virtual source can never redirect capture into this application.
pub fn start(config: AudioConfig, shared: Arc<AudioShared>) -> Result<AudioHandle> {
    let config = resolve_config(&config)?;
    let input = config
        .input_device
        .clone()
        .context("resolved input missing")?;
    let output = config
        .output_device
        .clone()
        .context("resolved output missing")?;
    // Previous streams have stopped before the engine creates a replacement handle. These
    // queues have this backend as their sole consumer; do not replay their stale contents.
    shared.soundboard.discard();
    shared.preview.discard();
    let (stop, receiver) = pw::channel::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let state = shared.clone();
    let thread = thread::Builder::new()
        .name("voice-pipewire".into())
        .spawn(move || {
            let result = run(
                config,
                state.clone(),
                input,
                output,
                receiver,
                ready_tx.clone(),
            );
            if let Err(error) = result {
                let _ = ready_tx.try_send(Err(error.to_string()));
            }
            state.metrics.active.store(false, Ordering::Release);
            state
                .metrics
                .algorithmic_latency_frames
                .store(0, Ordering::Relaxed);
        })?;
    match ready_rx.recv_timeout(CONNECT_TIMEOUT + Duration::from_secs(1)) {
        Ok(Ok(())) => Ok(AudioHandle {
            stop,
            thread: Some(thread),
            shared,
        }),
        result => {
            let _ = stop.send(());
            // All main-loop waits have a timer; joining cannot wait on an audio-device read.
            let _ = thread.join();
            match result {
                Ok(Err(message)) => Err(anyhow!(message)),
                Err(error) => Err(anyhow!("PipeWire initialization failed: {error}")),
                Ok(Ok(())) => unreachable!(),
            }
        }
    }
}

fn select_device(
    devices: &[DeviceInfo],
    requested: Option<&str>,
    default: Option<&str>,
    kind: DeviceKind,
) -> Result<String> {
    let candidates = || {
        devices
            .iter()
            .filter(|device| device.kind == kind && eligible_device(&device.name, &device.kind))
    };
    let selected = if let Some(requested) = requested {
        let id = requested.parse::<u32>().ok();
        candidates().find(|device| device.name == requested || id == Some(device.id))
    } else {
        default
            .and_then(|name| candidates().find(|device| device.name == name))
            .or_else(|| candidates().min_by(|a, b| a.name.cmp(&b.name)))
    };
    selected.map(|device| device.name.clone()).ok_or_else(|| {
        anyhow!(
            "no matching {kind:?} PipeWire device{}",
            requested.map(|v| format!(": {v}")).unwrap_or_default()
        )
    })
}

fn eligible_device(name: &str, kind: &DeviceKind) -> bool {
    !name.starts_with("voice-app.") && !(*kind == DeviceKind::Input && name.ends_with(".monitor"))
}

type Ready = Rc<RefCell<Option<mpsc::SyncSender<std::result::Result<(), String>>>>>;

fn stream_state(
    bit: u8,
    bits: &Cell<u8>,
    ready: &Ready,
    shared: &AudioShared,
    mainloop: &pw::main_loop::MainLoopRc,
    state: pw::stream::StreamState,
) {
    match state {
        pw::stream::StreamState::Paused | pw::stream::StreamState::Streaming => {
            bits.set(bits.get() | bit);
            if bits.get() == 7 {
                shared.metrics.active.store(true, Ordering::Release);
                if let Some(sender) = ready.borrow_mut().take() {
                    let _ = sender.try_send(Ok(()));
                }
            }
        }
        pw::stream::StreamState::Error(error) => {
            shared.metrics.active.store(false, Ordering::Release);
            if let Some(sender) = ready.borrow_mut().take() {
                let _ = sender.try_send(Err(error));
            }
            mainloop.quit();
        }
        pw::stream::StreamState::Unconnected if bits.get() == 7 => {
            shared.metrics.active.store(false, Ordering::Release);
            mainloop.quit();
        }
        _ => {}
    }
}

fn format_changed(
    id: u32,
    param: Option<&spa::pod::Pod>,
    channels: u32,
    ready: &Ready,
    shared: &AudioShared,
    mainloop: &pw::main_loop::MainLoopRc,
) {
    if id != spa::param::ParamType::Format.as_raw() {
        return;
    }
    let Some(param) = param else {
        return;
    };
    let mut format = spa::param::audio::AudioInfoRaw::new();
    if format.parse(param).is_err()
        || format.format() != spa::param::audio::AudioFormat::F32LE
        || format.rate() != voice_core::SAMPLE_RATE
        || format.channels() != channels
    {
        shared.metrics.active.store(false, Ordering::Release);
        if let Some(sender) = ready.borrow_mut().take() {
            let _ = sender.try_send(Err(format!(
                "PipeWire negotiated an unsupported PCM format; expected f32/48000Hz with {channels} channel(s)"
            )));
        }
        mainloop.quit();
    }
}

fn run(
    config: AudioConfig,
    shared: Arc<AudioShared>,
    input_name: String,
    output_name: String,
    stop: pw::channel::Receiver<()>,
    ready_tx: mpsc::SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let quit = mainloop.clone();
    let _stop = stop.attach(mainloop.loop_(), move |_| quit.quit());
    let ready: Ready = Rc::new(RefCell::new(Some(ready_tx)));
    let bits = Rc::new(Cell::new(0));
    let quit = mainloop.clone();
    let ready_error = ready.clone();
    let failed_state = shared.clone();
    let _core = core
        .add_listener_local()
        .error(move |id, _, _, message| {
            if id == pw::core::PW_ID_CORE {
                failed_state.metrics.active.store(false, Ordering::Release);
                if let Some(sender) = ready_error.borrow_mut().take() {
                    let _ = sender.try_send(Err(message.into()));
                }
                quit.quit();
            }
        })
        .register();
    let registry = core.get_registry()?;
    let selected_ids = Rc::new(RefCell::new([None::<u32>; 2]));
    let found = selected_ids.clone();
    let capture_target = input_name.clone();
    let monitor_target = output_name.clone();
    let removed = selected_ids.clone();
    let gone = shared.clone();
    let quit = mainloop.clone();
    let _devices = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(props) = global.props.as_ref() else {
                return;
            };
            if props.get("node.name") == Some(capture_target.as_str()) {
                found.borrow_mut()[0] = Some(global.id);
            }
            if props.get("node.name") == Some(monitor_target.as_str()) {
                found.borrow_mut()[1] = Some(global.id);
            }
        })
        .global_remove(move |id| {
            if removed.borrow().contains(&Some(id)) {
                gone.metrics.active.store(false, Ordering::Release);
                quit.quit();
            }
        })
        .register();
    let quit = mainloop.clone();
    let ready_timeout = ready.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        if let Some(sender) = ready_timeout.borrow_mut().take() {
            let _ = sender.try_send(Err(
                "PipeWire streams did not become ready before timeout".into()
            ));
            quit.quit();
        }
    });
    timer
        .update_timer(Some(CONNECT_TIMEOUT), None)
        .into_result()?;
    // This timer runs on the control loop even if the server stops scheduling RT buffers.
    // The always-processing virtual source must continue to advance while audio is active.
    let watched = shared.clone();
    let quit = mainloop.clone();
    let last_frames = Cell::new(shared.metrics.frames.load(Ordering::Relaxed));
    let last_progress = Cell::new(Instant::now());
    let watchdog = mainloop.loop_().add_timer(move |_| {
        let frames = watched.metrics.frames.load(Ordering::Relaxed);
        if frames != last_frames.get() {
            last_frames.set(frames);
            last_progress.set(Instant::now());
        } else if watched.metrics.active.load(Ordering::Acquire)
            && last_progress.get().elapsed() > Duration::from_secs(2)
        {
            watched.metrics.active.store(false, Ordering::Release);
            quit.quit();
        }
    });
    watchdog
        .update_timer(
            Some(Duration::from_millis(250)),
            Some(Duration::from_millis(250)),
        )
        .into_result()?;

    let latency = format!("{}/{}", config.quantum, config.sample_rate);
    let capture = pw::stream::StreamBox::new(
        &core,
        "voice-app capture",
        properties! {
            "media.type" => "Audio", "media.category" => "Capture", "media.role" => "Communication",
            "node.name" => "voice-app.capture", "target.object" => input_name,
            "adapter.auto-port-config" => "{ mode = dsp position = preserve }",
            "node.dont-fallback" => "true", "node.link-group" => "voice-app.internal",
            "node.group" => "voice-app.audio",
            "node.latency" => latency.as_str(), "stream.dont-remix" => "false",
        },
    )?;
    let source = pw::stream::StreamBox::new(
        &core,
        "voice-app virtual microphone",
        properties! {
            "media.type" => "Audio", "media.category" => "Playback", "media.role" => "Communication",
            "media.class" => "Audio/Source", "node.name" => voice_core::VIRTUAL_NODE_NAME,
            "adapter.auto-port-config" => "{ mode = dsp position = preserve }",
            "node.description" => "Voice App Virtual Microphone", "node.virtual" => "true",
            "node.link-group" => "voice-app.internal", "node.always-process" => "true",
            "node.group" => "voice-app.audio",
            "node.latency" => latency.as_str(), "priority.session" => "0", "object.linger" => "false",
        },
    )?;
    let monitor = pw::stream::StreamBox::new(
        &core,
        "voice-app monitor",
        properties! {
            "media.type" => "Audio", "media.category" => "Playback", "media.role" => "Music",
            "node.name" => "voice-app.monitor", "target.object" => output_name,
            "adapter.auto-port-config" => "{ mode = dsp position = preserve }",
            "node.dont-fallback" => "true", "node.link-group" => "voice-app.internal",
            "node.group" => "voice-app.audio",
            "node.latency" => latency.as_str(),
        },
    )?;

    let mic = Arc::new(SampleQueue::new(QUEUE_SIZE));
    let listening = Arc::new(SampleQueue::new(QUEUE_SIZE));
    let input = Capture {
        shared: shared.clone(),
        queue: mic.clone(),
        scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
    };
    let b = bits.clone();
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let fr = ready.clone();
    let fs = shared.clone();
    let fl = mainloop.clone();
    let _capture = capture
        .add_local_listener_with_user_data(input)
        .state_changed(move |_, _, _, state| stream_state(1, &b, &r, &s, &l, state))
        .param_changed(move |_, _, id, param| format_changed(id, param, 1, &fr, &fs, &fl))
        .process(capture_process)
        .register()?;
    let processor =
        SourceProcessor::new(config.sample_rate, shared.clone(), mic, listening.clone());
    let b = bits.clone();
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let fr = ready.clone();
    let fs = shared.clone();
    let fl = mainloop.clone();
    let quantum = config.quantum as usize;
    let _source = source
        .add_local_listener_with_user_data(processor)
        .state_changed(move |_, _, _, state| stream_state(2, &b, &r, &s, &l, state))
        .param_changed(move |_, _, id, param| format_changed(id, param, 1, &fr, &fs, &fl))
        .process(move |stream, processor| output_process(stream, processor, quantum))
        .register()?;
    let b = bits.clone();
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let fr = ready.clone();
    let fs = shared.clone();
    let fl = mainloop.clone();
    let _monitor = monitor
        .add_local_listener_with_user_data(Monitor {
            shared: shared.clone(),
            queue: listening,
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
        })
        .state_changed(move |_, _, _, state| stream_state(4, &b, &r, &s, &l, state))
        .param_changed(move |_, _, id, param| {
            format_changed(id, param, MONITOR_POSITION.len() as u32, &fr, &fs, &fl)
        })
        .process(move |stream, monitor| monitor_process(stream, monitor, quantum))
        .register()?;

    let format = raw_format(config.sample_rate, &MONO_POSITION)?;
    let monitor_format = raw_format(config.sample_rate, &MONITOR_POSITION)?;
    let flags = pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS;
    capture.connect(
        spa::utils::Direction::Input,
        None,
        flags | pw::stream::StreamFlags::AUTOCONNECT,
        &mut [spa::pod::Pod::from_bytes(&format).context("invalid format pod")?],
    )?;
    source.connect(
        spa::utils::Direction::Output,
        None,
        flags,
        &mut [spa::pod::Pod::from_bytes(&format).context("invalid format pod")?],
    )?;
    monitor.connect(
        spa::utils::Direction::Output,
        None,
        flags | pw::stream::StreamFlags::AUTOCONNECT,
        &mut [spa::pod::Pod::from_bytes(&monitor_format).context("invalid monitor format pod")?],
    )?;
    mainloop.run();
    // Quiesce realtime processing before listeners release their prepared buffers on this thread.
    let _ = capture.disconnect();
    let _ = source.disconnect();
    let _ = monitor.disconnect();
    Ok(())
}

fn raw_format(rate: u32, positions: &[u32]) -> Result<Vec<u8>> {
    let mut audio = spa::param::audio::AudioInfoRaw::new();
    audio.set_format(spa::param::audio::AudioFormat::F32LE);
    audio.set_rate(rate);
    audio.set_channels(positions.len() as u32);
    let mut layout = [0; spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    layout[..positions.len()].copy_from_slice(positions);
    audio.set_position(layout);
    let pod = spa::pod::Value::Object(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio.into(),
    });
    Ok(
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &pod)
            .map_err(|e| anyhow!("serialize audio format: {e:?}"))?
            .0
            .into_inner(),
    )
}

struct Capture {
    shared: Arc<AudioShared>,
    queue: Arc<SampleQueue>,
    scratch: Box<[f32]>,
}
fn capture_process(stream: &pw::stream::Stream, state: &mut Capture) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let offset = data.chunk().offset() as usize;
    let size = data.chunk().size() as usize;
    let Some(bytes) = data.data() else {
        return;
    };
    let end = offset.saturating_add(size).min(bytes.len());
    if offset > end {
        return;
    }
    for block in bytes[offset..end].chunks(MAX_BLOCK * 4) {
        let n = block.len() / 4;
        for (out, bytes) in state.scratch[..n].iter_mut().zip(block.as_chunks::<4>().0) {
            *out = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let written = state.queue.push(&state.scratch[..n]);
        if written < n {
            state
                .shared
                .metrics
                .input_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct SourceProcessor {
    shared: Arc<AudioShared>,
    input: Arc<SampleQueue>,
    monitor: Arc<SampleQueue>,
    dsp: voice_dsp::Processor,
    output: Box<[f32]>,
    scratch: Box<[f32]>,
    listening: Box<[f32]>,
}
impl SourceProcessor {
    fn new(
        rate: u32,
        shared: Arc<AudioShared>,
        input: Arc<SampleQueue>,
        monitor: Arc<SampleQueue>,
    ) -> Self {
        Self {
            shared,
            input,
            monitor,
            dsp: voice_dsp::Processor::new(rate),
            output: vec![0.; MAX_BLOCK].into_boxed_slice(),
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
            listening: vec![0.; MAX_BLOCK].into_boxed_slice(),
        }
    }
    fn process(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        // A delayed consumer drops old capture PCM instead of retaining a long stale backlog.
        // Only this thread owns the queue's read index; producers never discard samples.
        if trim_backlog(&self.input, &mut self.scratch, n) > 0 {
            self.shared
                .metrics
                .input_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.output[..n].fill(0.);
        let received = self.input.pop(&mut self.output[..n]);
        if received < n {
            self.shared
                .metrics
                .input_underruns
                .fetch_add(1, Ordering::Relaxed);
        }
        let mut invalid = 0_u64;
        // Sanitize before stateful DSP so one bad input cannot poison filter/delay state.
        for sample in &mut self.output[..n] {
            if !sample.is_finite() {
                *sample = 0.;
                invalid += 1;
            }
        }
        let settings = self.shared.dsp.load();
        let bypass = self.shared.bypass.load(Ordering::Relaxed);
        let latency = if bypass {
            0
        } else {
            self.dsp.latency_frames(&settings)
        };
        self.shared
            .metrics
            .algorithmic_latency_frames
            .store(latency.min(u32::MAX as usize) as u32, Ordering::Relaxed);
        if !bypass {
            self.dsp.process(&mut self.output[..n], &settings);
        }
        if self.shared.muted.load(Ordering::Relaxed) {
            self.output[..n].fill(0.);
        }
        self.scratch[..n].fill(0.);
        self.shared.soundboard.pop(&mut self.scratch[..n]);
        let soundboard_gain = finite_gain(self.shared.soundboard_gain.load(Ordering::Relaxed));
        let mut peak = 0.0_f32;
        for (out, sound) in self.output[..n].iter_mut().zip(&self.scratch[..n]) {
            let value = *out + sound * soundboard_gain;
            *out = if value.is_finite() {
                value.clamp(-1., 1.)
            } else {
                invalid += 1;
                0.
            };
            peak = peak.max(out.abs());
        }
        self.shared
            .metrics
            .non_finite_samples
            .fetch_add(invalid, Ordering::Relaxed);
        self.shared
            .metrics
            .output_peak
            .store(peak.to_bits(), Ordering::Relaxed);
        self.shared
            .metrics
            .frames
            .fetch_add(n as u64, Ordering::Relaxed);
        let written = self.shared.recording.push(&self.output[..n]);
        if written < n {
            self.shared
                .metrics
                .recording_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.listening[..n].fill(0.);
        if self.shared.monitor_enabled.load(Ordering::Relaxed) {
            let gain = finite_gain(self.shared.monitor_gain.load(Ordering::Relaxed));
            for (out, source) in self.listening[..n].iter_mut().zip(&self.output[..n]) {
                *out = source * gain;
            }
        }
        self.scratch[..n].fill(0.);
        self.shared.preview.pop(&mut self.scratch[..n]);
        let mut invalid_preview = 0_u64;
        for (out, preview) in self.listening[..n].iter_mut().zip(&self.scratch[..n]) {
            let value = *out + preview;
            *out = if value.is_finite() {
                value.clamp(-1., 1.)
            } else {
                invalid_preview += 1;
                0.
            };
        }
        self.shared
            .metrics
            .non_finite_samples
            .fetch_add(invalid_preview, Ordering::Relaxed);
        let written = self.monitor.push(&self.listening[..n]);
        if written < n {
            self.shared
                .metrics
                .monitor_producer_overruns
                .fetch_add(1, Ordering::Relaxed);
            self.shared
                .metrics
                .monitor_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn finite_gain(bits: u32) -> f32 {
    let v = f32::from_bits(bits);
    if v.is_finite() { v.clamp(0., 4.) } else { 0. }
}

fn trim_backlog(queue: &SampleQueue, scratch: &mut [f32], frames: usize) -> usize {
    if frames == 0 {
        return 0;
    }
    let backlog = queue.len();
    let mut dropped = 0;
    if backlog > frames.saturating_mul(4) {
        let mut discard = backlog.saturating_sub(frames.saturating_mul(2));
        while discard > 0 {
            let n = discard.min(scratch.len());
            let consumed = queue.pop(&mut scratch[..n]);
            if consumed == 0 {
                break;
            }
            discard -= consumed;
            dropped += consumed;
        }
    }
    dropped
}

fn output_process(stream: &pw::stream::Stream, state: &mut SourceProcessor, quantum: usize) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let requested = buffer.requested() as usize;
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let n = if let Some(bytes) = data.data() {
        let n = bytes.len() / 4;
        let n = n
            .min(if requested == 0 { quantum } else { requested })
            .min(MAX_BLOCK);
        state.process(n);
        for (bytes, value) in bytes[..n * 4]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&state.output[..n])
        {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        n
    } else {
        0
    };
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = 4;
    *chunk.size_mut() = (n * 4) as u32;
}

struct Monitor {
    shared: Arc<AudioShared>,
    queue: Arc<SampleQueue>,
    scratch: Box<[f32]>,
}
impl Monitor {
    fn render(&mut self, bytes: &mut [u8], requested_frames: usize) -> usize {
        let n = (bytes.len() / MONITOR_STRIDE)
            .min(requested_frames)
            .min(self.scratch.len());
        if n == 0 {
            self.shared
                .metrics
                .monitor_empty_callbacks
                .fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        let trimmed = trim_backlog(&self.queue, &mut self.scratch, n);
        if trimmed > 0 {
            self.shared
                .metrics
                .monitor_trimmed_frames
                .fetch_add(trimmed as u64, Ordering::Relaxed);
            self.shared
                .metrics
                .monitor_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.scratch[..n].fill(0.);
        self.queue.pop(&mut self.scratch[..n]);
        // The queue stays mono. Duplicate into explicitly positioned L/R frames only
        // at playback, so headphones do not depend on a session manager upmixing mono.
        for (frame, value) in bytes[..n * MONITOR_STRIDE]
            .as_chunks_mut::<MONITOR_STRIDE>()
            .0
            .iter_mut()
            .zip(&self.scratch[..n])
        {
            let sample = value.to_le_bytes();
            frame[..4].copy_from_slice(&sample);
            frame[4..].copy_from_slice(&sample);
        }
        n
    }
}
fn monitor_process(stream: &pw::stream::Stream, state: &mut Monitor, quantum: usize) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let requested = buffer.requested() as usize;
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let n = if let Some(bytes) = data.data() {
        state.render(bytes, if requested == 0 { quantum } else { requested })
    } else {
        0
    };
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = MONITOR_STRIDE as i32;
    *chunk.size_mut() = (n * MONITOR_STRIDE) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(id: u32, name: &str, kind: DeviceKind) -> DeviceInfo {
        DeviceInfo {
            id,
            name: name.into(),
            description: name.into(),
            kind,
        }
    }
    #[test]
    fn automatic_routes_prefer_available_desktop_defaults_of_the_right_kind() {
        let devices = vec![
            device(1, "a-microphone", DeviceKind::Input),
            device(2, "z-headset-microphone", DeviceKind::Input),
            device(3, "a-speakers", DeviceKind::Output),
            device(4, "z-headphones", DeviceKind::Output),
        ];
        assert_eq!(
            select_device(
                &devices,
                None,
                Some("z-headset-microphone"),
                DeviceKind::Input
            )
            .unwrap(),
            "z-headset-microphone"
        );
        assert_eq!(
            select_device(&devices, None, Some("z-headphones"), DeviceKind::Output).unwrap(),
            "z-headphones"
        );
        assert_eq!(
            select_device(&devices, None, Some("z-headphones"), DeviceKind::Input).unwrap(),
            "a-microphone"
        );
    }
    #[test]
    fn explicit_routes_resolve_ids_without_falling_back_to_another_microphone() {
        let devices = vec![
            device(12, "chosen-microphone", DeviceKind::Input),
            device(15, "desktop-microphone", DeviceKind::Input),
        ];
        for requested in ["12", "chosen-microphone"] {
            assert_eq!(
                select_device(
                    &devices,
                    Some(requested),
                    Some("desktop-microphone"),
                    DeviceKind::Input
                )
                .unwrap(),
                "chosen-microphone"
            );
        }
        assert!(
            select_device(
                &devices,
                Some("disconnected-microphone"),
                Some("desktop-microphone"),
                DeviceKind::Input
            )
            .is_err()
        );
        assert!(select_device(&devices, Some("12"), None, DeviceKind::Output).is_err());
    }
    #[test]
    fn automatic_fallback_is_deterministic_and_never_selects_self_or_sink_monitors() {
        let devices = vec![
            device(1, "voice-app.virtual-microphone", DeviceKind::Input),
            device(2, "a-speakers.monitor", DeviceKind::Input),
            device(3, "z-microphone", DeviceKind::Input),
            device(4, "b-microphone", DeviceKind::Input),
        ];
        for default in [
            None,
            Some("missing"),
            Some("voice-app.virtual-microphone"),
            Some("a-speakers.monitor"),
        ] {
            assert_eq!(
                select_device(&devices, None, default, DeviceKind::Input).unwrap(),
                "b-microphone"
            );
        }
        assert!(
            select_device(
                &devices,
                Some("voice-app.virtual-microphone"),
                None,
                DeviceKind::Input
            )
            .is_err()
        );
        assert!(
            select_device(
                &devices,
                Some("a-speakers.monitor"),
                None,
                DeviceKind::Input
            )
            .is_err()
        );
        assert!(select_device(&devices[..2], None, None, DeviceKind::Input).is_err());
    }
    #[test]
    fn desktop_metadata_names_require_valid_json_string_values() {
        assert_eq!(
            metadata_device_name(Some(r#"{"name":"usb.headset"}"#)).as_deref(),
            Some("usb.headset")
        );
        assert_eq!(
            metadata_device_name(Some(r#"{"name":"mic\u00f3"}"#)).as_deref(),
            Some("micó")
        );
        for value in [
            None,
            Some("broken json"),
            Some(r#"{"name":7}"#),
            Some(r#"{"name":""}"#),
            Some(r#"{}"#),
        ] {
            assert_eq!(metadata_device_name(value), None);
        }
    }
    #[test]
    fn pcm_formats_keep_mic_mono_and_position_headphones_left_right() {
        for positions in [&MONO_POSITION[..], &MONITOR_POSITION[..]] {
            let bytes = raw_format(48_000, positions).unwrap();
            let pod = spa::pod::Pod::from_bytes(&bytes).unwrap();
            let mut format = spa::param::audio::AudioInfoRaw::new();
            format.parse(pod).unwrap();
            assert_eq!(format.channels(), positions.len() as u32);
            assert_eq!(&format.position()[..positions.len()], positions);
            assert_eq!(format.format(), spa::param::audio::AudioFormat::F32LE);
            assert_eq!(format.rate(), 48_000);
        }
    }
    #[test]
    fn monitor_duplicates_frames_at_original_gain_and_silences_underruns() {
        let queue = Arc::new(SampleQueue::new(16));
        queue.push(&[0.25, -0.5]);
        let mut monitor = Monitor {
            shared: Arc::new(AudioShared::default()),
            queue: queue.clone(),
            scratch: vec![0.; 4].into_boxed_slice(),
        };
        let mut bytes = [0xCC; 4 * MONITOR_STRIDE + 7];
        assert_eq!(monitor.render(&mut bytes, 8), 4);
        let samples: Vec<_> = bytes[..4 * MONITOR_STRIDE]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|sample| f32::from_le_bytes(*sample))
            .collect();
        assert_eq!(samples, [0.25, 0.25, -0.5, -0.5, 0., 0., 0., 0.]);
        assert_eq!(&bytes[4 * MONITOR_STRIDE..], &[0xCC; 7]);
        queue.push(&[0.75, 0.5]);
        bytes.fill(0xCC);
        assert_eq!(monitor.render(&mut bytes, 1), 1);
        assert_eq!(&bytes[..4], &0.75_f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &0.75_f32.to_le_bytes());
        assert!(bytes[MONITOR_STRIDE..].iter().all(|byte| *byte == 0xCC));
        assert_eq!(queue.len(), 1);
    }
    #[test]
    fn monitor_without_output_frames_preserves_queued_audio() {
        for (byte_count, requested_frames) in [(0, 8), (7, 8), (16, 0)] {
            let shared = Arc::new(AudioShared::default());
            let queue = Arc::new(SampleQueue::new(16));
            assert_eq!(queue.push(&[0.25, -0.5]), 2);
            let mut monitor = Monitor {
                shared: shared.clone(),
                queue: queue.clone(),
                scratch: vec![0.; 4].into_boxed_slice(),
            };
            let mut bytes = vec![0xCC; byte_count];
            assert_eq!(monitor.render(&mut bytes, requested_frames), 0);
            assert!(bytes.iter().all(|byte| *byte == 0xCC));
            assert_eq!(queue.len(), 2);
            let metrics = shared.metrics.snapshot();
            assert_eq!(metrics.monitor_overruns, 0);
            assert_eq!(metrics.monitor_producer_overruns, 0);
            assert_eq!(metrics.monitor_trimmed_frames, 0);
            assert_eq!(metrics.monitor_empty_callbacks, 1);

            let mut output = [0; 2 * MONITOR_STRIDE];
            assert_eq!(monitor.render(&mut output, 2), 2);
            let samples: Vec<_> = output
                .as_chunks::<4>()
                .0
                .iter()
                .map(|sample| f32::from_le_bytes(*sample))
                .collect();
            assert_eq!(samples, [0.25, 0.25, -0.5, -0.5]);
            assert!(queue.is_empty());
            let metrics = shared.metrics.snapshot();
            assert_eq!(metrics.monitor_overruns, 0);
            assert_eq!(metrics.monitor_producer_overruns, 0);
            assert_eq!(metrics.monitor_trimmed_frames, 0);
            assert_eq!(metrics.monitor_empty_callbacks, 1);
        }
    }
    #[test]
    fn zero_frame_backlog_trim_preserves_the_fifo() {
        let queue = SampleQueue::new(16);
        let samples = [0.25, -0.5, 0.75, -1.0];
        assert_eq!(queue.push(&samples), samples.len());
        let mut scratch = [42.; 4];
        assert_eq!(trim_backlog(&queue, &mut scratch, 0), 0);
        assert_eq!(scratch, [42.; 4]);
        assert_eq!(queue.len(), samples.len());
        assert_eq!(trim_backlog(&queue, &mut scratch, samples.len()), 0);
        assert_eq!(queue.pop(&mut scratch), samples.len());
        assert_eq!(scratch, samples);
    }
    #[test]
    fn monitor_still_counts_and_recovers_from_a_real_backlog() {
        let shared = Arc::new(AudioShared::default());
        let queue = Arc::new(SampleQueue::new(64));
        let samples: Vec<_> = (0..64).map(|n| n as f32 / 64.).collect();
        assert_eq!(queue.push(&samples), samples.len());
        let mut monitor = Monitor {
            shared: shared.clone(),
            queue: queue.clone(),
            scratch: vec![0.; 8].into_boxed_slice(),
        };
        let mut bytes = [0; 8 * MONITOR_STRIDE];
        for (start, remaining) in [(48, 8), (56, 0)] {
            assert_eq!(monitor.render(&mut bytes, 8), 8);
            for (frame, expected) in bytes
                .as_chunks::<MONITOR_STRIDE>()
                .0
                .iter()
                .zip(&samples[start..start + 8])
            {
                assert_eq!(&frame[..4], &expected.to_le_bytes());
                assert_eq!(&frame[4..], &expected.to_le_bytes());
            }
            assert_eq!(queue.len(), remaining);
            let metrics = shared.metrics.snapshot();
            assert_eq!(metrics.monitor_overruns, 1);
            assert_eq!(metrics.monitor_producer_overruns, 0);
            assert_eq!(metrics.monitor_trimmed_frames, 48);
            assert_eq!(metrics.monitor_empty_callbacks, 0);
        }
    }
    #[test]
    fn monitor_queue_overflow_still_counts_rejected_audio() {
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(16));
        let monitor = Arc::new(SampleQueue::new(16));
        assert_eq!(input.push(&[0.25; 8]), 8);
        assert_eq!(shared.preview.push(&[0.4; 8]), 8);
        assert_eq!(monitor.push(&[0.1; 16]), 16);
        let mut processor = SourceProcessor::new(48_000, shared.clone(), input, monitor.clone());
        processor.process(8);
        let metrics = shared.metrics.snapshot();
        assert_eq!(metrics.monitor_overruns, 1);
        assert_eq!(metrics.monitor_producer_overruns, 1);
        assert_eq!(metrics.monitor_trimmed_frames, 0);
        assert_eq!(metrics.monitor_empty_callbacks, 0);
        assert_eq!(monitor.len(), 16);
        let mut queued = [0.; 16];
        assert_eq!(monitor.pop(&mut queued), 16);
        assert_eq!(queued, [0.1; 16]);
        assert_eq!(&processor.output[..8], &[0.25; 8]);
        let mut recorded = [0.; 8];
        assert_eq!(shared.recording.pop(&mut recorded), 8);
        assert_eq!(recorded, [0.25; 8]);
    }
    #[test]
    fn a_delayed_consumer_recovers_recent_audio_instead_of_old_backlog() {
        let q = SampleQueue::new(64);
        let samples: Vec<_> = (0..64).map(|n| n as f32).collect();
        q.push(&samples);
        let mut scratch = [0.; 16];
        assert_eq!(trim_backlog(&q, &mut scratch, 8), 48);
        let n = q.pop(&mut scratch);
        assert_eq!(n, 16);
        assert_eq!(&scratch, &samples[48..]);
    }
    #[test]
    fn preview_never_reaches_microphone_or_recording() {
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(16));
        let monitor = Arc::new(SampleQueue::new(16));
        input.push(&[0.2; 8]);
        shared.preview.push(&[0.4; 8]);
        let mut p = SourceProcessor::new(48_000, shared.clone(), input, monitor.clone());
        p.process(8);
        assert_eq!(&p.output[..8], &[0.2; 8]);
        let mut recorded = [0.; 8];
        shared.recording.pop(&mut recorded);
        assert_eq!(recorded, [0.2; 8]);
        let mut listened = [0.; 8];
        monitor.pop(&mut listened);
        assert_eq!(listened, [0.4; 8]);
    }
    #[test]
    fn mute_affects_microphone_while_soundboard_remains_available() {
        let shared = Arc::new(AudioShared::default());
        shared.muted.store(true, Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(16));
        input.push(&[0.7; 8]);
        shared.soundboard.push(&[0.3; 8]);
        let mut p = SourceProcessor::new(48_000, shared, input, Arc::new(SampleQueue::new(16)));
        p.process(8);
        assert_eq!(&p.output[..8], &[0.3; 8]);
    }
    #[test]
    fn invalid_audio_is_silenced_and_sum_is_limited() {
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(16));
        input.push(&[f32::NAN, 0.8]);
        shared.soundboard.push(&[0., 0.8]);
        let mut p = SourceProcessor::new(
            48_000,
            shared.clone(),
            input,
            Arc::new(SampleQueue::new(16)),
        );
        p.process(2);
        assert_eq!(&p.output[..2], &[0., 1.]);
        assert_eq!(shared.metrics.snapshot().non_finite_samples, 1);
    }
}
