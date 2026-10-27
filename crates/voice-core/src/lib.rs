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
    pub queue_overruns: AtomicU64,
    pub backend_xruns: AtomicU64,
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
            queue_overruns: AtomicU64::new(0),
            backend_xruns: AtomicU64::new(0),
            non_finite_samples: AtomicU64::new(0),
            peak: AtomicU32::new(0.0_f32.to_bits()),
        }
    }
}

pub const EFFECT_COUNT: usize = 13;

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
    Vocoder,
    Granular,
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
        Self::Vocoder,
        Self::Granular,
    ];
}

/// Every stage appears exactly once; its existing settings determine its strength.
/// Fixed storage keeps command/preset contracts Copy and callback updates bounded.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct EffectOrder(pub [EffectKind; EFFECT_COUNT]);
impl Default for EffectOrder {
    fn default() -> Self {
        Self([
            EffectKind::Denoise,
            EffectKind::PitchFormant,
            EffectKind::Gate,
            EffectKind::Gain,
            EffectKind::Filter,
            EffectKind::Compressor,
            EffectKind::Distortion,
            EffectKind::Robot,
            EffectKind::Vocoder,
            EffectKind::Granular,
            EffectKind::Chorus,
            EffectKind::Echo,
            EffectKind::Reverb,
        ])
    }
}
impl<'de> Deserialize<'de> for EffectOrder {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OrderVisitor;
        impl<'de> serde::de::Visitor<'de> for OrderVisitor {
            type Value = EffectOrder;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter
                    .write_str("a permutation of all 13 effects or the 11 or 12 legacy effects")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut order = EffectOrder::default();
                let mut count = 0;
                while count < EFFECT_COUNT {
                    let Some(effect) = sequence.next_element()? else {
                        break;
                    };
                    order.0[count] = effect;
                    count += 1;
                }
                if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::invalid_length(EFFECT_COUNT + 1, &self));
                }
                if !(11..=EFFECT_COUNT).contains(&count) {
                    return Err(serde::de::Error::invalid_length(count, &self));
                }
                // Each historical length has its own complete set of stable discriminants.
                // Reject missing, duplicated, or newer effects before inserting bypassed stages.
                let mut seen = 0_u16;
                for effect in &order.0[..count] {
                    let bit = 1_u16 << *effect as u8;
                    if seen & bit != 0 {
                        return Err(serde::de::Error::custom(
                            "effect_order must contain every effect exactly once",
                        ));
                    }
                    seen |= bit;
                }
                if seen != (1_u16 << count) - 1 {
                    return Err(serde::de::Error::custom(
                        "effect_order does not match its format's effects",
                    ));
                }
                while count < EFFECT_COUNT {
                    let (predecessor, effect) = if count == 11 {
                        (EffectKind::Robot, EffectKind::Vocoder)
                    } else {
                        (EffectKind::Vocoder, EffectKind::Granular)
                    };
                    let position = order.0[..count]
                        .iter()
                        .position(|current| *current == predecessor)
                        .expect("complete historical permutation contains its predecessor");
                    order.0.copy_within(position + 1..count, position + 2);
                    order.0[position + 1] = effect;
                    count += 1;
                }
                if !order.is_valid() {
                    return Err(serde::de::Error::custom(
                        "effect_order must contain every effect exactly once",
                    ));
                }
                Ok(order)
            }
        }
        deserializer.deserialize_seq(OrderVisitor)
    }
}

fn deserialize_effect_mix<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[f32; EFFECT_COUNT], D::Error> {
    struct MixVisitor;
    impl<'de> serde::de::Visitor<'de> for MixVisitor {
        type Value = [f32; EFFECT_COUNT];
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("13 effect intensities or 11 or 12 legacy intensities")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut mix = [1.0; EFFECT_COUNT];
            let mut count = 0;
            while count < EFFECT_COUNT {
                let Some(value) = sequence.next_element()? else {
                    break;
                };
                mix[count] = value;
                count += 1;
            }
            if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::invalid_length(EFFECT_COUNT + 1, &self));
            }
            if !(11..=EFFECT_COUNT).contains(&count) {
                return Err(serde::de::Error::invalid_length(count, &self));
            }
            Ok(mix)
        }
    }
    deserializer.deserialize_seq(MixVisitor)
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

