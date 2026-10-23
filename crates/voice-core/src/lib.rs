//! Shared, platform-independent control contracts and bounded real-time communication.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub const SAMPLE_RATE: u32 = 48_000;
pub const VIRTUAL_NODE_NAME: &str = "voice-app.virtual-microphone";
pub const FORMAT_VERSION: u32 = 1;

/// Recording routes have independent PCM queues and rolling history.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordingSource {
    #[default]
    ProcessedVoice,
    Desktop,
}
impl RecordingSource {
    pub const fn channels(self) -> u16 {
        match self {
            Self::ProcessedVoice => 1,
            Self::Desktop => 2,
        }
    }
}

/// Separate, bounded stereo capture transport. Never mixed into the virtual microphone.
pub struct DesktopShared {
    pub samples: SampleQueue,
    pub active: AtomicBool,
    pub frames: AtomicU64,
    pub overruns: AtomicU64,
    pub non_finite_samples: AtomicU64,
    pub peak: AtomicU32,
}
impl Default for DesktopShared {
    fn default() -> Self {
        Self {
            // Half a second of interleaved stereo at 48 kHz. Even capacities and
            // even producer/consumer slices preserve whole frames on overflow.
            samples: SampleQueue::new(SAMPLE_RATE as usize),
            active: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            non_finite_samples: AtomicU64::new(0),
            peak: AtomicU32::new(0.0_f32.to_bits()),
        }
    }
}

pub const EFFECT_COUNT: usize = 11;

/// Prepared processing stages. Pitch and formants share one spectral backend;
/// the high/low-pass filters share one filter stage. The safety ceiling is always last.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum EffectKind {
    Denoise,
    PitchFormant,
    Gate,
    Gain,
    Filter,
    Compressor,
    Distortion,
    Robot,
    Chorus,
    Echo,
    Reverb,
}
impl EffectKind {
    pub const ALL: [Self; EFFECT_COUNT] = [
        Self::Denoise,
        Self::PitchFormant,
        Self::Gate,
        Self::Gain,
        Self::Filter,
        Self::Compressor,
        Self::Distortion,
        Self::Robot,
        Self::Chorus,
        Self::Echo,
        Self::Reverb,
    ];
}

