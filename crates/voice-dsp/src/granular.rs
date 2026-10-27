//! Prepared, causal four-grain overlap-add. This is an effect, not neural conversion.
//!
//! Grain cursors advance at the pitch ratio; Hann windows overlap four at a time. The
//! centre of each grain follows a fixed delayed timeline, so time is not stretched.
//! Natural mode bounds pitch and grain size, aligns new grains by a short waveform
//! correlation, and retains delayed consonants/transients. Its perceptual quality is
//! experimental; waveform tests cannot establish natural speech quality.
use crate::{bounded, clean};
use std::f32::consts::TAU;
use voice_core::GranularMode;

const WINDOW_POINTS: usize = 1_025;
const GRAINS: usize = 4;
// Cover one 80 Hz source period across the alignment aperture. The extra causal
// guard keeps every actual source cursor behind capture even at 12 ms / +4 st.
const NATURAL_ALIGNMENT_US: usize = 6_500;
const NATURAL_GUARD_US: usize = 4_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Configuration {
    pub mode: GranularMode,
    pub frames: usize,
    pub delay: usize,
}

pub(crate) fn configuration(rate: u32, mode: GranularMode, grain_ms: f32) -> Configuration {
    let maximum = if mode == GranularMode::Natural {
        40.0
    } else {
        80.0
    };
    let frames = (bounded(grain_ms, 12.0, maximum, 32.0) * rate as f32 / 1_000.0).round() as usize;
    // Four equally spaced starts keep the overlap weights steady.
    let frames = frames.max(8).div_ceil(GRAINS) * GRAINS;
    let guard = if mode == GranularMode::Natural {
        (rate as usize * NATURAL_GUARD_US).div_ceil(1_000_000)
    } else {
        0
    };
    Configuration {
        mode,
        frames,
        delay: frames / 2 + guard + 2,
    }
}

pub(crate) fn maximum_delay(rate: u32) -> usize {
    configuration(rate, GranularMode::Stylized, 80.0).delay
}

#[derive(Clone, Copy)]
struct Grain {
    cursor: f64,
    ratio: f64,
    age: usize,
    length: usize,
}
impl Grain {
    const EMPTY: Self = Self {
        cursor: 0.0,
        ratio: 1.0,
        age: 0,
        length: 0,
    };
}

#[derive(Clone, Copy)]
pub(crate) struct Parameters {
    pub mix: f32,
    pub ratio: f32,
    pub scatter: f32,
    pub feedback: f32,
    pub clarity: f32,
}

pub(crate) struct Granular {
    history: Box<[f32]>,
    window: [f32; WINDOW_POINTS],
    grains: [Grain; GRAINS],
    clock: u64,
    filled: usize,
    until_launch: usize,
    next_grain: usize,
    random: u32,
    feedback_sample: f32,
    dry_low: f32,
    wet_low: f32,
    fast: f32,
    slow: f32,
    highpass_alpha: f32,
    fast_coefficient: f32,
    slow_coefficient: f32,
    rate: u32,
    active: bool,
}
impl Granular {
    pub fn new(rate: u32) -> Self {
        let maximum_frames = configuration(rate, GranularMode::Stylized, 80.0).frames;
        Self {
            // Includes the furthest backwards scatter, source-rate motion and guard.
            history: vec![0.0; maximum_frames * 3 + 8].into_boxed_slice(),
            window: std::array::from_fn(|i| {
                0.5 - 0.5 * (TAU * i as f32 / (WINDOW_POINTS - 1) as f32).cos()
            }),
            grains: [Grain::EMPTY; GRAINS],
            clock: 0,
            filled: 0,
            until_launch: 0,
            next_grain: 0,
            random: 0x762a_6e15,
            feedback_sample: 0.0,
            dry_low: 0.0,
            wet_low: 0.0,
            fast: 0.0,
            slow: 0.0,
            highpass_alpha: 1.0 - (-TAU * 1_600.0_f32.min(rate as f32 * 0.2) / rate as f32).exp(),
            fast_coefficient: (-1.0 / (0.001 * rate as f32)).exp(),
            slow_coefficient: (-1.0 / (0.030 * rate as f32)).exp(),
            rate,
            active: false,
        }
    }

