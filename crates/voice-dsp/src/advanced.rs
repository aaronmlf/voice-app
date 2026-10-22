//! Prepared external algorithms. Neither backend is constructed or dropped during processing.
use nnnoiseless::DenoiseState;
use signalsmith_stretch::Stretch;

const PITCH_BLOCK: usize = 128;
const NOISE_BLOCK: usize = DenoiseState::FRAME_SIZE;

pub(crate) struct Pitch {
    engine: Stretch,
    input: [f32; PITCH_BLOCK],
    output: [f32; PITCH_BLOCK],
    position: usize,
    active: bool,
    pitch: f32,
    formant: f32,
    step: f32,
    latency: usize,
}

impl Pitch {
    pub fn new(sample_rate: u32) -> Self {
        let window = (sample_rate as usize * 4_096 / 48_000)
            .next_power_of_two()
            .clamp(512, 16_384);
        let mut engine = Stretch::new(1, window, window / 8);
        let latency = engine.input_latency() + engine.output_latency() + PITCH_BLOCK;
        // Exercise the spectral and formant paths before real-time use. Their vectors are sized
        // in configure(); process() only resizes within that capacity (pinned backend 0.1.3).
        let input = [0.01; PITCH_BLOCK];
        let mut output = [0.0; PITCH_BLOCK];
        engine.set_transpose_factor_semitones(1.0, None);
        engine.set_formant_factor_semitones(1.0, true);
        for _ in 0..window / PITCH_BLOCK + 4 {
            engine.process(input, &mut output);
        }
        engine.reset();
        Self {
            engine,
            input: [0.0; PITCH_BLOCK],
            output: [0.0; PITCH_BLOCK],
            position: 0,
            active: false,
            pitch: 0.0,
            formant: 0.0,
            step: 1.0 - (-(PITCH_BLOCK as f32) / (0.020 * sample_rate as f32)).exp(),
            latency,
        }
    }

    pub fn latency(&self) -> usize {
        self.latency
    }

    pub fn reset(&mut self) {
        self.engine.reset();
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.position = 0;
        self.active = false;
        self.pitch = 0.0;
        self.formant = 0.0;
    }

    pub fn tick(&mut self, input: f32, pitch: f32, formant: f32) -> f32 {
        let enabled = pitch.abs() > 1.0e-4 || formant.abs() > 1.0e-4;
        if !enabled {
            self.active = false;
            return input;
        }
        if !self.active {
            self.reset();
            self.active = true;
            self.pitch = pitch;
            self.formant = formant;
        }
        let result = self.output[self.position];
        self.input[self.position] = input;
        self.position += 1;
        if self.position == PITCH_BLOCK {
            self.pitch += (pitch - self.pitch) * self.step;
            self.formant += (formant - self.formant) * self.step;
            self.engine.set_transpose_factor_semitones(self.pitch, None);
            self.engine.set_formant_factor_semitones(self.formant, true);
            self.engine.process(self.input, &mut self.output);
            self.position = 0;
        }
        result
    }
}

pub(crate) struct Denoise {
    engine: Box<DenoiseState<'static>>,
    input: [f32; NOISE_BLOCK],
    output: [f32; NOISE_BLOCK],
    position: usize,
    active: bool,
    discard_first: bool,
    supported: bool,
}

impl Denoise {
    pub fn new(sample_rate: u32) -> Self {
        let mut engine = DenoiseState::new();
        let input = [0.0; NOISE_BLOCK];
        let mut output = [0.0; NOISE_BLOCK];
        // Force shared tables before entering the callback; pinned 0.5.1 owns its FFT scratch.
        engine.process_frame(&mut output, &input);
        Self {
            engine,
            input,
            output,
            position: 0,
            active: false,
            discard_first: true,
            supported: sample_rate == 48_000,
        }
    }

    pub fn latency(&self) -> usize {
        if self.supported { NOISE_BLOCK * 2 } else { 0 }
    }

    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.position = 0;
        self.active = false;
        self.discard_first = true;
    }

    pub fn tick(&mut self, sample: f32, enabled: bool) -> f32 {
        if !enabled || !self.supported {
            self.active = false;
            return sample;
        }
        if !self.active {
            self.reset();
            self.active = true;
        }
        let result = self.output[self.position] / 32_768.0;
        self.input[self.position] = sample.clamp(-1.0, 1.0) * 32_768.0;
        self.position += 1;
        if self.position == NOISE_BLOCK {
            self.engine.process_frame(&mut self.output, &self.input);
            // The model keeps analysis history. Discard the first block when re-enabled so
            // old overlap audio cannot reappear. The streaming adapter itself adds 480 frames.
            if self.discard_first {
                self.output.fill(0.0);
                self.discard_first = false;
            }
            self.position = 0;
        }
        result
    }
}