/// Every stage appears exactly once; its existing settings determine its strength.
/// Fixed storage keeps command/preset contracts Copy and callback updates bounded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct EffectOrder(pub [EffectKind; EFFECT_COUNT]);
impl Default for EffectOrder {
    fn default() -> Self {
        Self(EffectKind::ALL)
    }
}
impl EffectOrder {
    pub fn is_valid(&self) -> bool {
        let mut seen = 0_u16;
        for effect in self.0 {
            let bit = 1_u16 << effect as u8;
            if seen & bit != 0 {
                return false;
            }
            seen |= bit;
        }
        seen == (1_u16 << EFFECT_COUNT) - 1
    }
    fn packed(self) -> u64 {
        let order = if self.is_valid() {
            self
        } else {
            Self::default()
        };
        order.0.iter().enumerate().fold(0, |bits, (index, effect)| {
            bits | ((*effect as u64) << (index * 4))
        })
    }
    fn unpack(bits: u64) -> Self {
        let mut order = Self::default();
        for (index, effect) in order.0.iter_mut().enumerate() {
            let id = ((bits >> (index * 4)) & 0xF) as usize;
            let Some(value) = EffectKind::ALL.get(id) else {
                return Self::default();
            };
            *effect = *value;
        }
        if order.is_valid() {
            order
        } else {
            Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DspSettings {
    pub effect_order: EffectOrder,
    /// Wet proportion by EffectKind discriminant; missing legacy fields default to fully wet.
    pub effect_mix: [f32; EFFECT_COUNT],
    /// Optional second, non-recursive prepared voice chain.
    pub blend: Option<VoiceBlend>,
    pub gain_db: f32,
    pub gate_db: f32,
    pub highpass_hz: f32,
    pub lowpass_hz: f32,
    pub compressor: bool,
    pub drive: f32,
    pub robot_mix: f32,
    pub robot_hz: f32,
    pub echo_mix: f32,
    pub echo_delay_ms: f32,
    pub echo_feedback: f32,
    pub reverb_mix: f32,
    pub pitch_semitones: f32,
    pub formant_semitones: f32,
    pub chorus_mix: f32,
    pub denoise: bool,
}
impl Default for DspSettings {
    fn default() -> Self {
        Self {
            effect_order: EffectOrder::default(),
            effect_mix: [1.0; EFFECT_COUNT],
            blend: None,
            gain_db: 0.0,
            gate_db: -80.0,
            highpass_hz: 20.0,
            lowpass_hz: 20_000.0,
            compressor: false,
            drive: 0.0,
            robot_mix: 0.0,
            robot_hz: 75.0,
            echo_mix: 0.0,
            echo_delay_ms: 180.0,
            echo_feedback: 0.25,
            reverb_mix: 0.0,
            pitch_semitones: 0.0,
            formant_semitones: 0.0,
            chorus_mix: 0.0,
            denoise: false,
        }
    }
}
impl DspSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.effect_order.is_valid() {
            return Err("effect_order must contain every effect exactly once".into());
        }
        for mix in self.effect_mix {
            if !mix.is_finite() || !(0.0..=1.0).contains(&mix) {
                return Err("effect_mix values must be finite and between zero and one".into());
            }
        }
        if let Some(blend) = self.blend {
            for mix in [blend.primary_mix, blend.secondary_mix] {
                if !mix.is_finite() || !(0.0..=1.0).contains(&mix) {
                    return Err("voice mix levels must be finite and between zero and one".into());
                }
            }
            blend.secondary.to_settings().validate()?;
        }
        for (name, value, min, max) in [
            ("gain_db", self.gain_db, -60.0, 24.0),
            ("gate_db", self.gate_db, -100.0, 0.0),
            ("highpass_hz", self.highpass_hz, 10.0, 10_000.0),
            ("lowpass_hz", self.lowpass_hz, 100.0, 23_000.0),
            ("drive", self.drive, 0.0, 1.0),
            ("robot_mix", self.robot_mix, 0.0, 1.0),
            ("robot_hz", self.robot_hz, 1.0, 2000.0),
            ("echo_mix", self.echo_mix, 0.0, 1.0),
            ("echo_delay_ms", self.echo_delay_ms, 10.0, 1500.0),
            ("echo_feedback", self.echo_feedback, 0.0, 0.9),
            ("reverb_mix", self.reverb_mix, 0.0, 1.0),
            ("pitch_semitones", self.pitch_semitones, -24.0, 24.0),
            ("formant_semitones", self.formant_semitones, -12.0, 12.0),
            ("chorus_mix", self.chorus_mix, 0.0, 1.0),
        ] {
            if !value.is_finite() || value < min || value > max {
                return Err(format!("{name} must be finite and between {min} and {max}"));
            }
        }
        if self.highpass_hz >= self.lowpass_hz {
            return Err("highpass_hz must be below lowpass_hz".into());
        }
        Ok(())
    }
}

/// A snapshot of one voice, without another nested mix. Fixed fields keep callback state Copy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceLayer {
    pub effect_order: EffectOrder,
    pub effect_mix: [f32; EFFECT_COUNT],
    pub gain_db: f32,
    pub gate_db: f32,
    pub highpass_hz: f32,
    pub lowpass_hz: f32,
    pub compressor: bool,
    pub drive: f32,
    pub robot_mix: f32,
    pub robot_hz: f32,
    pub echo_mix: f32,
    pub echo_delay_ms: f32,
    pub echo_feedback: f32,
    pub reverb_mix: f32,
    pub pitch_semitones: f32,
    pub formant_semitones: f32,
    pub chorus_mix: f32,
    pub denoise: bool,
}
impl Default for VoiceLayer {
    fn default() -> Self {
        Self::from(DspSettings::default())
    }
}
impl From<DspSettings> for VoiceLayer {
    fn from(settings: DspSettings) -> Self {
        Self {
            effect_order: settings.effect_order,
            effect_mix: settings.effect_mix,
            gain_db: settings.gain_db,
            gate_db: settings.gate_db,
            highpass_hz: settings.highpass_hz,
            lowpass_hz: settings.lowpass_hz,
            compressor: settings.compressor,
            drive: settings.drive,
            robot_mix: settings.robot_mix,
            robot_hz: settings.robot_hz,
            echo_mix: settings.echo_mix,
            echo_delay_ms: settings.echo_delay_ms,
            echo_feedback: settings.echo_feedback,
            reverb_mix: settings.reverb_mix,
            pitch_semitones: settings.pitch_semitones,
            formant_semitones: settings.formant_semitones,
            chorus_mix: settings.chorus_mix,
            denoise: settings.denoise,
        }
    }
}
impl VoiceLayer {
    pub fn to_settings(self) -> DspSettings {
        DspSettings {
            effect_order: self.effect_order,
            effect_mix: self.effect_mix,
            gain_db: self.gain_db,
            gate_db: self.gate_db,
            highpass_hz: self.highpass_hz,
            lowpass_hz: self.lowpass_hz,
            compressor: self.compressor,
            drive: self.drive,
            robot_mix: self.robot_mix,
            robot_hz: self.robot_hz,
            echo_mix: self.echo_mix,
            echo_delay_ms: self.echo_delay_ms,
            echo_feedback: self.echo_feedback,
            reverb_mix: self.reverb_mix,
            pitch_semitones: self.pitch_semitones,
            formant_semitones: self.formant_semitones,
            chorus_mix: self.chorus_mix,
            denoise: self.denoise,
            blend: None,
        }
    }
}

