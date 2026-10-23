use crate::client::{read_local_frame, write_local_frame};
use crate::platform::{LocalListener, private_create_new, private_directory};
use crate::protocol::{Command, Request, Response};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File};
#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use voice_core::{
    AudioConfig, AudioShared, DesktopShared, DspSettings, FORMAT_VERSION, RecordingSource,
    SAMPLE_RATE, SampleQueue,
};
use voice_library::{Library, Mixer, Preset, ReplayBuffer};

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub socket: PathBuf,
}
impl Paths {
    /// Use the same private data directory for offline tools and the running service.
    pub fn prepare_data(&self) -> Result<()> {
        private_directory(&self.data)
    }

    pub fn discover() -> Result<Self> {
        crate::platform::discover_paths()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Session {
    version: u32,
    config: AudioConfig,
    audio_enabled: bool,
    settings: DspSettings,
    selected_preset: Option<String>,
    bypass: bool,
    muted: bool,
    monitor_gain: f32,
    soundboard_gain: f32,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            config: AudioConfig::default(),
            audio_enabled: false,
            settings: DspSettings::default(),
            selected_preset: None,
            bypass: false,
            muted: false,
            monitor_gain: 0.5,
            soundboard_gain: 1.0,
        }
    }
}
impl Session {
    fn validate(&self) -> Result<()> {
        if self.version != FORMAT_VERSION {
            bail!("unsupported session format version {}", self.version);
        }
        self.config.validate().map_err(anyhow::Error::msg)?;
        self.settings.validate().map_err(anyhow::Error::msg)?;
        validate_gain(self.monitor_gain)?;
        validate_gain(self.soundboard_gain)?;
        Ok(())
    }
}
fn validate_gain(gain: f32) -> Result<()> {
    if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
        bail!("volume must be finite and between 0 and 2");
    }
    Ok(())
}

fn stable_audio_preferences(config: &AudioConfig, resolved: &AudioConfig) -> AudioConfig {
    let mut saved = config.clone();
    if saved.input_device.is_some() {
        saved.input_device = resolved.input_device.clone();
    }
    if saved.output_device.is_some() {
        saved.output_device = resolved.output_device.clone();
    }
    saved
}

