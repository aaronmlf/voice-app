//! Streaming filter-bank vocoder. All coefficients and storage are prepared on construction.
//!
//! Twenty matched analysis/synthesis bands transfer speech envelopes onto an internal
//! band-limited sawtooth. A separate highpass speech path preserves consonants. There is
//! no FFT, audio-block accumulation, heap allocation or whole-stream lookahead here;
//! filters/envelopes still have frequency-dependent response and settling time.
use crate::clean;
use std::f32::consts::TAU;

const BANDS: usize = 20;

#[derive(Clone, Copy)]
struct Band {
    b0: f32,
    a1: f32,
    a2: f32,
    analysis: [f32; 2],
    synthesis: [f32; 2],
    envelope: f32,
    carrier_gain: f32,
}

impl Band {
    fn new(center: f32, q: f32, rate: f32) -> Self {
        // Constant-peak-gain RBJ bandpass; the two zero coefficients are b1=0,b2=-b0.
        let omega = TAU * center / rate;
        let alpha = omega.sin() / (2.0 * q);
        let reciprocal = 1.0 / (1.0 + alpha);
        Self {
            b0: alpha * reciprocal,
            a1: -2.0 * omega.cos() * reciprocal,
            a2: (1.0 - alpha) * reciprocal,
            analysis: [0.0; 2],
            synthesis: [0.0; 2],
            envelope: 0.0,
            // Sawtooth band RMS falls approximately with sqrt(fundamental/center).
            // Fixed spectral compensation prevents upper formants disappearing.
            carrier_gain: 3.0 * (center / 110.0).sqrt(),
        }
    }

    fn filter(input: f32, state: &mut [f32; 2], b0: f32, a1: f32, a2: f32) -> f32 {
        let output = clean(b0 * input + state[0]);
        state[0] = clean(state[1] - a1 * output);
        state[1] = clean(-b0 * input - a2 * output);
        output
    }

    fn tick(&mut self, speech: f32, carrier: f32, attack: f32, release: f32) -> f32 {
        let level = Self::filter(speech, &mut self.analysis, self.b0, self.a1, self.a2).abs();
        let follower = if level > self.envelope {
            attack
        } else {
            release
        };
        self.envelope = clean(level + follower * (self.envelope - level));
        let synthesized = Self::filter(carrier, &mut self.synthesis, self.b0, self.a1, self.a2);
        synthesized * self.envelope * self.carrier_gain
    }

    fn reset(&mut self) {
        self.analysis = [0.0; 2];
        self.synthesis = [0.0; 2];
        self.envelope = 0.0;
    }
}

pub(crate) struct Vocoder {
    bands: [Band; BANDS],
    phase: f32,
    attack: f32,
    consonant_pole: f32,
    consonant_input: f32,
    consonant_output: f32,
    active: bool,
}

impl Vocoder {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate as f32;
        let upper = 8_000.0_f32.min(rate * 0.40);
        let ratio = (upper / 100.0).powf(1.0 / (BANDS - 1) as f32);
        let q = 1.0 / (ratio - ratio.recip());
        Self {
            bands: std::array::from_fn(|index| {
                Band::new(100.0 * ratio.powi(index as i32), q, rate)
            }),
            phase: 0.0,
            attack: (-1.0 / (0.002 * rate)).exp(),
            consonant_pole: (-TAU * 1_800.0_f32.min(rate * 0.20) / rate).exp(),
            consonant_input: 0.0,
            consonant_output: 0.0,
            active: false,
        }
    }

    pub fn reset(&mut self) {
        for band in &mut self.bands {
            band.reset();
        }
        self.phase = 0.0;
        self.consonant_input = 0.0;
        self.consonant_output = 0.0;
        self.active = false;
    }

    pub fn tick(
        &mut self,
        input: f32,
        mix: f32,
        carrier_step: f32,
        carrier_level: f32,
        unvoiced: f32,
        release: f32,
    ) -> f32 {
        // Exact bypass and no filter work for legacy presets. The control ramp reaches
        // zero before clearing history, so re-enabling cannot reveal an older phrase.
        if mix == 0.0 {
            if self.active {
                self.reset();
            }
            return input;
        }
        self.active = true;
        let phase = self.phase;
        let discontinuity = if phase < carrier_step {
            let t = phase / carrier_step;
            t + t - t * t - 1.0
        } else if phase > 1.0 - carrier_step {
            let t = (phase - 1.0) / carrier_step;
            t * t + t + t + 1.0
        } else {
            0.0
        };
        let carrier = (2.0 * phase - 1.0 - discontinuity) * carrier_level;
        self.phase += carrier_step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let highpass =
            clean(self.consonant_pole * (self.consonant_output + input - self.consonant_input));
        self.consonant_input = input;
        self.consonant_output = highpass;
        let mut synthesized = 0.0;
        for band in &mut self.bands {
            synthesized += band.tick(input, carrier, self.attack, release);
        }
        // Overlapping bands add coherently around voiced harmonics. Keep their
        // sum at a conversational level rather than spending the limiter's headroom;
        // the consonant path has its own level and must retain its articulation.
        let wet = clean(synthesized * 0.25 + unvoiced * highpass);
        clean(input + mix * (wet - input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_vocoder_state_has_no_heap_buffer_and_fits_in_four_kibibytes() {
        assert!(std::mem::size_of::<Vocoder>() <= 4_096);
        assert!(!std::mem::needs_drop::<Vocoder>());
    }
}
