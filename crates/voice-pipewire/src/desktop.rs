//! Sink monitoring has a separate producer and queue. It never feeds the virtual microphone.
use super::*;
use voice_core::{DesktopShared, SAMPLE_RATE};

/// Owns one explicitly selected desktop-output capture, independent of microphone routing.
pub struct DesktopCaptureHandle {
    target: String,
    stop: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<DesktopShared>,
}
impl DesktopCaptureHandle {
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn stop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.shared.active.store(false, Ordering::Release);
        self.shared.peak.store(0.0_f32.to_bits(), Ordering::Relaxed);
    }
}
impl Drop for DesktopCaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Capture the monitor of an explicit eligible output. No default device is changed and an
/// unavailable selection never falls back to a different sink. The worker owns queue draining.
pub fn start_desktop(target: String, shared: Arc<DesktopShared>) -> Result<DesktopCaptureHandle> {
    if target.trim().is_empty() || target.starts_with("voice-app.") {
        bail!("select an explicit desktop output outside voice-app");
    }
    let target = select_device(
        &device_snapshot()?.devices,
        Some(&target),
        None,
        DeviceKind::Output,
    )?;
    shared.active.store(false, Ordering::Release);
    shared.frames.store(0, Ordering::Relaxed);
    shared.overruns.store(0, Ordering::Relaxed);
    shared.non_finite_samples.store(0, Ordering::Relaxed);
    shared.peak.store(0.0_f32.to_bits(), Ordering::Relaxed);
    let (stop, receiver) = pw::channel::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let state = shared.clone();
    let selected = target.clone();
    let thread = thread::Builder::new()
        .name("voice-desktop-capture".into())
        .spawn(move || {
            if let Err(error) = run(selected, state.clone(), receiver, ready_tx.clone()) {
                let _ = ready_tx.try_send(Err(error.to_string()));
            }
            state.active.store(false, Ordering::Release);
            state.peak.store(0.0_f32.to_bits(), Ordering::Relaxed);
        })?;
    match ready_rx.recv_timeout(CONNECT_TIMEOUT + Duration::from_secs(1)) {
        Ok(Ok(())) => Ok(DesktopCaptureHandle {
            target,
            stop,
            thread: Some(thread),
            shared,
        }),
        result => {
            let _ = stop.send(());
            let _ = thread.join();
            match result {
                Ok(Err(message)) => Err(anyhow!(message)),
                Err(error) => Err(anyhow!("desktop capture initialization failed: {error}")),
                Ok(Ok(())) => unreachable!(),
            }
        }
    }
}

fn fail(
    ready: &Ready,
    mainloop: &pw::main_loop::MainLoopRc,
    shared: &DesktopShared,
    message: String,
) {
    shared.active.store(false, Ordering::Release);
    if let Some(sender) = ready.borrow_mut().take() {
        let _ = sender.try_send(Err(message));
    }
    mainloop.quit();
}