    pub fn state_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + std::mem::size_of_val(&*self.history)
    }

    // O(1): the epoch/filled guards invalidate every old slot without clearing memory.
    pub fn reset(&mut self) {
        self.grains = [Grain::EMPTY; GRAINS];
        self.clock = 0;
        self.filled = 0;
        self.until_launch = 0;
        self.next_grain = 0;
        self.random = 0x762a_6e15;
        self.feedback_sample = 0.0;
        self.dry_low = 0.0;
        self.wet_low = 0.0;
        self.fast = 0.0;
        self.slow = 0.0;
        self.active = false;
    }

    fn integer_sample(&self, time: i64) -> f32 {
        if time < 0 {
            return 0.0;
        }
        let time = time as u64;
        let oldest = self.clock + 1 - self.filled as u64;
        if time < oldest || time > self.clock {
            0.0
        } else {
            self.history[time as usize % self.history.len()]
        }
    }

    fn sample(&self, time: f64) -> f32 {
        let floor = time.floor();
        let first = self.integer_sample(floor as i64);
        let second = self.integer_sample(floor as i64 + 1);
        first + (time - floor) as f32 * (second - first)
    }

    fn weight(&self, grain: &Grain) -> f32 {
        if grain.age >= grain.length || grain.length < 2 {
            return 0.0;
        }
        let phase = grain.age as f32 / (grain.length - 1) as f32 * (WINDOW_POINTS - 1) as f32;
        let index = (phase as usize).min(WINDOW_POINTS - 2);
        self.window[index] + (phase - index as f32) * (self.window[index + 1] - self.window[index])
    }

    fn alignment_score(&self, candidate: f64, ratio: f64, reference: Grain) -> f32 {
        let stride = (self.rate as usize / 8_000).max(1) as f64;
        let mut cross = 0.0;
        let mut a_energy = 0.0;
        let mut b_energy = 0.0;
        for j in 0..16 {
            let a = self.sample(candidate + j as f64 * stride * ratio);
            let b = self.sample(reference.cursor + j as f64 * stride * reference.ratio);
            cross += a * b;
            a_energy += a * a;
            b_energy += b * b;
        }
        if a_energy > 1.0e-10 && b_energy > 1.0e-10 {
            cross / (a_energy * b_energy).sqrt()
        } else {
            -1.0
        }
    }

    fn aligned_start(&self, start: f64, ratio: f64, mode: GranularMode) -> f64 {
        // Follow the newest overlapping grain, not two interleaved reference streams.
        // Independent even/odd references can accumulate different rounding errors and
        // cancel one another despite each pair having a good correlation locally.
        let reference = self
            .grains
            .iter()
            .copied()
            .filter(|grain| grain.age > 0 && grain.age < grain.length)
            .min_by_key(|grain| grain.age);
        let Some(reference) = reference else {
            return start;
        };
        // At most 313 candidates and 16 comparisons at 192 kHz, then a fixed
        // three-point refinement. No unbounded pitch-tracking search is performed.
        let radius = (self.rate as usize * NATURAL_ALIGNMENT_US / 1_000_000).max(1);
        // Stylized pitch has no extra forward-read guard. At zero scatter, align
        // only backwards over a full 80 Hz period; even +12 st stays causal.
        let (lower, upper) = if mode == GranularMode::Natural {
            (-(radius as i32), radius as i32)
        } else {
            (-(radius as i32) * 2, 0)
        };
        let step = (self.rate as usize / 24_000).max(1);
        let mut best = start;
        let mut best_score = -1.0_f32;
        for offset in (lower..=upper).step_by(step) {
            let candidate = start + offset as f64;
            let score = self.alignment_score(candidate, ratio, reference);
            // Only break effectively exact periodic ties in favour of the nominal
            // timeline; do not trade audible phase error for a shorter displacement.
            let ranked = score - (offset.unsigned_abs() as f32 / radius as f32) * 0.000_001;
            if ranked > best_score {
                best_score = ranked;
                best = candidate;
            }
        }
        // Fractional refinement prevents the integer search grid's phase error from
        // accumulating between successive grains (particularly for low fundamentals).
        let left = self.alignment_score(best - step as f64, ratio, reference);
        let centre = self.alignment_score(best, ratio, reference);
        let right = self.alignment_score(best + step as f64, ratio, reference);
        let curvature = left - 2.0 * centre + right;
        let correction = if curvature < -1.0e-7 {
            (0.5 * (left - right) / curvature).clamp(-1.0, 1.0) as f64 * step as f64
        } else {
            0.0
        };
        (best + correction).clamp(start + lower as f64, start + upper as f64)
    }

    pub fn tick(
        &mut self,
        input: f32,
        dry: f32,
        config: Configuration,
        parameters: Parameters,
    ) -> f32 {
        let Parameters {
            mix,
            ratio,
            scatter,
            feedback,
            clarity,
        } = parameters;
        if mix == 0.0 {
            if self.active {
                self.reset();
            }
            return input;
        }
        self.active = true;
        let natural = config.mode == GranularMode::Natural;
        let ratio = if natural {
            ratio.clamp(2.0_f32.powf(-4.0 / 12.0), 2.0_f32.powf(4.0 / 12.0))
        } else {
            ratio.clamp(0.5, 2.0)
        };
        let scatter = if natural { 0.0 } else { scatter };
        let feedback = if natural { 0.0 } else { feedback };
        let neutral = (ratio - 1.0).abs() < 1.0e-6 && scatter == 0.0 && feedback == 0.0;
        let write = self.clock as usize % self.history.len();
        self.history[write] = clean((input + self.feedback_sample * feedback).clamp(-16.0, 16.0));
        self.filled = (self.filled + 1).min(self.history.len());
        if self.until_launch == 0 {
            self.random ^= self.random << 13;
            self.random ^= self.random >> 17;
            self.random ^= self.random << 5;
            let jitter =
                self.random as f64 / u32::MAX as f64 * scatter as f64 * config.frames as f64;
            let mut start = self.clock as f64 - config.delay as f64
                + (1.0 - ratio as f64) * (config.frames - 1) as f64 * 0.5
                - jitter;
            if !neutral && (natural || scatter == 0.0) {
                start = self.aligned_start(start, ratio as f64, config.mode);
            }
            self.grains[self.next_grain] = Grain {
                cursor: start,
                ratio: ratio as f64,
                age: 0,
                length: config.frames,
            };
            self.next_grain = (self.next_grain + 1) % GRAINS;
            self.until_launch = config.frames / GRAINS;
        }
        self.until_launch -= 1;
        let mut sum = 0.0;
        let mut weights = 0.0;
        for index in 0..GRAINS {
            let grain = self.grains[index];
            let weight = self.weight(&grain);
            if weight > 0.0 {
                sum += self.sample(grain.cursor) * weight;
                weights += weight;
            }
            if grain.age < grain.length {
                self.grains[index].age += 1;
                self.grains[index].cursor += grain.ratio;
            }
        }
        let mut wet = if neutral {
            dry
        } else if weights > 1.0e-6 {
            sum / weights
        } else {
            0.0
        };
        self.feedback_sample = clean(wet);
        self.dry_low = clean(self.dry_low + self.highpass_alpha * (dry - self.dry_low));
        self.wet_low = clean(self.wet_low + self.highpass_alpha * (wet - self.wet_low));
        let level = dry.abs();
        self.fast = clean(level + self.fast_coefficient * (self.fast - level));
        self.slow = clean(level + self.slow_coefficient * (self.slow - level));
        if !neutral {
            // Complementary high-band replacement keeps sibilants at their original
            // pitch; short attacks can recover the aligned full-band transient too.
            let high_preserve = if natural { clarity } else { clarity * 0.5 };
            wet += high_preserve * ((dry - self.dry_low) - (wet - self.wet_low));
            let transient = ((self.fast - self.slow * 1.8) / (self.fast + 1.0e-6)).clamp(0.0, 1.0);
            wet += clarity * transient * (dry - wet);
        }
        self.clock += 1;
        clean(dry + mix * (wet - dry))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(mode: GranularMode, ratio: f32, clarity: f32, frequency: f32) -> Vec<f32> {
        let mut granular = Granular::new(48_000);
        let config = configuration(48_000, mode, 32.0);
        (0..48_000)
            .map(|frame| {
                let input = 0.2 * (TAU * frequency * frame as f32 / 48_000.0).sin();
                granular.tick(
                    input,
                    if frame >= config.delay {
                        0.2 * (TAU * frequency * (frame - config.delay) as f32 / 48_000.0).sin()
                    } else {
                        0.0
                    },
                    config,
                    Parameters {
                        mix: 1.0,
                        ratio,
                        scatter: 0.0,
                        feedback: 0.0,
                        clarity,
                    },
                )
            })
            .collect()
    }

    fn projection(audio: &[f32], frequency: f32) -> f32 {
        let mut sin = 0.0;
        let mut cos = 0.0;
        for (frame, value) in audio.iter().enumerate() {
            let phase = TAU * frequency * frame as f32 / 48_000.0;
            sin += value * phase.sin();
            cos += value * phase.cos();
        }
        (sin * sin + cos * cos).sqrt() / audio.len() as f32
    }

    #[test]
    fn neutral_grains_equal_the_aligned_dry_sample_for_sample_at_all_supported_rates() {
        for rate in [8_000, 44_100, 48_000, 192_000] {
            for mode in [GranularMode::Stylized, GranularMode::Natural] {
                for grain_ms in [12.0, 32.0, 80.0] {
                    let config = configuration(rate, mode, grain_ms);
                    let source: Vec<_> = (0..config.frames * 3)
                        .map(|i| ((i * 173 % 997) as f32 / 997.0 - 0.5) * 0.2)
                        .collect();
                    for mix in [0.2, 0.65, 1.0] {
                        let mut granular = Granular::new(rate);
                        for (frame, input) in source.iter().copied().enumerate() {
                            let expected = if frame >= config.delay {
                                source[frame - config.delay]
                            } else {
                                0.0
                            };
                            let output = granular.tick(
                                input,
                                expected,
                                config,
                                Parameters {
                                    mix,
                                    ratio: 1.0,
                                    scatter: 0.0,
                                    feedback: 0.0,
                                    clarity: 0.75,
                                },
                            );
                            assert_eq!(
                                output, expected,
                                "{rate} {mode:?} {grain_ms} {mix} at {frame}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn natural_pitch_actually_changes_the_fundamental_with_conservative_bounds() {
        for frequency in [80.0, 120.0, 220.0, 330.0] {
            for semitones in [-4.0, 3.0, 4.0] {
                let ratio = 2.0_f32.powf(semitones / 12.0);
                let output = render(GranularMode::Natural, ratio, 0.0, frequency);
                let expected = frequency * ratio;
                let best = ((expected - 15.0) as usize..=(expected + 15.0) as usize)
                    .map(|frequency| (frequency, projection(&output[12_000..], frequency as f32)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                let rms = (output[12_000..].iter().map(|x| x * x).sum::<f32>() / 36_000.0).sqrt();
                println!(
                    "natural {frequency}Hz {semitones}st expected={expected} peak={} amplitude={} exact_projection={} rms={rms}",
                    best.0,
                    best.1,
                    projection(&output[12_000..], expected)
                );
                assert!(
                    (best.0 as f32 - expected).abs() < expected * 0.025,
                    "{frequency}Hz {semitones} semitones: expected {expected}, peak {} ({})",
                    best.0,
                    best.1
                );
                assert!(
                    best.1 > 0.025,
                    "{frequency}Hz {semitones} semitones: shifted tone must remain audible, got {}",
                    best.1
                );
                assert!(
                    best.1 > projection(&output[12_000..], frequency) * 1.5,
                    "natural mode must not merely replay a delayed original"
                );
            }
        }
        let octave = render(GranularMode::Stylized, 2.0, 0.0, 250.0);
        assert!(projection(&octave[12_000..], 500.0) > 0.08);
        assert!(projection(&octave[12_000..], 500.0) > projection(&octave[12_000..], 250.0) * 4.0);
    }

    #[test]
    fn stylized_zero_scatter_transposes_non_grid_tones_without_cancellation() {
        for frequency in [80.0, 330.0] {
            for semitones in [-12.0, -5.0, 7.0, 12.0] {
                let ratio = 2.0_f32.powf(semitones / 12.0);
                let output = render(GranularMode::Stylized, ratio, 0.0, frequency);
                let expected = frequency * ratio;
                let amplitude = projection(&output[12_000..], expected);
                let rms = (output[12_000..].iter().map(|x| x * x).sum::<f32>() / 36_000.0).sqrt();
                assert!(
                    amplitude > 0.06,
                    "stylized {frequency}Hz {semitones}st expected={expected} projection={amplitude} rms={rms}"
                );
                assert!(
                    amplitude > projection(&output[12_000..], frequency) * 2.0,
                    "stylized pitch must not replay the unshifted original"
                );
            }
        }
    }

    #[test]
    fn natural_ignores_scatter_feedback_and_caps_excess_pitch() {
        let mut a = Granular::new(48_000);
        let mut b = Granular::new(48_000);
        let config = configuration(48_000, GranularMode::Natural, 80.0);
        assert_eq!(config.frames, 1_920);
        let bound = 2.0_f32.powf(4.0 / 12.0);
        for frame in 0..12_000 {
            let input = 0.2 * (TAU * 330.0 * frame as f32 / 48_000.0).sin();
            assert_eq!(
                a.tick(
                    input,
                    0.0,
                    config,
                    Parameters {
                        mix: 0.8,
                        ratio: 2.0,
                        scatter: 1.0,
                        feedback: 0.65,
                        clarity: 0.75
                    }
                ),
                b.tick(
                    input,
                    0.0,
                    config,
                    Parameters {
                        mix: 0.8,
                        ratio: bound,
                        scatter: 0.0,
                        feedback: 0.0,
                        clarity: 0.75
                    }
                )
            );
        }
    }

    #[test]
    fn natural_consonant_path_preserves_high_frequencies_and_grain_seams_are_continuous() {
        let ratio = 2.0_f32.powf(3.0 / 12.0);
        let shifted = render(GranularMode::Natural, ratio, 0.0, 5_173.0);
        let clear = render(GranularMode::Natural, ratio, 1.0, 5_173.0);
        let a = projection(&shifted[12_000..], 5_173.0);
        let b = projection(&clear[12_000..], 5_173.0);
        assert!(b > 0.055 && b > a * 3.0, "sibilant projection {a} -> {b}");
        let voiced = render(GranularMode::Natural, ratio, 0.75, 330.0);
        let maximum_step = voiced[12_000..]
            .windows(2)
            .map(|x| (x[1] - x[0]).abs())
            .fold(0.0, f32::max);
        assert!(maximum_step < 0.04, "grain boundary step {maximum_step}");
    }

    #[test]
    fn history_guards_reject_future_unwritten_and_previous_epoch_audio() {
        let mut granular = Granular::new(48_000);
        let config = configuration(48_000, GranularMode::Stylized, 80.0);
        for _ in 0..granular.history.len() + 100 {
            granular.tick(
                0.2,
                0.2,
                config,
                Parameters {
                    mix: 1.0,
                    ratio: 0.5,
                    scatter: 1.0,
                    feedback: 0.65,
                    clarity: 0.75,
                },
            );
        }
        assert_eq!(granular.integer_sample(granular.clock as i64 + 1), 0.0);
        assert_eq!(granular.integer_sample(0), 0.0);
        granular.reset();
        assert_eq!(granular.sample(0.0), 0.0);
        for _ in 0..config.frames * 4 {
            assert_eq!(
                granular.tick(
                    0.0,
                    0.0,
                    config,
                    Parameters {
                        mix: 1.0,
                        ratio: 2.0,
                        scatter: 1.0,
                        feedback: 0.65,
                        clarity: 1.0
                    }
                ),
                0.0
            );
        }
        for rate in [8_000, 44_100, 48_000, 192_000] {
            for mode in [GranularMode::Stylized, GranularMode::Natural] {
                for grain_ms in [12.0, 80.0] {
                    for ratio in [0.5, 2.0] {
                        let mut granular = Granular::new(rate);
                        let config = configuration(rate, mode, grain_ms);
                        for frame in 0..config.frames * 3 {
                            granular.tick(
                                (frame % 997) as f32 * 0.0001,
                                0.0,
                                config,
                                Parameters {
                                    mix: 1.0,
                                    ratio,
                                    scatter: 0.0,
                                    feedback: 0.0,
                                    clarity: 0.0,
                                },
                            );
                            for grain in &granular.grains {
                                if grain.age > 0 && grain.age <= grain.length {
                                    let last_read = grain.cursor - grain.ratio;
                                    assert!(
                                        last_read <= granular.clock as f64 - 1.0,
                                        "future cursor at {rate}, {mode:?}, {grain_ms}, {ratio}, {frame}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
