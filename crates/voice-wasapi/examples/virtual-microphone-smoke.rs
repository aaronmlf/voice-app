//! Explicit installed-endpoint acceptance probe. Never opens a physical microphone or headphones.
//! For VB-CABLE, run on Windows with: cargo run -p voice-wasapi --example
//! virtual-microphone-smoke -- --external-cable --virtual-output EXACT_ID --report NEW_PATH.json
//! A missing, ambiguous or unsupported pair is an unsuccessful result, never a pass.
//! The original --report-only invocation remains for the legacy first-party driver.

// Reuse the exact prepared publisher exercised by the application. Excluding this
// module from example test harnesses avoids duplicating its unit tests/allocator.
#[cfg(all(windows, not(test)))]
#[allow(dead_code)]
#[path = "../src/rt.rs"]
mod production_rt;

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "virtual-microphone-smoke requires native Windows and a validated installed endpoint pair; VB-CABLE requires --external-cable --virtual-output EXACT_ID --report NEW_PATH.json"
    );
    std::process::exit(2);
}

#[cfg(all(windows, test))]
fn main() {}

#[cfg(all(windows, not(test)))]
fn main() {
    std::process::exit(native::run());
}

#[cfg(all(windows, not(test)))]
mod native {
    use crate::production_rt::{INPUT_QUEUE, MAX_BLOCK, MONITOR_QUEUE, VoiceOutput};
    use anyhow::{Context, Result, ensure};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{BufferSize, Device, DeviceId, Host, HostId, SampleFormat, Stream, StreamConfig};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    };
    use std::time::{Duration, Instant};
    use voice_core::{AudioShared, DspSettings, EffectKind, SAMPLE_RATE, SampleQueue};

    const RATE: usize = SAMPLE_RATE as usize;
    const CAPTURE_FRAMES: usize = RATE * 5;
    const CAPTURE_QUEUE_SAMPLES: usize = 32768;
    const DRAIN_SAMPLES: usize = 32768;
    const BASELINE_FRAMES: u64 = SAMPLE_RATE as u64 / 4;
    const SIGNAL_FRAMES: u64 = SAMPLE_RATE as u64 * 2;
    const POST_STOP_FRAMES: u64 = SAMPLE_RATE as u64;
    const WINDOW_OFFSET: usize = RATE * 3 / 4;
    const WINDOW_FRAMES: usize = RATE;
    const MAX_ALIGNMENT: usize = RATE / 2;
    const VOICE_HZ: f64 = 503.0;
    const SOUNDBOARD_HZ: f64 = 997.0;
    const PREVIEW_HZ: f64 = 1777.0;
    const VOICE_AMPLITUDE: f64 = 0.12;
    const SOUNDBOARD_AMPLITUDE: f64 = 0.07;
    const PREVIEW_AMPLITUDE: f64 = 0.19;
    const GAIN_DB: f32 = -6.0;
    const SOUNDBOARD_GAIN: f32 = 0.75;
    const SILENCE_PEAK_LIMIT: f64 = 0.001;
    const SILENCE_RMS_LIMIT: f64 = 0.0001;

    #[derive(Default)]
    struct Progress {
        captured_frames: AtomicU64,
        rendered_frames: AtomicU64,
        first_render_capture_frame: AtomicU64,
        callback_error: AtomicU32,
    }

    fn fail(progress: &Progress, running: &AtomicBool, code: u32) {
        let _ =
            progress
                .callback_error
                .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
        running.store(false, Ordering::Release);
    }

    struct Waves {
        voice: Box<[f32]>,
        soundboard: Box<[f32]>,
        preview: Box<[f32]>,
    }

    impl Waves {
        fn new() -> Self {
            let wave = |hz: f64, amplitude: f64| {
                (0..RATE)
                    .map(|frame| {
                        (amplitude
                            * (std::f64::consts::TAU * hz * frame as f64 / RATE as f64).sin())
                            as f32
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            };
            Self {
                voice: wave(VOICE_HZ, VOICE_AMPLITUDE),
                soundboard: wave(SOUNDBOARD_HZ, SOUNDBOARD_AMPLITUDE),
                preview: wave(PREVIEW_HZ, PREVIEW_AMPLITUDE),
            }
        }
    }

    /// This callback owns all synthetic producers and the production VoiceOutput.
    /// No hardware capture is used. Every sample array is prepared before stream creation.
    struct SyntheticPublisher {
        publisher: VoiceOutput,
        shared: Arc<AudioShared>,
        input: Arc<SampleQueue>,
        waves: Waves,
        running: Arc<AtomicBool>,
        progress: Arc<Progress>,
        frame: usize,
    }

    fn push_wave(queue: &SampleQueue, wave: &[f32], offset: usize, frames: usize) -> bool {
        let first = frames.min(wave.len() - offset);
        queue.push(&wave[offset..offset + first]) == first
            && (first == frames || queue.push(&wave[..frames - first]) == frames - first)
    }

    impl SyntheticPublisher {
        fn render(&mut self, stereo: &mut [f32]) {
            stereo.fill(0.);
            if !self.running.load(Ordering::Acquire) || stereo.is_empty() {
                return;
            }
            if !stereo.len().is_multiple_of(2) {
                fail(&self.progress, &self.running, 6);
                return;
            }
            let _ = self.progress.first_render_capture_frame.compare_exchange(
                u64::MAX,
                self.progress.captured_frames.load(Ordering::Acquire),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            for output in stereo.chunks_mut(MAX_BLOCK * 2) {
                let frames = output.len() / 2;
                let offset = self.frame % RATE;
                let code = if !push_wave(&self.input, &self.waves.voice, offset, frames) {
                    1
                } else if !push_wave(
                    &self.shared.soundboard,
                    &self.waves.soundboard,
                    offset,
                    frames,
                ) {
                    2
                } else if !push_wave(&self.shared.preview, &self.waves.preview, offset, frames) {
                    3
                } else {
                    0
                };
                if code != 0 {
                    fail(&self.progress, &self.running, code);
                    stereo.fill(0.);
                    return;
                }
                self.publisher.render_if_running(output, &self.running);
                self.frame = self.frame.wrapping_add(frames);
                self.progress
                    .rendered_frames
                    .fetch_add(frames as u64, Ordering::Release);
            }
            if !self.running.load(Ordering::Acquire) {
                stereo.fill(0.);
            }
        }
    }

    struct CaptureLog {
        samples: Box<[f32]>,
        drain: Box<[f32]>,
        used: usize,
    }

    impl CaptureLog {
        fn new() -> Self {
            Self {
                samples: vec![0.; CAPTURE_FRAMES * 2].into_boxed_slice(),
                drain: vec![0.; DRAIN_SAMPLES].into_boxed_slice(),
                used: 0,
            }
        }

        fn collect(&mut self, queue: &SampleQueue) -> Result<()> {
            // Fixed iterations also bound control-thread work if a broken device clocks too fast.
            for _ in 0..4 {
                let count = queue.pop(&mut self.drain);
                if count == 0 {
                    break;
                }
                ensure!(
                    count <= self.samples.len() - self.used,
                    "capture exceeded the fixed five-second sample buffer"
                );
                self.samples[self.used..self.used + count].copy_from_slice(&self.drain[..count]);
                self.used += count;
            }
            Ok(())
        }

        fn frames(&self, begin: usize, count: usize) -> Result<&[f32]> {
            let end = begin
                .checked_add(count)
                .and_then(|frames| frames.checked_mul(2))
                .context("capture frame range overflow")?;
            ensure!(
                end <= self.used,
                "capture did not contain the required analysis window"
            );
            Ok(&self.samples[begin * 2..end])
        }
    }

    fn discard(queue: &SampleQueue, scratch: &mut [f32]) {
        // Each queue has exactly this control-thread consumer during the probe.
        for _ in 0..32 {
            if queue.pop(scratch) == 0 {
                break;
            }
        }
    }

    fn pump_until(
        done: impl Fn() -> bool,
        deadline: Instant,
        capture: &SampleQueue,
        log: &mut CaptureLog,
        shared: &AudioShared,
        monitor: &SampleQueue,
        progress: &Progress,
    ) -> Result<()> {
        let mut scratch = [0.; MAX_BLOCK];
        loop {
            log.collect(capture)?;
            discard(&shared.recording, &mut scratch);
            discard(monitor, &mut scratch);
            let error = progress.callback_error.load(Ordering::Acquire);
            ensure!(
                error == 0,
                "stream callback failed (code {error}); measurement rejected"
            );
            ensure!(
                Instant::now() < deadline,
                "ten-second acceptance deadline exceeded"
            );
            if done() {
                // The callback publishes queue data before its frame counter. It may
                // have published between collect() and done(); acquire that tail now.
                log.collect(capture)?;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    struct Streams {
        render: Option<Stream>,
        capture: Option<Stream>,
        running: Arc<AtomicBool>,
    }

    impl Streams {
        fn stop_render(&mut self) {
            self.running.store(false, Ordering::Release);
            drop(self.render.take());
        }
    }

    impl Drop for Streams {
        fn drop(&mut self) {
            self.stop_render();
            drop(self.capture.take());
        }
    }

    fn endpoint(host: &Host, id: &str) -> Result<Device> {
        host.device_by_id(
            &id.parse::<DeviceId>()
                .context("invalid owned endpoint ID")?,
        )
        .context("the exact owned endpoint disappeared; no alternative was selected")
    }

    fn config(device: &Device, capture: bool) -> Result<StreamConfig> {
        // These queries concern the exact validated endpoint's mix format, never a default device.
        let native = if capture {
            device.default_input_config()
        } else {
            device.default_output_config()
        }
        .context("read the owned endpoint's shared mix format")?;
        ensure!(
            native.channels() == 2
                && native.sample_rate() == SAMPLE_RATE
                && native.sample_format() == SampleFormat::F32,
            "owned endpoints must expose native stereo 48000 Hz F32 shared mode"
        );
        let mut config = native.config();
        config.buffer_size = BufferSize::Fixed(256);
        Ok(config)
    }

    fn connection_timeout(deadline: Instant) -> Result<Duration> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(
            !remaining.is_zero(),
            "acceptance deadline elapsed before stream creation"
        );
        Ok(remaining.min(Duration::from_secs(1)))
    }

    #[derive(Default)]
    struct Report {
        outcome: &'static str,
        phase: &'static str,
        diagnostic: Option<String>,
        elapsed_ms: u128,
        pair_available: bool,
        owned_capture_exercised: bool,
        provider: Option<String>,
        explicit_render_selection: bool,
        external_cable_required: bool,
        captured_frames: u64,
        rendered_frames: u64,
        baseline_peak: Option<f64>,
        voice_amplitude: Option<[f64; 2]>,
        soundboard_amplitude: Option<[f64; 2]>,
        preview_amplitude: Option<[f64; 2]>,
        correlation: Option<[f64; 2]>,
        alignment_frames: Option<usize>,
        stereo_max_difference: Option<f64>,
        post_stop_peak: Option<f64>,
        post_stop_rms: Option<f64>,
        post_stop_frames: u64,
    }

    fn record_progress(report: &mut Report, progress: &Progress) {
        report.captured_frames = progress.captured_frames.load(Ordering::Acquire);
        report.rendered_frames = progress.rendered_frames.load(Ordering::Acquire);
    }

    struct Spectrum {
        cosine: [[f64; 3]; 2],
        sine: [[f64; 3]; 2],
        energy: [f64; 2],
    }

    fn analyze_signal(samples: &[f32], report: &mut Report) -> Result<()> {
        let mut spectrum = Spectrum {
            cosine: [[0.; 3]; 2],
            sine: [[0.; 3]; 2],
            energy: [0.; 2],
        };
        for (frame, stereo) in samples.as_chunks::<2>().0.iter().enumerate() {
            for (channel, sample) in stereo.iter().enumerate() {
                let value = f64::from(*sample);
                spectrum.energy[channel] += value * value;
            }
            for (index, hz) in [VOICE_HZ, SOUNDBOARD_HZ, PREVIEW_HZ]
                .into_iter()
                .enumerate()
            {
                let (sine, cosine) =
                    (std::f64::consts::TAU * hz * frame as f64 / RATE as f64).sin_cos();
                for (channel, sample) in stereo.iter().enumerate() {
                    spectrum.cosine[channel][index] += f64::from(*sample) * cosine;
                    spectrum.sine[channel][index] += f64::from(*sample) * sine;
                }
            }
        }
        let amplitudes = |index: usize| {
            std::array::from_fn::<_, 2, _>(|channel| {
                2. * spectrum.cosine[channel][index].hypot(spectrum.sine[channel][index])
                    / WINDOW_FRAMES as f64
            })
        };
        let voice = amplitudes(0);
        let soundboard = amplitudes(1);
        let preview = amplitudes(2);
        report.voice_amplitude = Some(voice);
        report.soundboard_amplitude = Some(soundboard);
        report.preview_amplitude = Some(preview);
        let expected_voice = VOICE_AMPLITUDE * 10.0_f64.powf(f64::from(GAIN_DB) / 20.);
        let expected_sound = SOUNDBOARD_AMPLITUDE * f64::from(SOUNDBOARD_GAIN);
        let expected_energy = WINDOW_FRAMES as f64
            * 0.5
            * (expected_voice * expected_voice + expected_sound * expected_sound);
        let mut best = [-1.0_f64; 2];
        let mut best_lag = 0;
        // For these integer-frequency one-second payloads, correlation with the exact
        // prepared composite follows directly from its two Fourier coefficients.
        // This is O(frames + max_lag), not O(frames * max_lag), with no FFT allocation.
        for lag in 0..=MAX_ALIGNMENT {
            let offset = WINDOW_OFFSET as f64 - lag as f64;
            let (voice_sine, voice_cosine) =
                (std::f64::consts::TAU * VOICE_HZ * offset / RATE as f64).sin_cos();
            let (sound_sine, sound_cosine) =
                (std::f64::consts::TAU * SOUNDBOARD_HZ * offset / RATE as f64).sin_cos();
            let scores = std::array::from_fn::<_, 2, _>(|channel| {
                let dot = expected_voice
                    * (spectrum.cosine[channel][0] * voice_sine
                        + spectrum.sine[channel][0] * voice_cosine)
                    + expected_sound
                        * (spectrum.cosine[channel][1] * sound_sine
                            + spectrum.sine[channel][1] * sound_cosine);
                let norm = (expected_energy * spectrum.energy[channel]).sqrt();
                if norm > 0. { dot / norm } else { 0. }
            });
            if scores[0].min(scores[1]) > best[0].min(best[1]) {
                best = scores;
                best_lag = lag;
            }
        }
        report.correlation = Some(best);
        report.alignment_frames = Some(best_lag);
        for (channel, voice_amplitude) in voice.iter().enumerate() {
            ensure!(
                (*voice_amplitude - expected_voice).abs() <= expected_voice * 0.15,
                "channel {channel}: processed voice amplitude differs from -6 dB DSP expectation"
            );
            ensure!(
                (soundboard[channel] - expected_sound).abs() <= expected_sound * 0.15,
                "channel {channel}: simultaneous soundboard amplitude differs from its 0.75 gain"
            );
            ensure!(
                preview[channel] < 0.002,
                "channel {channel}: local preview leaked into capture"
            );
            ensure!(
                best[channel] >= 0.98,
                "channel {channel}: capture did not correlate with the prepared voice/soundboard payload"
            );
        }
        Ok(())
    }

    fn peak_and_rms(samples: &[f32]) -> (f64, f64) {
        let mut peak = 0.0_f64;
        let mut energy = 0.;
        for sample in samples {
            let value = f64::from(*sample);
            peak = peak.max(value.abs());
            energy += value * value;
        }
        (peak, (energy / samples.len().max(1) as f64).sqrt())
    }

    fn exercise(deadline: Instant, options: &Options, report: &mut Report) -> Result<()> {
        report.phase = "probe_exact_installed_pair";
        let pair = voice_wasapi::virtual_microphone_status_for(options.virtual_output.as_deref())?;
        if !pair.available {
            report.outcome = "unavailable";
            report.diagnostic = pair
                .diagnostic
                .or_else(|| Some("No validated installed endpoint pair is available".into()));
            return Ok(());
        }
        report.pair_available = true;
        let render_id = pair
            .render_device
            .context("probe omitted the owned render endpoint")?;
        let capture_id = pair
            .capture_device
            .context("probe omitted the owned capture endpoint")?;
        let target = pair
            .targets
            .iter()
            .find(|target| target.render_device == render_id && target.capture_device == capture_id)
            .context("probe summary does not match a validated installed target")?;
        report.provider = Some(target.provider.clone());
        if options.external_cable {
            ensure!(
                target.provider == "vb_cable",
                "--external-cable requires a validated VB-CABLE target; no other provider is opened"
            );
        } else {
            ensure!(
                target.provider == "voice_app",
                "the legacy invocation accepts only the Voice App provider; VB-CABLE requires --external-cable and an explicit --virtual-output ID"
            );
        }
        if let Some(selected) = &options.virtual_output {
            ensure!(
                render_id == *selected,
                "probe substituted the explicitly selected render endpoint"
            );
        }
        ensure!(
            render_id != capture_id,
            "render/capture endpoint IDs must be distinct"
        );
        report.phase = "prepare_synthetic_audio";
        let host = cpal::host_from_id(HostId::Wasapi)?;
        let render_device = endpoint(&host, &render_id)?;
        let capture_device = endpoint(&host, &capture_id)?;
        let render_config = config(&render_device, false)?;
        let capture_config = config(&capture_device, true)?;
        let shared = Arc::new(AudioShared::default());
        let settings = DspSettings {
            gain_db: GAIN_DB,
            effect_mix: std::array::from_fn(|index| {
                if index == EffectKind::Gain as usize {
                    1.
                } else {
                    0.
                }
            }),
            ..Default::default()
        };
        settings.validate().map_err(anyhow::Error::msg)?;
        shared.dsp.store(settings);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        shared
            .soundboard_gain
            .store(SOUNDBOARD_GAIN.to_bits(), Ordering::Relaxed);
        let monitor = Arc::new(SampleQueue::new(MONITOR_QUEUE));
        let input = Arc::new(SampleQueue::new(INPUT_QUEUE));
        let progress = Arc::new(Progress::default());
        progress
            .first_render_capture_frame
            .store(u64::MAX, Ordering::Relaxed);
        let running = Arc::new(AtomicBool::new(true));
        let mut publisher = SyntheticPublisher {
            publisher: VoiceOutput::publishing(
                shared.clone(),
                input.clone(),
                monitor.clone(),
                SAMPLE_RATE,
            ),
            shared: shared.clone(),
            input,
            waves: Waves::new(),
            running: running.clone(),
            progress: progress.clone(),
            frame: 0,
        };
        // Settle production DSP smoothing for exactly one payload period before opening streams.
        let mut warmup = [0.; 512];
        let mut discard_scratch = [0.; MAX_BLOCK];
        for frame in (0..RATE).step_by(256) {
            let frames = 256.min(RATE - frame);
            publisher.render(&mut warmup[..frames * 2]);
            discard(&shared.recording, &mut discard_scratch);
            discard(&monitor, &mut discard_scratch);
        }
        ensure!(
            progress.callback_error.load(Ordering::Acquire) == 0,
            "prepared synthetic publisher failed"
        );
        running.store(false, Ordering::Release);
        progress.rendered_frames.store(0, Ordering::Release);
        progress
            .first_render_capture_frame
            .store(u64::MAX, Ordering::Release);
        let capture_queue = Arc::new(SampleQueue::new(CAPTURE_QUEUE_SAMPLES));
        let mut log = CaptureLog::new();
        report.phase = "open_exact_owned_streams";
        let render_progress = progress.clone();
        let render_running = running.clone();
        let render = render_device.build_output_stream::<f32, _, _>(
            render_config,
            move |samples, _| publisher.render(samples),
            move |_| fail(&render_progress, &render_running, 8),
            Some(connection_timeout(deadline)?),
        )?;
        let queue = capture_queue.clone();
        let capture_progress = progress.clone();
        let capture_running = running.clone();
        let error_progress = progress.clone();
        let error_running = running.clone();
        let capture = capture_device.build_input_stream::<f32, _, _>(
            capture_config,
            move |samples, _| {
                if !samples.len().is_multiple_of(2) {
                    fail(&capture_progress, &capture_running, 5);
                    return;
                }
                if queue.push(samples) != samples.len() {
                    fail(&capture_progress, &capture_running, 4);
                }
                capture_progress
                    .captured_frames
                    .fetch_add((samples.len() / 2) as u64, Ordering::Release);
            },
            move |_| fail(&error_progress, &error_running, 7),
            Some(connection_timeout(deadline)?),
        )?;
        let mut streams = Streams {
            render: Some(render),
            capture: Some(capture),
            running: running.clone(),
        };
        streams
            .capture
            .as_ref()
            .context("capture stream missing")?
            .play()?;
        report.phase = "baseline_silence";
        let baseline = pump_until(
            || progress.captured_frames.load(Ordering::Acquire) >= BASELINE_FRAMES,
            deadline,
            &capture_queue,
            &mut log,
            &shared,
            &monitor,
            &progress,
        );
        record_progress(report, &progress);
        baseline?;
        report.owned_capture_exercised = true;
        let (baseline_peak, _) = peak_and_rms(log.frames(0, BASELINE_FRAMES as usize)?);
        report.baseline_peak = Some(baseline_peak);
        ensure!(
            baseline_peak <= SILENCE_PEAK_LIMIT,
            "owned capture was not silent before rendering; close other clients sending to virtual input"
        );
        report.phase = "render_processed_voice_and_soundboard";
        running.store(true, Ordering::Release);
        ensure!(
            progress.callback_error.load(Ordering::Acquire) == 0,
            "capture failed before render startup"
        );
        streams
            .render
            .as_ref()
            .context("render stream missing")?
            .play()?;
        let signal = pump_until(
            || {
                let first = progress.first_render_capture_frame.load(Ordering::Acquire);
                first != u64::MAX
                    && progress.rendered_frames.load(Ordering::Acquire) >= SIGNAL_FRAMES
                    && progress.captured_frames.load(Ordering::Acquire)
                        >= first.saturating_add(SIGNAL_FRAMES)
            },
            deadline,
            &capture_queue,
            &mut log,
            &shared,
            &monitor,
            &progress,
        );
        record_progress(report, &progress);
        signal?;
        let first_render_capture = progress.first_render_capture_frame.load(Ordering::Acquire);
        streams.stop_render();
        let stop_capture_frame = progress.captured_frames.load(Ordering::Acquire);
        report.phase = "silence_after_render_stream_stop";
        let post_stop = pump_until(
            || {
                progress.captured_frames.load(Ordering::Acquire)
                    >= stop_capture_frame.saturating_add(POST_STOP_FRAMES)
            },
            deadline,
            &capture_queue,
            &mut log,
            &shared,
            &monitor,
            &progress,
        );
        record_progress(report, &progress);
        post_stop?;
        report.post_stop_frames = report.captured_frames.saturating_sub(stop_capture_frame);
        // Both stream drops and all prepared-buffer destruction occur on this thread.
        drop(streams);
        log.collect(&capture_queue)?;
        ensure!(
            progress.callback_error.load(Ordering::Acquire) == 0,
            "stream failed during shutdown"
        );
        report.phase = "analyze_captured_synthetic_audio";
        ensure!(
            log.samples[..log.used]
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 1.0001),
            "capture contained non-finite or out-of-range audio"
        );
        let stereo_difference = log.samples[..log.used]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| f64::from((frame[0] - frame[1]).abs()))
            .fold(0.0_f64, f64::max);
        report.stereo_max_difference = Some(stereo_difference);
        ensure!(
            stereo_difference <= 0.001,
            "left and right channels differ unexpectedly"
        );
        let window_start = usize::try_from(first_render_capture)?
            .checked_add(WINDOW_OFFSET)
            .context("signal window overflow")?;
        analyze_signal(log.frames(window_start, WINDOW_FRAMES)?, report)?;
        let silence_start = usize::try_from(stop_capture_frame)?
            .checked_add(RATE * 3 / 4)
            .context("silence window overflow")?;
        let (peak, rms) = peak_and_rms(log.frames(silence_start, RATE / 4)?);
        report.post_stop_peak = Some(peak);
        report.post_stop_rms = Some(rms);
        ensure!(
            peak <= SILENCE_PEAK_LIMIT && rms <= SILENCE_RMS_LIMIT,
            "capture did not become silent after stopping the render stream"
        );
        ensure!(
            Instant::now() < deadline,
            "ten-second acceptance deadline exceeded during analysis"
        );
        report.phase = "complete";
        report.outcome = "passed";
        Ok(())
    }

    fn quoted(value: &str) -> String {
        let mut output = String::from("\"");
        for character in value.chars() {
            match character {
                '"' => output.push_str("\\\""),
                '\\' => output.push_str("\\\\"),
                '\n' => output.push_str("\\n"),
                '\r' => output.push_str("\\r"),
                '\t' => output.push_str("\\t"),
                value if value.is_control() => {
                    use std::fmt::Write as _;
                    let _ = write!(output, "\\u{:04x}", value as u32);
                }
                value => output.push(value),
            }
        }
        output.push('"');
        output
    }

    fn number(value: Option<f64>) -> String {
        value
            .filter(|value| value.is_finite())
            .map_or_else(|| "null".into(), |value| format!("{value:.9}"))
    }

    fn pair(value: Option<[f64; 2]>) -> String {
        value.map_or_else(
            || "null".into(),
            |values| format!("[{},{}]", number(Some(values[0])), number(Some(values[1]))),
        )
    }

    impl Report {
        fn write(&self, file: &mut File) -> Result<()> {
            let accepted = self.outcome == "passed";
            let harness_pcm_bytes = CAPTURE_FRAMES * 2 * 4
                + CAPTURE_QUEUE_SAMPLES * 4
                + DRAIN_SAMPLES * 4
                + RATE * 3 * 4
                + 512 * 4
                + MAX_BLOCK * 4 * 2;
            writeln!(file, "{{")?;
            writeln!(
                file,
                "  \"report_version\":2,\n  \"test\":\"windows_installed_virtual_microphone_synthetic\","
            )?;
            writeln!(
                file,
                "  \"outcome\":{},\n  \"accepted\":{accepted},\n  \"phase\":{},",
                quoted(self.outcome),
                quoted(self.phase)
            )?;
            writeln!(
                file,
                "  \"diagnostic\":{},",
                self.diagnostic
                    .as_deref()
                    .map_or_else(|| "null".into(), quoted)
            )?;
            writeln!(
                file,
                "  \"native_windows_required\":true,\n  \"validated_pair_available\":{},\n  \"validated_capture_exercised\":{},",
                self.pair_available, self.owned_capture_exercised
            )?;
            writeln!(
                file,
                "  \"provider\":{},\n  \"explicit_render_selection\":{},\n  \"external_vb_cable_required\":{},",
                self.provider
                    .as_deref()
                    .map_or_else(|| "null".into(), quoted),
                self.explicit_render_selection,
                self.external_cable_required
            )?;
            writeln!(
                file,
                "  \"installed_endpoint_audio_path_exercised\":{accepted},\n  \"driver_load_verified_by_this_test\":false,\n  \"driver_installation_attempted\":false,\n  \"physical_devices_opened\":false,\n  \"system_audio_defaults_changed\":false,"
            )?;
            writeln!(
                file,
                "  \"production_voice_output_exercised\":{},\n  \"microphone_audio_is_synthetic\":true,\n  \"monitor_enabled\":true,\n  \"monitor_gain\":0.0,\n  \"monitor_control_transitions_exercised\":false,\n  \"dsp_gain_db\":-6.0,\n  \"soundboard_gain\":0.75,",
                self.rendered_frames > 0
            )?;
            writeln!(
                file,
                "  \"sample_rate\":{SAMPLE_RATE},\n  \"captured_frames\":{},\n  \"rendered_frames\":{},\n  \"elapsed_ms\":{},",
                self.captured_frames, self.rendered_frames, self.elapsed_ms
            )?;
            writeln!(
                file,
                "  \"baseline_peak\":{},\n  \"voice_amplitude_lr\":{},\n  \"soundboard_amplitude_lr\":{},\n  \"preview_amplitude_lr\":{},",
                number(self.baseline_peak),
                pair(self.voice_amplitude),
                pair(self.soundboard_amplitude),
                pair(self.preview_amplitude)
            )?;
            writeln!(
                file,
                "  \"prepared_payload_correlation_lr\":{},\n  \"software_loopback_alignment_frames\":{},\n  \"software_loopback_alignment_ms\":{},",
                pair(self.correlation),
                self.alignment_frames
                    .map_or_else(|| "null".into(), |value| value.to_string()),
                number(
                    self.alignment_frames
                        .map(|value| value as f64 * 1000. / RATE as f64)
                )
            )?;
            writeln!(
                file,
                "  \"stereo_max_difference\":{},\n  \"post_stop_peak\":{},\n  \"post_stop_rms\":{},\n  \"post_stop_capture_frames\":{},",
                number(self.stereo_max_difference),
                number(self.post_stop_peak),
                number(self.post_stop_rms),
                self.post_stop_frames
            )?;
            writeln!(
                file,
                "  \"harness_prepared_pcm_bytes_upper_bound\":{harness_pcm_bytes},\n  \"capture_buffer_frames_cap\":{CAPTURE_FRAMES},\n  \"measurement_deadline_seconds\":10,"
            )?;
            writeln!(
                file,
                "  \"physical_end_to_end_latency_measured\":false,\n  \"discord_manually_tested\":false,\n  \"audacity_manually_tested\":false,\n  \"tenacity_manually_tested\":false,"
            )?;
            writeln!(
                file,
                "  \"limitations\":[\"Alignment fits periodic tones using callback counters/software capture; near-equivalent phase matches exist and it is not a latency measurement.\",\"PCM memory bound excludes production DSP/shared state, driver and Windows audio-engine buffers; process RSS is not measured.\",\"Synchronous OS discovery and teardown must return; the deadline rejects overruns but is not an external process watchdog.\",\"Stop silence checks only 750-1000 ms after rendering stops, allowing a 750 ms grace; immediate silence is not qualified. No crash, concurrent-client, reconnect, hardware, signing or long-session qualification.\",\"The probe never changes endpoint volumes; non-unity user volume can cause a failed amplitude check.\",\"Monitoring is enabled with zero gain throughout: published audio and preview exclusion are checked in that state. Changes to Ouvir-me enable/gain and audible headphone preview are not installed-device acceptance claims.\",\"Only the validated endpoint audio path is exercised; kernel provenance, driver signing, Discord and recorder applications are not verified.\"]"
            )?;
            writeln!(file, "}}")?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        }
    }

    struct Options {
        report: PathBuf,
        virtual_output: Option<String>,
        external_cable: bool,
    }

    fn options() -> Result<Options> {
        let mut arguments = std::env::args_os().skip(1);
        let mut report = None;
        let mut virtual_output = None;
        let mut external_cable = false;
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--report") => {
                    ensure!(report.is_none(), "--report cannot be repeated");
                    report = Some(PathBuf::from(
                        arguments
                            .next()
                            .context("--report requires a new output path")?,
                    ));
                }
                Some("--virtual-output") => {
                    ensure!(
                        virtual_output.is_none(),
                        "--virtual-output cannot be repeated"
                    );
                    let id = arguments
                        .next()
                        .context("--virtual-output requires an exact render endpoint ID")?
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("--virtual-output ID must be UTF-8"))?;
                    ensure!(
                        !id.is_empty() && id.len() <= 2048 && !id.contains('\0'),
                        "--virtual-output requires a nonempty render ID of at most 2048 bytes without NUL"
                    );
                    virtual_output = Some(id);
                }
                Some("--external-cable") => {
                    ensure!(!external_cable, "--external-cable cannot be repeated");
                    external_cable = true;
                }
                _ => anyhow::bail!(
                    "usage: virtual-microphone-smoke [--external-cable --virtual-output EXACT_ID] --report NEW_PATH.json"
                ),
            }
        }
        ensure!(
            !external_cable || virtual_output.is_some(),
            "--external-cable requires an explicit --virtual-output ID; no cable is selected automatically"
        );
        Ok(Options {
            report: report.context("--report NEW_PATH.json is required")?,
            virtual_output,
            external_cable,
        })
    }

    pub(super) fn run() -> i32 {
        let result = (|| -> Result<i32> {
            let options = options()?;
            // Reserve the report before touching audio. Existing files are never overwritten.
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&options.report)
                .context("create the requested new report file")?;
            let started = Instant::now();
            let mut report = Report {
                outcome: "failed",
                phase: "initialize",
                explicit_render_selection: options.virtual_output.is_some(),
                external_cable_required: options.external_cable,
                ..Default::default()
            };
            if let Err(error) = exercise(started + Duration::from_secs(10), &options, &mut report) {
                report.outcome = "failed";
                report.diagnostic = Some(format!("{error:#}"));
            }
            report.elapsed_ms = started.elapsed().as_millis();
            report.write(&mut file)?;
            eprintln!(
                "virtual microphone acceptance: {} ({})",
                report.outcome,
                options.report.display()
            );
            Ok(match report.outcome {
                "passed" => 0,
                "unavailable" => 2,
                _ => 1,
            })
        })();
        match result {
            Ok(code) => code,
            Err(error) => {
                eprintln!("virtual-microphone-smoke: {error:#}");
                1
            }
        }
    }
}