fn run(
    target: String,
    shared: Arc<DesktopShared>,
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

    // Control roundtrips distinguish a frozen server from a healthy but idle sink.
    // They run on the control loop; the realtime callback only handles prepared PCM.
    let pending = Rc::new(Cell::new(Some(core.sync(0)?)));
    let progress = Rc::new(Cell::new(Instant::now()));
    let p = pending.clone();
    let t = progress.clone();
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let _core = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && p.get() == Some(seq) {
                p.set(None);
                t.set(Instant::now());
            }
        })
        .error(move |id, _, _, message| {
            if id == pw::core::PW_ID_CORE {
                fail(&r, &l, &s, message.into());
            }
        })
        .register();
    let registry = core.get_registry()?;
    let selected_id = Rc::new(Cell::new(None::<u32>));
    let stream_ready = Rc::new(Cell::new(false));
    let selected = selected_id.clone();
    let removed = selected_id.clone();
    let target_name = target.clone();
    let target_ready = stream_ready.clone();
    let target_reply = ready.clone();
    let target_shared = shared.clone();
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let _registry = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ == pw::types::ObjectType::Node
                && global
                    .props
                    .as_ref()
                    .and_then(|props| props.get("node.name"))
                    == Some(target_name.as_str())
            {
                selected.set(Some(global.id));
                if target_ready.get() {
                    target_shared.active.store(true, Ordering::Release);
                    if let Some(sender) = target_reply.borrow_mut().take() {
                        let _ = sender.try_send(Ok(()));
                    }
                }
            }
        })
        .global_remove(move |id| {
            if removed.get() == Some(id) {
                fail(
                    &r,
                    &l,
                    &s,
                    "selected desktop output was disconnected".into(),
                );
            }
        })
        .register();
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let timeout = mainloop.loop_().add_timer(move |_| {
        if r.borrow().is_some() {
            fail(
                &r,
                &l,
                &s,
                "desktop capture did not become ready before timeout".into(),
            );
        }
    });
    timeout
        .update_timer(Some(CONNECT_TIMEOUT), None)
        .into_result()?;
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let c = core.clone();
    let watchdog = mainloop.loop_().add_timer(move |_| {
        if progress.get().elapsed() > Duration::from_secs(2) {
            fail(
                &r,
                &l,
                &s,
                "desktop capture lost its PipeWire connection".into(),
            );
        } else if pending.get().is_none() {
            match c.sync(0) {
                Ok(seq) => pending.set(Some(seq)),
                Err(error) => fail(
                    &r,
                    &l,
                    &s,
                    format!("desktop capture heartbeat failed: {error}"),
                ),
            }
        }
    });
    watchdog
        .update_timer(
            Some(Duration::from_millis(250)),
            Some(Duration::from_millis(250)),
        )
        .into_result()?;

    let stream = pw::stream::StreamBox::new(
        &core,
        "voice-app desktop capture",
        properties! {
            "media.type" => "Audio", "media.category" => "Capture", "media.role" => "Production",
            "node.name" => "voice-app.desktop-capture", "target.object" => target,
            "stream.capture.sink" => "true", "node.dont-fallback" => "true",
            "node.always-process" => "true", "node.pause-on-idle" => "false",
            "adapter.auto-port-config" => "{ mode = dsp position = preserve }",
            "node.link-group" => "voice-app.desktop-capture", "node.latency" => "256/48000",
        },
    )?;
    let r = ready.clone();
    let s = shared.clone();
    let l = mainloop.clone();
    let fr = ready.clone();
    let fs = shared.clone();
    let fl = mainloop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(DesktopCapture::new(shared))
        .state_changed(move |_, _, _, state| match state {
            pw::stream::StreamState::Paused | pw::stream::StreamState::Streaming => {
                stream_ready.set(true);
                // The target may disappear between enumeration and stream creation.
                // Both registry presence and stream readiness are required for success.
                if selected_id.get().is_some() {
                    s.active.store(true, Ordering::Release);
                    if let Some(sender) = r.borrow_mut().take() {
                        let _ = sender.try_send(Ok(()));
                    }
                }
            }
            pw::stream::StreamState::Error(error) => fail(&r, &l, &s, error),
            pw::stream::StreamState::Unconnected if stream_ready.get() => {
                fail(&r, &l, &s, "desktop capture stream was disconnected".into());
            }
            _ => {}
        })
        .param_changed(move |_, _, id, param| {
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else {
                return;
            };
            let mut info = spa::param::audio::AudioInfoRaw::new();
            if info.parse(param).is_err()
                || info.format() != spa::param::audio::AudioFormat::F32LE
                || info.rate() != SAMPLE_RATE
                || info.channels() != 2
            {
                fail(
                    &fr,
                    &fl,
                    &fs,
                    "desktop capture requires stereo f32/48000Hz PCM".into(),
                );
            }
        })
        .process(process)
        .register()?;
    let format = raw_format(SAMPLE_RATE, &MONITOR_POSITION)?;
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut [spa::pod::Pod::from_bytes(&format).context("invalid desktop capture format pod")?],
    )?;
    mainloop.run();
    // Disconnect on the control thread before the listener drops its prepared resources.
    let _ = stream.disconnect();
    Ok(())
}

struct DesktopCapture {
    shared: Arc<DesktopShared>,
    scratch: Box<[f32]>,
}
impl DesktopCapture {
    fn new(shared: Arc<DesktopShared>) -> Self {
        Self {
            shared,
            scratch: vec![0.0; MAX_BLOCK * 2].into_boxed_slice(),
        }
    }

