//! Prepared, platform-independent PCM processing; no device APIs are called here.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use voice_core::{AudioShared, DesktopShared, SampleQueue};

pub(crate) const MAX_BLOCK: usize = 8192;
pub(crate) const INPUT_QUEUE: usize = 8192;

pub(crate) struct InputCapture {
    shared: Arc<AudioShared>,
    input: Arc<SampleQueue>,
    scratch: Box<[f32]>,
}

impl InputCapture {
    pub(crate) fn new(shared: Arc<AudioShared>, input: Arc<SampleQueue>) -> Self {
        Self {
            shared,
            input,
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
        }
    }

    /// Only one input callback owns the queue's producer index.
    pub(crate) fn process(&mut self, samples: &[f32], channels: usize) {
        if !(1..=2).contains(&channels) {
            return;
        }
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
    dsp: voice_dsp::Processor,
    voice: Box<[f32]>,
    scratch: Box<[f32]>,
    listening: Box<[f32]>,
}

impl VoiceOutput {
    pub(crate) fn new(shared: Arc<AudioShared>, input: Arc<SampleQueue>, rate: u32) -> Self {
        Self {
            shared,
            input,
            dsp: voice_dsp::Processor::new(rate),
            voice: vec![0.; MAX_BLOCK].into_boxed_slice(),
            scratch: vec![0.; MAX_BLOCK].into_boxed_slice(),
            listening: vec![0.; MAX_BLOCK].into_boxed_slice(),
        }
    }

    /// The sole consumers of input/soundboard/preview and sole recording producer.
    /// Device output is stereo, with the same local-monitoring sample in left and right.
    pub(crate) fn render(&mut self, stereo: &mut [f32]) {
        stereo.fill(0.);
        for output in stereo.chunks_mut(MAX_BLOCK * 2) {
            let frames = output.len() / 2;
            if frames == 0 {
                continue;
            }
            self.process(frames);
            for (frame, sample) in output
                .chunks_exact_mut(2)
                .zip(self.listening[..frames].iter())
            {
                frame.fill(*sample);
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
        if trim_backlog(&self.input, &mut self.scratch, frames) > 0 {
            self.shared
                .metrics
                .input_overruns
                .fetch_add(1, Ordering::Relaxed);
        }
        self.voice[..frames].fill(0.);
        if self.input.pop(&mut self.voice[..frames]) < frames {
            self.shared
                .metrics
                .input_underruns
                .fetch_add(1, Ordering::Relaxed);
        }
        let mut invalid = 0;
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
        self.shared.soundboard.pop(&mut self.scratch[..frames]);
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
        self.shared.preview.pop(&mut self.scratch[..frames]);
        for (out, preview) in self.listening[..frames]
            .iter_mut()
            .zip(&self.scratch[..frames])
        {
            *out = finite_sample(*out + preview, &mut invalid);
        }
        self.shared
            .metrics
            .non_finite_samples
            .fetch_add(invalid, Ordering::Relaxed);
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

fn trim_backlog(queue: &SampleQueue, scratch: &mut [f32], frames: usize) -> usize {
    let backlog = queue.len();
    let mut dropped = 0;
    if backlog > frames.saturating_mul(4) {
        let mut remaining = backlog.saturating_sub(frames.saturating_mul(2));
        while remaining > 0 {
            let count = remaining.min(scratch.len());
            let consumed = queue.pop(&mut scratch[..count]);
            if consumed == 0 {
                break;
            }
            remaining -= consumed;
            dropped += consumed;
        }
    }
    dropped
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
        let output = VoiceOutput::new(shared.clone(), input.clone(), SAMPLE_RATE);
        (shared, input, output)
    }

    #[test]
    fn stereo_input_downmixes_and_sanitizes_before_dsp() {
        let (shared, input, _) = voice();
        let mut capture = InputCapture::new(shared.clone(), input.clone());
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
        let mut capture = InputCapture::new(shared.clone(), input);
        capture.process(&[0.1; 8], 1);
        assert_eq!(shared.metrics.input_overruns.load(Ordering::Relaxed), 1);
        output.render(&mut [0.; 16]);
        assert_eq!(shared.metrics.input_underruns.load(Ordering::Relaxed), 1);
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
        let mut capture = InputCapture::new(shared.clone(), input);
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
