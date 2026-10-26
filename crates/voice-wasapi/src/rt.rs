//! Prepared, platform-independent PCM processing; no device APIs are called here.
use crate::clock::ClockBridge;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use voice_core::{AudioShared, DesktopShared, SAMPLE_RATE, SampleQueue};

pub(crate) const MAX_BLOCK: usize = 8192;
pub(crate) const INPUT_QUEUE: usize = 8192;
pub(crate) const MONITOR_QUEUE: usize = 8192;

/// Control feedback belongs to one stream lifetime and tracks its latest batch.
/// Cumulative diagnostic maxima must never control a current or restarted FIFO.
#[derive(Clone, Default)]
pub(crate) struct CallbackBatches {
    input: Arc<AtomicUsize>,
    render: Arc<AtomicUsize>,
}

pub(crate) struct InputCapture {
    shared: Arc<AudioShared>,
    input: Arc<SampleQueue>,
    scratch: Box<[f32]>,
    batches: CallbackBatches,
}

impl InputCapture {
    pub(crate) fn new(
        shared: Arc<AudioShared>,
        input: Arc<SampleQueue>,
        batches: CallbackBatches,
    ) -> Self {
        Self {
            shared,
            input,
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
            batches,
        }
    }

    /// Only one input callback owns the queue's producer index.
    pub(crate) fn process(&mut self, samples: &[f32], channels: usize) {
        if !(1..=2).contains(&channels) {
            return;
        }
        self.batches
            .input
            .store(samples.len() / channels, Ordering::Release);
        self.shared
            .metrics
            .input_callback_frames_max
            .fetch_max((samples.len() / channels) as u64, Ordering::Relaxed);
        let mut invalid = 0;
        for block in samples.chunks(MAX_BLOCK * channels) {
            let frames = block.len() / channels;
            for (out, frame) in self.scratch[..frames]
                .iter_mut()
                .zip(block.chunks_exact(channels))
            {
                // Divide each term before summing so two finite f32::MAX values cannot
                // overflow while downmixing. Sanitize before entering stateful DSP.
                let mut value = 0.;
                for sample in frame {
                    if sample.is_finite() {
                        value += *sample / channels as f32;
                    } else {
                        invalid += 1;
                    }
                }
                *out = if value.is_finite() { value } else { 0. };
            }
            if self.input.push(&self.scratch[..frames]) < frames {
                self.shared
                    .metrics
                    .input_overruns
                    .fetch_add(1, Ordering::Relaxed);
                self.shared
                    .metrics
                    .input_queue_overruns
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        self.shared
            .metrics
            .non_finite_samples
            .fetch_add(invalid, Ordering::Relaxed);
    }
}

pub(crate) struct VoiceOutput {
    shared: Arc<AudioShared>,
    input: Arc<SampleQueue>,
    input_clock: ClockBridge,
    batches: CallbackBatches,
    dsp: voice_dsp::Processor,
    voice: Box<[f32]>,
    scratch: Box<[f32]>,
    listening: Box<[f32]>,
    // When present, this processor is clocked by the owned virtual render endpoint.
    monitor: Option<Arc<SampleQueue>>,
}

impl VoiceOutput {
    pub(crate) fn new(
        shared: Arc<AudioShared>,
        input: Arc<SampleQueue>,
        rate: u32,
        batches: CallbackBatches,
    ) -> Self {
        let input_clock = ClockBridge::new(input.capacity(), rate);
        Self {
            shared,
            input,
            input_clock,
            batches,
            dsp: voice_dsp::Processor::new(rate),
            voice: vec![0.; MAX_BLOCK].into_boxed_slice(),
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
            listening: vec![0.; MAX_BLOCK].into_boxed_slice(),
            monitor: None,
        }
    }

    pub(crate) fn publishing(
        shared: Arc<AudioShared>,
        input: Arc<SampleQueue>,
        monitor: Arc<SampleQueue>,
        rate: u32,
        batches: CallbackBatches,
    ) -> Self {
        let mut processor = Self::new(shared, input, rate, batches);
        processor.monitor = Some(monitor);
        processor
    }

    /// The sole consumers of input/soundboard/preview and sole recording producer.
    /// Device output is stereo. Publishing emits only the final voice/soundboard mix;
    /// local-only mode emits the independent listening mix, including preview.
    pub(crate) fn render(&mut self, stereo: &mut [f32]) {
        stereo.fill(0.);
        self.batches
            .render
            .store(stereo.len() / 2, Ordering::Release);
        self.shared
            .metrics
            .render_callback_frames_max
            .fetch_max((stereo.len() / 2) as u64, Ordering::Relaxed);
        for output in stereo.chunks_mut(MAX_BLOCK * 2) {
            let frames = output.len() / 2;
            if frames == 0 {
                continue;
            }
            self.process(frames);
            let samples = if self.monitor.is_some() {
                &self.voice[..frames]
            } else {
                &self.listening[..frames]
            };
            for (frame, sample) in output.as_chunks_mut::<2>().0.iter_mut().zip(samples.iter()) {
                frame.fill(*sample);
            }
            if self.monitor.is_some() {
                self.shared
                    .metrics
                    .virtual_microphone_frames
                    .fetch_add(frames as u64, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn render_if_running(&mut self, stereo: &mut [f32], running: &AtomicBool) {
        if !running.load(Ordering::Acquire) {
            stereo.fill(0.);
            self.shared.metrics.output_peak.store(0, Ordering::Relaxed);
            return;
        }
        self.render(stereo);
        // An error on the capture thread can arrive while this callback is processing.
        if !running.load(Ordering::Acquire) {
            stereo.fill(0.);
            self.shared.metrics.output_peak.store(0, Ordering::Relaxed);
            self.shared
                .metrics
                .algorithmic_latency_frames
                .store(0, Ordering::Relaxed);
        }
    }

    fn process(&mut self, frames: usize) {
        let input = self.input_clock.read(
            &self.input,
            &mut self.voice[..frames],
            self.batches.input.load(Ordering::Acquire),
        );
        if input.trimmed_frames > 0 {
            self.shared
                .metrics
                .input_overruns
                .fetch_add(1, Ordering::Relaxed);
            self.shared
                .metrics
                .input_queue_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        if input.missing_frames > 0 {
            self.shared
                .metrics
                .input_underruns
                .fetch_add(1, Ordering::Relaxed);
            self.shared
                .metrics
                .input_queue_underruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.shared
            .metrics
            .input_priming_frames
            .fetch_add(input.priming_frames as u64, Ordering::Relaxed);
        let mut invalid = input.non_finite_samples;
        for sample in &mut self.voice[..frames] {
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
            self.dsp.process(&mut self.voice[..frames], &settings);
        }
        if self.shared.muted.load(Ordering::Relaxed) {
            self.voice[..frames].fill(0.);
        }
        self.scratch[..frames].fill(0.);
        if self.shared.soundboard.pop(&mut self.scratch[..frames]) < frames
            && self.shared.soundboard_active.load(Ordering::Relaxed)
        {
            self.shared
                .metrics
                .soundboard_underruns
                .fetch_add(1, Ordering::Relaxed);
        }
        let soundboard_gain = finite_gain(self.shared.soundboard_gain.load(Ordering::Relaxed));
        let mut peak = 0.0_f32;
        for (voice, sound) in self.voice[..frames].iter_mut().zip(&self.scratch[..frames]) {
            *voice = finite_sample(*voice + sound * soundboard_gain, &mut invalid);
            peak = peak.max(voice.abs());
        }
        self.shared
            .metrics
            .output_peak
            .store(peak.to_bits(), Ordering::Relaxed);
        self.shared
            .metrics
            .frames
            .fetch_add(frames as u64, Ordering::Relaxed);
        if self.shared.recording.push(&self.voice[..frames]) < frames {
            self.shared
                .metrics
                .recording_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.listening[..frames].fill(0.);
        if self.shared.monitor_enabled.load(Ordering::Relaxed) {
            let gain = finite_gain(self.shared.monitor_gain.load(Ordering::Relaxed));
            for (out, sample) in self.listening[..frames]
                .iter_mut()
                .zip(&self.voice[..frames])
            {
                *out = *sample * gain;
            }
        }
        self.scratch[..frames].fill(0.);
        if self.shared.preview.pop(&mut self.scratch[..frames]) < frames
            && self.shared.preview_active.load(Ordering::Relaxed)
        {
            self.shared
                .metrics
                .preview_underruns
                .fetch_add(1, Ordering::Relaxed);
        }
        for (out, preview) in self.listening[..frames]
            .iter_mut()
            .zip(&self.scratch[..frames])
        {
            *out = finite_sample(*out + preview, &mut invalid);
        }
        if let Some(monitor) = &self.monitor
            && monitor.push(&self.listening[..frames]) < frames
        {
            self.shared
                .metrics
                .monitor_producer_overruns
                .fetch_add(1, Ordering::Relaxed);
            self.shared
                .metrics
                .monitor_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.shared
            .metrics
            .non_finite_samples
            .fetch_add(invalid, Ordering::Relaxed);
    }
}

/// The sole monitor-queue consumer; never consumes microphone, soundboard or preview.
pub(crate) struct LocalMonitor {
    shared: Arc<AudioShared>,
    queue: Arc<SampleQueue>,
    clock: ClockBridge,
    batches: CallbackBatches,
    scratch: Box<[f32]>,
}

impl LocalMonitor {
    pub(crate) fn new(
        shared: Arc<AudioShared>,
        queue: Arc<SampleQueue>,
        batches: CallbackBatches,
    ) -> Self {
        let clock = ClockBridge::new(queue.capacity(), SAMPLE_RATE);
        Self {
            shared,
            queue,
            clock,
            batches,
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
        }
    }

    pub(crate) fn render_if_running(&mut self, stereo: &mut [f32], running: &AtomicBool) {
        stereo.fill(0.);
        if !running.load(Ordering::Acquire) {
            return;
        }
        self.shared
            .metrics
            .monitor_callback_frames_max
            .fetch_max((stereo.len() / 2) as u64, Ordering::Relaxed);
        let mut invalid = 0;
        for output in stereo.chunks_mut(MAX_BLOCK * 2) {
            let frames = output.len() / 2;
            if frames == 0 {
                continue;
            }
            let read = self.clock.read(
                &self.queue,
                &mut self.scratch[..frames],
                self.batches.render.load(Ordering::Acquire),
            );
            if read.trimmed_frames > 0 {
                self.shared
                    .metrics
                    .monitor_trimmed_frames
                    .fetch_add(read.trimmed_frames as u64, Ordering::Relaxed);
                self.shared
                    .metrics
                    .monitor_overruns
                    .fetch_add(1, Ordering::Relaxed);
            }
            if read.missing_frames > 0 {
                self.shared
                    .metrics
                    .monitor_queue_underruns
                    .fetch_add(1, Ordering::Relaxed);
                self.shared
                    .metrics
                    .monitor_overruns
                    .fetch_add(1, Ordering::Relaxed);
            }
            self.shared
                .metrics
                .monitor_priming_frames
                .fetch_add(read.priming_frames as u64, Ordering::Relaxed);
            invalid += read.non_finite_samples;
            for (frame, sample) in output
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(&self.scratch[..frames])
            {
                frame.fill(finite_sample(*sample, &mut invalid));
            }
        }
        self.shared
            .metrics
            .non_finite_samples
            .fetch_add(invalid, Ordering::Relaxed);
        if !running.load(Ordering::Acquire) {
            stereo.fill(0.);
        }
    }
}

pub(crate) struct DesktopCapture {
    shared: Arc<DesktopShared>,
    scratch: Box<[f32]>,
}

impl DesktopCapture {
    pub(crate) fn new(shared: Arc<DesktopShared>) -> Self {
        Self {
            shared,
            scratch: vec![0.; MAX_BLOCK * 2].into_boxed_slice(),
        }
    }

    /// Explicit loopback always stays stereo and has its own bounded queue.
    pub(crate) fn process(&mut self, stereo: &[f32]) {
        let mut invalid = 0;
        let mut peak = 0.0_f32;
        for input in stereo.chunks(MAX_BLOCK * 2) {
            let samples = input.len() / 2 * 2;
            for (out, sample) in self.scratch[..samples].iter_mut().zip(input) {
                *out = finite_sample(*sample, &mut invalid);
                peak = peak.max(out.abs());
            }
            if self.shared.samples.push(&self.scratch[..samples]) < samples {
                self.shared.overruns.fetch_add(1, Ordering::Relaxed);
                self.shared.queue_overruns.fetch_add(1, Ordering::Relaxed);
            }
            self.shared
                .frames
                .fetch_add((samples / 2) as u64, Ordering::Relaxed);
        }
        self.shared
            .non_finite_samples
            .fetch_add(invalid, Ordering::Relaxed);
        self.shared.peak.store(peak.to_bits(), Ordering::Relaxed);
    }
}

fn finite_sample(sample: f32, invalid: &mut u64) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1., 1.)
    } else {
        *invalid += 1;
        0.
    }
}

fn finite_gain(bits: u32) -> f32 {
    let gain = f32::from_bits(bits);
    if gain.is_finite() {
        gain.clamp(0., 4.)
    } else {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use voice_core::{DspSettings, SAMPLE_RATE, VoiceBlend, VoiceLayer};

    struct TrackingAllocator;
    thread_local! {
        static TRACK: Cell<bool> = const { Cell::new(false) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }
    #[global_allocator]
    static ALLOCATOR: TrackingAllocator = TrackingAllocator;

    // The test allocator delegates unchanged pointers and layouts to System.
    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            track_allocation();
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            track_allocation();
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            track_allocation();
            unsafe { System.realloc(ptr, layout, size) }
        }
    }

    fn track_allocation() {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            let _ = CALLS.try_with(|calls| calls.set(calls.get() + 1));
        }
    }

    fn voice() -> (Arc<AudioShared>, Arc<SampleQueue>, VoiceOutput) {
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(INPUT_QUEUE));
        let mut output = VoiceOutput::new(
            shared.clone(),
            input.clone(),
            SAMPLE_RATE,
            CallbackBatches::default(),
        );
        output.input_clock = ClockBridge::immediate(INPUT_QUEUE, SAMPLE_RATE);
        (shared, input, output)
    }

    fn publishing() -> (
        Arc<AudioShared>,
        Arc<SampleQueue>,
        VoiceOutput,
        LocalMonitor,
    ) {
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(INPUT_QUEUE));
        let queue = Arc::new(SampleQueue::new(MONITOR_QUEUE));
        let batches = CallbackBatches::default();
        let mut output = VoiceOutput::publishing(
            shared.clone(),
            input.clone(),
            queue.clone(),
            SAMPLE_RATE,
            batches.clone(),
        );
        output.input_clock = ClockBridge::immediate(INPUT_QUEUE, SAMPLE_RATE);
        let mut monitor = LocalMonitor::new(shared.clone(), queue, batches);
        monitor.clock = ClockBridge::immediate(MONITOR_QUEUE, SAMPLE_RATE);
        (shared, input, output, monitor)
    }

    #[test]
    fn virtual_output_includes_soundboard_with_monitor_off_and_excludes_preview() {
        let (shared, input, mut output, mut monitor) = publishing();
        shared
            .monitor_gain
            .store(0_f32.to_bits(), Ordering::Relaxed);
        input.push(&[0.4, -0.2]);
        shared.soundboard.push(&[0.1, 0.1]);
        shared.preview.push(&[0.7, 0.6]);
        let mut virtual_pcm = [0.; 4];
        let mut headphones = [0.; 4];
        output.render_if_running(&mut virtual_pcm, &AtomicBool::new(true));
        monitor.render_if_running(&mut headphones, &AtomicBool::new(true));
        assert_eq!(virtual_pcm, [0.5, 0.5, -0.1, -0.1]);
        assert_eq!(headphones, [0.7, 0.7, 0.6, 0.6]);
        let mut recorded = [0.; 2];
        assert_eq!(shared.recording.pop(&mut recorded), 2);
        assert_eq!(recorded, [0.5, -0.1]);
        assert_eq!(
            shared
                .metrics
                .virtual_microphone_frames
                .load(Ordering::Relaxed),
            2
        );
    }

    #[test]
    fn cable_final_mix_is_independent_of_listen_toggle_gain_and_preview() {
        for enabled in [false, true] {
            for gain in [0_f32, 0.25, 1.0, 2.0] {
                let (shared, input, mut output, mut monitor) = publishing();
                shared.monitor_enabled.store(enabled, Ordering::Relaxed);
                shared.monitor_gain.store(gain.to_bits(), Ordering::Relaxed);
                input.push(&[0.2; 2]);
                shared.soundboard.push(&[0.1; 2]);
                shared.preview.push(&[0.05; 2]);
                let mut published = [0.; 4];
                let mut headphones = [0.; 4];
                let running = AtomicBool::new(true);
                output.render_if_running(&mut published, &running);
                monitor.render_if_running(&mut headphones, &running);
                assert!(published.iter().all(|value| (*value - 0.3).abs() < 1e-6));
                let listening = if enabled { 0.3 * gain } else { 0.0 } + 0.05;
                assert!(
                    headphones
                        .iter()
                        .all(|value| (*value - listening).abs() < 1e-6)
                );
                let mut recorded = [0.; 2];
                assert_eq!(shared.recording.pop(&mut recorded), 2);
                assert!(recorded.iter().all(|value| (*value - 0.3).abs() < 1e-6));
            }
        }
    }

    #[test]
    fn virtual_publishing_advances_without_monitor_callbacks_and_has_bounded_backlog() {
        let (shared, input, mut output, mut monitor) = publishing();
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(1_f32.to_bits(), Ordering::Relaxed);
        let mut virtual_pcm = [0.; 512];
        for _ in 0..40 {
            input.push(&[0.1; 256]);
            output.render(&mut virtual_pcm);
        }
        assert_eq!(
            shared
                .metrics
                .virtual_microphone_frames
                .load(Ordering::Relaxed),
            10240
        );
        assert_eq!(monitor.queue.len(), MONITOR_QUEUE);
        assert_eq!(
            shared
                .metrics
                .monitor_producer_overruns
                .load(Ordering::Relaxed),
            8
        );
        let mut headphones = [0.; 512];
        monitor.render_if_running(&mut headphones, &AtomicBool::new(true));
        assert!(headphones.iter().all(|sample| *sample == 0.1));
        assert_eq!(
            shared
                .metrics
                .monitor_trimmed_frames
                .load(Ordering::Relaxed),
            (MONITOR_QUEUE - 256) as u64
        );
        assert_eq!(monitor.queue.capacity(), MONITOR_QUEUE);
        assert_eq!(monitor.scratch.len(), MAX_BLOCK);
    }

    #[test]
    fn virtual_muted_voice_keeps_soundboard_and_failure_silences_all_routes() {
        let (shared, input, mut output, mut monitor) = publishing();
        shared.muted.store(true, Ordering::Relaxed);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        input.push(&[0.8]);
        shared.soundboard.push(&[0.3]);
        let mut virtual_pcm = [f32::NAN; 2];
        output.render_if_running(&mut virtual_pcm, &AtomicBool::new(true));
        assert_eq!(virtual_pcm, [0.3; 2]);
        input.push(&[0.8]);
        shared.soundboard.push(&[0.3]);
        let mut headphones = [f32::NAN; 2];
        let stopped = AtomicBool::new(false);
        output.render_if_running(&mut virtual_pcm, &stopped);
        monitor.render_if_running(&mut headphones, &stopped);
        assert_eq!(virtual_pcm, [0.; 2]);
        assert_eq!(headphones, [0.; 2]);
        assert_eq!(input.len(), 1);
        assert_eq!(shared.soundboard.len(), 1);
        assert_eq!(monitor.queue.len(), 1);
    }

    #[test]
    fn empty_monitor_callback_does_not_discard_backlog_and_underflow_is_silent() {
        let (shared, _, _, mut monitor) = publishing();
        monitor.queue.push(&[0.2; 16]);
        monitor.render_if_running(&mut [], &AtomicBool::new(true));
        assert_eq!(monitor.queue.len(), 16);
        assert_eq!(shared.metrics.monitor_overruns.load(Ordering::Relaxed), 0);
        let mut headphones = [1.; 32];
        monitor.render_if_running(&mut headphones, &AtomicBool::new(true));
        assert_eq!(headphones, [0.2; 32]);
        headphones.fill(1.);
        monitor.render_if_running(&mut headphones, &AtomicBool::new(true));
        assert_eq!(headphones, [0.; 32]);
    }

    #[test]
    fn virtual_pcm_sanitizes_non_finite_mix_and_bounds_the_safety_ceiling() {
        let (shared, input, mut output, _) = publishing();
        input.push(&[0.8, f32::NAN, f32::INFINITY, -0.8]);
        shared.soundboard.push(&[0.8, 0.3, f32::NAN, -0.8]);
        let mut virtual_pcm = [f32::NAN; 8];
        output.render(&mut virtual_pcm);
        assert_eq!(virtual_pcm, [1., 1., 0.3, 0.3, 0., 0., -1., -1.]);
        assert_eq!(shared.metrics.non_finite_samples.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn virtual_effects_blend_soundboard_and_monitor_callbacks_allocate_or_free_nothing() {
        let (shared, input, mut output, mut monitor) = publishing();
        shared.bypass.store(false, Ordering::Relaxed);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared.dsp.store(DspSettings {
            gain_db: -6.,
            pitch_semitones: -2.,
            robot_mix: 0.15,
            chorus_mix: 0.2,
            blend: Some(VoiceBlend {
                secondary: VoiceLayer::from(DspSettings {
                    formant_semitones: -1.,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut virtual_pcm = [0.; 512];
        let mut headphones = [0.; 512];
        let mut recording = [0.; 256];
        let running = AtomicBool::new(true);
        CALLS.with(|calls| calls.set(0));
        TRACK.with(|track| track.set(true));
        for _ in 0..64 {
            input.push(&[0.1; 256]);
            shared.soundboard.push(&[0.03; 256]);
            shared.preview.push(&[0.02; 256]);
            output.render_if_running(&mut virtual_pcm, &running);
            monitor.render_if_running(&mut headphones, &running);
            shared.recording.pop(&mut recording);
        }
        TRACK.with(|track| track.set(false));
        assert_eq!(CALLS.with(Cell::get), 0);
        assert!(
            virtual_pcm
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 1.)
        );
        assert!(
            headphones
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 1.)
        );
        assert!(virtual_pcm.iter().any(|sample| *sample != 0.));
        assert!(
            virtual_pcm
                .iter()
                .zip(headphones)
                .any(|(published, local)| (*published - local).abs() > 0.001)
        );
    }

    #[test]
    fn stereo_input_downmixes_and_sanitizes_before_dsp() {
        let (shared, input, _) = voice();
        let mut capture =
            InputCapture::new(shared.clone(), input.clone(), CallbackBatches::default());
        capture.process(&[0.4, 0.8, f32::NAN, 0.6, f32::INFINITY, -0.2], 2);
        let mut mono = [0.; 3];
        assert_eq!(input.pop(&mut mono), 3);
        assert!((mono[0] - 0.6).abs() < 1e-6);
        assert_eq!(mono[1..], [0.3, -0.1]);
        assert_eq!(shared.metrics.non_finite_samples.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn monitoring_is_stereo_and_preview_never_enters_recording() {
        let (shared, input, mut output) = voice();
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(0.5_f32.to_bits(), Ordering::Relaxed);
        input.push(&[0.4, -0.8]);
        shared.preview.push(&[0.1, 0.1]);
        let mut stereo = [0.; 4];
        output.render(&mut stereo);
        assert_eq!(stereo, [0.3, 0.3, -0.3, -0.3]);
        let mut recorded = [0.; 2];
        assert_eq!(shared.recording.pop(&mut recorded), 2);
        assert_eq!(recorded, [0.4, -0.8]);
    }

    #[test]
    fn monitor_off_still_records_and_previews_without_microphone_playback() {
        let (shared, input, mut output) = voice();
        input.push(&[0.75]);
        shared.preview.push(&[0.2]);
        let mut stereo = [0.; 2];
        output.render(&mut stereo);
        assert_eq!(stereo, [0.2, 0.2]);
        let mut recorded = [0.; 1];
        shared.recording.pop(&mut recorded);
        assert_eq!(recorded, [0.75]);
    }

    #[test]
    fn stopped_callback_is_silent_and_does_not_consume_or_record() {
        let (shared, input, mut output) = voice();
        input.push(&[0.75]);
        shared.preview.push(&[0.2]);
        let mut stereo = [0.8; 2];
        output.render_if_running(&mut stereo, &AtomicBool::new(false));
        assert_eq!(stereo, [0., 0.]);
        assert_eq!(input.len(), 1);
        assert_eq!(shared.preview.len(), 1);
        assert!(shared.recording.is_empty());
        assert_eq!(shared.metrics.frames.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn muting_voice_preserves_soundboard_and_invalid_gain_is_silent() {
        let (shared, input, mut output) = voice();
        shared.muted.store(true, Ordering::Relaxed);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(1_f32.to_bits(), Ordering::Relaxed);
        input.push(&[0.8]);
        shared.soundboard.push(&[0.3]);
        let mut stereo = [0.; 2];
        output.render(&mut stereo);
        assert_eq!(stereo, [0.3, 0.3]);
        shared
            .monitor_gain
            .store(f32::NAN.to_bits(), Ordering::Relaxed);
        shared.soundboard.push(&[0.3]);
        output.render(&mut stereo);
        assert_eq!(stereo, [0., 0.]);
    }

    #[test]
    fn bounded_input_overflow_and_missing_samples_are_counted() {
        let (shared, _, mut output) = voice();
        let input = Arc::new(SampleQueue::new(2));
        let mut capture = InputCapture::new(shared.clone(), input, CallbackBatches::default());
        capture.process(&[0.1; 8], 1);
        assert_eq!(shared.metrics.input_overruns.load(Ordering::Relaxed), 1);
        assert_eq!(
            shared.metrics.input_queue_overruns.load(Ordering::Relaxed),
            1
        );
        output.render(&mut [0.; 16]);
        assert_eq!(shared.metrics.input_underruns.load(Ordering::Relaxed), 1);
        assert_eq!(
            shared.metrics.input_queue_underruns.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn active_playback_shortages_are_counted_even_with_the_microphone_muted() {
        let (shared, input, mut output, _) = publishing();
        shared.muted.store(true, Ordering::Relaxed);
        input.push(&[0.1; 4]);
        output.render(&mut [0.; 8]);
        assert_eq!(
            shared.metrics.soundboard_underruns.load(Ordering::Relaxed),
            0
        );
        assert_eq!(shared.metrics.preview_underruns.load(Ordering::Relaxed), 0);
        shared.soundboard_active.store(true, Ordering::Relaxed);
        shared.preview_active.store(true, Ordering::Relaxed);
        input.push(&[0.1; 4]);
        output.render(&mut [0.; 8]);
        assert_eq!(
            shared.metrics.soundboard_underruns.load(Ordering::Relaxed),
            1
        );
        assert_eq!(shared.metrics.preview_underruns.load(Ordering::Relaxed), 1);
        shared.soundboard_active.store(false, Ordering::Relaxed);
        shared.preview_active.store(false, Ordering::Relaxed);
        input.push(&[0.1; 4]);
        output.render(&mut [0.; 8]);
        assert_eq!(
            shared.metrics.soundboard_underruns.load(Ordering::Relaxed),
            1
        );
        assert_eq!(shared.metrics.preview_underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn native_unequal_clock_batches_prime_without_losses_or_callback_allocation() {
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(1_f32.to_bits(), Ordering::Relaxed);
        let input = Arc::new(SampleQueue::new(INPUT_QUEUE));
        let queue = Arc::new(SampleQueue::new(MONITOR_QUEUE));
        let batches = CallbackBatches::default();
        let mut capture = InputCapture::new(shared.clone(), input.clone(), batches.clone());
        let mut output = VoiceOutput::publishing(
            shared.clone(),
            input.clone(),
            queue.clone(),
            SAMPLE_RATE,
            batches.clone(),
        );
        let mut monitor = LocalMonitor::new(shared.clone(), queue.clone(), batches);
        let microphone = [0.1; 480];
        let mut published = [0.; 192];
        let mut headphones = [0.; 480];
        let mut recording = [0.; 192];
        let running = AtomicBool::new(true);
        let mut heard = false;
        CALLS.with(|calls| calls.set(0));
        TRACK.with(|track| track.set(true));
        // Native 48 kHz: 480-frame input bursts, 96-frame cable callbacks,
        // 240-frame headphone callbacks, all independently phased.
        for millisecond in 0..1000 {
            if millisecond % 10 == 0 {
                capture.process(&microphone, 1);
            }
            if millisecond % 2 == 0 {
                output.render_if_running(&mut published, &running);
                shared.recording.pop(&mut recording);
            }
            if millisecond % 5 == 1 {
                monitor.render_if_running(&mut headphones, &running);
                heard |= headphones.iter().any(|sample| *sample > 0.09);
            }
        }
        TRACK.with(|track| track.set(false));
        assert_eq!(CALLS.with(Cell::get), 0);
        assert!(heard);
        assert!(published.iter().chain(&headphones).all(|v| v.is_finite()));
        assert!(shared.metrics.input_priming_frames.load(Ordering::Relaxed) > 0);
        assert!(
            shared
                .metrics
                .monitor_priming_frames
                .load(Ordering::Relaxed)
                > 0
        );
        assert_eq!(shared.metrics.input_underruns.load(Ordering::Relaxed), 0);
        assert_eq!(shared.metrics.input_overruns.load(Ordering::Relaxed), 0);
        assert_eq!(shared.metrics.monitor_overruns.load(Ordering::Relaxed), 0);
        assert_eq!(
            shared
                .metrics
                .input_callback_frames_max
                .load(Ordering::Relaxed),
            480
        );
        assert_eq!(
            shared
                .metrics
                .render_callback_frames_max
                .load(Ordering::Relaxed),
            96
        );
        assert_eq!(
            shared
                .metrics
                .monitor_callback_frames_max
                .load(Ordering::Relaxed),
            240
        );
        let before = (
            input.len(),
            queue.len(),
            shared.metrics.frames.load(Ordering::Relaxed),
        );
        published.fill(f32::NAN);
        headphones.fill(f32::NAN);
        running.store(false, Ordering::Release);
        output.render_if_running(&mut published, &running);
        monitor.render_if_running(&mut headphones, &running);
        assert!(published.iter().chain(&headphones).all(|v| *v == 0.));
        assert_eq!(
            (
                input.len(),
                queue.len(),
                shared.metrics.frames.load(Ordering::Relaxed)
            ),
            before
        );
    }

    #[test]
    fn oversized_batches_recover_in_place_and_after_reconnect_without_resetting_maxima() {
        fn stream(shared: &Arc<AudioShared>) -> (InputCapture, VoiceOutput, LocalMonitor) {
            let input = Arc::new(SampleQueue::new(INPUT_QUEUE));
            let queue = Arc::new(SampleQueue::new(MONITOR_QUEUE));
            let batches = CallbackBatches::default();
            (
                InputCapture::new(shared.clone(), input.clone(), batches.clone()),
                VoiceOutput::publishing(
                    shared.clone(),
                    input,
                    queue.clone(),
                    SAMPLE_RATE,
                    batches.clone(),
                ),
                LocalMonitor::new(shared.clone(), queue, batches),
            )
        }
        fn failures(shared: &AudioShared) -> (u64, u64, u64, u64) {
            (
                shared.metrics.input_underruns.load(Ordering::Relaxed),
                shared.metrics.input_overruns.load(Ordering::Relaxed),
                shared.metrics.monitor_overruns.load(Ordering::Relaxed),
                shared
                    .metrics
                    .monitor_trimmed_frames
                    .load(Ordering::Relaxed),
            )
        }
        let shared = Arc::new(AudioShared::default());
        shared.bypass.store(true, Ordering::Relaxed);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        shared
            .monitor_gain
            .store(1_f32.to_bits(), Ordering::Relaxed);
        let (mut capture, mut output, mut monitor) = stream(&shared);
        let mut published = [0.; 480];
        let mut headphones = [0.; 480];
        let microphone = [0.1; 480];
        let running = AtomicBool::new(true);
        capture.process(&[0.1; INPUT_QUEUE], 1);
        output.render(&mut published);
        output.render(&mut [0.; 12_000]);
        monitor.render_if_running(&mut headphones, &running);
        assert!(shared.metrics.input_queue_underruns.load(Ordering::Relaxed) > 0);
        assert!(
            shared
                .metrics
                .monitor_queue_underruns
                .load(Ordering::Relaxed)
                > 0
        );
        let mut settled = None;
        for tick in 0..200 {
            if tick % 2 == 0 {
                capture.process(&microphone, 1);
            }
            output.render(&mut published);
            monitor.render_if_running(&mut headphones, &running);
            shared.recording.pop(&mut [0.; 480]);
            if tick == 20 {
                settled = Some(failures(&shared));
            }
        }
        assert_eq!(Some(failures(&shared)), settled);
        assert!(
            published
                .iter()
                .chain(&headphones)
                .all(|sample| *sample > 0.09)
        );
        assert_eq!(output.batches.input.load(Ordering::Acquire), 480);
        assert_eq!(monitor.batches.render.load(Ordering::Acquire), 240);
        assert_eq!(
            shared
                .metrics
                .input_callback_frames_max
                .load(Ordering::Relaxed),
            8192
        );
        assert_eq!(
            shared
                .metrics
                .render_callback_frames_max
                .load(Ordering::Relaxed),
            6000
        );

        let before = failures(&shared);
        drop((capture, output, monitor));
        let (mut capture, mut output, mut monitor) = stream(&shared);
        assert_eq!(output.batches.input.load(Ordering::Acquire), 0);
        assert_eq!(monitor.batches.render.load(Ordering::Acquire), 0);
        // Unknown latest batch uses the normal priming policy, even though the
        // diagnostic lifetime maxima still describe the prior oversized stream.
        output.render(&mut published);
        monitor.render_if_running(&mut headphones, &running);
        assert_eq!(failures(&shared), before);
        for tick in 0..200 {
            if tick % 2 == 0 {
                capture.process(&microphone, 1);
            }
            output.render(&mut published);
            monitor.render_if_running(&mut headphones, &running);
            shared.recording.pop(&mut [0.; 480]);
        }
        assert_eq!(failures(&shared), before);
        assert!(
            published
                .iter()
                .chain(&headphones)
                .all(|sample| *sample > 0.09)
        );
        assert_eq!(
            shared
                .metrics
                .input_callback_frames_max
                .load(Ordering::Relaxed),
            8192
        );
        assert_eq!(
            shared
                .metrics
                .render_callback_frames_max
                .load(Ordering::Relaxed),
            6000
        );
    }

    #[test]
    fn desktop_keeps_channel_order_sanitizes_and_stays_separate() {
        let shared = Arc::new(DesktopShared::default());
        let voice = Arc::new(AudioShared::default());
        let mut capture = DesktopCapture::new(shared.clone());
        capture.process(&[0.2, -0.4, f32::NAN, f32::INFINITY, 2., -2.]);
        let mut pcm = [0.; 6];
        assert_eq!(shared.samples.pop(&mut pcm), 6);
        assert_eq!(pcm, [0.2, -0.4, 0., 0., 1., -1.]);
        assert_eq!(shared.frames.load(Ordering::Relaxed), 3);
        assert_eq!(shared.non_finite_samples.load(Ordering::Relaxed), 2);
        assert!(voice.recording.is_empty());
        assert!(voice.soundboard.is_empty());
    }

    #[test]
    fn desktop_overflow_preserves_whole_stereo_frames() {
        let shared = Arc::new(DesktopShared::default());
        let mut capture = DesktopCapture::new(shared.clone());
        let input = vec![0.1; shared.samples.capacity() + 2];
        capture.process(&input);
        assert_eq!(shared.samples.len(), shared.samples.capacity());
        assert_eq!(shared.samples.len() % 2, 0);
        assert!(shared.overruns.load(Ordering::Relaxed) > 0);
        assert_eq!(
            shared.overruns.load(Ordering::Relaxed),
            shared.queue_overruns.load(Ordering::Relaxed)
        );
        assert_eq!(shared.backend_xruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn large_callbacks_are_chunked_without_growing_prepared_buffers() {
        let (shared, _, mut output) = voice();
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        let mut stereo = vec![f32::NAN; (MAX_BLOCK + 7) * 2];
        output.render(&mut stereo);
        assert!(stereo.iter().all(|sample| *sample == 0.));
        assert_eq!(
            shared.metrics.frames.load(Ordering::Relaxed),
            (MAX_BLOCK + 7) as u64
        );
        assert_eq!(output.voice.len(), MAX_BLOCK);
        assert_eq!(output.scratch.len(), MAX_BLOCK);
        assert_eq!(output.listening.len(), MAX_BLOCK);
    }

    #[test]
    fn prepared_pcm_callbacks_make_no_rust_allocations_or_frees_with_effects_and_blending() {
        let (shared, input, mut output) = voice();
        shared.bypass.store(false, Ordering::Relaxed);
        shared.monitor_enabled.store(true, Ordering::Relaxed);
        let secondary = VoiceLayer::from(DspSettings {
            pitch_semitones: -3.,
            formant_semitones: -2.,
            robot_mix: 0.2,
            ..Default::default()
        });
        let settings = DspSettings {
            pitch_semitones: 2.,
            formant_semitones: 1.,
            denoise: true,
            chorus_mix: 0.2,
            blend: Some(VoiceBlend {
                secondary,
                ..Default::default()
            }),
            ..Default::default()
        };
        shared.dsp.store(settings);
        let mut capture = InputCapture::new(shared.clone(), input, output.batches.clone());
        let desktop = Arc::new(DesktopShared::default());
        let mut desktop_capture = DesktopCapture::new(desktop.clone());
        let input_pcm = [0.1; 512];
        let mut output_pcm = [0.; 512];
        let mut voice_drain = [0.; 256];
        let mut desktop_drain = [0.; 512];
        CALLS.with(|calls| calls.set(0));
        TRACK.with(|track| track.set(true));
        for _ in 0..64 {
            capture.process(&input_pcm, 2);
            output.render(&mut output_pcm);
            shared.recording.pop(&mut voice_drain);
            desktop_capture.process(&input_pcm);
            desktop.samples.pop(&mut desktop_drain);
        }
        TRACK.with(|track| track.set(false));
        assert_eq!(CALLS.with(Cell::get), 0);
        assert!(output_pcm.iter().all(|sample| sample.is_finite()));
    }
}