/// Two independent levels. Their sum is normalized when it exceeds one, preserving headroom.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct VoiceBlend {
    pub secondary: VoiceLayer,
    pub primary_mix: f32,
    pub secondary_mix: f32,
}
impl Default for VoiceBlend {
    fn default() -> Self {
        Self {
            secondary: VoiceLayer::default(),
            primary_mix: 0.5,
            secondary_mix: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Input,
    Output,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub kind: DeviceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Explicit owned Windows virtual render endpoint; physical output stays the headphones.
    pub virtual_output_device: Option<String>,
    pub sample_rate: u32,
    pub quantum: u32,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: None,
            output_device: None,
            virtual_output_device: None,
            sample_rate: SAMPLE_RATE,
            quantum: 256,
        }
    }
}
impl AudioConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.sample_rate != SAMPLE_RATE {
            return Err("the initial engine requires 48000 Hz".into());
        }
        if self
            .virtual_output_device
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.len() > 2048 || id.contains('\0'))
        {
            return Err(
                "virtual output must be a nonempty endpoint ID of at most 2048 bytes".into(),
            );
        }
        if !(64..=2048).contains(&self.quantum) || !self.quantum.is_power_of_two() {
            return Err("quantum must be a power of two from 64 to 2048".into());
        }
        if self
            .input_device
            .as_deref()
            .is_some_and(|s| s.starts_with("voice-app."))
        {
            return Err("cannot capture this application's own nodes".into());
        }
        Ok(())
    }
}

/// A detected endpoint pair is distinct from an actively transmitting audio route.
/// Detection never installs a driver or changes system audio defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VirtualMicrophoneStatus {
    pub available: bool,
    pub active: bool,
    pub render_device: Option<String>,
    pub capture_device: Option<String>,
    pub diagnostic: Option<String>,
}