enum WorkerCommand {
    Play {
        id: String,
        pcm: Arc<Vec<f32>>,
        gain: f32,
        looped: bool,
        overlap: bool,
        preview: bool,
        reply: mpsc::Sender<Result<()>>,
    },
    StopAll,
    StopClip {
        id: String,
        reply: mpsc::Sender<Result<()>>,
    },
    /// Sent only after the previous backend has stopped producing recorded samples.
    /// The worker is the sole recording queue consumer and acknowledges the reset.
    Reset {
        clear_replay: bool,
        reply: mpsc::Sender<Result<()>>,
    },
    ResetDesktop {
        clear_replay: bool,
        reply: mpsc::Sender<Result<()>>,
    },
    Record {
        path: PathBuf,
        source: RecordingSource,
        reply: mpsc::Sender<Result<()>>,
    },
    Finish {
        reply: mpsc::Sender<Result<Value>>,
    },
    Replay {
        path: PathBuf,
        source: RecordingSource,
        seconds: u32,
        reply: mpsc::Sender<Result<Value>>,
    },
}
struct Worker {
    tx: mpsc::SyncSender<WorkerCommand>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    playing: Arc<AtomicUsize>,
    recording: Arc<AtomicBool>,
    recording_source: Arc<AtomicUsize>,
    replay_samples: [Arc<AtomicUsize>; 2],
    error: Arc<std::sync::Mutex<Option<String>>>,
}
type RecordingWriter = hound::WavWriter<std::io::BufWriter<File>>;
fn new_recording(path: &Path, channels: u16) -> Result<RecordingWriter> {
    let file = private_create_new(path).with_context(|| {
        format!(
            "cannot create {} (existing files are never overwritten)",
            path.display()
        )
    })?;
    Ok(hound::WavWriter::new(
        std::io::BufWriter::new(file),
        hound::WavSpec {
            channels,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?)
}
fn write_samples(writer: &mut RecordingWriter, samples: &[f32]) -> Result<()> {
    for &s in samples {
        let s = if s.is_finite() { s.clamp(-1., 1.) } else { 0. };
        writer.write_sample((s * 32767.) as i16)?;
    }
    Ok(())
}
fn save_audio(path: &Path, samples: &[f32], channels: u16) -> Result<Value> {
    let mut writer = new_recording(path, channels)?;
    write_samples(&mut writer, samples)?;
    writer.finalize()?;
    Ok(
        json!({"path":path,"channels":channels,"frames":samples.len()/channels as usize,"seconds":samples.len() as f64/channels as f64/SAMPLE_RATE as f64}),
    )
}

struct CaptureState {
    source: RecordingSource,
    replay: ReplayBuffer,
    replay_samples: Arc<AtomicUsize>,
    recording_source: Arc<AtomicUsize>,
    writer: Option<RecordingWriter>,
    frame_count: u64,
    recording: Arc<AtomicBool>,
    error: Arc<std::sync::Mutex<Option<String>>>,
}
impl CaptureState {
    fn new(
        source: RecordingSource,
        recording: Arc<AtomicBool>,
        recording_source: Arc<AtomicUsize>,
        replay_samples: Arc<AtomicUsize>,
        error: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            source,
            replay: ReplayBuffer::with_channels(30, source.channels()),
            replay_samples,
            recording_source,
            writer: None,
            frame_count: 0,
            recording,
            error,
        }
    }
    fn remember_error(&self, error: &anyhow::Error) {
        if let Ok(mut current) = self.error.lock() {
            *current = Some(format!("{error:#}"));
        }
    }
    fn drain(&mut self, queue: &SampleQueue) {
        let mut recorded = [0.0_f32; 4096];
        // Bound each drain to the samples present at entry. A slow disk must not
        // prevent the worker from servicing Stop/Finish while capture continues.
        let mut remaining = queue.len();
        while remaining > 0 {
            let limit = remaining.min(recorded.len());
            let count = queue.pop(&mut recorded[..limit]);
            if count == 0 {
                break;
            }
            remaining -= count;
            self.replay.push(&recorded[..count]);
            self.replay_samples
                .store(self.replay.len(), Ordering::Relaxed);
            if let Some(writer) = self.writer.as_mut() {
                match write_samples(writer, &recorded[..count]) {
                    Ok(()) => self.frame_count += count as u64 / self.source.channels() as u64,
                    Err(error) => {
                        self.remember_error(&error);
                        if let Some(writer) = self.writer.take() {
                            let _ = writer.finalize();
                        }
                        self.recording.store(false, Ordering::Release);
                        self.recording_source.store(0, Ordering::Release);
                    }
                }
            }
        }
    }
    fn record(&mut self, path: &Path) -> Result<()> {
        if self.recording.load(Ordering::Acquire) {
            bail!("a recording is already in progress; finish it before changing source");
        }
        self.writer = Some(new_recording(path, self.source.channels())?);
        self.frame_count = 0;
        if let Ok(mut error) = self.error.lock() {
            *error = None;
        }
        self.recording_source
            .store(source_index(self.source) + 1, Ordering::Release);
        self.recording.store(true, Ordering::Release);
        Ok(())
    }
    fn finish(&mut self, required: bool) -> Result<Value> {
        let Some(writer) = self.writer.take() else {
            if required {
                bail!("no recording in progress");
            }
            return Ok(json!({"finished":false}));
        };
        self.recording.store(false, Ordering::Release);
        self.recording_source.store(0, Ordering::Release);
        if let Err(error) = writer.finalize().map_err(anyhow::Error::from) {
            self.remember_error(&error);
            return Err(error).context("finalize recording WAV");
        }
        Ok(
            json!({"finished":true,"source":self.source,"channels":self.source.channels(),"frames":self.frame_count,"seconds":self.frame_count as f64/SAMPLE_RATE as f64}),
        )
    }
}
fn source_index(source: RecordingSource) -> usize {
    match source {
        RecordingSource::ProcessedVoice => 0,
        RecordingSource::Desktop => 1,
    }
}
impl Worker {
    fn spawn(shared: Arc<AudioShared>, desktop: Arc<DesktopShared>) -> Self {
        let (tx, rx) = mpsc::sync_channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        let playing = Arc::new(AtomicUsize::new(0));
        let recording = Arc::new(AtomicBool::new(false));
        let error = Arc::new(std::sync::Mutex::new(None));
        let recording_source = Arc::new(AtomicUsize::new(0));
        let replay_samples = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
        let source_state = recording_source.clone();
        let replay_lengths = replay_samples.clone();
        let (done, voices, rec, err) = (
            stop.clone(),
            playing.clone(),
            recording.clone(),
            error.clone(),
        );
        let join = thread::spawn(move || {
            let (mut mix, mut preview) = (Mixer::new(), Mixer::new());
            let mut captures = [
                CaptureState::new(
                    RecordingSource::ProcessedVoice,
                    rec.clone(),
                    source_state.clone(),
                    replay_lengths[0].clone(),
                    err.clone(),
                ),
                CaptureState::new(
                    RecordingSource::Desktop,
                    rec,
                    source_state,
                    replay_lengths[1].clone(),
                    err,
                ),
            ];
            let mut mixed = [0_f32; 480];
            while !done.load(Ordering::Acquire) {
                while let Ok(command) = rx.try_recv() {
                    // Drain before command boundaries so Finish/Replay include queued tails,
                    // while Record does not capture audio queued before its start request.
                    captures[0].drain(&shared.recording);
                    captures[1].drain(&desktop.samples);
                    match command {
                        WorkerCommand::Play {
                            id,
                            pcm,
                            gain,
                            looped,
                            overlap,
                            preview: is_preview,
                            reply,
                        } => {
                            let target = if is_preview { &mut preview } else { &mut mix };
                            let _ = reply.send(target.play(&id, pcm, gain, looped, overlap));
                        }
                        WorkerCommand::StopAll => {
                            mix.stop_all();
                            preview.stop_all();
                        }
                        WorkerCommand::StopClip { id, reply } => {
                            mix.stop(&id);
                            preview.stop(&id);
                            voices.store(
                                mix.active_count() + preview.active_count(),
                                Ordering::Relaxed,
                            );
                            // Mixed PCM already queued can contain other sounds:
                            // retain that bounded prepared tail rather than interrupt
                            // unrelated clips or add a second queue consumer.
                            let _ = reply.send(Ok(()));
                        }
                        WorkerCommand::Reset {
                            clear_replay,
                            reply,
                        } => {
                            mix.stop_all();
                            preview.stop_all();
                            voices.store(0, Ordering::Relaxed);
                            let capture = &mut captures[0];
                            let result = capture.finish(false).map(|_| ());
                            if clear_replay {
                                capture.replay.clear();
                                capture.replay_samples.store(0, Ordering::Relaxed);
                                // Only this worker consumes/discards the callback's recording queue.
                                // The backend must already be stopped before requesting Reset.
                                shared.recording.discard();
                            }
                            let _ = reply.send(result);
                        }
                        WorkerCommand::ResetDesktop {
                            clear_replay,
                            reply,
                        } => {
                            let capture = &mut captures[1];
                            let result = capture.finish(false).map(|_| ());
                            if clear_replay {
                                capture.replay.clear();
                                capture.replay_samples.store(0, Ordering::Relaxed);
                                desktop.samples.discard();
                            }
                            let _ = reply.send(result);
                        }
                        WorkerCommand::Record {
                            path,
                            source,
                            reply,
                        } => {
                            let _ = reply.send(captures[source_index(source)].record(&path));
                        }
                        WorkerCommand::Finish { reply } => {
                            let index =
                                captures.iter().position(|capture| capture.writer.is_some());
                            let result = match index {
                                Some(index) => captures[index].finish(true),
                                None => Err(anyhow!("no recording in progress")),
                            };
                            let _ = reply.send(result);
                        }
                        WorkerCommand::Replay {
                            path,
                            source,
                            seconds,
                            reply,
                        } => {
                            let capture = &captures[source_index(source)];
                            let channels = source.channels();
                            let samples = capture.replay.snapshot();
                            let start = samples.len().saturating_sub(
                                seconds as usize * SAMPLE_RATE as usize * channels as usize,
                            );
                            let result = if samples.is_empty() {
                                Err(anyhow!(
                                    "replay is empty; start the selected capture source first"
                                ))
                            } else {
                                save_audio(&path, &samples[start..], channels)
                            };
                            let _ = reply.send(result);
                        }
                    }
                }
                // One producer per queue; keep at most 20 ms queued instead of accumulating latency.
                if shared.metrics.active.load(Ordering::Relaxed) {
                    for (mixer, queue) in [
                        (&mut mix, &shared.soundboard),
                        (&mut preview, &shared.preview),
                    ] {
                        while queue.len() < 960 {
                            mixer.process(&mut mixed);
                            queue.push(&mixed);
                        }
                    }
                } else {
                    mix.stop_all();
                    preview.stop_all();
                }
                voices.store(
                    mix.active_count() + preview.active_count(),
                    Ordering::Relaxed,
                );
                captures[0].drain(&shared.recording);
                captures[1].drain(&desktop.samples);
                thread::sleep(Duration::from_millis(5));
            }
            captures[0].drain(&shared.recording);
            captures[1].drain(&desktop.samples);
            for capture in &mut captures {
                let _ = capture.finish(false);
            }
            voices.store(0, Ordering::Relaxed);
        });
        Self {
            tx,
            stop,
            join: Some(join),
            playing,
            recording,
            recording_source,
            replay_samples,
            error,
        }
    }
    fn send(&self, command: WorkerCommand) -> Result<()> {
        self.tx
            .try_send(command)
            .map_err(|e| anyhow!("audio worker busy or disconnected: {e}"))
    }
    fn stop_clip(&self, id: String) -> Result<()> {
        let (reply, acknowledgement) = mpsc::channel();
        self.send(WorkerCommand::StopClip { id, reply })?;
        acknowledgement
            .recv_timeout(Duration::from_secs(3))
            .context("audio worker clip-stop acknowledgement timed out")?
    }
    fn reset_desktop(&self, clear_replay: bool) -> Result<()> {
        let (reply, acknowledgement) = mpsc::channel();
        self.send(WorkerCommand::ResetDesktop {
            clear_replay,
            reply,
        })?;
        acknowledgement
            .recv_timeout(Duration::from_secs(3))
            .context("desktop worker reset acknowledgement timed out")?
    }
    fn replay_seconds(&self, source: RecordingSource) -> f64 {
        self.replay_samples[source_index(source)].load(Ordering::Relaxed) as f64
            / source.channels() as f64
            / SAMPLE_RATE as f64
    }
    fn reset(&self, clear_replay: bool) -> Result<()> {
        let (reply, acknowledgement) = mpsc::channel();
        self.send(WorkerCommand::Reset {
            clear_replay,
            reply,
        })?;
        acknowledgement
            .recv_timeout(Duration::from_secs(3))
            .context("audio worker reset acknowledgement timed out")?
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct Engine {
    paths: Paths,
    library: Library,
    preset_image_revision: u64,
    session: Session,
    shared: Arc<AudioShared>,
    worker: Worker,
    audio: Option<crate::audio::AudioHandle>,
    desktop_shared: Arc<DesktopShared>,
    desktop: Option<crate::audio::DesktopCaptureHandle>,
    desktop_target: Option<String>,
    desktop_error: Option<String>,
    resolved_config: Option<AudioConfig>,
    next_audio_retry: Instant,
    retry_delay: Duration,
    next_device_check: Instant,
    last_audio_error: Option<String>,
}
impl Engine {
    fn new(paths: Paths) -> Result<Self> {
        private_directory(&paths.data)?;
        let path = paths.data.join("session.json");
        let session: Session = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)
                .context("invalid saved session; preserve session.json and repair its contents")?
        } else {
            Session::default()
        };
        session.validate()?;
        let shared = Arc::new(AudioShared::default());
        shared.dsp.store(session.settings);
        shared.bypass.store(session.bypass, Ordering::Relaxed);
        shared.muted.store(session.muted, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(session.monitor_gain.to_bits(), Ordering::Relaxed);
        shared
            .soundboard_gain
            .store(session.soundboard_gain.to_bits(), Ordering::Relaxed);
        let library = Library::open(&paths.data.join("library"))?;
        let desktop_shared = Arc::new(DesktopShared::default());
        let worker = Worker::spawn(shared.clone(), desktop_shared.clone());
        Ok(Self {
            paths,
            library,
            preset_image_revision: 0,
            session,
            shared,
            worker,
            audio: None,
            desktop_shared,
            desktop: None,
            desktop_target: None,
            desktop_error: None,
            resolved_config: None,
            next_audio_retry: Instant::now(),
            retry_delay: Duration::from_secs(2),
            next_device_check: Instant::now(),
            last_audio_error: None,
        })
    }
    fn commit_session(&mut self, candidate: Session) -> Result<()> {
        // Keep both the live controls and the in-memory session unchanged until
        // the complete candidate has been validated and atomically persisted.
        candidate.validate()?;
        let path = self.paths.data.join("session.json");
        let mut bytes = serde_json::to_vec_pretty(&candidate)?;
        bytes.push(b'\n');
        persist_private_file(&path, &bytes)?;
        self.session = candidate;
        Ok(())
    }
    fn stop_audio(&mut self, clear_replay: bool) -> Result<()> {
        // Drop joins the producer before the worker drains/finalizes its tail.
        self.audio.take();
        self.resolved_config = None;
        self.shared.metrics.active.store(false, Ordering::Release);
        self.worker.reset(clear_replay)
    }
    fn stop_desktop(&mut self, clear_replay: bool) -> Result<()> {
        self.desktop.take();
        self.desktop_shared.active.store(false, Ordering::Release);
        self.worker.reset_desktop(clear_replay)
    }
    fn check_audio_health(&mut self) {
        if self.desktop.is_some() && !self.desktop_shared.active.load(Ordering::Acquire) {
            let result = self.stop_desktop(false);
            self.desktop_error = Some(match result {
                Ok(()) => "Captura do desktop desconectada. Selecione a saída e inicie novamente."
                    .to_owned(),
                Err(error) => {
                    format!("Captura desconectada; falha ao finalizar gravação: {error:#}")
                }
            });
        }
        if self.audio.is_some() && !self.shared.metrics.active.load(Ordering::Acquire) {
            let finalization = self.stop_audio(false);
            let mut message =
                "Áudio desconectado. Aguardando os dispositivos para reconectar automaticamente."
                    .to_owned();
            if let Err(error) = finalization {
                message.push_str(&format!(" Recording finalization failed: {error:#}"));
            }
            self.last_audio_error = Some(message);
            self.next_audio_retry = Instant::now() + Duration::from_secs(2);
            self.retry_delay = Duration::from_secs(2);
        }
        if !self.session.audio_enabled {
            return;
        }
        let now = Instant::now();
        // Only automatic routes follow desktop default changes. Fixed names never
        // fall back to another microphone. Discovery runs outside process callbacks.
        if self.audio.is_some()
            && (self.session.config.input_device.is_none()
                || self.session.config.output_device.is_none())
            && now >= self.next_device_check
        {
            self.next_device_check = now + Duration::from_secs(2);
            if let Ok(resolved) = crate::audio::resolve_config(&self.session.config)
                && self.resolved_config.as_ref() != Some(&resolved)
            {
                if let Err(error) = self.stop_audio(false) {
                    self.last_audio_error =
                        Some(format!("Não foi possível finalizar a gravação: {error:#}"));
                }
                self.next_audio_retry = Instant::now();
            }
        }
        if self.audio.is_none() && Instant::now() >= self.next_audio_retry {
            let result = crate::audio::resolve_config(&self.session.config)
                .and_then(|resolved| self.start_resolved(resolved));
            if let Err(error) = result {
                self.schedule_retry(&error);
            }
        }
    }
    fn start_resolved(&mut self, config: AudioConfig) -> Result<()> {
        let handle = crate::audio::start(config.clone(), self.shared.clone())?;
        self.audio = Some(handle);
        self.resolved_config = Some(config);
        self.last_audio_error = None;
        self.retry_delay = Duration::from_secs(2);
        self.next_device_check = Instant::now() + Duration::from_secs(2);
        Ok(())
    }
    fn schedule_retry(&mut self, error: &anyhow::Error) {
        self.last_audio_error = Some(format!(
            "Aguardando os dispositivos para reconectar automaticamente: {error:#}"
        ));
        self.next_audio_retry = Instant::now() + self.retry_delay;
        self.retry_delay = (self.retry_delay * 2).min(Duration::from_secs(5));
    }
    fn execute(&mut self, command: Command, stop: &AtomicBool) -> Result<Value> {
        if !matches!(
            &command,
            Command::Stop | Command::Shutdown | Command::Start { .. }
        ) {
            self.check_audio_health();
        }
        match command {
            Command::Status => Ok(
                json!({"version":env!("CARGO_PKG_VERSION"),"backend":crate::audio::BACKEND_NAME,"audio_running":self.shared.metrics.active.load(Ordering::Relaxed),"config":self.session.config,"resolved_config":self.resolved_config,"audio_requested":self.session.audio_enabled,"audio_connecting":self.session.audio_enabled && self.audio.is_none(),"settings":self.session.settings,"selected_preset":self.session.selected_preset,"bypass":self.shared.bypass.load(Ordering::Relaxed),"muted":self.shared.muted.load(Ordering::Relaxed),"monitor":self.shared.monitor_enabled.load(Ordering::Relaxed),"monitor_gain":self.session.monitor_gain,"soundboard_gain":self.session.soundboard_gain,"metrics":self.shared.metrics.snapshot(),"playing":self.worker.playing.load(Ordering::Relaxed),"recording":self.worker.recording.load(Ordering::Relaxed),"recording_source":match self.worker.recording_source.load(Ordering::Acquire) {1=>Some(RecordingSource::ProcessedVoice),2=>Some(RecordingSource::Desktop),_=>None},"replay_seconds":self.worker.replay_seconds(RecordingSource::ProcessedVoice),"desktop":{"active":self.desktop_shared.active.load(Ordering::Acquire),"output_device":self.desktop_target,"error":self.desktop_error,"replay_seconds":self.worker.replay_seconds(RecordingSource::Desktop),"metrics":{"frames":self.desktop_shared.frames.load(Ordering::Relaxed),"overruns":self.desktop_shared.overruns.load(Ordering::Relaxed),"non_finite_samples":self.desktop_shared.non_finite_samples.load(Ordering::Relaxed),"peak":f32::from_bits(self.desktop_shared.peak.load(Ordering::Relaxed))}},"recording_error":self.worker.error.lock().ok().and_then(|v|v.clone()),"last_audio_error":self.last_audio_error,"data_dir":self.paths.data,"preset_image_revision":self.preset_image_revision,"capabilities":{"dsp":true,"soundboard":true,"processed_recording":true,"replay":true,"pitch":true,"formant":true,"chorus":true,"noise_reduction":true,"reorder_effects":true,"preset_images":true,"voice_blend":true,"effect_intensity":true,"neural_voice":false,"desktop_capture":crate::audio::DESKTOP_CAPTURE,"virtual_microphone":crate::audio::VIRTUAL_MICROPHONE,"global_shortcuts":false}}),
            ),
            Command::Devices => Ok(serde_json::to_value(crate::audio::list_devices()?)?),
            Command::Start { config } => {
                config.validate().map_err(anyhow::Error::msg)?;
                let resolved = crate::audio::resolve_config(&config);
                if resolved.is_err()
                    && [&config.input_device, &config.output_device]
                        .into_iter()
                        .any(|target| {
                            target
                                .as_deref()
                                .is_some_and(|name| name.parse::<u32>().is_ok())
                        })
                {
                    // Node IDs are transient and may identify a different microphone
                    // after a server restart. Only resolved names are safe to remember.
                    bail!(
                        "Não foi possível identificar os dispositivos. Use o nome do dispositivo para aguardar sua reconexão."
                    );
                }
                let mut candidate = self.session.clone();
                candidate.config = resolved
                    .as_ref()
                    .map(|resolved| stable_audio_preferences(&config, resolved))
                    .unwrap_or_else(|_| config.clone());
                candidate.audio_enabled = true;
                self.commit_session(candidate)?;
                self.stop_audio(true)?;
                self.retry_delay = Duration::from_secs(2);
                match resolved.and_then(|resolved| self.start_resolved(resolved)) {
                    Ok(()) => Ok(json!({"started":true})),
                    Err(error) => {
                        self.schedule_retry(&error);
                        Ok(json!({"started":false,"waiting_for_devices":true}))
                    }
                }
            }
            Command::Stop => {
                // Stop is immediate even if saving preferences fails. A full disk
                // must never leave capture or automatic retries enabled.
                self.session.audio_enabled = false;
                self.last_audio_error = None;
                let stopped = self.stop_audio(false);
                self.commit_session(self.session.clone()).context(
                    "Áudio desconectado, mas não foi possível salvar a preferência. A próxima abertura poderá retomar a configuração anterior",
                )?;
                stopped?;
                Ok(json!({"stopped":true}))
            }
            Command::Configure { settings } => {
                settings.validate().map_err(anyhow::Error::msg)?;
                let mut candidate = self.session.clone();
                candidate.settings = settings;
                candidate.selected_preset = None;
                self.commit_session(candidate)?;
                self.shared.dsp.store(settings);
                Ok(json!({"settings":settings}))
            }
            Command::SetControls {
                bypass,
                muted,
                monitor,
                monitor_gain,
                soundboard_gain,
            } => {
                if let Some(v) = monitor_gain {
                    validate_gain(v)?;
                }
                if let Some(v) = soundboard_gain {
                    validate_gain(v)?;
                }
                let mut candidate = self.session.clone();
                if let Some(v) = bypass {
                    candidate.bypass = v;
                }
                if let Some(v) = muted {
                    candidate.muted = v;
                }
                if let Some(v) = monitor_gain {
                    candidate.monitor_gain = v;
                }
                if let Some(v) = soundboard_gain {
                    candidate.soundboard_gain = v;
                }
                self.commit_session(candidate)?;
                self.shared
                    .bypass
                    .store(self.session.bypass, Ordering::Relaxed);
                self.shared
                    .muted
                    .store(self.session.muted, Ordering::Relaxed);
                self.shared
                    .monitor_gain
                    .store(self.session.monitor_gain.to_bits(), Ordering::Relaxed);
                self.shared
                    .soundboard_gain
                    .store(self.session.soundboard_gain.to_bits(), Ordering::Relaxed);
                if let Some(v) = monitor {
                    self.shared.monitor_enabled.store(v, Ordering::Relaxed);
                }
                Ok(json!({"updated":true}))
            }
            Command::Presets => Ok(serde_json::to_value(self.library.presets()?)?),
            Command::PresetImage { id } => {
                Ok(serde_json::to_value(self.library.preset_image(&id)?)?)
            }
            Command::SetPresetImage { id, path } => {
                let image = self.library.set_preset_image(&id, &path)?;
                let result = serde_json::to_value(image)?;
                self.preset_image_revision = self.preset_image_revision.wrapping_add(1);
                Ok(result)
            }
            Command::ClearPresetImage { id } => {
                self.library.clear_preset_image(&id)?;
                self.preset_image_revision = self.preset_image_revision.wrapping_add(1);
                Ok(json!({"cleared":true}))
            }
            Command::SelectPreset { id } => {
                let preset = self
                    .library
                    .presets()?
                    .into_iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| anyhow!("unknown preset: {id}"))?;
                let mut candidate = self.session.clone();
                candidate.settings = preset.settings;
                candidate.selected_preset = Some(preset.id.clone());
                self.commit_session(candidate)?;
                self.shared.dsp.store(preset.settings);
                Ok(serde_json::to_value(preset)?)
            }
            Command::SavePreset { id, name, settings } => {
                settings.validate().map_err(anyhow::Error::msg)?;
                let preset = Preset { id, name, settings };
                self.library.save_preset(&preset)?;
                Ok(serde_json::to_value(preset)?)
            }
            Command::ImportPreset { path } => {
                Ok(serde_json::to_value(self.library.import_preset(&path)?)?)
            }
            Command::ExportPreset { id, path } => {
                self.library.export_preset(&id, &path)?;
                Ok(json!({"path":path}))
            }
            Command::Boards => Ok(serde_json::to_value(self.library.boards()?)?),
            Command::CreateBoard { name } => {
                Ok(serde_json::to_value(self.library.create_board(&name)?)?)
            }
            Command::Clips { board_id } => {
                Ok(serde_json::to_value(self.library.clips(&board_id)?)?)
            }
            Command::ImportClip {
                path,
                board_id,
                name,
            } => Ok(serde_json::to_value(
                self.library.import_clip(&path, &board_id, &name)?,
            )?),
            Command::TrimClip {
                id,
                start_seconds,
                end_seconds,
                name,
            } => Ok(serde_json::to_value(self.library.trim_clip(
                &id,
                start_seconds,
                end_seconds,
                &name,
            )?)?),
            Command::RemoveClip { id } => {
                // Validate ID and existence using metadata before stopping any
                // voices. Removal never loads PCM or deletes retained audio.
                self.library.clip(&id)?;
                self.worker.stop_clip(id.clone())?;
                self.library.remove_clip(&id)?;
                Ok(json!({"removed":true,"id":id}))
            }
            Command::Play {
                id,
                preview,
                looped,
                overlap,
                gain,
            } => {
                if !self.shared.metrics.active.load(Ordering::Relaxed) {
                    bail!("start audio before playing sounds");
                }
                validate_gain(gain)?;
                let pcm = self.library.load_clip(&id)?;
                let (tx, rx) = mpsc::channel();
                self.worker.send(WorkerCommand::Play {
                    id,
                    pcm,
                    gain,
                    looped,
                    overlap,
                    preview,
                    reply: tx,
                })?;
                rx.recv_timeout(Duration::from_secs(3))??;
                Ok(json!({"playing":true,"preview":preview}))
            }
            Command::StopSounds => {
                self.worker.send(WorkerCommand::StopAll)?;
                Ok(json!({"stopped":true}))
            }
            Command::StartDesktopCapture { output_device } => {
                if self.desktop.is_some() {
                    bail!("desktop capture is already active; stop it before changing output");
                }
                if output_device.is_empty() || output_device.starts_with("voice-app.") {
                    bail!("select an explicit desktop output device");
                }
                match crate::audio::start_desktop(output_device, self.desktop_shared.clone()) {
                    Ok(handle) => {
                        // Reset only after a successful connection so a rejected
                        // target preserves the previous replay. The worker remains
                        // the only queue consumer, including this boundary.
                        self.worker.reset_desktop(true)?;
                        self.desktop_target = Some(handle.target().to_owned());
                        self.desktop = Some(handle);
                        self.desktop_error = None;
                        Ok(json!({"desktop_capture":true,"output_device":self.desktop_target}))
                    }
                    Err(error) => {
                        self.desktop_error = Some(format!("{error:#}"));
                        Err(error)
                    }
                }
            }
            Command::StopDesktopCapture => {
                self.stop_desktop(false)?;
                self.desktop_error = None;
                Ok(json!({"desktop_capture":false}))
            }
            Command::StartRecording { path, source } => {
                let active = match source {
                    RecordingSource::ProcessedVoice => {
                        self.shared.metrics.active.load(Ordering::Relaxed)
                    }
                    RecordingSource::Desktop => self.desktop_shared.active.load(Ordering::Relaxed),
                };
                if !active {
                    bail!("start the selected audio capture before recording");
                }
                let (tx, rx) = mpsc::channel();
                self.worker.send(WorkerCommand::Record {
                    path,
                    source,
                    reply: tx,
                })?;
                rx.recv_timeout(Duration::from_secs(3))??;
                Ok(json!({"recording":true,"source":source}))
            }
            Command::StopRecording => {
                let (tx, rx) = mpsc::channel();
                self.worker.send(WorkerCommand::Finish { reply: tx })?;
                rx.recv_timeout(Duration::from_secs(3))?
            }
            Command::SaveReplay {
                path,
                seconds,
                source,
            } => {
                if !(1..=30).contains(&seconds) {
                    bail!("replay seconds must be between 1 and 30");
                }
                let (tx, rx) = mpsc::channel();
                self.worker.send(WorkerCommand::Replay {
                    path,
                    source,
                    seconds,
                    reply: tx,
                })?;
                rx.recv_timeout(Duration::from_secs(3))?
            }
            Command::Shutdown => {
                let finalization = self.stop_audio(false);
                let desktop_finalization = self.stop_desktop(false);
                stop.store(true, Ordering::Release);
                finalization?;
                desktop_finalization?;
                Ok(json!({"shutdown":true}))
            }
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.stop_audio(false);
        let _ = self.stop_desktop(false);
    }
}

/// Atomically replace an application-owned file after writing it privately.
/// The bytes are persisted exactly as supplied. Never call this from an audio callback.
pub fn persist_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::platform::persist_private_file(path, bytes)
}

pub struct ServiceHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<()>>>,
}
impl ServiceHandle {
    pub fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }
    pub fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            j.join().map_err(|_| anyhow!("engine thread panicked"))??;
        }
        Ok(())
    }
}
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub fn spawn_service(paths: Paths) -> Result<ServiceHandle> {
    let mut listener = LocalListener::bind(&paths.socket)?;
    let mut engine = Engine::new(paths)?;
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let join = thread::spawn(move || {
        while !done.load(Ordering::Acquire) {
            engine.check_audio_health();
            match listener.try_accept() {
                Ok(Some(mut stream)) => {
                    let response = match read_local_frame(&mut stream, Duration::from_secs(2))
                        .and_then(|data| {
                            serde_json::from_slice::<Request>(&data).map_err(Into::into)
                        }) {
                        Ok(request) => match engine.execute(request.command, &done) {
                            Ok(value) => Response::success(request.id, value),
                            Err(e) => Response::failure(request.id, format!("{e:#}")),
                        },
                        Err(e) => Response::failure(0, e.to_string()),
                    };
                    let _ = write_local_frame(&mut stream, &response);
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    });
    Ok(ServiceHandle {
        stop,
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    fn test_paths() -> Paths {
        let p = std::env::temp_dir().join(format!(
            "voice-engine-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        Paths {
            data: p.join("data"),
            socket: p.join("runtime/control.sock"),
        }
    }
    #[test]
    fn session_socket_persistence_and_exclusive_owner() -> Result<()> {
        let paths = test_paths();
        let mut server = spawn_service(paths.clone())?;
        #[cfg(unix)]
        assert_eq!(fs::metadata(&paths.socket)?.mode() & 0o777, 0o600);
        let status = crate::client::send(&paths.socket, Command::Status)?;
        assert_eq!(status["monitor"], false);
        assert_eq!(status["audio_running"], false);
        assert!(spawn_service(paths.clone()).is_err());
        let settings = DspSettings {
            gain_db: 6.0,
            ..Default::default()
        };
        crate::client::send(&paths.socket, Command::Configure { settings })?;
        server.stop()?;
        assert!(crate::client::send(&paths.socket, Command::Status).is_err());
        #[cfg(unix)]
        assert!(!paths.socket.exists());
        let mut server = spawn_service(paths.clone())?;
        let status = crate::client::send(&paths.socket, Command::Status)?;
        assert_eq!(status["settings"]["gain_db"], 6.0);
        assert!(
            crate::client::send(
                &paths.socket,
                Command::SaveReplay {
                    path: paths.data.join("empty.wav"),
                    seconds: 30,
                    source: RecordingSource::ProcessedVoice,
                }
            )
            .is_err()
        );
        server.stop()?;
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
    #[test]
    fn malformed_control_frame_returns_error_and_preserves_service() -> Result<()> {
        let paths = test_paths();
        let mut server = spawn_service(paths.clone())?;
        let mut stream = crate::platform::LocalConnection::connect(&paths.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.write_all(b"invalid JSON\n")?;
        let response: Response =
            serde_json::from_slice(&read_local_frame(&mut stream, Duration::from_secs(3))?)?;
        assert!(!response.ok);
        assert_eq!(response.id, 0);
        drop(stream);
        assert_eq!(
            crate::client::send(&paths.socket, Command::Status)?["audio_running"],
            false
        );
        server.stop()?;
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    #[test]
    fn remove_clip_stops_only_its_mix_and_preview_instances_and_persists() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let target = engine.library.save_clip(
            &[0.1; 480],
            voice_library::DEFAULT_BOARD_ID,
            "Remove this sound",
        )?;
        let other = engine.library.save_clip(
            &[0.2; 480],
            voice_library::DEFAULT_BOARD_ID,
            "Keep this sound",
        )?;
        let target_bytes = fs::read(&target.path)?;
        let other_bytes = fs::read(&other.path)?;
        let stop = AtomicBool::new(false);
        assert_eq!(
            serde_json::to_value(Command::RemoveClip {
                id: target.id.clone(),
            })?,
            json!({"op":"remove_clip","id":target.id})
        );
        // Only the synthetic producer-ready flag is enabled: no device/backend
        // is opened, and this test is the sole consumer of the prepared queues.
        engine.shared.metrics.active.store(true, Ordering::Release);
        for (id, preview) in [
            (&target.id, false),
            (&target.id, false),
            (&target.id, true),
            (&other.id, false),
            (&other.id, true),
        ] {
            engine.execute(
                Command::Play {
                    id: id.clone(),
                    preview,
                    looped: true,
                    overlap: true,
                    gain: 1.0,
                },
                &stop,
            )?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while engine.worker.playing.load(Ordering::Relaxed) != 5 {
            assert!(
                Instant::now() < deadline,
                "worker did not start all test voices"
            );
            thread::sleep(Duration::from_millis(1));
        }
        for id in ["../invalid.wav", "clip.missing"] {
            assert!(
                engine
                    .execute(Command::RemoveClip { id: id.into() }, &stop)
                    .is_err()
            );
            assert_eq!(engine.worker.playing.load(Ordering::Relaxed), 5);
        }
        let result = engine.execute(
            Command::RemoveClip {
                id: target.id.clone(),
            },
            &stop,
        )?;
        assert_eq!(result, json!({"removed":true,"id":target.id}));
        // The worker acknowledgment follows both mixer stops and the count update.
        assert_eq!(engine.worker.playing.load(Ordering::Relaxed), 2);
        assert!(engine.library.clip(&target.id).is_err());
        assert_eq!(
            engine.library.clips(voice_library::DEFAULT_BOARD_ID)?,
            vec![other.clone()]
        );
        // Leave the existing bounded mixed tail intact, then consume it as the
        // synthetic callback would. Newly produced audio contains the other ID.
        let mut tail = [0.0; 960];
        assert_eq!(engine.shared.soundboard.pop(&mut tail), 960);
        assert_eq!(engine.shared.preview.pop(&mut tail), 960);
        let deadline = Instant::now() + Duration::from_secs(3);
        while engine.shared.soundboard.len() < 480 || engine.shared.preview.len() < 480 {
            assert!(
                Instant::now() < deadline,
                "worker did not resume the remaining voices"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let mut samples = [0.0; 480];
        for queue in [&engine.shared.soundboard, &engine.shared.preview] {
            assert_eq!(queue.pop(&mut samples), samples.len());
            assert!(samples.iter().all(|sample| (*sample - 0.2).abs() < 0.00001));
        }
        assert_eq!(engine.shared.metrics.frames.load(Ordering::Relaxed), 0);
        assert!(
            engine
                .execute(
                    Command::RemoveClip {
                        id: target.id.clone()
                    },
                    &stop
                )
                .is_err()
        );
        assert_eq!(engine.worker.playing.load(Ordering::Relaxed), 2);
        assert_eq!(fs::read(&target.path)?, target_bytes);
        assert_eq!(fs::read(&other.path)?, other_bytes);
        drop(engine);

        let reopened = Engine::new(paths.clone())?;
        assert!(reopened.library.clip(&target.id).is_err());
        assert_eq!(
            reopened.library.clips(voice_library::DEFAULT_BOARD_ID)?,
            vec![other.clone()]
        );
        assert_eq!(fs::read(&target.path)?, target_bytes);
        assert_eq!(fs::read(&other.path)?, other_bytes);
        drop(reopened);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
    #[test]
    fn preset_image_commands_persist_without_changing_the_audio_session() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let stop = AtomicBool::new(false);
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            0
        );
        engine.execute(
            Command::SavePreset {
                id: "custom.image".into(),
                name: "Custom image".into(),
                settings: DspSettings::default(),
            },
            &stop,
        )?;
        let baseline = serde_json::to_value(&engine.session)?;
        let source = paths.data.join("source.png");
        // One original RGBA pixel, encoded as a complete PNG fixture.
        fs::write(
            &source,
            [
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 80,
                112, 104, 248, 15, 0, 3, 68, 1, 224, 50, 170, 191, 132, 0, 0, 0, 0, 73, 69, 78, 68,
                174, 66, 96, 130,
            ],
        )?;
        assert!(
            engine
                .execute(
                    Command::PresetImage {
                        id: "custom.image".into()
                    },
                    &stop
                )?
                .is_null()
        );
        let imported = engine.execute(
            Command::SetPresetImage {
                id: "custom.image".into(),
                path: source.clone(),
            },
            &stop,
        )?;
        let managed = PathBuf::from(imported["path"].as_str().context("managed image path")?);
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            1
        );
        assert!(managed.exists());
        assert_eq!(imported["preset_id"], "custom.image");
        assert_eq!(baseline, serde_json::to_value(&engine.session)?);
        assert!(
            engine
                .execute(
                    Command::SetPresetImage {
                        id: "builtin.clean".into(),
                        path: source.clone()
                    },
                    &stop
                )
                .is_err()
        );
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            1
        );
        engine.execute(
            Command::ClearPresetImage {
                id: "custom.image".into(),
            },
            &stop,
        )?;
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            2
        );
        assert!(!managed.exists());
        // Restore an image so the same test also covers persistence across engine restart.
        let imported = engine.execute(
            Command::SetPresetImage {
                id: "custom.image".into(),
                path: source.clone(),
            },
            &stop,
        )?;
        let managed = PathBuf::from(imported["path"].as_str().context("managed image path")?);
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            3
        );
        drop(engine);
        fs::remove_file(source)?;
        let mut engine = Engine::new(paths.clone())?;
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            0
        );
        assert_eq!(
            engine.execute(
                Command::PresetImage {
                    id: "custom.image".into()
                },
                &stop
            )?,
            imported
        );
        assert_eq!(
            engine.execute(
                Command::ClearPresetImage {
                    id: "custom.image".into()
                },
                &stop
            )?["cleared"],
            true
        );
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            1
        );
        assert!(
            engine
                .execute(
                    Command::ClearPresetImage {
                        id: "builtin.clean".into()
                    },
                    &stop
                )
                .is_err()
        );
        assert_eq!(
            engine.execute(Command::Status, &stop)?["preset_image_revision"],
            1
        );
        assert!(!managed.exists());
        assert!(
            engine
                .execute(
                    Command::PresetImage {
                        id: "custom.image".into()
                    },
                    &stop
                )?
                .is_null()
        );
        assert_eq!(baseline, serde_json::to_value(&engine.session)?);
        drop(engine);
        fs::remove_dir_all(paths.data.parent().context("test root")?)?;
        Ok(())
    }
    #[test]
    fn voice_blend_and_effect_intensity_roundtrip_and_restore_the_selected_voice() -> Result<()> {
        use voice_core::{EffectKind, VoiceBlend, VoiceLayer};

        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let stop = AtomicBool::new(false);
        let mut secondary = DspSettings {
            pitch_semitones: 2.0,
            robot_mix: 0.3,
            ..Default::default()
        };
        secondary.effect_mix[EffectKind::Gate as usize] = 0.65;
        let mut settings = DspSettings {
            gain_db: -2.0,
            pitch_semitones: -3.5,
            blend: Some(VoiceBlend {
                secondary: VoiceLayer::from(secondary),
                primary_mix: 0.6,
                secondary_mix: 0.4,
            }),
            ..Default::default()
        };
        settings.effect_mix[EffectKind::Filter as usize] = 0.45;
        settings.effect_mix[EffectKind::Compressor as usize] = 0.25;
        engine.execute(Command::Configure { settings }, &stop)?;
        assert_eq!(engine.shared.dsp.load(), settings);
        let status = engine.execute(Command::Status, &stop)?;
        assert_eq!(
            serde_json::from_value::<DspSettings>(status["settings"].clone())?,
            settings
        );
        for capability in ["preset_images", "voice_blend", "effect_intensity"] {
            assert_eq!(status["capabilities"][capability], true);
        }
        let id = "custom.blended";
        let saved = engine.execute(
            Command::SavePreset {
                id: id.into(),
                name: "Two voices".into(),
                settings,
            },
            &stop,
        )?;
        assert_eq!(serde_json::from_value::<Preset>(saved)?.settings, settings);
        let export = paths.data.join("two-voices.json");
        engine.execute(
            Command::ExportPreset {
                id: id.into(),
                path: export.clone(),
            },
            &stop,
        )?;
        engine.execute(
            Command::SavePreset {
                id: id.into(),
                name: "Temporary overwrite".into(),
                settings: DspSettings::default(),
            },
            &stop,
        )?;
        let imported = engine.execute(Command::ImportPreset { path: export }, &stop)?;
        assert_eq!(
            serde_json::from_value::<Preset>(imported)?.settings,
            settings
        );
        engine.execute(Command::SelectPreset { id: id.into() }, &stop)?;
        assert_eq!(engine.session.settings, settings);
        assert_eq!(engine.session.selected_preset.as_deref(), Some(id));
        let persisted = fs::read(paths.data.join("session.json"))?;
        let mut invalid = settings;
        invalid.effect_mix[0] = 1.1;
        assert!(
            engine
                .execute(Command::Configure { settings: invalid }, &stop)
                .is_err()
        );
        let mut invalid = settings;
        invalid.blend.as_mut().expect("blend fixture").secondary_mix = f32::NAN;
        assert!(
            engine
                .execute(Command::Configure { settings: invalid }, &stop)
                .is_err()
        );
        assert_eq!(fs::read(paths.data.join("session.json"))?, persisted);
        assert_eq!(engine.shared.dsp.load(), settings);
        drop(engine);

        let mut engine = Engine::new(paths.clone())?;
        let status = engine.execute(Command::Status, &stop)?;
        assert_eq!(status["selected_preset"], id);
        assert_eq!(
            serde_json::from_value::<DspSettings>(status["settings"].clone())?,
            settings
        );
        assert_eq!(engine.shared.dsp.load(), settings);
        assert_eq!(status["audio_requested"], false);
        assert_eq!(status["audio_running"], false);
        drop(engine);
        fs::remove_dir_all(paths.data.parent().context("test root")?)?;
        Ok(())
    }
    #[test]
    fn effect_order_commands_persist_and_invalid_orders_leave_session_untouched() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let stop = AtomicBool::new(false);
        let mut settings = DspSettings {
            gain_db: 6.0,
            drive: 0.4,
            ..Default::default()
        };
        settings.effect_order.0.swap(
            voice_core::EffectKind::Gain as usize,
            voice_core::EffectKind::Distortion as usize,
        );
        engine.execute(Command::Configure { settings }, &stop)?;
        assert_eq!(engine.shared.dsp.load(), settings);
        engine.execute(
            Command::SavePreset {
                id: "custom.ordered".into(),
                name: "Ordered voice".into(),
                settings,
            },
            &stop,
        )?;
        engine.execute(
            Command::Configure {
                settings: DspSettings::default(),
            },
            &stop,
        )?;
        engine.execute(
            Command::SelectPreset {
                id: "custom.ordered".into(),
            },
            &stop,
        )?;
        assert_eq!(engine.shared.dsp.load(), settings);
        let baseline = serde_json::to_value(&engine.session)?;
        let persisted = fs::read(paths.data.join("session.json"))?;
        let mut invalid = settings;
        invalid.effect_order.0[0] = invalid.effect_order.0[1];
        for command in [
            Command::Configure { settings: invalid },
            Command::SavePreset {
                id: "custom.ordered".into(),
                name: "Must not replace the preset".into(),
                settings: invalid,
            },
        ] {
            assert!(engine.execute(command, &stop).is_err());
            assert_eq!(serde_json::to_value(&engine.session)?, baseline);
            assert_eq!(engine.shared.dsp.load(), settings);
            assert_eq!(fs::read(paths.data.join("session.json"))?, persisted);
        }
        let saved = engine
            .library
            .presets()?
            .into_iter()
            .find(|preset| preset.id == "custom.ordered")
            .unwrap();
        assert_eq!(saved.name, "Ordered voice");
        assert_eq!(saved.settings, settings);
        drop(engine);
        let mut reopened = Engine::new(paths.clone())?;
        assert_eq!(reopened.session.settings, settings);
        assert_eq!(reopened.shared.dsp.load(), settings);
        assert_eq!(
            reopened.session.selected_preset.as_deref(),
            Some("custom.ordered")
        );
        let status = reopened.execute(Command::Status, &stop)?;
        assert_eq!(status["capabilities"]["reorder_effects"], true);
        assert_eq!(
            status["settings"]["effect_order"],
            serde_json::to_value(settings.effect_order)?
        );
        assert!(!reopened.shared.metrics.active.load(Ordering::Acquire));
        drop(reopened);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
    #[test]
    fn version_one_session_without_effect_order_loads_the_original_chain() -> Result<()> {
        let paths = test_paths();
        private_directory(&paths.data)?;
        let original = br#"{"version":1,"settings":{"gain_db":4.0,"drive":0.25}}"#;
        fs::write(paths.data.join("session.json"), original)?;
        let engine = Engine::new(paths.clone())?;
        assert_eq!(
            engine.session.settings.effect_order,
            voice_core::EffectOrder::default()
        );
        assert_eq!(
            engine.shared.dsp.load().effect_order,
            voice_core::EffectOrder::default()
        );
        assert_eq!(engine.session.settings.gain_db, 4.0);
        assert_eq!(fs::read(paths.data.join("session.json"))?, original);
        assert!(!engine.shared.metrics.active.load(Ordering::Acquire));
        drop(engine);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
    #[test]
    fn saved_routes_keep_automatic_intent_and_replace_transient_ids_with_names() {
        let config = AudioConfig {
            input_device: Some("52".into()),
            output_device: None,
            ..Default::default()
        };
        let resolved = AudioConfig {
            input_device: Some("usb-microphone".into()),
            output_device: Some("headphones".into()),
            ..Default::default()
        };
        let saved = stable_audio_preferences(&config, &resolved);
        assert_eq!(saved.input_device.as_deref(), Some("usb-microphone"));
        assert!(saved.output_device.is_none());
    }
    #[test]
    fn stop_disables_capture_and_retries_even_when_session_cannot_be_saved() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        engine.session.audio_enabled = true;
        engine.shared.metrics.active.store(true, Ordering::Release);
        let path = paths.data.join("stop-with-full-disk.wav");
        record_on_worker(&engine.worker, path.clone())?;
        engine.shared.recording.push(&[0.1, 0.2]);
        fs::create_dir(paths.data.join("session.json.tmp"))?;
        let result = engine.execute(Command::Stop, &AtomicBool::new(false));
        assert!(result.is_err());
        assert!(!engine.session.audio_enabled);
        assert!(!engine.shared.metrics.active.load(Ordering::Acquire));
        assert!(!engine.worker.recording.load(Ordering::Acquire));
        assert_eq!(wav_samples(&path)?.len(), 2);
        engine.check_audio_health();
        assert!(engine.audio.is_none());
        drop(engine);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
    #[test]
    fn connection_intent_is_persisted_but_stop_cancels_it_without_starting_capture() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        assert!(!engine.session.audio_enabled);
        let mut candidate = engine.session.clone();
        candidate.audio_enabled = true;
        candidate.config.input_device = Some("saved-usb-microphone".into());
        candidate.config.output_device = Some("saved-headphones".into());
        candidate.monitor_gain = 0.35;
        engine.commit_session(candidate)?;
        drop(engine);

        // Engine::new only loads intent; the service loop owns discovery/recovery.
        // Stop must be able to cancel that saved intent before any hardware is opened.
        let mut engine = Engine::new(paths.clone())?;
        assert!(engine.session.audio_enabled);
        assert!(engine.audio.is_none());
        assert_eq!(engine.session.monitor_gain, 0.35);
        assert!(!engine.shared.monitor_enabled.load(Ordering::Relaxed));
        engine.execute(Command::Stop, &AtomicBool::new(false))?;
        assert!(!engine.session.audio_enabled);
        assert!(engine.audio.is_none());
        drop(engine);
        let engine = Engine::new(paths.clone())?;
        assert!(!engine.session.audio_enabled);
        assert_eq!(
            engine.session.config.input_device.as_deref(),
            Some("saved-usb-microphone")
        );
        assert_eq!(
            engine.session.config.output_device.as_deref(),
            Some("saved-headphones")
        );
        drop(engine);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
    #[test]
    fn reject_future_session_without_overwrite() -> Result<()> {
        let paths = test_paths();
        private_directory(&paths.data)?;
        let bytes = b"{\"version\":999}";
        fs::write(paths.data.join("session.json"), bytes)?;
        assert!(spawn_service(paths.clone()).is_err());
        assert_eq!(fs::read(paths.data.join("session.json"))?, bytes);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    fn record_on_worker(worker: &Worker, path: PathBuf) -> Result<()> {
        let (reply, received) = mpsc::channel();
        worker.send(WorkerCommand::Record {
            path,
            source: RecordingSource::ProcessedVoice,
            reply,
        })?;
        received.recv_timeout(Duration::from_secs(3))??;
        Ok(())
    }

    fn replay_from_worker(worker: &Worker, path: PathBuf) -> Result<Value> {
        let (reply, received) = mpsc::channel();
        worker.send(WorkerCommand::Replay {
            path,
            seconds: 30,
            source: RecordingSource::ProcessedVoice,
            reply,
        })?;
        received.recv_timeout(Duration::from_secs(3))?
    }

    fn wav_samples(path: &Path) -> Result<Vec<i16>> {
        Ok(hound::WavReader::open(path)?
            .into_samples::<i16>()
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    #[test]
    fn worker_finish_includes_tail_and_sanitizes_non_finite_audio() -> Result<()> {
        let paths = test_paths();
        private_directory(&paths.data)?;
        let shared = Arc::new(AudioShared::default());
        let worker = Worker::spawn(shared.clone(), Arc::new(DesktopShared::default()));
        let recorded = paths.data.join("recorded.wav");
        record_on_worker(&worker, recorded.clone())?;
        assert_eq!(
            shared
                .recording
                .push(&[0.25, f32::NAN, f32::INFINITY, -2.0, 2.0]),
            5
        );
        let (reply, received) = mpsc::channel();
        worker.send(WorkerCommand::Finish { reply })?;
        assert_eq!(received.recv_timeout(Duration::from_secs(3))??["frames"], 5);
        assert!(!worker.recording.load(Ordering::Acquire));
        assert_eq!(wav_samples(&recorded)?, [8191, 0, 0, -32767, 32767]);
        worker.reset(false)?;
        let replay = paths.data.join("replay.wav");
        assert_eq!(replay_from_worker(&worker, replay.clone())?["frames"], 5);
        assert_eq!(wav_samples(&replay)?, wav_samples(&recorded)?);
        // A new session discards every old replay sample and acknowledges before Start.
        shared.recording.push(&[0.5]);
        worker.reset(true)?;
        assert!(shared.recording.is_empty());
        assert!(replay_from_worker(&worker, paths.data.join("empty.wav")).is_err());
        drop(worker);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    #[test]
    fn worker_drop_drains_and_finalizes_recording() -> Result<()> {
        let paths = test_paths();
        private_directory(&paths.data)?;
        let shared = Arc::new(AudioShared::default());
        let worker = Worker::spawn(shared.clone(), Arc::new(DesktopShared::default()));
        let recording = worker.recording.clone();
        let path = paths.data.join("drop.wav");
        record_on_worker(&worker, path.clone())?;
        let samples = [0.1; 1024];
        assert_eq!(shared.recording.push(&samples), samples.len());
        drop(worker);
        assert_eq!(wav_samples(&path)?.len(), samples.len());
        assert!(!recording.load(Ordering::Acquire));
        #[cfg(unix)]
        assert_eq!(fs::metadata(&path)?.mode() & 0o777, 0o600);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    #[test]
    fn stop_and_shutdown_finalize_recording_and_preserve_replay() -> Result<()> {
        for shutdown in [false, true] {
            let paths = test_paths();
            let mut engine = Engine::new(paths.clone())?;
            let stop = AtomicBool::new(false);
            engine.shared.metrics.active.store(true, Ordering::Release);
            let path = paths.data.join("capture.wav");
            engine.execute(
                Command::StartRecording {
                    path: path.clone(),
                    source: RecordingSource::ProcessedVoice,
                },
                &stop,
            )?;
            assert_eq!(engine.shared.recording.push(&[0.2; 512]), 512);
            engine.execute(
                if shutdown {
                    Command::Shutdown
                } else {
                    Command::Stop
                },
                &stop,
            )?;
            assert_eq!(stop.load(Ordering::Acquire), shutdown);
            assert!(!engine.worker.recording.load(Ordering::Acquire));
            assert!(!engine.shared.metrics.active.load(Ordering::Acquire));
            assert_eq!(wav_samples(&path)?.len(), 512);
            let replay = paths.data.join("stopped-replay.wav");
            assert_eq!(
                engine.execute(
                    Command::SaveReplay {
                        path: replay.clone(),
                        seconds: 30,
                        source: RecordingSource::ProcessedVoice,
                    },
                    &stop
                )?["frames"],
                512
            );
            assert_eq!(wav_samples(&replay)?, wav_samples(&path)?);
            drop(engine);
            fs::remove_dir_all(paths.data.parent().unwrap())?;
        }
        Ok(())
    }

    #[test]
    fn persistence_failure_changes_neither_session_nor_live_controls() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let stop = AtomicBool::new(false);
        engine.execute(
            Command::SelectPreset {
                id: "builtin.radio".into(),
            },
            &stop,
        )?;
        let baseline = serde_json::to_value(&engine.session)?;
        let persisted = fs::read(paths.data.join("session.json"))?;
        // Deterministic failure also when tests run as root: opening a directory as
        // the temporary session file fails without relying on Unix write permissions.
        fs::create_dir(paths.data.join("session.json.tmp"))?;
        for command in [
            Command::Configure {
                settings: DspSettings {
                    gain_db: 12.0,
                    ..Default::default()
                },
            },
            Command::SelectPreset {
                id: "builtin.robot".into(),
            },
            Command::SetControls {
                bypass: Some(true),
                muted: Some(true),
                monitor: Some(true),
                monitor_gain: Some(1.5),
                soundboard_gain: Some(0.3),
            },
        ] {
            assert!(engine.execute(command, &stop).is_err());
            assert_eq!(serde_json::to_value(&engine.session)?, baseline);
            assert_eq!(engine.shared.dsp.load(), engine.session.settings);
            assert!(!engine.shared.bypass.load(Ordering::Relaxed));
            assert!(!engine.shared.muted.load(Ordering::Relaxed));
            assert!(!engine.shared.monitor_enabled.load(Ordering::Relaxed));
            assert_eq!(
                f32::from_bits(engine.shared.monitor_gain.load(Ordering::Relaxed)),
                0.5
            );
            assert_eq!(
                f32::from_bits(engine.shared.soundboard_gain.load(Ordering::Relaxed)),
                1.0
            );
            assert_eq!(fs::read(paths.data.join("session.json"))?, persisted);
        }
        fs::remove_dir(paths.data.join("session.json.tmp"))?;
        engine.execute(
            Command::SetControls {
                bypass: Some(true),
                muted: Some(true),
                monitor: Some(true),
                monitor_gain: Some(1.5),
                soundboard_gain: Some(0.3),
            },
            &stop,
        )?;
        assert!(engine.shared.bypass.load(Ordering::Relaxed));
        assert!(engine.shared.monitor_enabled.load(Ordering::Relaxed));
        assert_eq!(engine.session.monitor_gain, 1.5);
        drop(engine);
        let restarted = Engine::new(paths.clone())?;
        assert!(restarted.shared.bypass.load(Ordering::Relaxed));
        assert!(!restarted.shared.monitor_enabled.load(Ordering::Relaxed));
        assert_eq!(restarted.session.monitor_gain, 1.5);
        drop(restarted);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    #[test]
    fn invalid_controls_and_config_leave_recording_untouched() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let stop = AtomicBool::new(false);
        engine.shared.metrics.active.store(true, Ordering::Release);
        let path = paths.data.join("valid.wav");
        engine.execute(
            Command::StartRecording {
                path: path.clone(),
                source: RecordingSource::ProcessedVoice,
            },
            &stop,
        )?;
        for command in [
            Command::SetControls {
                bypass: Some(true),
                muted: Some(true),
                monitor: Some(true),
                monitor_gain: Some(f32::NAN),
                soundboard_gain: None,
            },
            Command::Configure {
                settings: DspSettings {
                    gain_db: f32::INFINITY,
                    ..Default::default()
                },
            },
            Command::Start {
                config: AudioConfig {
                    sample_rate: 0,
                    ..Default::default()
                },
            },
        ] {
            assert!(engine.execute(command, &stop).is_err());
            assert!(engine.worker.recording.load(Ordering::Acquire));
            assert!(engine.shared.metrics.active.load(Ordering::Acquire));
            assert!(!engine.shared.bypass.load(Ordering::Relaxed));
            assert!(!engine.shared.monitor_enabled.load(Ordering::Relaxed));
            assert_eq!(engine.shared.dsp.load(), DspSettings::default());
        }
        engine.shared.recording.push(&[0.1, 0.2]);
        drop(engine);
        assert_eq!(wav_samples(&path)?.len(), 2);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    #[test]
    fn desktop_recording_is_stereo_isolated_and_survives_microphone_stop() -> Result<()> {
        let paths = test_paths();
        let mut engine = Engine::new(paths.clone())?;
        let stop = AtomicBool::new(false);
        assert_eq!(
            engine.execute(Command::Status, &stop)?["desktop"]["active"],
            false
        );
        // Fake only the backend-ready flag; no user audio graph is opened by this test.
        engine.desktop_shared.active.store(true, Ordering::Release);
        let path = paths.data.join("desktop.wav");
        engine.execute(
            Command::StartRecording {
                path: path.clone(),
                source: RecordingSource::Desktop,
            },
            &stop,
        )?;
        assert!(
            engine
                .execute(
                    Command::StartRecording {
                        path: paths.data.join("second.wav"),
                        source: RecordingSource::Desktop
                    },
                    &stop
                )
                .is_err()
        );
        assert!(!paths.data.join("second.wav").exists());
        engine.shared.recording.push(&[0.9; 128]);
        engine
            .desktop_shared
            .samples
            .push(&[0.25, -0.5, f32::NAN, f32::INFINITY]);
        engine.execute(Command::Stop, &stop)?;
        let status = engine.execute(Command::Status, &stop)?;
        assert_eq!(status["recording_source"], "desktop");
        assert_eq!(status["desktop"]["active"], true);
        engine.execute(Command::StopDesktopCapture, &stop)?;
        let reader = hound::WavReader::open(&path)?;
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.duration(), 2);
        assert_eq!(wav_samples(&path)?, [8191, -16383, 0, 0]);
        let replay = paths.data.join("desktop-replay.wav");
        assert_eq!(
            engine.execute(
                Command::SaveReplay {
                    path: replay.clone(),
                    seconds: 30,
                    source: RecordingSource::Desktop
                },
                &stop
            )?["frames"],
            2
        );
        assert_eq!(wav_samples(&replay)?, wav_samples(&path)?);
        assert_eq!(
            engine.execute(Command::Status, &stop)?["recording_source"],
            Value::Null
        );
        drop(engine);
        let mut reopened = Engine::new(paths.clone())?;
        assert_eq!(
            reopened.execute(Command::Status, &stop)?["desktop"]["active"],
            false
        );
        drop(reopened);
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }

    #[test]
    fn legacy_record_and_replay_commands_keep_processed_source() -> Result<()> {
        let record: Command = serde_json::from_str(r#"{"op":"start_recording","path":"x.wav"}"#)?;
        assert!(matches!(
            record,
            Command::StartRecording {
                source: RecordingSource::ProcessedVoice,
                ..
            }
        ));
        let replay: Command =
            serde_json::from_str(r#"{"op":"save_replay","path":"x.wav","seconds":30}"#)?;
        assert!(matches!(
            replay,
            Command::SaveReplay {
                source: RecordingSource::ProcessedVoice,
                ..
            }
        ));
        assert!(
            serde_json::from_str::<Command>(
                r#"{"op":"start_recording","path":"x.wav","source":"microphone"}"#
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn recording_finalization_failure_is_visible_and_clears_active_flag() -> Result<()> {
        let paths = test_paths();
        private_directory(&paths.data)?;
        let path = paths.data.join("readonly.wav");
        fs::write(&path, b"original")?;
        let recording = Arc::new(AtomicBool::new(true));
        let error = Arc::new(std::sync::Mutex::new(None));
        let mut capture = CaptureState::new(
            RecordingSource::ProcessedVoice,
            recording.clone(),
            Arc::new(AtomicUsize::new(1)),
            Arc::new(AtomicUsize::new(0)),
            error.clone(),
        );
        // Buffering accepts the header locally; flushing the read-only FD fails.
        capture.writer = Some(hound::WavWriter::new(
            std::io::BufWriter::new(File::open(&path)?),
            hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?);
        assert!(capture.finish(true).is_err());
        assert!(!recording.load(Ordering::Acquire));
        assert!(capture.writer.is_none());
        assert!(error.lock().unwrap().is_some());
        assert_eq!(fs::read(&path)?, b"original");
        assert!(new_recording(&path, 1).is_err());
        fs::remove_dir_all(paths.data.parent().unwrap())?;
        Ok(())
    }
}
