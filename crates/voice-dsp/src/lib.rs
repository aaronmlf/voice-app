//! Prepared mono voice processing. Construction owns allocation; processing does not.
//!
//! Conventional DSP, Signalsmith pitch/formants and RNNoise speech noise suppression.
//! The final safety ceiling has no lookahead: it bounds peaks and may distort overloaded audio.
mod advanced;
use std::f32::consts::TAU;
use voice_core::{DspSettings, EFFECT_COUNT, EffectKind, EffectOrder};

const CEILING: f32 = 0.98;

fn bounded(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn clean(value: f32) -> f32 {
    if value.is_finite() && value.abs() >= 1.0e-20 {
        value
    } else {
        0.0
    }
}

fn coefficient(seconds: f32, rate: f32) -> f32 {
    (-1.0 / (seconds * rate)).exp()
}

fn delay_sample(buffer: &[f32], write: usize, delay: f32) -> f32 {
    let length = buffer.len();
    let position = (write as f32 - delay).rem_euclid(length as f32);
    let read = position as usize;
    // IEEE rounding can return exactly the divisor for a tiny negative remainder. That
    // represents the wrapped start, not an index one past the allocation.
    if read >= length {
        return buffer[0];
    }
    let next = if read + 1 == length { 0 } else { read + 1 };
    buffer[read] + (position - read as f32) * (buffer[next] - buffer[read])
}

#[derive(Clone, Copy)]
struct Controls {
    gain: f32,
    gate: f32,
    hp: f32,
    lp: f32,
    compressor: f32,
    drive: f32,
    robot_mix: f32,
    robot_step: f32,
    echo_mix: f32,
    echo_delay: f32,
    echo_feedback: f32,
    reverb_mix: f32,
    chorus_mix: f32,
}

impl Controls {
    fn from_settings(settings: &DspSettings, rate: f32) -> Self {
        let d = DspSettings::default();
        // Atomic controls can be observed mid-update, so defend even against invalid combinations.
        let hp_hz = bounded(settings.highpass_hz, 10.0, rate * 0.45, d.highpass_hz);
        let lp_hz = bounded(
            settings.lowpass_hz,
            100.0,
            rate * 0.49,
            d.lowpass_hz.min(rate * 0.49),
        )
        .max(hp_hz);
        Self {
            gain: 10.0_f32.powf(bounded(settings.gain_db, -60.0, 24.0, d.gain_db) / 20.0),
            gate: 10.0_f32.powf(bounded(settings.gate_db, -100.0, 0.0, d.gate_db) / 20.0),
            hp: (-TAU * hp_hz / rate).exp(),
            lp: 1.0 - (-TAU * lp_hz / rate).exp(),
            compressor: if settings.compressor { 1.0 } else { 0.0 },
            drive: bounded(settings.drive, 0.0, 1.0, d.drive),
            robot_mix: bounded(settings.robot_mix, 0.0, 1.0, d.robot_mix),
            robot_step: TAU * bounded(settings.robot_hz, 1.0, 2_000.0, d.robot_hz) / rate,
            echo_mix: bounded(settings.echo_mix, 0.0, 1.0, d.echo_mix),
            echo_delay: bounded(settings.echo_delay_ms, 10.0, 1_500.0, d.echo_delay_ms) * rate
                / 1_000.0,
            echo_feedback: bounded(settings.echo_feedback, 0.0, 0.9, d.echo_feedback),
            reverb_mix: bounded(settings.reverb_mix, 0.0, 1.0, d.reverb_mix),
            chorus_mix: bounded(settings.chorus_mix, 0.0, 1.0, d.chorus_mix),
        }
    }

    fn approach(&mut self, target: Self, step: f32) {
        macro_rules! smooth { ($($field:ident),+) => { $(self.$field += (target.$field - self.$field) * step;)+ }; }
        smooth!(
            gain,
            gate,
            hp,
            lp,
            compressor,
            drive,
            robot_mix,
            robot_step,
            echo_mix,
            echo_delay,
            echo_feedback,
            reverb_mix,
            chorus_mix
        );
    }
}

struct Comb {
    delay: Box<[f32]>,
    position: usize,
    damped: f32,
    feedback: f32,
}

impl Comb {
    fn new(rate: f32, delay_seconds: f32, feedback: f32) -> Self {
        Self {
            delay: vec![0.0; (rate * delay_seconds).round() as usize].into_boxed_slice(),
            position: 0,
            damped: 0.0,
            feedback,
        }
    }

    fn tick(&mut self, input: f32) -> f32 {
        let delayed = self.delay[self.position];
        self.damped = clean(self.damped + 0.25 * (delayed - self.damped));
        self.delay[self.position] = clean(input + self.damped * self.feedback);
        self.position += 1;
        if self.position == self.delay.len() {
            self.position = 0;
        }
        delayed
    }

    fn reset(&mut self) {
        self.delay.fill(0.0);
        self.position = 0;
        self.damped = 0.0;
    }
}

/// A mono processor. Prepare one per audio stream on the control thread.
///
/// Input/output samples use normalized floating-point PCM. Every call is bounded by the
/// supplied slice length and uses only preallocated storage. Do not construct or drop this
/// value in an audio callback. Settings and sample data are defensively sanitized.
struct Chain {
    sample_rate: u32,
    controls: Controls,
    smoothing_step: f32,
    envelope_attack: f32,
    envelope_release: f32,
    gate_attack: f32,
    gate_release: f32,
    envelope: f32,
    compressor_envelope: f32,
    gate_gain: f32,
    compressor_gain: f32,
    hp_input: f32,
    hp_output: f32,
    lp_output: f32,
    robot_phase: f32,
    echo: Box<[f32]>,
    echo_position: usize,
    reverb: [Comb; 4],
    pitch: advanced::Pitch,
    denoise: advanced::Denoise,
    chorus: Box<[f32]>,
    chorus_position: usize,
    chorus_phase: f32,
    effect_order: EffectOrder,
    order_initialized: bool,
    transition_gain: f32,
    transition_step: f32,
    effect_mix: [f32; EFFECT_COUNT],
    pitch_dry: AlignmentDelay,
    denoise_dry: AlignmentDelay,
    active_pitch: bool,
    active_denoise: bool,
}

impl Chain {
    /// Allocate echo/reverb storage. Zero means 48 kHz; other rates are bounded to 8–192 kHz.
    pub fn new(sample_rate: u32) -> Self {
        let sample_rate = if sample_rate == 0 {
            48_000
        } else {
            sample_rate.clamp(8_000, 192_000)
        };
        let rate = sample_rate as f32;
        let pitch = advanced::Pitch::new(sample_rate);
        let denoise = advanced::Denoise::new(sample_rate);
        let pitch_dry = AlignmentDelay::new(pitch.latency());
        let denoise_dry = AlignmentDelay::new(denoise.latency());
        Self {
            sample_rate,
            controls: Controls::from_settings(&DspSettings::default(), rate),
            smoothing_step: 1.0 - coefficient(0.010, rate),
            envelope_attack: coefficient(0.003, rate),
            envelope_release: coefficient(0.080, rate),
            gate_attack: coefficient(0.003, rate),
            gate_release: coefficient(0.025, rate),
            envelope: 0.0,
            compressor_envelope: 0.0,
            gate_gain: 0.0,
            compressor_gain: 1.0,
            hp_input: 0.0,
            hp_output: 0.0,
            lp_output: 0.0,
            robot_phase: 0.0,
            echo: vec![0.0; sample_rate as usize * 3 / 2 + 2].into_boxed_slice(),
            echo_position: 0,
            reverb: [
                Comb::new(rate, 0.0297, 0.73),
                Comb::new(rate, 0.0371, 0.77),
                Comb::new(rate, 0.0411, 0.71),
                Comb::new(rate, 0.0437, 0.75),
            ],
            pitch,
            denoise,
            chorus: vec![0.0; sample_rate as usize * 35 / 1_000 + 2].into_boxed_slice(),
            chorus_position: 0,
            chorus_phase: 0.0,
            effect_order: EffectOrder::default(),
            order_initialized: false,
            transition_gain: 1.0,
            transition_step: 1.0 / (0.005 * rate),
            effect_mix: [1.0; EFFECT_COUNT],
            pitch_dry,
            denoise_dry,
            active_pitch: false,
            active_denoise: false,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Minimum latency with optional pitch/formants/denoise disabled.
    pub fn latency_samples(&self) -> usize {
        0
    }

    /// Algorithm buffering only, excluding PipeWire/device queues and intentional echo/reverb.
    /// At 48 kHz: denoise adds 960 frames; pitch/formants add the configured spectral delay
    /// plus a 128-frame streaming adapter. Enabling either restarts its delayed stream.
    pub fn latency_frames(&self, settings: &DspSettings) -> usize {
        let pitch = bounded(settings.pitch_semitones, -24.0, 24.0, 0.0);
        let formant = bounded(settings.formant_semitones, -12.0, 12.0, 0.0);
        let shifted = (pitch.abs() > 1.0e-4 || formant.abs() > 1.0e-4)
            && settings.effect_mix[EffectKind::PitchFormant as usize] > 0.0;
        (if shifted { self.pitch.latency() } else { 0 })
            + if settings.denoise && settings.effect_mix[EffectKind::Denoise as usize] > 0.0 {
                self.denoise.latency()
            } else {
                0
            }
    }

    /// Clear delayed audio and filter/envelope history without allocating.
    /// Large buffers are filled, so prefer calling this while the stream is stopped.
    pub fn reset(&mut self) {
        self.echo.fill(0.0);
        for comb in &mut self.reverb {
            comb.reset();
        }
        self.echo_position = 0;
        self.envelope = 0.0;
        self.compressor_envelope = 0.0;
        self.gate_gain = 0.0;
        self.compressor_gain = 1.0;
        self.hp_input = 0.0;
        self.hp_output = 0.0;
        self.lp_output = 0.0;
        self.robot_phase = 0.0;
        self.pitch.reset();
        self.denoise.reset();
        self.chorus.fill(0.0);
        self.chorus_position = 0;
        self.chorus_phase = 0.0;
        self.order_initialized = false;
        self.transition_gain = 1.0;
        self.effect_mix = [1.0; EFFECT_COUNT];
        self.pitch_dry.reset();
        self.denoise_dry.reset();
        self.active_pitch = false;
        self.active_denoise = false;
    }

    /// Process normalized mono PCM in place, with no allocation, locks, logging or I/O.
    /// Invalid input/configuration produces silence. A new order fades down for 5 ms,
    /// switches at zero and fades up for 5 ms, preserving prepared effect histories.
    /// The first nonempty call adopts its order immediately, including offline renders.
    pub fn process(&mut self, samples: &mut [f32], settings: &DspSettings) {
        if !settings.effect_order.is_valid() || !valid_opacities(&settings.effect_mix) {
            samples.fill(0.0);
            return;
        }
        if samples.is_empty() {
            return;
        }
        let requested_pitch = (settings.pitch_semitones.abs() > 1.0e-4
            || settings.formant_semitones.abs() > 1.0e-4)
            && settings.effect_mix[EffectKind::PitchFormant as usize] > 0.0;
        let requested_denoise =
            settings.denoise && settings.effect_mix[EffectKind::Denoise as usize] > 0.0;
        if !self.order_initialized {
            self.effect_order = settings.effect_order;
            self.effect_mix = settings.effect_mix;
            self.active_pitch = requested_pitch;
            self.active_denoise = requested_denoise;
            self.order_initialized = true;
        }
        let target = Controls::from_settings(settings, self.sample_rate as f32);
        let pitch = bounded(settings.pitch_semitones, -24.0, 24.0, 0.0);
        let formant = bounded(settings.formant_semitones, -12.0, 12.0, 0.0);
        for sample in samples {
            self.controls.approach(target, self.smoothing_step);
            let c = self.controls;
            // The slew starts at its current level, so rapid edits never jump the
            // output gain back to one. Only the latest requested order is applied.
            if self.effect_order != settings.effect_order
                || self.active_pitch != requested_pitch
                || self.active_denoise != requested_denoise
            {
                self.transition_gain = (self.transition_gain - self.transition_step).max(0.0);
                if self.transition_gain == 0.0 {
                    self.effect_order = settings.effect_order;
                    if self.active_pitch != requested_pitch {
                        self.pitch_dry.reset();
                        self.pitch.reset();
                    }
                    if self.active_denoise != requested_denoise {
                        self.denoise_dry.reset();
                        self.denoise.reset();
                    }
                    self.active_pitch = requested_pitch;
                    self.active_denoise = requested_denoise;
                }
            } else {
                self.transition_gain = (self.transition_gain + self.transition_step).min(1.0);
            }
            let valid = sample.is_finite();
            let mut value = if valid {
                sample.clamp(-16.0, 16.0)
            } else {
                0.0
            };
            for effect in self.effect_order.0 {
                let index = effect as usize;
                let target_mix = settings.effect_mix[index];
                self.effect_mix[index] +=
                    (target_mix - self.effect_mix[index]) * self.smoothing_step;
                // Snap exact endpoints once inaudibly close; this also makes 0 a true bypass.
                if (self.effect_mix[index] - target_mix).abs() < 1.0e-6 {
                    self.effect_mix[index] = target_mix;
                }
                let dry = match effect {
                    EffectKind::PitchFormant if self.active_pitch => {
                        self.pitch_dry.tick(value, self.pitch.latency())
                    }
                    EffectKind::Denoise if self.active_denoise => {
                        self.denoise_dry.tick(value, self.denoise.latency())
                    }
                    _ => value,
                };
                let wet = clean(self.process_stage(
                    effect,
                    value,
                    &c,
                    if self.active_pitch { pitch } else { 0.0 },
                    if self.active_pitch { formant } else { 0.0 },
                    self.active_denoise,
                ));
                let mix = self.effect_mix[index];
                value = if mix == 1.0 {
                    wet
                } else if mix == 0.0 {
                    dry
                } else {
                    clean(dry * (1.0 - mix) + wet * mix)
                };
            }
            // This ceiling cannot be reordered or bypassed by a preset.
            // Bound before the fade too: a very hot chain must fade smoothly rather
            // than remain clipped until the last few samples of the transition.
            let value = clean(value).clamp(-CEILING, CEILING);
            *sample = if valid {
                clean(value * self.transition_gain).clamp(-CEILING, CEILING)
            } else {
                0.0
            };
        }
    }

    #[inline]
    fn process_stage(
        &mut self,
        effect: EffectKind,
        input: f32,
        c: &Controls,
        pitch: f32,
        formant: f32,
        denoise: bool,
    ) -> f32 {
        match effect {
            EffectKind::Denoise => self.denoise.tick(input, denoise),
            EffectKind::PitchFormant => self.pitch.tick(input, pitch, formant),
            EffectKind::Gate => {
                let level = input.abs();
                let detector = if level > self.envelope {
                    self.envelope_attack
                } else {
                    self.envelope_release
                };
                self.envelope = clean(level + detector * (self.envelope - level));
                // A soft 6 dB transition avoids chatter around the gate threshold.
                let gate_target = ((self.envelope / c.gate - 0.5) * 2.0).clamp(0.0, 1.0);
                let gate_coeff = if gate_target > self.gate_gain {
                    self.gate_attack
                } else {
                    self.gate_release
                };
                self.gate_gain = clean(gate_target + gate_coeff * (self.gate_gain - gate_target));
                input * self.gate_gain
            }
            EffectKind::Gain => input * c.gain,
            EffectKind::Filter => {
                let high = c.hp * (self.hp_output + input - self.hp_input);
                self.hp_input = input;
                self.hp_output = clean(high);
                self.lp_output = clean(self.lp_output + c.lp * (high - self.lp_output));
                self.lp_output
            }
            EffectKind::Compressor => {
                // Each detector follows the signal at its own stage. Sharing the
                // gate envelope would make compression depend on that stage's position.
                let level = input.abs();
                let detector = if level > self.compressor_envelope {
                    self.envelope_attack
                } else {
                    self.envelope_release
                };
                self.compressor_envelope =
                    clean(level + detector * (self.compressor_envelope - level));
                let threshold = 0.125_892_53; // -18 dBFS, feed-forward 4:1 compression.
                let compression = if self.compressor_envelope > threshold {
                    (threshold / self.compressor_envelope).powf(0.75)
                } else {
                    1.0
                };
                let desired_gain = 1.0 + c.compressor * (compression - 1.0);
                let comp_coeff = if desired_gain < self.compressor_gain {
                    self.envelope_attack
                } else {
                    self.envelope_release
                };
                self.compressor_gain =
                    desired_gain + comp_coeff * (self.compressor_gain - desired_gain);
                input * self.compressor_gain
            }
            EffectKind::Distortion => {
                if c.drive > 1.0e-6 {
                    let amount = 1.0 + c.drive * 20.0;
                    let distorted = (input * amount).tanh() / amount.tanh();
                    input + c.drive * (distorted - input)
                } else {
                    input
                }
            }
            EffectKind::Robot => {
                let ring = self.robot_phase.sin();
                self.robot_phase += c.robot_step;
                if self.robot_phase >= TAU {
                    self.robot_phase -= TAU;
                }
                input * (1.0 - c.robot_mix + c.robot_mix * ring)
            }
            EffectKind::Chorus => {
                let rate = self.sample_rate as f32;
                self.chorus[self.chorus_position] = input;
                let mut chorus = 0.0;
                for offset in [0.0, std::f32::consts::FRAC_PI_2] {
                    let delay = rate * (0.012 + 0.004 * (self.chorus_phase + offset).sin());
                    chorus += delay_sample(&self.chorus, self.chorus_position, delay);
                }
                self.chorus_position += 1;
                if self.chorus_position == self.chorus.len() {
                    self.chorus_position = 0;
                }
                self.chorus_phase += TAU * 0.28 / rate;
                if self.chorus_phase >= TAU {
                    self.chorus_phase -= TAU;
                }
                input + chorus * (0.35 * c.chorus_mix)
            }
            EffectKind::Echo => {
                let delayed = delay_sample(&self.echo, self.echo_position, c.echo_delay);
                self.echo[self.echo_position] =
                    clean((input + delayed * c.echo_feedback).clamp(-16.0, 16.0));
                self.echo_position += 1;
                if self.echo_position == self.echo.len() {
                    self.echo_position = 0;
                }
                input + delayed * c.echo_mix
            }
            EffectKind::Reverb => {
                let mut reverberated = 0.0;
                for comb in &mut self.reverb {
                    reverberated += comb.tick(input.clamp(-16.0, 16.0));
                }
                input + reverberated * (0.25 * c.reverb_mix)
            }
        }
    }
}

/// Integer delay prepared outside the callback. Resetting its readable history is O(1);
/// stale samples cannot escape after a topology change.
struct AlignmentDelay {
    samples: Box<[f32]>,
    position: usize,
    filled: usize,
}
impl AlignmentDelay {
    fn new(maximum: usize) -> Self {
        Self {
            samples: vec![0.0; maximum + 1].into_boxed_slice(),
            position: 0,
            filled: 0,
        }
    }
    fn reset(&mut self) {
        self.position = 0;
        self.filled = 0;
    }
    fn tick(&mut self, input: f32, delay: usize) -> f32 {
        if delay >= self.samples.len() {
            return 0.0;
        }
        self.samples[self.position] = clean(input);
        let read = (self.position + self.samples.len() - delay) % self.samples.len();
        let output = if self.filled >= delay {
            self.samples[read]
        } else {
            0.0
        };
        self.position = (self.position + 1) % self.samples.len();
        self.filled = (self.filled + 1).min(self.samples.len());
        output
    }
}
fn valid_opacities(values: &[f32; EFFECT_COUNT]) -> bool {
    values
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
}
fn valid_level(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

/// Two prepared mono chains. A legacy preset uses only the primary chain; an optional
/// second voice consumes bounded scratch/delay storage and never allocates in process().
/// Branch levels are normalized by max(1, their sum), then the final safety ceiling applies.
pub struct Processor {
    primary: Chain,
    secondary: Chain,
    primary_delay: AlignmentDelay,
    secondary_delay: AlignmentDelay,
    scratch: [f32; 256],
    route: Option<(bool, usize, usize, usize, u8)>,
    mix: [f32; 2],
    mix_initialized: bool,
    smoothing_step: f32,
    transition_frames: usize,
    fade_out: usize,
    warmup_frames: usize,
    fade_in: usize,
    held_output: f32,
    last_output: f32,
}
impl Processor {
    pub fn new(sample_rate: u32) -> Self {
        let primary = Chain::new(sample_rate);
        let secondary = Chain::new(sample_rate);
        let maximum = primary.pitch.latency() + primary.denoise.latency();
        let rate = primary.sample_rate() as f32;
        Self {
            primary,
            secondary,
            primary_delay: AlignmentDelay::new(maximum),
            secondary_delay: AlignmentDelay::new(maximum),
            scratch: [0.0; 256],
            route: None,
            mix: [1.0, 0.0],
            mix_initialized: false,
            smoothing_step: 1.0 - coefficient(0.010, rate),
            transition_frames: (0.005 * rate).round().max(1.0) as usize,
            fade_out: 0,
            warmup_frames: 0,
            fade_in: 0,
            held_output: 0.0,
            last_output: 0.0,
        }
    }
    pub fn sample_rate(&self) -> u32 {
        self.primary.sample_rate()
    }
    pub fn latency_samples(&self) -> usize {
        self.primary.latency_samples()
    }
    pub fn latency_frames(&self, settings: &DspSettings) -> usize {
        let primary = self.primary.latency_frames(settings);
        match settings.blend {
            Some(blend) => {
                let secondary = self
                    .secondary
                    .latency_frames(&blend.secondary.to_settings());
                (if blend.primary_mix > 0.0 { primary } else { 0 }).max(
                    if blend.secondary_mix > 0.0 {
                        secondary
                    } else {
                        0
                    },
                )
            }
            None => primary,
        }
    }
    pub fn reset(&mut self) {
        self.primary.reset();
        self.secondary.reset();
        self.primary_delay.reset();
        self.secondary_delay.reset();
        self.route = None;
        self.mix = [1.0, 0.0];
        self.mix_initialized = false;
        self.fade_out = 0;
        self.warmup_frames = 0;
        self.fade_in = 0;
        self.held_output = 0.0;
        self.last_output = 0.0;
    }
    pub fn process(&mut self, samples: &mut [f32], settings: &DspSettings) {
        if samples.is_empty() {
            return;
        }
        if !settings.effect_order.is_valid() || !valid_opacities(&settings.effect_mix) {
            samples.fill(0.0);
            self.last_output = 0.0;
            return;
        }
        let secondary_settings = settings.blend.map(|blend| blend.secondary.to_settings());
        if let Some(blend) = settings.blend
            && (!valid_level(blend.primary_mix)
                || !valid_level(blend.secondary_mix)
                || !blend.secondary.effect_order.is_valid()
                || !valid_opacities(&blend.secondary.effect_mix))
        {
            samples.fill(0.0);
            self.last_output = 0.0;
            return;
        }
        let primary_latency = self.primary.latency_frames(settings);
        let secondary_latency = secondary_settings
            .as_ref()
            .map_or(0, |s| self.secondary.latency_frames(s));
        let total_latency = self.latency_frames(settings);
        let requested_mix = settings
            .blend
            .map_or([1.0, 0.0], |b| [b.primary_mix, b.secondary_mix]);
        let audible = u8::from(requested_mix[0] > 0.0) | (u8::from(requested_mix[1] > 0.0) << 1);
        let route = (
            settings.blend.is_some(),
            primary_latency,
            secondary_latency,
            total_latency,
            audible,
        );
        if self.route != Some(route) {
            if self.route.is_some() {
                self.fade_out = self.transition_frames;
                // The internal latency stage may switch only after its own fade-out.
                // Wait for fresh delayed samples before fading the new route back in.
                self.warmup_frames = total_latency;
                self.fade_in = self.transition_frames;
                self.held_output = self.last_output;
            }
            // A reused secondary chain must never release audio from an older mix.
            if route.0 && route.4 & 2 != 0 && self.route.is_none_or(|old| !old.0 || old.4 & 2 == 0)
            {
                self.secondary.reset();
            }
            self.primary_delay.reset();
            self.secondary_delay.reset();
            self.route = Some(route);
        }
        if !self.mix_initialized || settings.blend.is_none() {
            self.mix = requested_mix;
            self.mix_initialized = true;
        }
        if requested_mix == [0.0, 0.0] {
            self.held_output = 0.0;
            self.fade_out = 0;
            self.warmup_frames = 0;
            self.fade_in = 0;
        }
        if settings.blend.is_none()
            && self.fade_out == 0
            && self.warmup_frames == 0
            && self.fade_in == 0
        {
            self.primary.process(samples, settings);
            self.last_output = *samples.last().unwrap_or(&0.0);
            return;
        }
        for chunk in samples.chunks_mut(self.scratch.len()) {
            let length = chunk.len();
            self.scratch[..length].copy_from_slice(chunk);
            let valid: [bool; 256] =
                std::array::from_fn(|index| index >= length || chunk[index].is_finite());
            self.primary.process(chunk, settings);
            if let Some(other) = &secondary_settings {
                self.secondary.process(&mut self.scratch[..length], other);
            }
            for index in 0..length {
                for (mix, target) in self.mix.iter_mut().zip(requested_mix) {
                    // A muted voice cannot retain a smoothed contribution after its
                    // latency has been removed from the alignment calculation.
                    if target == 0.0 {
                        *mix = 0.0;
                    } else {
                        *mix += (target - *mix) * self.smoothing_step;
                    }
                    if (*mix - target).abs() < 1.0e-6 {
                        *mix = target;
                    }
                }
                let p = self
                    .primary_delay
                    .tick(chunk[index], total_latency.saturating_sub(primary_latency));
                let mut value = if settings.blend.is_some() {
                    let s = self.secondary_delay.tick(
                        self.scratch[index],
                        total_latency.saturating_sub(secondary_latency),
                    );
                    let denominator = (self.mix[0] + self.mix[1]).max(1.0);
                    clean((p * self.mix[0] + s * self.mix[1]) / denominator)
                } else {
                    p
                };
                if self.fade_out > 0 {
                    self.fade_out -= 1;
                    value = self.held_output * self.fade_out as f32 / self.transition_frames as f32;
                } else if self.warmup_frames > 0 {
                    self.warmup_frames -= 1;
                    value = 0.0;
                } else if self.fade_in > 0 {
                    value *= 1.0 - self.fade_in as f32 / self.transition_frames as f32;
                    self.fade_in -= 1;
                }
                chunk[index] = if valid[index] && requested_mix != [0.0, 0.0] {
                    clean(value).clamp(-CEILING, CEILING)
                } else {
                    0.0
                };
                self.last_output = chunk[index];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct TrackingAllocator;
    thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; static CALLS: Cell<usize> = const { Cell::new(0) }; }
    #[global_allocator]
    static ALLOCATOR: TrackingAllocator = TrackingAllocator;
    // Only the test allocator uses unsafe, delegating the original pointer/layout to System.
    unsafe impl GlobalAlloc for TrackingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if TRACK.try_with(Cell::get).unwrap_or(false) {
                let _ = CALLS.try_with(|n| n.set(n.get() + 1));
            }
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if TRACK.try_with(Cell::get).unwrap_or(false) {
                let _ = CALLS.try_with(|n| n.set(n.get() + 1));
            }
            unsafe {
                System.dealloc(ptr, layout);
            }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            if TRACK.try_with(Cell::get).unwrap_or(false) {
                let _ = CALLS.try_with(|n| n.set(n.get() + 1));
            }
            unsafe { System.realloc(ptr, layout, size) }
        }
    }

    fn sine(frequency: f32, amplitude: f32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| (TAU * frequency * i as f32 / 48_000.0).sin() * amplitude)
            .collect()
    }
    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt()
    }
    fn settled(settings: DspSettings, frequency: f32, amplitude: f32) -> Vec<f32> {
        let mut processor = Processor::new(48_000);
        let mut audio = sine(frequency, amplitude, 48_000);
        processor.process(&mut audio, &settings);
        audio
    }

    #[test]
    fn silence_and_invalid_inputs_are_finite_and_bounded() {
        let mut processor = Processor::new(48_000);
        let mut silence = [0.0; 512];
        processor.process(&mut silence, &DspSettings::default());
        assert!(silence.iter().all(|v| *v == 0.0));
        let mut input = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            -f32::MAX,
            0.5,
        ];
        let corrupt = DspSettings {
            effect_order: EffectOrder::default(),
            gain_db: f32::NAN,
            gate_db: f32::INFINITY,
            highpass_hz: f32::MAX,
            lowpass_hz: -f32::MAX,
            drive: f32::NAN,
            robot_mix: f32::INFINITY,
            robot_hz: f32::NAN,
            echo_mix: 2.0,
            echo_delay_ms: f32::NEG_INFINITY,
            echo_feedback: f32::NAN,
            reverb_mix: f32::MAX,
            compressor: true,
            pitch_semitones: f32::NAN,
            formant_semitones: f32::INFINITY,
            chorus_mix: f32::NAN,
            denoise: true,
            ..Default::default()
        };
        processor.process(&mut input, &corrupt);
        assert_eq!(&input[..3], &[0.0; 3]);
        assert!(input.iter().all(|v| v.is_finite() && v.abs() <= CEILING));
        processor.process(&mut silence, &DspSettings::default());
        assert!(silence.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn default_new_controls_preserve_the_existing_single_chain_bit_for_bit() {
        let settings = DspSettings {
            gain_db: -3.0,
            compressor: true,
            drive: 0.2,
            robot_mix: 0.2,
            echo_mix: 0.1,
            reverb_mix: 0.1,
            pitch_semitones: -3.0,
            formant_semitones: -1.0,
            ..Default::default()
        };
        let mut original = sine(220.0, 0.1, 16_000);
        let mut wrapped = original.clone();
        Chain::new(48_000).process(&mut original, &settings);
        Processor::new(48_000).process(&mut wrapped, &settings);
        assert_eq!(original, wrapped);
    }

    #[test]
    fn effect_opacity_has_true_bypass_endpoints_and_interpolates() {
        let mut settings = DspSettings {
            gain_db: 12.0,
            ..Default::default()
        };
        let mut dry_settings = settings;
        dry_settings.effect_mix[EffectKind::Gain as usize] = 0.0;
        let dry = settled(dry_settings, 440.0, 0.03);
        let wet = settled(settings, 440.0, 0.03);
        settings.effect_mix[EffectKind::Gain as usize] = 0.5;
        let mixed = settled(settings, 440.0, 0.03);
        for ((dry, wet), mixed) in dry.iter().zip(&wet).zip(&mixed) {
            assert!((mixed - (dry + wet) * 0.5).abs() < 2.0e-6);
        }
        let bypass = DspSettings {
            effect_mix: [0.0; EFFECT_COUNT],
            gain_db: 24.0,
            gate_db: 0.0,
            pitch_semitones: -12.0,
            denoise: true,
            ..Default::default()
        };
        let original = sine(220.0, 0.1, 1024);
        let mut output = original.clone();
        let mut processor = Processor::new(48_000);
        assert_eq!(processor.latency_frames(&bypass), 0);
        processor.process(&mut output, &bypass);
        assert_eq!(output, original);
    }

    #[test]
    fn partial_pitch_opacity_aligns_its_dry_voice_to_the_spectral_delay() {
        let source = sine(220.0, 0.1, 16_000);
        let settings = DspSettings {
            effect_mix: {
                let mut mix = [0.0; EFFECT_COUNT];
                mix[EffectKind::PitchFormant as usize] = 1.0;
                mix
            },
            pitch_semitones: 7.0,
            ..Default::default()
        };
        let mut processor = Processor::new(48_000);
        let latency = processor.latency_frames(&settings);
        assert_eq!(latency, 4224);
        let mut wet = source.clone();
        processor.process(&mut wet, &settings);
        let mut half_settings = settings;
        half_settings.effect_mix[EffectKind::PitchFormant as usize] = 0.5;
        let mut half = source.clone();
        Processor::new(48_000).process(&mut half, &half_settings);
        for index in 0..source.len() {
            let dry = if index >= latency {
                source[index - latency]
            } else {
                0.0
            };
            assert!(
                (half[index] - (dry + wet[index]) * 0.5).abs() < 1.0e-6,
                "frame {index}"
            );
        }
    }

    #[test]
    fn two_voices_match_independent_aligned_renders_and_normalize_headroom() {
        use voice_core::{VoiceBlend, VoiceLayer};
        let source = sine(220.0, 0.1, 16_000);
        let primary = DspSettings {
            effect_mix: [0.0; EFFECT_COUNT],
            ..Default::default()
        };
        let secondary = DspSettings {
            pitch_semitones: 7.0,
            effect_mix: {
                let mut mix = [0.0; EFFECT_COUNT];
                mix[EffectKind::PitchFormant as usize] = 1.0;
                mix
            },
            ..Default::default()
        };
        let mut primary_output = source.clone();
        Processor::new(48_000).process(&mut primary_output, &primary);
        let mut secondary_output = source.clone();
        Processor::new(48_000).process(&mut secondary_output, &secondary);
        let combined = DspSettings {
            blend: Some(VoiceBlend {
                secondary: VoiceLayer::from(secondary),
                primary_mix: 1.0,
                secondary_mix: 1.0,
            }),
            ..primary
        };
        let mut processor = Processor::new(48_000);
        let latency = processor.latency_frames(&combined);
        assert_eq!(latency, 4224);
        let mut output = source.clone();
        processor.process(&mut output, &combined);
        for index in 0..output.len() {
            let dry = if index >= latency {
                primary_output[index - latency]
            } else {
                0.0
            };
            assert!(
                (output[index] - (dry + secondary_output[index]) * 0.5).abs() < 1.0e-6,
                "frame {index}"
            );
        }
        let mut muted = combined;
        muted.blend.as_mut().unwrap().primary_mix = 0.0;
        muted.blend.as_mut().unwrap().secondary_mix = 0.0;
        processor.process(&mut output, &muted);
        assert!(output.iter().all(|value| *value == 0.0));
    }

    #[test]
    fn muting_and_reopening_one_voice_realigns_without_a_sample_jump() {
        use voice_core::{VoiceBlend, VoiceLayer};
        let settings = DspSettings {
            pitch_semitones: 7.0,
            effect_mix: {
                let mut mix = [0.0; EFFECT_COUNT];
                mix[EffectKind::PitchFormant as usize] = 1.0;
                mix
            },
            blend: Some(VoiceBlend {
                secondary: VoiceLayer::from(DspSettings {
                    effect_mix: [0.0; EFFECT_COUNT],
                    ..Default::default()
                }),
                primary_mix: 0.5,
                secondary_mix: 0.5,
            }),
            ..Default::default()
        };
        let mut processor = Processor::new(48_000);
        let mut chunked_processor = Processor::new(48_000);
        let mut warm = sine(220.0, 0.1, 16_000);
        let mut other_warm = warm.clone();
        processor.process(&mut warm, &settings);
        chunked_processor.process(&mut other_warm, &settings);
        let mut last = *warm.last().unwrap();
        let mut cursor = warm.len();
        for (primary, secondary) in [(0.0, 0.5), (0.5, 0.5), (0.5, 0.0), (0.5, 0.5)] {
            let mut edited = settings;
            let blend = edited.blend.as_mut().unwrap();
            blend.primary_mix = primary;
            blend.secondary_mix = secondary;
            let expected_latency = if primary == 0.0 { 0 } else { 4224 };
            assert_eq!(processor.latency_frames(&edited), expected_latency);
            let mut changed: Vec<_> = (cursor..cursor + 8192)
                .map(|index| (TAU * 220.0 * index as f32 / 48_000.0).sin() * 0.1)
                .collect();
            let mut chunked = changed.clone();
            processor.process(&mut changed, &edited);
            for block in chunked.chunks_mut(37) {
                chunked_processor.process(block, &edited);
            }
            assert_eq!(changed, chunked);
            let mut jump = 0.0_f32;
            for &value in &changed {
                assert!(value.is_finite() && value.abs() <= CEILING);
                jump = jump.max((value - last).abs());
                last = value;
            }
            assert!(
                jump < 0.02,
                "audible branch change created a jump of {jump}"
            );
            assert!(rms(&changed[6000..]) > 0.01);
            if primary == 0.0 {
                assert_eq!(processor.mix[0], 0.0);
            }
            if secondary == 0.0 {
                assert_eq!(processor.mix[1], 0.0);
            }
            cursor += changed.len();
        }
    }

    #[test]
    fn voice_mix_changes_are_finite_chunk_independent_and_do_not_allocate_or_free() {
        use voice_core::{VoiceBlend, VoiceLayer};
        let base = DspSettings {
            pitch_semitones: 3.0,
            denoise: true,
            effect_mix: [0.6; EFFECT_COUNT],
            blend: Some(VoiceBlend {
                secondary: VoiceLayer::from(DspSettings {
                    robot_mix: 0.5,
                    ..Default::default()
                }),
                primary_mix: 0.7,
                secondary_mix: 0.4,
            }),
            ..Default::default()
        };
        let mut processor = Processor::new(48_000);
        let mut chunked_processor = Processor::new(48_000);
        let source = sine(220.0, 0.1, 4096);
        let mut whole = source.clone();
        let mut chunks = source.clone();
        processor.process(&mut whole, &base);
        for block in chunks.chunks_mut(37) {
            chunked_processor.process(block, &base);
        }
        assert_eq!(whole, chunks);
        let mut block = [0.1; 513];
        CALLS.with(|n| n.set(0));
        TRACK.with(|v| v.set(true));
        for index in 0..8 {
            let mut edited = base;
            if index % 2 == 0 {
                edited.effect_mix[EffectKind::PitchFormant as usize] = 0.0;
                edited.effect_order.0.reverse();
            }
            if index % 3 == 0 {
                edited.blend = None;
            }
            processor.process(&mut block, &edited);
        }
        let mut invalid = base;
        invalid.blend.as_mut().unwrap().secondary_mix = f32::NAN;
        processor.process(&mut block, &invalid);
        TRACK.with(|v| v.set(false));
        assert_eq!(CALLS.with(Cell::get), 0);
        assert!(block.iter().all(|value| *value == 0.0));
    }

    #[test]
    fn gain_and_gate_have_audible_signal_effects() {
        let plain = settled(DspSettings::default(), 1_000.0, 0.05);
        let gain = settled(
            DspSettings {
                gain_db: 6.0,
                ..Default::default()
            },
            1_000.0,
            0.05,
        );
        let ratio = rms(&gain[24_000..]) / rms(&plain[24_000..]);
        assert!((ratio - 1.995).abs() < 0.03, "gain ratio {ratio}");
        let gated = settled(
            DspSettings {
                gate_db: -20.0,
                ..Default::default()
            },
            1_000.0,
            0.001,
        );
        assert!(rms(&gated[24_000..]) < 1.0e-7);
        assert!(rms(&plain[24_000..]) > 0.02);
    }

    #[test]
    fn changing_gain_and_distortion_order_changes_rendered_audio() {
        let before = DspSettings {
            gain_db: -18.0,
            drive: 1.0,
            ..Default::default()
        };
        let mut after = before;
        after.effect_order.0.swap(3, 6); // Gain before/after the nonlinear distortion.
        let a = settled(before, 500.0, 0.2);
        let b = settled(after, 500.0, 0.2);
        assert!(rms(&a[24_000..]) > rms(&b[24_000..]) * 2.5);
        assert!(rms(&b[24_000..]) > 0.03);
        assert!(
            a.iter()
                .chain(&b)
                .all(|x| x.is_finite() && x.abs() <= CEILING)
        );
    }

    #[test]
    fn moving_effects_fades_without_a_discontinuous_sample_jump_or_chunk_dependence() {
        let mut a = Processor::new(48_000);
        let mut b = Processor::new(48_000);
        let before = DspSettings {
            gain_db: -18.0,
            drive: 1.0,
            ..Default::default()
        };
        let mut after = before;
        after.effect_order.0.swap(3, 6);
        // Change at a carrier peak: an immediate order switch would produce a
        // large discontinuity between the saturated and attenuated waveforms.
        let warm_frames = 48_120;
        let mut warm = sine(100.0, 0.2, warm_frames);
        let mut other_warm = warm.clone();
        a.process(&mut warm, &before);
        b.process(&mut other_warm, &before);
        let previous = *warm.last().unwrap();
        let mut changed: Vec<_> = (warm_frames..warm_frames + 1_024)
            .map(|i| (TAU * 100.0 * i as f32 / 48_000.0).sin() * 0.2)
            .collect();
        let mut chunked = changed.clone();
        a.process(&mut changed, &after);
        for chunk in chunked.chunks_mut(37) {
            b.process(chunk, &after);
        }
        assert_eq!(changed, chunked);
        let (_, jump) = changed
            .iter()
            .fold((previous, 0.0_f32), |(last, jump), &value| {
                (value, jump.max((value - last).abs()))
            });
        assert!(jump < 0.04, "order switch created a sample jump of {jump}");
        assert!(rms(&changed[700..]) > 0.03, "new route remained silent");
        assert!(changed.iter().all(|x| x.is_finite() && x.abs() <= CEILING));
    }

    #[test]
    fn moving_an_effect_preserves_existing_echo_history() {
        let mut processor = Processor::new(48_000);
        let mut settings = DspSettings {
            echo_mix: 1.0,
            echo_delay_ms: 100.0,
            echo_feedback: 0.5,
            ..Default::default()
        };
        let mut warm = vec![0.0; 48_000];
        processor.process(&mut warm, &settings);
        let mut burst = sine(1_000.0, 0.1, 480);
        processor.process(&mut burst, &settings);
        settings.effect_order.0.swap(3, 6);
        let mut tail = vec![0.0; 9_600];
        processor.process(&mut tail, &settings);
        assert!(
            rms(&tail[4_320..4_800]) > 0.01,
            "moving an effect erased the pending echo"
        );
    }

    #[test]
    fn invalid_effect_orders_are_silenced_without_allocating() {
        let mut processor = Processor::new(48_000);
        let mut settings = DspSettings::default();
        settings.effect_order.0[1] = settings.effect_order.0[0];
        let mut block = [0.2; 512];
        CALLS.with(|n| n.set(0));
        TRACK.with(|v| v.set(true));
        processor.process(&mut block, &settings);
        TRACK.with(|v| v.set(false));
        assert_eq!(CALLS.with(Cell::get), 0);
        assert_eq!(block, [0.0; 512]);
    }

    #[test]
    fn reversed_and_rotated_orders_stay_finite_with_all_effects_at_extremes() {
        for rotation in 0..=voice_core::EFFECT_COUNT {
            let mut settings = DspSettings {
                gain_db: 24.0,
                gate_db: -100.0,
                highpass_hz: 10.0,
                lowpass_hz: 23_000.0,
                compressor: true,
                drive: 1.0,
                robot_mix: 1.0,
                robot_hz: 2_000.0,
                echo_mix: 1.0,
                echo_delay_ms: 10.0,
                echo_feedback: 0.9,
                reverb_mix: 1.0,
                pitch_semitones: 7.0,
                formant_semitones: -3.0,
                chorus_mix: 1.0,
                denoise: true,
                ..Default::default()
            };
            if rotation == voice_core::EFFECT_COUNT {
                settings.effect_order.0.reverse();
            } else {
                settings.effect_order.0.rotate_left(rotation);
            }
            let mut processor = Processor::new(48_000);
            let mut signal = sine(220.0, 0.8, 12_288);
            processor.process(&mut signal, &settings);
            assert!(
                signal.iter().all(|x| x.is_finite() && x.abs() <= CEILING),
                "rotation {rotation}"
            );
        }
    }

    #[test]
    fn filters_reject_dc_bass_and_treble() {
        let default = DspSettings::default();
        let bass = settled(default, 50.0, 0.1);
        let cut_bass = settled(
            DspSettings {
                highpass_hz: 800.0,
                ..default
            },
            50.0,
            0.1,
        );
        assert!(rms(&cut_bass[24_000..]) < rms(&bass[24_000..]) * 0.09);
        let treble = settled(default, 10_000.0, 0.1);
        let cut_treble = settled(
            DspSettings {
                lowpass_hz: 800.0,
                ..default
            },
            10_000.0,
            0.1,
        );
        assert!(rms(&cut_treble[24_000..]) < rms(&treble[24_000..]) * 0.15);
        let mut dc = vec![0.2; 48_000];
        Processor::new(48_000).process(&mut dc, &default);
        assert!(rms(&dc[24_000..]) < 1.0e-5);
    }

    #[test]
    fn compressor_reduces_loud_audio_and_drive_creates_harmonics() {
        let dry = settled(DspSettings::default(), 500.0, 0.6);
        let compressed = settled(
            DspSettings {
                compressor: true,
                ..Default::default()
            },
            500.0,
            0.6,
        );
        assert!(rms(&compressed[24_000..]) < rms(&dry[24_000..]) * 0.5);
        let driven = settled(
            DspSettings {
                drive: 1.0,
                ..Default::default()
            },
            500.0,
            0.05,
        );
        let harmonic = |a: &[f32], hz: f32| -> f32 {
            let (s, c) = a.iter().enumerate().fold((0.0, 0.0), |(s, c), (i, x)| {
                let phase = TAU * hz * i as f32 / 48_000.0;
                (s + x * phase.sin(), c + x * phase.cos())
            });
            (s * s + c * c).sqrt() / a.len() as f32
        };
        let driven_harmonic = harmonic(&driven[24_000..], 1_500.0);
        let clean = settled(DspSettings::default(), 500.0, 0.05);
        assert!(driven_harmonic > 0.01);
        assert!(driven_harmonic > harmonic(&clean[24_000..], 1_500.0) * 100.0);
    }

    #[test]
    fn robot_moves_a_tone_to_sidebands() {
        let output = settled(
            DspSettings {
                robot_mix: 1.0,
                robot_hz: 100.0,
                ..Default::default()
            },
            1_000.0,
            0.1,
        );
        let energy = |hz: f32| -> f32 {
            let (s, c) = output[24_000..]
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(s, c), (i, x)| {
                    let phase = TAU * hz * i as f32 / 48_000.0;
                    (s + x * phase.sin(), c + x * phase.cos())
                });
            (s * s + c * c).sqrt()
        };
        // Ring modulation removes the original carrier and produces 900/1100 Hz components.
        assert!(energy(900.0) > energy(1_000.0) * 30.0);
        assert!(energy(1_100.0) > energy(1_000.0) * 30.0);
    }

    #[test]
    fn echo_and_reverb_have_real_decaying_tails_and_reset_clears_them() {
        let mut processor = Processor::new(48_000);
        let settings = DspSettings {
            echo_mix: 1.0,
            echo_delay_ms: 100.0,
            echo_feedback: 0.5,
            reverb_mix: 0.5,
            ..Default::default()
        };
        let mut warmup = vec![0.0; 48_000];
        processor.process(&mut warmup, &settings);
        let mut signal = vec![0.0; 96_000];
        signal[..480].copy_from_slice(&sine(1_000.0, 0.1, 480));
        processor.process(&mut signal, &settings);
        assert!(rms(&signal[4_800..5_280]) > 0.01, "100 ms echo must exist");
        assert!(
            rms(&signal[1_400..2_600]) > 0.0001,
            "short reverb tail must exist"
        );
        assert!(rms(&signal[72_000..]) < rms(&signal[4_800..9_600]) * 0.03);
        processor.reset();
        warmup.fill(0.0);
        processor.process(&mut warmup, &settings);
        assert!(warmup.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn controls_are_smoothed_and_chunk_boundaries_do_not_change_audio() {
        let mut a = Processor::new(48_000);
        let mut b = Processor::new(48_000);
        let settings = DspSettings {
            gain_db: 12.0,
            compressor: true,
            drive: 0.2,
            robot_mix: 0.3,
            echo_mix: 0.5,
            echo_delay_ms: 83.0,
            reverb_mix: 0.4,
            ..Default::default()
        };
        let mut whole = sine(440.0, 0.1, 24_000);
        let mut chunks = whole.clone();
        a.process(&mut whole, &settings);
        for chunk in chunks.chunks_mut(173) {
            b.process(chunk, &settings);
        }
        assert_eq!(whole, chunks);
        let mut warmup = sine(1_000.0, 0.05, 48_000);
        a.reset();
        a.process(&mut warmup, &DspSettings::default());
        let previous = a.primary.controls.gain;
        let mut one = [0.0];
        a.process(
            &mut one,
            &DspSettings {
                gain_db: 24.0,
                ..Default::default()
            },
        );
        assert!(a.primary.controls.gain > previous && a.primary.controls.gain - previous < 0.04);
    }

    #[test]
    fn processing_and_parameter_changes_do_not_allocate_or_free() {
        let mut processor = Processor::new(48_000);
        let mut input = [0.1; 256];
        let settings = DspSettings {
            compressor: true,
            drive: 1.0,
            robot_mix: 0.8,
            echo_mix: 1.0,
            reverb_mix: 1.0,
            echo_delay_ms: 1_500.0,
            echo_feedback: 0.9,
            ..Default::default()
        };
        CALLS.with(|n| n.set(0));
        TRACK.with(|v| v.set(true));
        processor.process(&mut input, &settings);
        processor.process(&mut input, &DspSettings::default());
        TRACK.with(|v| v.set(false));
        assert_eq!(CALLS.with(Cell::get), 0);
    }

    #[test]
    fn extreme_settings_stay_stable_at_supported_sample_rates() {
        let default = DspSettings::default();
        for rate in [0, 8_000, 44_100, 48_000, 192_000, u32::MAX] {
            let mut processor = Processor::new(rate);
            let settings = DspSettings {
                gain_db: 24.0,
                gate_db: -100.0,
                highpass_hz: 10.0,
                lowpass_hz: 23_000.0,
                compressor: true,
                drive: 1.0,
                robot_mix: 1.0,
                robot_hz: 2_000.0,
                echo_mix: 1.0,
                echo_delay_ms: 10.0,
                echo_feedback: 0.9,
                reverb_mix: 1.0,
                ..Default::default()
            };
            let mut state = 123_u32;
            let mut block = [0.0; 256];
            for i in 0..400 {
                for x in &mut block {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    *x = state as f32 / u32::MAX as f32 * 2.0 - 1.0;
                }
                processor.process(&mut block, if i % 2 == 0 { &settings } else { &default });
                assert!(block.iter().all(|v| v.is_finite() && v.abs() <= CEILING));
            }
        }
    }

    #[test]
    fn spectral_pitch_transposes_the_actual_signal() {
        for (semitones, expected) in [(12.0, 440.0), (-12.0, 110.0)] {
            let output = settled(
                DspSettings {
                    pitch_semitones: semitones,
                    ..Default::default()
                },
                220.0,
                0.1,
            );
            let stable = &output[24_000..];
            // Spectral algorithms can produce tiny sidebands that create extra zero crossings;
            // measure the strongest sinusoidal component instead of counting crossings.
            let energy = |hz: f32| {
                let (s, c) = stable
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(s, c), (i, x)| {
                        let phase = TAU * hz * i as f32 / 48_000.0;
                        (s + x * phase.sin(), c + x * phase.cos())
                    });
                s * s + c * c
            };
            let frequency = (-15..=15)
                .map(|offset| expected + offset as f32)
                .max_by(|a, b| energy(*a).total_cmp(&energy(*b)))
                .unwrap();
            assert!(
                (frequency - expected).abs() < 4.0,
                "{semitones} st: got {frequency} Hz, expected {expected}"
            );
            assert!(rms(stable) > 0.01);
        }
    }

    #[test]
    fn formant_control_moves_the_harmonic_spectral_envelope() {
        let source: Vec<f32> = (0..48_000)
            .map(|i| {
                (1..=35)
                    .map(|harmonic| {
                        let frequency = harmonic as f32 * 100.0;
                        // A voiced excitation retains a fundamental and harmonic spacing; a lone
                        // narrow high-frequency cluster is not a useful vowel/formant test.
                        let weight = (0.5
                            + 8.0 * (-0.5 * ((frequency - 700.0) / 100.0).powi(2)).exp()
                            + 5.0 * (-0.5 * ((frequency - 1_200.0) / 120.0).powi(2)).exp())
                            / harmonic as f32;
                        0.06 * weight * (TAU * frequency * i as f32 / 48_000.0).sin()
                    })
                    .sum()
            })
            .collect();
        let centroid = |audio: &[f32]| -> f32 {
            let mut weighted = 0.0;
            let mut energy = 0.0;
            for harmonic in 1..=35 {
                let frequency = harmonic as f32 * 100.0;
                let (s, c) =
                    audio[24_000..]
                        .iter()
                        .enumerate()
                        .fold((0.0, 0.0), |(s, c), (i, x)| {
                            let phase = TAU * frequency * i as f32 / 48_000.0;
                            (s + x * phase.sin(), c + x * phase.cos())
                        });
                let amplitude = s * s + c * c;
                weighted += amplitude * frequency;
                energy += amplitude;
            }
            weighted / energy
        };
        let mut raised = source.clone();
        let mut lowered = source.clone();
        Processor::new(48_000).process(
            &mut raised,
            &DspSettings {
                formant_semitones: 6.0,
                ..Default::default()
            },
        );
        Processor::new(48_000).process(
            &mut lowered,
            &DspSettings {
                formant_semitones: -6.0,
                ..Default::default()
            },
        );
        let baseline = centroid(&source);
        let high = centroid(&raised);
        let low = centroid(&lowered);
        assert!(
            high > baseline * 1.15,
            "raised centroid {high}, baseline {baseline}"
        );
        assert!(
            low < baseline * 0.9,
            "lowered centroid {low}, baseline {baseline}"
        );
    }

    #[test]
    fn chorus_adds_delayed_audio_and_denoiser_reduces_stationary_noise() {
        let mut dry_processor = Processor::new(48_000);
        let mut chorus_processor = Processor::new(48_000);
        let chorus_settings = DspSettings {
            chorus_mix: 1.0,
            ..Default::default()
        };
        let mut warmup = vec![0.0; 48_000];
        chorus_processor.process(&mut warmup, &chorus_settings);
        dry_processor.process(&mut warmup, &DspSettings::default());
        let mut dry = vec![0.0; 4_096];
        dry[..480].copy_from_slice(&sine(1_000.0, 0.1, 480));
        let mut chorus = dry.clone();
        dry_processor.process(&mut dry, &DspSettings::default());
        chorus_processor.process(&mut chorus, &chorus_settings);
        assert!(rms(&chorus[800..1_400]) > rms(&dry[800..1_400]) * 4.0);

        let mut state = 4_129_923_u32;
        let source: Vec<f32> = (0..96_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.03
            })
            .collect();
        let mut suppressed = source.clone();
        Processor::new(48_000).process(
            &mut suppressed,
            &DspSettings {
                denoise: true,
                ..Default::default()
            },
        );
        let ratio = rms(&suppressed[48_000..]) / rms(&source[48_000..]);
        assert!(ratio < 0.5, "stationary noise RMS ratio {ratio}");
        assert!(
            suppressed
                .iter()
                .all(|x| x.is_finite() && x.abs() <= CEILING)
        );
    }

    #[test]
    fn extended_algorithms_move_to_a_new_callback_thread_without_allocations() {
        let mut processor = Processor::new(48_000);
        let settings = DspSettings {
            pitch_semitones: 7.0,
            formant_semitones: -3.0,
            chorus_mix: 0.5,
            denoise: true,
            ..Default::default()
        };
        assert_eq!(processor.latency_frames(&DspSettings::default()), 0);
        assert_eq!(
            processor.latency_frames(&DspSettings {
                denoise: true,
                ..Default::default()
            }),
            960
        );
        assert_eq!(processor.latency_frames(&settings), 5_184);
        let calls = std::thread::spawn(move || {
            let mut block = [0.01; 173];
            CALLS.with(|n| n.set(0));
            TRACK.with(|v| v.set(true));
            for i in 0..200 {
                block.fill(if i % 2 == 0 { 0.02 } else { -0.01 });
                let mut reordered = settings;
                reordered
                    .effect_order
                    .0
                    .rotate_left(i % voice_core::EFFECT_COUNT);
                processor.process(&mut block, &reordered);
            }
            // Cover disable/re-enable; reset must reuse the prepared storage.
            processor.process(&mut block, &DspSettings::default());
            processor.process(&mut block, &settings);
            TRACK.with(|v| v.set(false));
            CALLS.with(Cell::get)
        })
        .join()
        .unwrap();
        assert_eq!(calls, 0, "callback made {calls} allocator calls");
    }

    #[test]
    fn fractional_delay_wraps_a_remainder_rounded_to_the_buffer_length() {
        let mut chorus = [0.0; 1_682];
        chorus[0] = 0.25;
        assert_eq!(delay_sample(&chorus, 380, 380.000_03), 0.25);
        let mut echo = vec![0.0; 72_002];
        echo[0] = 0.5;
        assert_eq!(delay_sample(&echo, 0, 0.001), 0.5);
    }
}