/// Atomic settings are independent scalar controls, not a transactionally consistent preset snapshot.
/// The control thread is the only writer; callbacks sample the latest values without waiting.
pub struct AtomicDspSettings {
    effect_order: AtomicU64,
    values: [AtomicU32; 14],
    compressor: AtomicBool,
    denoise: AtomicBool,
    effect_mix: [AtomicU32; EFFECT_COUNT],
    blend_enabled: AtomicBool,
    blend_primary_mix: AtomicU32,
    blend_secondary_mix: AtomicU32,
    secondary: AtomicVoiceLayer,
}
impl Default for AtomicDspSettings {
    fn default() -> Self {
        Self::new(DspSettings::default())
    }
}
impl AtomicDspSettings {
    pub fn new(v: DspSettings) -> Self {
        Self {
            effect_order: AtomicU64::new(v.effect_order.packed()),
            values: Self::floats(v).map(|v| AtomicU32::new(v.to_bits())),
            compressor: AtomicBool::new(v.compressor),
            denoise: AtomicBool::new(v.denoise),
            effect_mix: v.effect_mix.map(|mix| AtomicU32::new(mix.to_bits())),
            blend_enabled: AtomicBool::new(v.blend.is_some()),
            blend_primary_mix: AtomicU32::new(v.blend.map_or(1.0, |b| b.primary_mix).to_bits()),
            blend_secondary_mix: AtomicU32::new(v.blend.map_or(0.0, |b| b.secondary_mix).to_bits()),
            secondary: AtomicVoiceLayer::new(
                v.blend.map_or_else(VoiceLayer::default, |b| b.secondary),
            ),
        }
    }
    fn floats(v: DspSettings) -> [f32; 14] {
        [
            v.gain_db,
            v.gate_db,
            v.highpass_hz,
            v.lowpass_hz,
            v.drive,
            v.robot_mix,
            v.robot_hz,
            v.echo_mix,
            v.echo_delay_ms,
            v.echo_feedback,
            v.reverb_mix,
            v.pitch_semitones,
            v.formant_semitones,
            v.chorus_mix,
        ]
    }
    pub fn store(&self, v: DspSettings) {
        if let Some(blend) = v.blend {
            self.secondary.store(blend.secondary);
            self.blend_primary_mix
                .store(blend.primary_mix.to_bits(), Ordering::Relaxed);
            self.blend_secondary_mix
                .store(blend.secondary_mix.to_bits(), Ordering::Relaxed);
        } else {
            self.blend_enabled.store(false, Ordering::Release);
        }
        for (a, v) in self.values.iter().zip(Self::floats(v)) {
            a.store(v.to_bits(), Ordering::Relaxed);
        }
        self.compressor.store(v.compressor, Ordering::Relaxed);
        self.denoise.store(v.denoise, Ordering::Relaxed);
        for (atomic, mix) in self.effect_mix.iter().zip(v.effect_mix) {
            atomic.store(mix.to_bits(), Ordering::Relaxed);
        }
        // One atomic value prevents observing a partly reordered permutation.
        self.effect_order
            .store(v.effect_order.packed(), Ordering::Relaxed);
        if v.blend.is_some() {
            self.blend_enabled.store(true, Ordering::Release);
        }
    }
    pub fn load(&self) -> DspSettings {
        let f = self
            .values
            .each_ref()
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)));
        DspSettings {
            effect_order: EffectOrder::unpack(self.effect_order.load(Ordering::Relaxed)),
            gain_db: f[0],
            gate_db: f[1],
            highpass_hz: f[2],
            lowpass_hz: f[3],
            drive: f[4],
            robot_mix: f[5],
            robot_hz: f[6],
            echo_mix: f[7],
            echo_delay_ms: f[8],
            echo_feedback: f[9],
            reverb_mix: f[10],
            compressor: self.compressor.load(Ordering::Relaxed),
            pitch_semitones: f[11],
            formant_semitones: f[12],
            chorus_mix: f[13],
            denoise: self.denoise.load(Ordering::Relaxed),
            effect_mix: self
                .effect_mix
                .each_ref()
                .map(|a| f32::from_bits(a.load(Ordering::Relaxed))),
            blend: self
                .blend_enabled
                .load(Ordering::Acquire)
                .then(|| VoiceBlend {
                    secondary: self.secondary.load(),
                    primary_mix: f32::from_bits(self.blend_primary_mix.load(Ordering::Relaxed)),
                    secondary_mix: f32::from_bits(self.blend_secondary_mix.load(Ordering::Relaxed)),
                }),
        }
    }
}