/// Granular processing intent. Natural mode constrains its effective shift and disables
/// scatter and feedback in the prepared DSP; stored values remain available when switching modes.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum GranularMode {
    #[default]
    Stylized,
    Natural,
}
impl GranularMode {
    fn from_atomic(value: u32) -> Self {
        match value {
            1 => Self::Natural,
            _ => Self::Stylized,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DspSettings {
    pub effect_order: EffectOrder,
    /// Wet proportion by EffectKind discriminant; missing legacy fields default to fully wet.
    #[serde(deserialize_with = "deserialize_effect_mix")]
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
    pub vocoder_mix: f32,
    pub vocoder_carrier_hz: f32,
    pub vocoder_unvoiced: f32,
    pub vocoder_release_ms: f32,
    pub granular_mode: GranularMode,
    pub granular_mix: f32,
    pub granular_pitch_semitones: f32,
    pub granular_grain_ms: f32,
    pub granular_scatter: f32,
    pub granular_feedback: f32,
    pub granular_clarity: f32,
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
            vocoder_mix: 0.0,
            vocoder_carrier_hz: 110.0,
            vocoder_unvoiced: 0.5,
            vocoder_release_ms: 60.0,
            granular_mode: GranularMode::Stylized,
            granular_mix: 0.0,
            granular_pitch_semitones: 0.0,
            granular_grain_ms: 32.0,
            granular_scatter: 0.0,
            granular_feedback: 0.0,
            granular_clarity: 0.75,
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
            ("vocoder_mix", self.vocoder_mix, 0.0, 1.0),
            ("vocoder_carrier_hz", self.vocoder_carrier_hz, 55.0, 440.0),
            ("vocoder_unvoiced", self.vocoder_unvoiced, 0.0, 1.0),
            ("vocoder_release_ms", self.vocoder_release_ms, 10.0, 200.0),
            ("granular_mix", self.granular_mix, 0.0, 1.0),
            (
                "granular_pitch_semitones",
                self.granular_pitch_semitones,
                -12.0,
                12.0,
            ),
            ("granular_grain_ms", self.granular_grain_ms, 12.0, 80.0),
            ("granular_scatter", self.granular_scatter, 0.0, 1.0),
            ("granular_feedback", self.granular_feedback, 0.0, 0.65),
            ("granular_clarity", self.granular_clarity, 0.0, 1.0),
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
    #[serde(deserialize_with = "deserialize_effect_mix")]
    pub effect_mix: [f32; EFFECT_COUNT],
    pub gain_db: f32,
    pub gate_db: f32,
    pub highpass_hz: f32,
    pub lowpass_hz: f32,
    pub compressor: bool,
    pub drive: f32,
    pub robot_mix: f32,
    pub robot_hz: f32,
    pub vocoder_mix: f32,
    pub vocoder_carrier_hz: f32,
    pub vocoder_unvoiced: f32,
    pub vocoder_release_ms: f32,
    pub granular_mode: GranularMode,
    pub granular_mix: f32,
    pub granular_pitch_semitones: f32,
    pub granular_grain_ms: f32,
    pub granular_scatter: f32,
    pub granular_feedback: f32,
    pub granular_clarity: f32,
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
            vocoder_mix: settings.vocoder_mix,
            vocoder_carrier_hz: settings.vocoder_carrier_hz,
            vocoder_unvoiced: settings.vocoder_unvoiced,
            vocoder_release_ms: settings.vocoder_release_ms,
            granular_mode: settings.granular_mode,
            granular_mix: settings.granular_mix,
            granular_pitch_semitones: settings.granular_pitch_semitones,
            granular_grain_ms: settings.granular_grain_ms,
            granular_scatter: settings.granular_scatter,
            granular_feedback: settings.granular_feedback,
            granular_clarity: settings.granular_clarity,
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
            vocoder_mix: self.vocoder_mix,
            vocoder_carrier_hz: self.vocoder_carrier_hz,
            vocoder_unvoiced: self.vocoder_unvoiced,
            vocoder_release_ms: self.vocoder_release_ms,
            granular_mode: self.granular_mode,
            granular_mix: self.granular_mix,
            granular_pitch_semitones: self.granular_pitch_semitones,
            granular_grain_ms: self.granular_grain_ms,
            granular_scatter: self.granular_scatter,
            granular_feedback: self.granular_feedback,
            granular_clarity: self.granular_clarity,
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
    /// Explicit validated Windows virtual render endpoint; output stays the headphones.
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
        if let Some(id) = self.virtual_output_device.as_deref()
            && (self.input_device.as_deref() == Some(id)
                || self.output_device.as_deref() == Some(id))
        {
            return Err("virtual output must be separate from microphone and headphones".into());
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

/// An installed virtual cable pair; labels are display text, never an identity proof.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VirtualMicrophoneTarget {
    pub provider: String,
    pub render_device: String,
    pub capture_device: String,
    pub render_description: String,
    pub capture_description: String,
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
    pub targets: Vec<VirtualMicrophoneTarget>,
}

/// Atomic settings are independent scalar controls, not a transactionally consistent preset snapshot.
/// The control thread is the only writer; callbacks sample the latest values without waiting.
pub struct AtomicDspSettings {
    effect_order: AtomicU64,
    values: [AtomicU32; 24],
    granular_mode: AtomicU32,
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
            granular_mode: AtomicU32::new(v.granular_mode as u32),
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
    fn floats(v: DspSettings) -> [f32; 24] {
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
            v.vocoder_mix,
            v.vocoder_carrier_hz,
            v.vocoder_unvoiced,
            v.vocoder_release_ms,
            v.granular_mix,
            v.granular_pitch_semitones,
            v.granular_grain_ms,
            v.granular_scatter,
            v.granular_feedback,
            v.granular_clarity,
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
        self.granular_mode
            .store(v.granular_mode as u32, Ordering::Relaxed);
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
            vocoder_mix: f[14],
            vocoder_carrier_hz: f[15],
            vocoder_unvoiced: f[16],
            vocoder_release_ms: f[17],
            granular_mode: GranularMode::from_atomic(self.granular_mode.load(Ordering::Relaxed)),
            granular_mix: f[18],
            granular_pitch_semitones: f[19],
            granular_grain_ms: f[20],
            granular_scatter: f[21],
            granular_feedback: f[22],
            granular_clarity: f[23],
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
    values: [AtomicU32; 24],
    granular_mode: AtomicU32,
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
            granular_mode: AtomicU32::new(layer.granular_mode as u32),
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
        self.granular_mode
            .store(layer.granular_mode as u32, Ordering::Relaxed);
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
            vocoder_mix: f[14],
            vocoder_carrier_hz: f[15],
            vocoder_unvoiced: f[16],
            vocoder_release_ms: f[17],
            granular_mode: GranularMode::from_atomic(self.granular_mode.load(Ordering::Relaxed)),
            granular_mix: f[18],
            granular_pitch_semitones: f[19],
            granular_grain_ms: f[20],
            granular_scatter: f[21],
            granular_feedback: f[22],
            granular_clarity: f[23],
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
    /// WASAPI diagnostics distinguish FIFO starvation/overflow from native discontinuities.
    #[serde(default)]
    pub input_queue_underruns: u64,
    #[serde(default)]
    pub input_queue_overruns: u64,
    #[serde(default)]
    pub input_capture_discontinuities: u64,
    #[serde(default)]
    pub monitor_queue_underruns: u64,
    #[serde(default)]
    pub monitor_backend_xruns: u64,
    /// Initial/recovery jitter-buffer silence; exposed separately, never erased.
    #[serde(default)]
    pub input_priming_frames: u64,
    #[serde(default)]
    pub monitor_priming_frames: u64,
    #[serde(default)]
    pub soundboard_underruns: u64,
    #[serde(default)]
    pub preview_underruns: u64,
    /// Actual callback maxima, not the requested endpoint buffer duration.
    #[serde(default)]
    pub input_callback_frames_max: u64,
    #[serde(default)]
    pub render_callback_frames_max: u64,
    #[serde(default)]
    pub monitor_callback_frames_max: u64,
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
    pub input_queue_underruns: AtomicU64,
    pub input_queue_overruns: AtomicU64,
    pub input_capture_discontinuities: AtomicU64,
    pub monitor_queue_underruns: AtomicU64,
    pub monitor_backend_xruns: AtomicU64,
    pub input_priming_frames: AtomicU64,
    pub monitor_priming_frames: AtomicU64,
    pub soundboard_underruns: AtomicU64,
    pub preview_underruns: AtomicU64,
    pub input_callback_frames_max: AtomicU64,
    pub render_callback_frames_max: AtomicU64,
    pub monitor_callback_frames_max: AtomicU64,
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
            input_queue_underruns: self.input_queue_underruns.load(Ordering::Relaxed),
            input_queue_overruns: self.input_queue_overruns.load(Ordering::Relaxed),
            input_capture_discontinuities: self
                .input_capture_discontinuities
                .load(Ordering::Relaxed),
            monitor_queue_underruns: self.monitor_queue_underruns.load(Ordering::Relaxed),
            monitor_backend_xruns: self.monitor_backend_xruns.load(Ordering::Relaxed),
            input_priming_frames: self.input_priming_frames.load(Ordering::Relaxed),
            monitor_priming_frames: self.monitor_priming_frames.load(Ordering::Relaxed),
            soundboard_underruns: self.soundboard_underruns.load(Ordering::Relaxed),
            preview_underruns: self.preview_underruns.load(Ordering::Relaxed),
            input_callback_frames_max: self.input_callback_frames_max.load(Ordering::Relaxed),
            render_callback_frames_max: self.render_callback_frames_max.load(Ordering::Relaxed),
            monitor_callback_frames_max: self.monitor_callback_frames_max.load(Ordering::Relaxed),
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
    pub soundboard_active: AtomicBool,
    pub preview_active: AtomicBool,
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
            soundboard_active: AtomicBool::new(false),
            preview_active: AtomicBool::new(false),
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
    fn external_cable_status_preserves_exact_ids_and_legacy_defaults() {
        let old: VirtualMicrophoneStatus = serde_json::from_str(
            r#"{"available":true,"active":false,"render_device":"legacy-output"}"#,
        )
        .unwrap();
        assert!(old.targets.is_empty());
        let status = VirtualMicrophoneStatus {
            targets: vec![VirtualMicrophoneTarget {
                provider: "vb_cable".into(),
                render_device: "wasapi-render-id".into(),
                capture_device: "wasapi-capture-id".into(),
                render_description: "CABLE Input".into(),
                capture_description: "CABLE Output".into(),
            }],
            ..old
        };
        let encoded = serde_json::to_string(&status).unwrap();
        assert_eq!(
            serde_json::from_str::<VirtualMicrophoneStatus>(&encoded).unwrap(),
            status
        );
        assert!(!status.active);
    }
    #[test]
    fn publishing_cannot_reuse_the_microphone_or_headphone_endpoint() {
        for (input, output) in [(Some("cable"), None), (None, Some("cable"))] {
            let config = AudioConfig {
                input_device: input.map(str::to_owned),
                output_device: output.map(str::to_owned),
                virtual_output_device: Some("cable".into()),
                ..Default::default()
            };
            assert!(config.validate().is_err());
        }
        assert!(
            AudioConfig {
                input_device: Some("physical-mic".into()),
                output_device: Some("headphones".into()),
                virtual_output_device: Some("cable".into()),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
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
    fn legacy_permutations_and_partial_intensities_migrate_in_both_layers() {
        for legacy_count in [11, 12] {
            let mut old_order = EffectKind::ALL[..legacy_count].to_vec();
            old_order.reverse();
            let old_mix: Vec<f32> = (0..legacy_count)
                .map(|index| index as f32 / legacy_count as f32)
                .collect();
            let legacy = serde_json::json!({
                "effect_order": old_order,
                "effect_mix": old_mix,
                "pitch_semitones": -5.0,
                "robot_mix": 0.35,
            });
            let mut expected_order = old_order.clone();
            if legacy_count == 11 {
                let robot = expected_order
                    .iter()
                    .position(|effect| *effect == EffectKind::Robot)
                    .unwrap();
                expected_order.insert(robot + 1, EffectKind::Vocoder);
            }
            let vocoder = expected_order
                .iter()
                .position(|effect| *effect == EffectKind::Vocoder)
                .unwrap();
            expected_order.insert(vocoder + 1, EffectKind::Granular);
            let mut expected_mix = [1.0; EFFECT_COUNT];
            expected_mix[..legacy_count].copy_from_slice(&old_mix);
            let mut primary_json = legacy.clone();
            primary_json["blend"] = serde_json::json!({
                "primary_mix": 0.7,
                "secondary_mix": 0.2,
                "secondary": legacy,
            });
            let settings: DspSettings = serde_json::from_value(primary_json).unwrap();
            settings.validate().unwrap();
            let secondary = settings.blend.unwrap().secondary;
            for layer in [VoiceLayer::from(settings), secondary] {
                assert_eq!(layer.effect_order.0.as_slice(), expected_order);
                assert_eq!(layer.effect_mix, expected_mix);
                assert_eq!(layer.pitch_semitones, -5.0);
                assert_eq!(layer.robot_mix, 0.35);
                assert_eq!(layer.vocoder_mix, 0.0);
                assert_eq!(layer.vocoder_carrier_hz, 110.0);
                assert_eq!(layer.vocoder_unvoiced, 0.5);
                assert_eq!(layer.vocoder_release_ms, 60.0);
                assert_eq!(layer.granular_mode, GranularMode::Stylized);
                assert_eq!(layer.granular_mix, 0.0);
                assert_eq!(layer.granular_pitch_semitones, 0.0);
                assert_eq!(layer.granular_grain_ms, 32.0);
                assert_eq!(layer.granular_scatter, 0.0);
                assert_eq!(layer.granular_feedback, 0.0);
                assert_eq!(layer.granular_clarity, 0.75);
                let retained_order: Vec<_> = layer
                    .effect_order
                    .0
                    .iter()
                    .copied()
                    .filter(|effect| (*effect as usize) < legacy_count)
                    .collect();
                assert_eq!(retained_order, old_order);
            }
            let encoded = serde_json::to_value(settings).unwrap();
            assert_eq!(
                encoded["effect_order"].as_array().unwrap().len(),
                EFFECT_COUNT
            );
            assert_eq!(
                encoded["effect_mix"].as_array().unwrap().len(),
                EFFECT_COUNT
            );
            assert_eq!(
                serde_json::from_value::<DspSettings>(encoded).unwrap(),
                settings
            );
            assert_eq!(AtomicDspSettings::new(settings).load(), settings);
        }
        assert_eq!(FORMAT_VERSION, 1);
        assert_eq!(EffectKind::Reverb as usize, 10);
        assert_eq!(EffectKind::Vocoder as usize, 11);
        assert_eq!(EffectKind::Granular as usize, 12);
    }
    #[test]
    fn malformed_legacy_and_current_arrays_are_rejected_before_use() {
        let original = EffectKind::ALL.to_vec();
        let mut orders = vec![Vec::new(), original[..10].to_vec()];
        let mut too_long = original.clone();
        too_long.push(EffectKind::Gain);
        orders.push(too_long);
        let mut duplicate = original.clone();
        duplicate[1] = duplicate[0];
        orders.push(duplicate);
        // Historical lengths must have exactly their historical set of effects.
        orders.push(original[1..].to_vec());
        orders.push(original[2..].to_vec());
        let mut old_with_vocoder = original[..11].to_vec();
        old_with_vocoder[0] = EffectKind::Vocoder;
        orders.push(old_with_vocoder);
        let mut previous_with_granular = original[..12].to_vec();
        previous_with_granular[11] = EffectKind::Granular;
        orders.push(previous_with_granular);
        let mut old_duplicate = original[..11].to_vec();
        old_duplicate[1] = old_duplicate[0];
        orders.push(old_duplicate);
        for order in orders {
            let layer = serde_json::json!({ "effect_order": order });
            assert!(serde_json::from_value::<DspSettings>(layer.clone()).is_err());
            let nested = serde_json::json!({ "blend": { "secondary": layer } });
            assert!(serde_json::from_value::<DspSettings>(nested).is_err());
        }
        for count in [0, 1, 10, 14, 24] {
            let layer = serde_json::json!({ "effect_mix": vec![0.5; count] });
            assert!(serde_json::from_value::<DspSettings>(layer.clone()).is_err());
            let nested = serde_json::json!({ "blend": { "secondary": layer } });
            assert!(serde_json::from_value::<DspSettings>(nested).is_err());
        }
    }
    #[test]
    fn vocoder_parameters_roundtrip_with_reordered_blended_voices() {
        let mut settings = DspSettings {
            vocoder_mix: 0.65,
            vocoder_carrier_hz: 95.0,
            vocoder_unvoiced: 0.7,
            vocoder_release_ms: 45.0,
            ..Default::default()
        };
        settings.effect_mix[EffectKind::Vocoder as usize] = 0.8;
        settings.effect_order.0.reverse();
        let mut secondary = VoiceLayer::from(DspSettings {
            vocoder_mix: 0.4,
            vocoder_carrier_hz: 220.0,
            vocoder_unvoiced: 0.9,
            vocoder_release_ms: 30.0,
            ..Default::default()
        });
        secondary.effect_mix[EffectKind::Vocoder as usize] = 0.3;
        secondary.effect_order.0.rotate_left(3);
        settings.blend = Some(VoiceBlend {
            secondary,
            primary_mix: 0.6,
            secondary_mix: 0.4,
        });
        settings.validate().unwrap();
        let encoded = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<DspSettings>(&encoded).unwrap(),
            settings
        );
        let atomic = AtomicDspSettings::default();
        atomic.store(settings);
        assert_eq!(atomic.load(), settings);
        assert_eq!(VoiceLayer::from(settings).to_settings().vocoder_mix, 0.65);
        atomic.store(DspSettings::default());
        assert_eq!(atomic.load(), DspSettings::default());
    }
    #[test]
    fn granular_parameters_roundtrip_with_reordered_blended_voices() {
        let mut settings = DspSettings {
            granular_mode: GranularMode::Natural,
            granular_mix: 0.65,
            granular_pitch_semitones: -3.5,
            granular_grain_ms: 24.0,
            granular_scatter: 0.8,
            granular_feedback: 0.4,
            granular_clarity: 0.9,
            ..Default::default()
        };
        settings.effect_mix[EffectKind::Granular as usize] = 0.6;
        settings.effect_order.0.reverse();
        let mut secondary = VoiceLayer::from(DspSettings {
            granular_mode: GranularMode::Stylized,
            granular_mix: 0.3,
            granular_pitch_semitones: 12.0,
            granular_grain_ms: 64.0,
            granular_scatter: 0.35,
            granular_feedback: 0.6,
            granular_clarity: 0.5,
            ..Default::default()
        });
        secondary.effect_mix[EffectKind::Granular as usize] = 0.25;
        secondary.effect_order.0.rotate_left(4);
        settings.blend = Some(VoiceBlend {
            secondary,
            primary_mix: 0.8,
            secondary_mix: 0.2,
        });
        settings.validate().unwrap();
        let encoded = serde_json::to_value(settings).unwrap();
        assert_eq!(encoded["granular_mode"], "natural");
        assert_eq!(encoded["blend"]["secondary"]["granular_mode"], "stylized");
        assert_eq!(
            serde_json::from_value::<DspSettings>(encoded).unwrap(),
            settings
        );
        assert_eq!(AtomicDspSettings::new(settings).load(), settings);
        let atomics = AtomicDspSettings::default();
        atomics.store(settings);
        assert_eq!(atomics.load(), settings);
        let primary = VoiceLayer::from(settings).to_settings();
        assert_eq!(VoiceLayer::from(primary), VoiceLayer::from(settings));
        assert!(primary.blend.is_none());
        atomics.store(DspSettings::default());
        assert_eq!(atomics.load(), DspSettings::default());
    }
    #[test]
    fn granular_mode_rejects_unknown_names_and_bounds_atomic_tags() {
        assert_eq!(GranularMode::default(), GranularMode::Stylized);
        for mode in [serde_json::json!("unknown"), serde_json::json!(2)] {
            let layer = serde_json::json!({ "granular_mode": mode });
            assert!(serde_json::from_value::<DspSettings>(layer.clone()).is_err());
            let nested = serde_json::json!({ "blend": { "secondary": layer } });
            assert!(serde_json::from_value::<DspSettings>(nested).is_err());
        }
        let atomics = AtomicDspSettings::new(DspSettings {
            granular_mode: GranularMode::Natural,
            blend: Some(VoiceBlend {
                secondary: VoiceLayer::from(DspSettings {
                    granular_mode: GranularMode::Natural,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        });
        atomics.granular_mode.store(u32::MAX, Ordering::Relaxed);
        atomics.secondary.granular_mode.store(2, Ordering::Relaxed);
        let loaded = atomics.load();
        assert_eq!(loaded.granular_mode, GranularMode::Stylized);
        assert_eq!(
            loaded.blend.unwrap().secondary.granular_mode,
            GranularMode::Stylized
        );
    }
    #[test]
    fn granular_ranges_reject_invalid_main_and_secondary_values() {
        fn assign(settings: &mut DspSettings, name: &str, value: f32) {
            match name {
                "granular_mix" => settings.granular_mix = value,
                "granular_pitch_semitones" => settings.granular_pitch_semitones = value,
                "granular_grain_ms" => settings.granular_grain_ms = value,
                "granular_scatter" => settings.granular_scatter = value,
                "granular_feedback" => settings.granular_feedback = value,
                "granular_clarity" => settings.granular_clarity = value,
                _ => unreachable!(),
            }
        }
        for (name, minimum, maximum) in [
            ("granular_mix", 0.0, 1.0),
            ("granular_pitch_semitones", -12.0, 12.0),
            ("granular_grain_ms", 12.0, 80.0),
            ("granular_scatter", 0.0, 1.0),
            ("granular_feedback", 0.0, 0.65),
            ("granular_clarity", 0.0, 1.0),
        ] {
            for mode in [GranularMode::Stylized, GranularMode::Natural] {
                // Natural mode retains the same stored range; only its effective DSP is limited.
                for value in [minimum, maximum] {
                    let mut settings = DspSettings {
                        granular_mode: mode,
                        ..Default::default()
                    };
                    assign(&mut settings, name, value);
                    settings.validate().unwrap();
                    DspSettings {
                        blend: Some(VoiceBlend {
                            secondary: VoiceLayer::from(settings),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                    .validate()
                    .unwrap();
                }
                for value in [
                    minimum - 0.1,
                    maximum + 0.1,
                    f32::NAN,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                ] {
                    let mut settings = DspSettings {
                        granular_mode: mode,
                        ..Default::default()
                    };
                    assign(&mut settings, name, value);
                    assert!(settings.validate().unwrap_err().starts_with(name));
                    let nested = DspSettings {
                        blend: Some(VoiceBlend {
                            secondary: VoiceLayer::from(settings),
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    assert!(nested.validate().unwrap_err().starts_with(name));
                }
            }
        }
    }
    #[test]
    fn vocoder_ranges_reject_invalid_main_and_secondary_values() {
        for (name, minimum, maximum) in [
            ("vocoder_mix", 0.0, 1.0),
            ("vocoder_carrier_hz", 55.0, 440.0),
            ("vocoder_unvoiced", 0.0, 1.0),
            ("vocoder_release_ms", 10.0, 200.0),
        ] {
            for value in [minimum, maximum] {
                let settings: DspSettings =
                    serde_json::from_value(serde_json::json!({ name: value })).unwrap();
                settings.validate().unwrap();
            }
            for value in [minimum - 0.1, maximum + 0.1] {
                let layer = serde_json::json!({ name: value });
                let settings: DspSettings = serde_json::from_value(layer.clone()).unwrap();
                assert!(settings.validate().unwrap_err().starts_with(name));
                let nested = serde_json::json!({ "blend": { "secondary": layer } });
                let settings: DspSettings = serde_json::from_value(nested).unwrap();
                assert!(settings.validate().unwrap_err().starts_with(name));
            }
        }
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for settings in [
                DspSettings {
                    vocoder_mix: value,
                    ..Default::default()
                },
                DspSettings {
                    vocoder_carrier_hz: value,
                    ..Default::default()
                },
                DspSettings {
                    vocoder_unvoiced: value,
                    ..Default::default()
                },
                DspSettings {
                    vocoder_release_ms: value,
                    ..Default::default()
                },
            ] {
                assert!(settings.validate().is_err());
                assert!(
                    DspSettings {
                        blend: Some(VoiceBlend {
                            secondary: VoiceLayer::from(settings),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                    .validate()
                    .is_err()
                );
            }
        }
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