    fn ingest(&mut self, bytes: &[u8]) {
        // Ignore an incomplete trailing stereo frame; all queue operations keep L/R aligned.
        let bytes = &bytes[..bytes.len() / MONITOR_STRIDE * MONITOR_STRIDE];
        let mut peak = 0.0_f32;
        for block in bytes.chunks(MAX_BLOCK * MONITOR_STRIDE) {
            let count = block.len() / size_of::<f32>();
            for (out, raw) in self.scratch[..count]
                .iter_mut()
                .zip(block.as_chunks::<4>().0)
            {
                let value = f32::from_le_bytes(*raw);
                *out = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    self.shared
                        .non_finite_samples
                        .fetch_add(1, Ordering::Relaxed);
                    0.0
                };
                peak = peak.max(out.abs());
            }
            let written = self.shared.samples.push(&self.scratch[..count]);
            if written < count {
                self.shared.overruns.fetch_add(1, Ordering::Relaxed);
            }
            self.shared
                .frames
                .fetch_add((count / 2) as u64, Ordering::Relaxed);
        }
        self.shared.peak.store(peak.to_bits(), Ordering::Relaxed);
    }
}

fn process(stream: &pw::stream::Stream, state: &mut DesktopCapture) {
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
    if offset <= end {
        state.ingest(&bytes[offset..end]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingAllocator;
    thread_local! {
        static COUNT_ENABLED: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }
    // SAFETY: all operations delegate their original pointer/layout to the system allocator.
    // The thread-local counters only observe operations on the explicitly measured thread.
    unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            let _ = COUNT_ENABLED.try_with(|enabled| {
                if enabled.get() {
                    ALLOCATIONS.with(|count| count.set(count.get() + 1));
                }
            });
            // SAFETY: the caller's valid allocation layout is forwarded unchanged.
            unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            let _ = COUNT_ENABLED.try_with(|enabled| {
                if enabled.get() {
                    DEALLOCATIONS.with(|count| count.set(count.get() + 1));
                }
            });
            // SAFETY: the caller's allocated pointer and matching layout are forwarded.
            unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, ptr, layout) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    #[test]
    fn prepared_desktop_ingest_never_allocates_or_deallocates() {
        let shared = Arc::new(DesktopShared::default());
        let mut capture = DesktopCapture::new(shared.clone());
        let bytes: Vec<u8> = [0.25_f32, -0.5]
            .into_iter()
            .cycle()
            .take(1024)
            .flat_map(f32::to_le_bytes)
            .collect();
        let mut output = [0.0; 1024];
        ALLOCATIONS.with(|count| count.set(0));
        DEALLOCATIONS.with(|count| count.set(0));
        COUNT_ENABLED.with(|enabled| enabled.set(true));
        for _ in 0..100 {
            capture.ingest(&bytes);
            shared.samples.pop(&mut output);
        }
        COUNT_ENABLED.with(|enabled| enabled.set(false));
        assert_eq!(ALLOCATIONS.with(Cell::get), 0);
        assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
    }

    #[test]
    fn ingest_preserves_stereo_frames_and_silences_non_finite() {
        let shared = Arc::new(DesktopShared::default());
        let mut capture = DesktopCapture::new(shared.clone());
        let mut bytes: Vec<u8> = [0.25_f32, -0.5, f32::NAN, f32::INFINITY]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        bytes.extend_from_slice(&0.9_f32.to_le_bytes());
        capture.ingest(&bytes);
        let mut samples = [0.0; 8];
        assert_eq!(shared.samples.pop(&mut samples), 4);
        assert_eq!(&samples[..4], &[0.25, -0.5, 0.0, 0.0]);
        assert_eq!(shared.frames.load(Ordering::Relaxed), 2);
        assert_eq!(shared.non_finite_samples.load(Ordering::Relaxed), 2);
        assert_eq!(f32::from_bits(shared.peak.load(Ordering::Relaxed)), 0.5);
    }

    #[test]
    fn overrun_is_bounded_and_preserves_channel_alignment() {
        let shared = Arc::new(DesktopShared::default());
        let mut capture = DesktopCapture::new(shared.clone());
        let bytes: Vec<u8> = [0.2_f32, -0.3]
            .into_iter()
            .cycle()
            .take(shared.samples.capacity() + 2048)
            .flat_map(f32::to_le_bytes)
            .collect();
        capture.ingest(&bytes);
        assert_eq!(shared.samples.len(), shared.samples.capacity());
        assert!(shared.overruns.load(Ordering::Relaxed) > 0);
        let mut frame = [0.0; 2];
        while shared.samples.pop(&mut frame) == 2 {
            assert_eq!(frame, [0.2, -0.3]);
        }
    }
}