struct AtomicVoiceLayer {
    effect_order: AtomicU64,
    values: [AtomicU32; 14],
    compressor: AtomicBool,
    denoise: AtomicBool,
    effect_mix: [AtomicU32; EFFECT_COUNT],
}
impl AtomicVoiceLayer {
    fn new(layer: VoiceLayer) -> Self {
        Self {
            effect_order: AtomicU64::new(layer.effect_order.packed()),
            values: AtomicDspSettings::floats(layer.to_settings())
                .map(|v| AtomicU32::new(v.to_bits())),
            compressor: AtomicBool::new(layer.compressor),
            denoise: AtomicBool::new(layer.denoise),
            effect_mix: layer.effect_mix.map(|mix| AtomicU32::new(mix.to_bits())),
        }
    }
    fn store(&self, layer: VoiceLayer) {
        for (atomic, value) in self
            .values
            .iter()
            .zip(AtomicDspSettings::floats(layer.to_settings()))
        {
            atomic.store(value.to_bits(), Ordering::Relaxed);
        }
        self.compressor.store(layer.compressor, Ordering::Relaxed);
        self.denoise.store(layer.denoise, Ordering::Relaxed);
        for (atomic, mix) in self.effect_mix.iter().zip(layer.effect_mix) {
            atomic.store(mix.to_bits(), Ordering::Relaxed);
        }
        self.effect_order
            .store(layer.effect_order.packed(), Ordering::Relaxed);
    }
    fn load(&self) -> VoiceLayer {
        let f = self
            .values
            .each_ref()
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)));
        VoiceLayer {
            effect_order: EffectOrder::unpack(self.effect_order.load(Ordering::Relaxed)),
            effect_mix: self
                .effect_mix
                .each_ref()
                .map(|a| f32::from_bits(a.load(Ordering::Relaxed))),
            gain_db: f[0],
            gate_db: f[1],
            highpass_hz: f[2],
            lowpass_hz: f[3],
            drive: f[4],
            robot_mix: f[5],
            robot_hz: f[6],
            echo_mix: f[7],
            echo_delay_ms: f[8],
            echo_feedback: f[9],
            reverb_mix: f[10],
            pitch_semitones: f[11],
            formant_semitones: f[12],
            chorus_mix: f[13],
            compressor: self.compressor.load(Ordering::Relaxed),
            denoise: self.denoise.load(Ordering::Relaxed),
        }
    }
}

/// Bounded single-producer/single-consumer FIFO. Each sample is atomic: no locks or unsafe code.
/// A queue must have exactly one logical producer and one logical consumer. On overflow, new
/// samples are rejected. The consumer must handle a short read by writing silence.
pub struct SampleQueue {
    samples: Box<[AtomicU32]>,
    capacity: usize,
    read: AtomicUsize,
    write: AtomicUsize,
}
impl SampleQueue {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0 && capacity < usize::MAX / 2);
        // Physical power-of-two storage keeps indices continuous when usize counters wrap.
        Self {
            samples: (0..capacity.next_power_of_two())
                .map(|_| AtomicU32::new(0))
                .collect(),
            capacity,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
        }
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn len(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .wrapping_sub(self.read.load(Ordering::Acquire))
            .min(self.capacity())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn push(&self, input: &[f32]) -> usize {
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        let n = input
            .len()
            .min(self.capacity().saturating_sub(write.wrapping_sub(read)));
        for (i, sample) in input.iter().take(n).enumerate() {
            self.samples[write.wrapping_add(i) & (self.samples.len() - 1)]
                .store(sample.to_bits(), Ordering::Relaxed);
        }
        self.write.store(write.wrapping_add(n), Ordering::Release);
        n
    }
    pub fn pop(&self, output: &mut [f32]) -> usize {
        let read = self.read.load(Ordering::Relaxed);
        let write = self.write.load(Ordering::Acquire);
        let n = output.len().min(write.wrapping_sub(read));
        for (i, sample) in output.iter_mut().take(n).enumerate() {
            *sample = f32::from_bits(
                self.samples[read.wrapping_add(i) & (self.samples.len() - 1)]
                    .load(Ordering::Relaxed),
            );
        }
        self.read.store(read.wrapping_add(n), Ordering::Release);
        n
    }
    /// Consumer-only discard; never call this from the producer.
    pub fn discard(&self) {
        let write = self.write.load(Ordering::Acquire);
        self.read.store(write, Ordering::Release);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub frames: u64,
    pub input_underruns: u64,
    pub input_overruns: u64,
    pub monitor_overruns: u64,
    /// PipeWire diagnostic: incomplete monitor queue writes, counted as events.
    #[serde(default)]
    pub monitor_producer_overruns: u64,
    /// PipeWire diagnostic: frames discarded to recover a delayed monitor consumer.
    #[serde(default)]
    pub monitor_trimmed_frames: u64,
    /// PipeWire diagnostic: renders with no output frames; these are not overruns.
    #[serde(default)]
    pub monitor_empty_callbacks: u64,
    pub recording_overruns: u64,
    pub non_finite_samples: u64,
    pub output_peak: f32,
    pub active: bool,
    pub algorithmic_latency_frames: u32,
    #[serde(default)]
    pub virtual_microphone_active: bool,
    #[serde(default)]
    pub virtual_microphone_frames: u64,
    #[serde(default)]
    pub virtual_microphone_overruns: u64,
}
#[derive(Default)]
pub struct AudioMetrics {
    pub frames: AtomicU64,
    pub input_underruns: AtomicU64,
    pub input_overruns: AtomicU64,
    pub monitor_overruns: AtomicU64,
    pub monitor_producer_overruns: AtomicU64,
    pub monitor_trimmed_frames: AtomicU64,
    pub monitor_empty_callbacks: AtomicU64,
    pub recording_overruns: AtomicU64,
    pub non_finite_samples: AtomicU64,
    pub output_peak: AtomicU32,
    pub active: AtomicBool,
    pub algorithmic_latency_frames: AtomicU32,
    pub virtual_microphone_active: AtomicBool,
    pub virtual_microphone_frames: AtomicU64,
    pub virtual_microphone_overruns: AtomicU64,
}
impl AudioMetrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            frames: self.frames.load(Ordering::Relaxed),
            input_underruns: self.input_underruns.load(Ordering::Relaxed),
            input_overruns: self.input_overruns.load(Ordering::Relaxed),
            monitor_overruns: self.monitor_overruns.load(Ordering::Relaxed),
            monitor_producer_overruns: self.monitor_producer_overruns.load(Ordering::Relaxed),
            monitor_trimmed_frames: self.monitor_trimmed_frames.load(Ordering::Relaxed),
            monitor_empty_callbacks: self.monitor_empty_callbacks.load(Ordering::Relaxed),
            recording_overruns: self.recording_overruns.load(Ordering::Relaxed),
            non_finite_samples: self.non_finite_samples.load(Ordering::Relaxed),
            output_peak: f32::from_bits(self.output_peak.load(Ordering::Relaxed)),
            active: self.active.load(Ordering::Relaxed),
            algorithmic_latency_frames: self.algorithmic_latency_frames.load(Ordering::Relaxed),
            virtual_microphone_active: self.virtual_microphone_active.load(Ordering::Acquire),
            virtual_microphone_frames: self.virtual_microphone_frames.load(Ordering::Relaxed),
            virtual_microphone_overruns: self.virtual_microphone_overruns.load(Ordering::Relaxed),
        }
    }
}

/// Queue ownership: audio callback consumes soundboard/preview and produces recorded output;
/// the engine's single worker is the opposite endpoint for all three queues.
pub struct AudioShared {
    pub dsp: AtomicDspSettings,
    pub bypass: AtomicBool,
    pub muted: AtomicBool,
    pub monitor_enabled: AtomicBool,
    pub monitor_gain: AtomicU32,
    pub soundboard_gain: AtomicU32,
    pub soundboard: SampleQueue,
    pub preview: SampleQueue,
    pub recording: SampleQueue,
    pub metrics: AudioMetrics,
}
impl Default for AudioShared {
    fn default() -> Self {
        Self {
            dsp: AtomicDspSettings::default(),
            bypass: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(false),
            monitor_gain: AtomicU32::new(0.5_f32.to_bits()),
            soundboard_gain: AtomicU32::new(1.0_f32.to_bits()),
            soundboard: SampleQueue::new(SAMPLE_RATE as usize / 2),
            preview: SampleQueue::new(SAMPLE_RATE as usize / 2),
            recording: SampleQueue::new(SAMPLE_RATE as usize * 2),
            metrics: AudioMetrics::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_metrics_preserve_health_counters_without_monitor_diagnostics() {
        let legacy = serde_json::json!({
            "frames": 256,
            "input_underruns": 1,
            "input_overruns": 2,
            "monitor_overruns": 3,
            "recording_overruns": 4,
            "non_finite_samples": 5,
            "output_peak": 0.25,
            "active": true,
            "algorithmic_latency_frames": 0,
        });
        let snapshot: MetricsSnapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(snapshot.monitor_overruns, 3);
        assert_eq!(snapshot.monitor_producer_overruns, 0);
        assert_eq!(snapshot.monitor_trimmed_frames, 0);
        assert_eq!(snapshot.monitor_empty_callbacks, 0);
        assert_eq!(snapshot.frames, 256);
        assert!(snapshot.active);
        assert!(!snapshot.virtual_microphone_active);
        assert_eq!(snapshot.virtual_microphone_frames, 0);
        assert_eq!(snapshot.virtual_microphone_overruns, 0);
    }
    #[test]
    fn legacy_audio_config_keeps_virtual_transmission_opt_in() {
        let config: AudioConfig = serde_json::from_str(
            r#"{"input_device":"mic","output_device":"headphones","sample_rate":48000,"quantum":256}"#,
        )
        .unwrap();
        assert_eq!(config.virtual_output_device, None);
        assert!(config.validate().is_ok());
        let enabled = AudioConfig {
            virtual_output_device: Some("owned-endpoint-id".into()),
            ..config
        };
        let saved = serde_json::to_string(&enabled).unwrap();
        assert_eq!(
            serde_json::from_str::<AudioConfig>(&saved).unwrap(),
            enabled
        );
    }
    #[test]
    fn malformed_virtual_endpoint_ids_are_rejected_before_device_access() {
        for id in [
            String::new(),
            " ".into(),
            "id\0suffix".into(),
            "x".repeat(2049),
        ] {
            assert!(
                AudioConfig {
                    virtual_output_device: Some(id),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn legacy_settings_get_default_order_and_invalid_orders_are_rejected() {
        let legacy: DspSettings = serde_json::from_str(r#"{"gain_db":-3.0}"#).unwrap();
        assert_eq!(legacy.effect_order, EffectOrder::default());
        assert!(legacy.validate().is_ok());
        let mut duplicate = legacy;
        duplicate.effect_order.0[1] = duplicate.effect_order.0[0];
        assert!(duplicate.validate().is_err());
        assert!(!duplicate.effect_order.is_valid());
        assert!(serde_json::from_str::<DspSettings>(r#"{"effect_order":["gain"]}"#).is_err());
        assert!(serde_json::from_str::<EffectKind>(r#""unknown_effect""#).is_err());
    }
    #[test]
    fn effect_order_survives_json_and_atomic_roundtrips() {
        let mut settings = DspSettings::default();
        settings.effect_order.0.reverse();
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: DspSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, settings);
        let controls = AtomicDspSettings::new(settings);
        assert_eq!(controls.load(), settings);
        let mut invalid = settings;
        invalid.effect_order.0[0] = invalid.effect_order.0[1];
        controls.store(invalid);
        assert_eq!(controls.load().effect_order, EffectOrder::default());
        controls.effect_order.store(u64::MAX, Ordering::Relaxed);
        assert_eq!(controls.load().effect_order, EffectOrder::default());
    }
    #[test]
    fn concurrent_reordering_never_exposes_a_partial_permutation() {
        let original = DspSettings::default();
        let mut reversed = original;
        reversed.effect_order.0.reverse();
        let controls = std::sync::Arc::new(AtomicDspSettings::new(original));
        let writer = controls.clone();
        let producer = std::thread::spawn(move || {
            for _ in 0..10_000 {
                writer.store(reversed);
                writer.store(original);
            }
        });
        for _ in 0..10_000 {
            let order = controls.load().effect_order;
            assert!(order == original.effect_order || order == reversed.effect_order);
        }
        producer.join().unwrap();
    }
    #[test]
    fn opacity_and_two_voice_settings_preserve_legacy_json_and_roundtrip() {
        let legacy: DspSettings = serde_json::from_str(r#"{"pitch_semitones":-4.0}"#).unwrap();
        assert_eq!(legacy.effect_mix, [1.0; EFFECT_COUNT]);
        assert_eq!(legacy.blend, None);
        let mut settings = legacy;
        settings.effect_mix[EffectKind::Robot as usize] = 0.25;
        let mut secondary = VoiceLayer::from(DspSettings {
            robot_mix: 0.8,
            ..Default::default()
        });
        secondary.effect_mix[EffectKind::Filter as usize] = 0.5;
        settings.blend = Some(VoiceBlend {
            secondary,
            primary_mix: 0.7,
            secondary_mix: 0.4,
        });
        settings.validate().unwrap();
        let decoded: DspSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(decoded, settings);
        let atomic = AtomicDspSettings::new(settings);
        assert_eq!(atomic.load(), settings);
        atomic.store(DspSettings::default());
        assert_eq!(atomic.load(), DspSettings::default());
        atomic.store(settings);
        assert_eq!(atomic.load(), settings);
        // A layer is a deliberate snapshot rather than a recursive graph.
        assert!(VoiceLayer::from(settings).to_settings().blend.is_none());
    }
    #[test]
    fn invalid_opacities_and_recursive_secondary_are_rejected() {
        let mut settings = DspSettings::default();
        for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            settings.effect_mix[0] = value;
            assert!(settings.validate().is_err());
        }
        settings = DspSettings::default();
        settings.blend = Some(VoiceBlend {
            primary_mix: f32::NAN,
            ..Default::default()
        });
        assert!(settings.validate().is_err());
        settings.blend = Some(VoiceBlend {
            secondary_mix: 1.1,
            ..Default::default()
        });
        assert!(settings.validate().is_err());
        assert!(
            serde_json::from_str::<DspSettings>(r#"{"blend":{"secondary":{"blend":{}}}}"#).is_err()
        );
    }

    #[test]
    fn fifo_bounded_wrap_and_order() {
        let q = SampleQueue::new(3);
        assert_eq!(q.push(&[1., 2., 3., 4.]), 3);
        let mut out = [0.; 2];
        assert_eq!(q.pop(&mut out), 2);
        assert_eq!(out, [1., 2.]);
        assert_eq!(q.push(&[4., 5.]), 2);
        let mut out = [0.; 4];
        assert_eq!(q.pop(&mut out), 3);
        assert_eq!(&out[..3], &[3., 4., 5.]);
    }
    #[test]
    fn fifo_concurrent_order() {
        let q = std::sync::Arc::new(SampleQueue::new(97));
        let producer = q.clone();
        let t = std::thread::spawn(move || {
            for i in 0..20000 {
                while producer.push(&[i as f32]) == 0 {
                    std::thread::yield_now();
                }
            }
        });
        for i in 0..20000 {
            let mut v = [0.];
            while q.pop(&mut v) == 0 {
                std::thread::yield_now();
            }
            assert_eq!(v[0], i as f32);
        }
        t.join().unwrap();
    }
    #[test]
    fn fifo_counter_wrap_preserves_non_power_of_two_capacity() {
        let q = SampleQueue::new(3);
        q.read.store(usize::MAX - 1, Ordering::Relaxed);
        q.write.store(usize::MAX - 1, Ordering::Relaxed);
        assert_eq!(q.push(&[1., 2., 3.]), 3);
        let mut out = [0.; 3];
        assert_eq!(q.pop(&mut out), 3);
        assert_eq!(out, [1., 2., 3.]);
        assert_eq!(q.push(&[4., 5., 6., 7.]), 3);
        assert_eq!(q.pop(&mut out), 3);
        assert_eq!(out, [4., 5., 6.]);
        assert!(q.is_empty());
    }
    #[test]
    fn atomic_settings_roundtrip() {
        let settings = DspSettings {
            pitch_semitones: 12.,
            formant_semitones: -3.,
            chorus_mix: 0.3,
            denoise: true,
            ..Default::default()
        };
        let controls = AtomicDspSettings::new(settings);
        assert_eq!(controls.load(), settings);
        controls.store(DspSettings::default());
        assert_eq!(controls.load(), DspSettings::default());
    }
    #[test]
    fn reject_nonfinite_and_feedback_routes() {
        let p = DspSettings {
            gain_db: f32::NAN,
            ..Default::default()
        };
        assert!(p.validate().is_err());
        let c = AudioConfig {
            input_device: Some(VIRTUAL_NODE_NAME.into()),
            ..Default::default()
        };
        assert!(c.validate().is_err());
        assert!(DspSettings::default().validate().is_ok());
    }
}
