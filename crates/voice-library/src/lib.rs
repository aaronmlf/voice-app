//! Offline library, decoding and mixing. All methods belong on the engine worker,
//! never on a real-time audio callback: they may allocate, perform I/O or release PCM.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use voice_core::{DspSettings, FORMAT_VERSION, SAMPLE_RATE};

mod artwork;
pub use artwork::{MAX_PRESET_IMAGE_BYTES, PRESET_IMAGE_SIDE, PresetImage};

pub const MAX_PLAYING_CLIPS: usize = 16;
pub const DEFAULT_BOARD_ID: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub settings: DspSettings,
    /// Local collection membership; independent of builtin ownership and not exported.
    #[serde(default)]
    pub original: bool,
}

impl Preset {
    pub fn is_original(&self) -> bool {
        self.id.starts_with("builtin.") || self.original
    }

    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        validate_name(&self.name)?;
        self.settings.validate().map_err(anyhow::Error::msg)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Board {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub id: String,
    pub board_id: String,
    pub name: String,
    pub path: PathBuf,
    /// Mono frames at 48 kHz; also the number of samples.
    pub frames: u64,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct ImportLimits {
    pub max_bytes: u64,
    pub max_duration_seconds: f64,
}
impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_bytes: 20 * 1024 * 1024,
            max_duration_seconds: 8.0 * 60.0,
        }
    }
}
impl ImportLimits {
    fn validate(&self) -> Result<()> {
        ensure!(self.max_bytes > 0, "max_bytes must be positive");
        ensure!(
            self.max_duration_seconds.is_finite()
                && self.max_duration_seconds > 0.0
                && self.max_duration_seconds <= 3600.0,
            "duration limit must be finite and between zero and one hour"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetFile {
    version: u32,
    preset: Preset,
}

/// A managed directory with `library.sqlite3` and normalized WAV files in `clips/`.
/// One engine worker owns the connection. Corrupt/newer databases return an error;
/// they are never silently replaced.
pub struct Library {
    root: PathBuf,
    db: Connection,
}
impl Library {
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root.join("clips")).context("create audio library directory")?;
        let db = Connection::open(root.join("library.sqlite3"))
            .context("open audio library database")?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= FORMAT_VERSION,
            "library database version {version} is newer than supported version {FORMAT_VERSION}"
        );
        db.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS presets(id TEXT PRIMARY KEY, name TEXT NOT NULL, settings TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS preset_origins(
                 preset_id TEXT PRIMARY KEY REFERENCES presets(id) ON DELETE CASCADE,
                 original INTEGER NOT NULL CHECK(original IN (0,1)));
             CREATE TABLE IF NOT EXISTS preset_images(
                 preset_id TEXT PRIMARY KEY REFERENCES presets(id) ON DELETE CASCADE,
                 file_name TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS boards(id TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS clips(
                 id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id),
                 name TEXT NOT NULL, file_name TEXT NOT NULL, frames INTEGER NOT NULL CHECK(frames>0));
             INSERT OR IGNORE INTO boards(id,name) VALUES('default','My soundboard');
             PRAGMA user_version=1;
             COMMIT;",
        )?;
        let library = Self {
            root: root.to_path_buf(),
            db,
        };
        for preset in builtin_presets() {
            library.db.execute(
                "INSERT OR IGNORE INTO presets(id,name,settings) VALUES(?1,?2,?3)",
                params![
                    preset.id,
                    preset.name,
                    serde_json::to_string(&preset.settings)?
                ],
            )?;
        }
        Ok(library)
    }

    pub fn presets(&self) -> Result<Vec<Preset>> {
        let mut query = self.db.prepare(
            "SELECT p.id,p.name,p.settings,COALESCE(o.original,0)
                FROM presets p LEFT JOIN preset_origins o ON o.preset_id=p.id
                ORDER BY p.name,p.id",
        )?;
        let rows = query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?;
        let mut presets = Vec::new();
        for row in rows {
            let (id, name, settings, original) = row?;
            let original = original || id.starts_with("builtin.");
            let preset = Preset {
                original,
                id,
                name,
                settings: serde_json::from_str(&settings)
                    .context("corrupt preset settings in library")?,
            };
            preset.validate()?;
            presets.push(preset);
        }
        Ok(presets)
    }

    /// Saving a definition preserves its existing local collection membership.
    /// New IDs start as personal, regardless of metadata supplied by an import.
    pub fn save_preset(&self, preset: &Preset) -> Result<()> {
        preset.validate()?;
        self.db.execute(
            "INSERT INTO presets(id,name,settings) VALUES(?1,?2,?3)
            ON CONFLICT(id) DO UPDATE SET name=excluded.name,settings=excluded.settings",
            params![
                preset.id,
                preset.name,
                serde_json::to_string(&preset.settings)?
            ],
        )?;
        Ok(())
    }

    /// Curate a local collection without modifying the definition, image or ID.
    pub fn set_preset_original(&self, id: &str, original: bool) -> Result<Preset> {
        validate_id(id)?;
        ensure!(
            original || !id.starts_with("builtin."),
            "builtin voices are always original"
        );
        let mut preset = self
            .presets()?
            .into_iter()
            .find(|p| p.id == id)
            .context("preset not found")?;
        if !id.starts_with("builtin.") {
            self.db.execute(
                "INSERT INTO preset_origins(preset_id,original) VALUES(?1,?2)
                 ON CONFLICT(preset_id) DO UPDATE SET original=excluded.original",
                params![id, original],
            )?;
        }
        preset.original = original;
        Ok(preset)
    }

    pub fn delete_preset(&self, id: &str) -> Result<()> {
        let image = self.preset_image(id)?;
        ensure!(
            self.db.execute("DELETE FROM presets WHERE id=?1", [id])? > 0,
            "preset not found: {id}"
        );
        if let Some(image) = image {
            let _ = fs::remove_file(image.path);
        }
        Ok(())
    }

    pub fn export_preset(&self, id: &str, path: &Path) -> Result<()> {
        let preset = self
            .presets()?
            .into_iter()
            .find(|p| p.id == id)
            .context("preset not found")?;
        fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": FORMAT_VERSION,
                "preset": {"id": preset.id, "name": preset.name, "settings": preset.settings}
            }))?,
        )
        .context("write preset export")
    }

    /// Import validates the complete envelope before saving. Same IDs update existing presets.
    pub fn import_preset(&self, path: &Path) -> Result<Preset> {
        let mut bytes = Vec::new();
        File::open(path)?.take(65_537).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 65_536, "preset JSON exceeds 64 KiB");
        let file: PresetFile = serde_json::from_slice(&bytes).context("invalid preset JSON")?;
        ensure!(
            file.version == FORMAT_VERSION,
            "unsupported preset format version: {}",
            file.version
        );
        file.preset.validate()?;
        self.save_preset(&file.preset)?;
        self.presets()?
            .into_iter()
            .find(|p| p.id == file.preset.id)
            .context("imported preset not found")
    }

    pub fn boards(&self) -> Result<Vec<Board>> {
        let mut query = self
            .db
            .prepare("SELECT id,name FROM boards ORDER BY name,id")?;
        Ok(query
            .query_map([], |r| {
                Ok(Board {
                    id: r.get(0)?,
                    name: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_board(&self, name: &str) -> Result<Board> {
        validate_name(name)?;
        let board = Board {
            id: new_id("board"),
            name: name.trim().into(),
        };
        self.db.execute(
            "INSERT INTO boards(id,name) VALUES(?1,?2)",
            params![board.id, board.name],
        )?;
        Ok(board)
    }

    pub fn clips(&self, board_id: &str) -> Result<Vec<Clip>> {
        let mut query = self.db.prepare("SELECT id,board_id,name,file_name,frames FROM clips WHERE board_id=?1 ORDER BY name,id")?;
        let rows = query
            .query_map([board_id], clip_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|r| self.finish_clip(r)).collect()
    }

    pub fn clip(&self, id: &str) -> Result<Clip> {
        validate_id(id)?;
        let row = self
            .db
            .query_row(
                "SELECT id,board_id,name,file_name,frames FROM clips WHERE id=?1",
                [id],
                clip_row,
            )
            .context("clip not found")?;
        self.finish_clip(row)
    }

    /// Remove a soundboard entry, keeping both its original source and normalized
    /// managed WAV intact. Retained audio still occupies disk space; this operation
    /// deliberately performs no decoding, file deletion or garbage collection.
    pub fn remove_clip(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        ensure!(
            self.db.execute("DELETE FROM clips WHERE id=?1", [id])? > 0,
            "clip not found: {id}"
        );
        Ok(())
    }

    fn finish_clip(&self, row: ClipRow) -> Result<Clip> {
        let (id, board_id, name, file_name, frames) = row;
        validate_id(&id)?;
        ensure!(
            file_name == format!("{id}.wav"),
            "invalid managed audio path in database"
        );
        ensure!(frames > 0, "invalid clip frame count");
        Ok(Clip {
            id,
            board_id,
            name,
            path: self.root.join("clips").join(file_name),
            frames,
            duration_seconds: frames as f64 / SAMPLE_RATE as f64,
        })
    }

    pub fn import_clip(&self, source: &Path, board_id: &str, name: &str) -> Result<Clip> {
        self.import_clip_with_limits(source, board_id, name, ImportLimits::default())
    }

    pub fn import_clip_with_limits(
        &self,
        source: &Path,
        board_id: &str,
        name: &str,
        limits: ImportLimits,
    ) -> Result<Clip> {
        validate_name(name)?;
        self.check_board(board_id)?;
        let samples = decode_audio(source, &limits)?;
        self.store_clip(&samples, board_id, name)
    }

    /// Save worker-owned mono 48 kHz samples to the managed library (e.g. replay capture).
    pub fn save_clip(&self, samples: &[f32], board_id: &str, name: &str) -> Result<Clip> {
        validate_name(name)?;
        self.check_board(board_id)?;
        ensure!(
            samples.len() <= SAMPLE_RATE as usize * 8 * 60,
            "clip exceeds eight minutes"
        );
        self.store_clip(samples, board_id, name)
    }

    fn check_board(&self, board_id: &str) -> Result<()> {
        let exists: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM boards WHERE id=?1)",
            [board_id],
            |r| r.get(0),
        )?;
        ensure!(exists, "soundboard not found: {board_id}");
        Ok(())
    }

    fn store_clip(&self, samples: &[f32], board_id: &str, name: &str) -> Result<Clip> {
        ensure!(!samples.is_empty(), "audio clip is empty");
        let id = new_id("clip");
        let file_name = format!("{id}.wav");
        let path = self.root.join("clips").join(&file_name);
        if let Err(error) = write_wav(&path, samples) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let result = self.db.execute(
            "INSERT INTO clips(id,board_id,name,file_name,frames) VALUES(?1,?2,?3,?4,?5)",
            params![id, board_id, name.trim(), file_name, samples.len() as u64],
        );
        if let Err(err) = result {
            let _ = fs::remove_file(&path);
            return Err(err.into());
        }
        Ok(Clip {
            id,
            board_id: board_id.into(),
            name: name.trim().into(),
            path,
            frames: samples.len() as u64,
            duration_seconds: samples.len() as f64 / SAMPLE_RATE as f64,
        })
    }

    pub fn load_clip(&self, id: &str) -> Result<Arc<Vec<f32>>> {
        let clip = self.clip(id)?;
        // Normalized float WAV is larger than a compressed source of equal duration.
        let samples = decode_audio(
            &clip.path,
            &ImportLimits {
                max_bytes: 4 * SAMPLE_RATE as u64 * 8 * 60 + 4096,
                ..ImportLimits::default()
            },
        )?;
        ensure!(
            samples.len() as u64 == clip.frames,
            "clip audio does not match its metadata"
        );
        Ok(Arc::new(samples))
    }

    /// Creates a trimmed copy, preserving the original clip. Times are seconds, end-exclusive.
    pub fn trim_clip(
        &self,
        id: &str,
        start_seconds: f64,
        end_seconds: f64,
        name: &str,
    ) -> Result<Clip> {
        validate_name(name)?;
        ensure!(
            start_seconds.is_finite()
                && end_seconds.is_finite()
                && start_seconds >= 0.0
                && end_seconds > start_seconds,
            "trim requires finite 0 <= start < end"
        );
        let clip = self.clip(id)?;
        ensure!(
            end_seconds <= clip.duration_seconds,
            "trim extends beyond clip duration"
        );
        let samples = self.load_clip(id)?;
        let start = (start_seconds * SAMPLE_RATE as f64).round() as usize;
        let end = ((end_seconds * SAMPLE_RATE as f64).round() as usize).min(samples.len());
        ensure!(start < end, "trim range contains no samples");
        self.store_clip(&samples[start..end], &clip.board_id, name)
    }
}

type ClipRow = (String, String, String, String, u64);
fn clip_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
        "ID must contain 1–128 ASCII letters, numbers, dots, underscores or hyphens"
    );
    Ok(())
}
fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty()
            && name.chars().count() <= 128
            && !name.chars().any(char::is_control),
        "name must contain 1–128 visible characters"
    );
    Ok(())
}
fn new_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}-{time:x}-{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn builtin_presets() -> Vec<Preset> {
    let clean = DspSettings::default();
    [
        ("clean", "Clean", clean),
        (
            "radio",
            "Radio",
            DspSettings {
                highpass_hz: 400.0,
                lowpass_hz: 3_200.0,
                compressor: true,
                drive: 0.35,
                ..clean
            },
        ),
        (
            "robot",
            "Robot",
            DspSettings {
                robot_mix: 0.85,
                robot_hz: 75.0,
                compressor: true,
                ..clean
            },
        ),
        (
            "echo",
            "Echo",
            DspSettings {
                echo_mix: 0.35,
                echo_delay_ms: 220.0,
                echo_feedback: 0.35,
                ..clean
            },
        ),
        (
            "space",
            "Space",
            DspSettings {
                reverb_mix: 0.4,
                echo_mix: 0.12,
                echo_delay_ms: 320.0,
                ..clean
            },
        ),
        (
            "deep",
            "Deep",
            DspSettings {
                pitch_semitones: -5.0,
                formant_semitones: -3.0,
                ..clean
            },
        ),
        (
            "high",
            "High",
            DspSettings {
                pitch_semitones: 5.0,
                formant_semitones: 2.0,
                ..clean
            },
        ),
        (
            "alien",
            "Alien",
            DspSettings {
                pitch_semitones: 3.0,
                formant_semitones: 6.0,
                robot_mix: 0.2,
                robot_hz: 42.0,
                ..clean
            },
        ),
        (
            "chorus",
            "Chorus",
            DspSettings {
                chorus_mix: 0.45,
                reverb_mix: 0.1,
                ..clean
            },
        ),
        (
            "broadcast",
            "Broadcast",
            DspSettings {
                denoise: true,
                gate_db: -45.0,
                compressor: true,
                highpass_hz: 70.0,
                lowpass_hz: 16_000.0,
                ..clean
            },
        ),
        (
            "monster",
            "Monster",
            DspSettings {
                pitch_semitones: -10.0,
                formant_semitones: -7.0,
                drive: 0.12,
                ..clean
            },
        ),
        (
            "megaphone",
            "Megaphone",
            DspSettings {
                highpass_hz: 650.0,
                lowpass_hz: 2_600.0,
                compressor: true,
                drive: 0.65,
                ..clean
            },
        ),
    ]
    .into_iter()
    .map(|(id, name, settings)| Preset {
        id: format!("builtin.{id}"),
        name: name.into(),
        settings,
        original: true,
    })
    .collect()
}

/// Decode WAV/MP3/Ogg Vorbis/FLAC, downmix to mono, then resample to 48 kHz.
/// Limits bound both source bytes and decoded duration; corruption returns an error.
pub fn decode_audio(path: &Path, limits: &ImportLimits) -> Result<Vec<f32>> {
    limits.validate()?;
    let file = File::open(path).context("open audio file")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limits.max_bytes,
        "audio file exceeds {} bytes",
        limits.max_bytes
    );
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|x| x.to_str()) {
        hint.with_extension(ext);
    }
    let probe = symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probe.format;
    let track = format
        .default_track()
        .context("audio file contains no default track")?;
    let track_id = track.id;
    let mut rate = track
        .codec_params
        .sample_rate
        .context("audio has no sample rate")?;
    ensure!(
        (8_000..=192_000).contains(&rate),
        "unsupported audio sample rate: {rate}"
    );
    if let Some(frames) = track.codec_params.n_frames {
        ensure!(
            frames as f64 / rate as f64 <= limits.max_duration_seconds + 0.1,
            "audio duration exceeds limit"
        );
    }
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut mono = Vec::new();
    let mut first = true;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(DecodeError::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e).context("read encoded audio packet"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).context("decode audio packet")?;
        let spec = *decoded.spec();
        if first {
            rate = spec.rate;
            first = false;
        }
        ensure!(
            spec.rate == rate && (8_000..=192_000).contains(&rate),
            "changing or unsupported audio sample rate"
        );
        let channels = spec.channels.count();
        ensure!(
            (1..=32).contains(&channels),
            "unsupported audio channel count"
        );
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let added = buffer.samples().len() / channels;
        ensure!(
            (mono.len() + added) as f64 / rate as f64 <= limits.max_duration_seconds,
            "audio duration exceeds {} seconds",
            limits.max_duration_seconds
        );
        for frame in buffer.samples().chunks_exact(channels) {
            mono.push(frame.iter().map(|&v| finite(v)).sum::<f32>() / channels as f32);
        }
    }
    ensure!(!mono.is_empty(), "audio contains no decoded samples");
    if rate == SAMPLE_RATE {
        return Ok(mono);
    }
    Ok(resample(&mono, rate, SAMPLE_RATE))
}

// Offline windowed-sinc interpolation includes a low-pass cutoff for downsampling.
// This deliberately favors bounded memory and quality over real-time performance.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    let len = (input.len() as f64 * to as f64 / from as f64).round() as usize;
    let ratio = from as f64 / to as f64;
    let cutoff = (to as f64 / from as f64).min(1.0) * 0.95;
    let radius = 32_i64;
    let mut output = Vec::with_capacity(len);
    for i in 0..len {
        let position = i as f64 * ratio;
        let center = position.floor() as i64;
        let mut sum = 0.0_f64;
        let mut weights = 0.0_f64;
        for sample in (center - radius + 1)..=(center + radius) {
            if sample < 0 || sample >= input.len() as i64 {
                continue;
            }
            let distance = position - sample as f64;
            let x = std::f64::consts::PI * distance * cutoff;
            let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
            let window = 0.5 * (1.0 + (std::f64::consts::PI * distance / radius as f64).cos());
            let weight = cutoff * sinc * window;
            sum += input[sample as usize] as f64 * weight;
            weights += weight;
        }
        output.push(if weights.abs() > 1e-12 {
            finite((sum / weights) as f32)
        } else {
            0.0
        });
    }
    output
}

/// Write normalized mono float WAV at 48 kHz. Parent directory must already exist.
pub fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    ensure!(!samples.is_empty(), "cannot write empty audio");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).context("create WAV file")?;
    for &sample in samples {
        writer.write_sample(finite(sample))?;
    }
    writer.finalize().context("finalize WAV file")
}

fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

struct PlayingClip {
    id: String,
    pcm: Arc<Vec<f32>>,
    position: usize,
    gain: f32,
    looped: bool,
}

/// Bounded polyphonic mixer for the engine worker, not a real-time callback.
/// `process` replaces output with the sum (clamped to [-1,1]); finished voices are released here.
#[derive(Default)]
pub struct Mixer {
    voices: Vec<PlayingClip>,
}
impl Mixer {
    pub fn new() -> Self {
        Self {
            voices: Vec::with_capacity(MAX_PLAYING_CLIPS),
        }
    }
    pub fn play(
        &mut self,
        id: &str,
        pcm: Arc<Vec<f32>>,
        gain: f32,
        looped: bool,
        overlap: bool,
    ) -> Result<()> {
        ensure!(!pcm.is_empty(), "cannot play empty clip");
        ensure!(
            gain.is_finite() && (0.0..=4.0).contains(&gain),
            "gain must be finite and between 0 and 4"
        );
        if !overlap {
            self.stop(id);
        }
        ensure!(
            self.voices.len() < MAX_PLAYING_CLIPS,
            "maximum of {MAX_PLAYING_CLIPS} simultaneous clips reached"
        );
        self.voices.push(PlayingClip {
            id: id.into(),
            pcm,
            position: 0,
            gain,
            looped,
        });
        Ok(())
    }
    pub fn stop(&mut self, id: &str) {
        self.voices.retain(|v| v.id != id);
    }
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }
    pub fn active_count(&self) -> usize {
        self.voices.len()
    }
    pub fn process(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        for voice in &mut self.voices {
            for sample in output.iter_mut() {
                if voice.position == voice.pcm.len() {
                    if voice.looped {
                        voice.position = 0;
                    } else {
                        break;
                    }
                }
                *sample += finite(voice.pcm[voice.position]) * voice.gain;
                voice.position += 1;
            }
        }
        self.voices.retain(|v| v.looped || v.position < v.pcm.len());
        for sample in output {
            *sample = finite(*sample);
        }
    }
}

/// Worker-owned rolling buffer. Snapshot is chronological, containing only captured samples.
pub struct ReplayBuffer {
    samples: Vec<f32>,
    next: usize,
    len: usize,
}
impl Default for ReplayBuffer {
    fn default() -> Self {
        Self::new(30)
    }
}
impl ReplayBuffer {
    pub fn new(seconds: usize) -> Self {
        Self::with_channels(seconds, 1)
    }
    /// Interleaved history; capacity is bounded independently of recording duration.
    pub fn with_channels(seconds: usize, channels: u16) -> Self {
        // Bound accidental configuration allocations to one hour; zero means disabled.
        Self {
            samples: vec![
                0.0;
                seconds.min(3600) * SAMPLE_RATE as usize * channels.clamp(1, 2) as usize
            ],
            next: 0,
            len: 0,
        }
    }
    pub fn push(&mut self, input: &[f32]) {
        if self.samples.is_empty() {
            return;
        }
        for &sample in input {
            self.samples[self.next] = finite(sample);
            self.next = (self.next + 1) % self.samples.len();
            self.len = (self.len + 1).min(self.samples.len());
        }
    }
    pub fn snapshot(&self) -> Vec<f32> {
        if self.len == 0 {
            return Vec::new();
        }
        let start = if self.len == self.samples.len() {
            self.next
        } else {
            0
        };
        (0..self.len)
            .map(|i| self.samples[(start + i) % self.samples.len()])
            .collect()
    }
    pub fn clear(&mut self) {
        self.next = 0;
        self.len = 0;
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(new_id("voice-library-test"));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn stereo_replay_has_a_fixed_30_second_capacity_and_keeps_channel_order() {
        let mut replay = ReplayBuffer::with_channels(30, 2);
        let capacity = 30 * SAMPLE_RATE as usize * 2;
        assert_eq!(replay.samples.len(), capacity);
        // More than the complete window: neither vector capacity nor retention grows.
        let chunk = [0.25_f32, -0.5].repeat(480);
        for _ in 0..3100 {
            replay.push(&chunk);
        }
        assert_eq!(replay.len(), capacity);
        assert_eq!(replay.samples.len(), capacity);
        assert_eq!(replay.samples.capacity(), capacity);
        let snapshot = replay.snapshot();
        assert!(
            snapshot
                .as_chunks::<2>()
                .0
                .iter()
                .all(|frame| *frame == [0.25, -0.5])
        );
        replay.clear();
        assert!(replay.is_empty());
        assert_eq!(replay.samples.capacity(), capacity);
    }

    #[test]
    fn preset_and_board_persist_with_versioned_interchange() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        let mut effect_order = voice_core::EffectOrder::default();
        effect_order.0.swap(
            voice_core::EffectKind::Gain as usize,
            voice_core::EffectKind::Distortion as usize,
        );
        let preset = Preset {
            original: false,
            id: "custom-test".into(),
            name: "My radio".into(),
            settings: DspSettings {
                effect_order,
                gain_db: 3.0,
                ..Default::default()
            },
        };
        let board;
        {
            let library = Library::open(&root)?;
            assert_eq!(library.presets()?.len(), 12);
            library.save_preset(&preset)?;
            board = library.create_board("Game")?;
            library.export_preset(&preset.id, &temp.0.join("voice.json"))?;
            library.delete_preset(&preset.id)?;
            assert_eq!(library.import_preset(&temp.0.join("voice.json"))?, preset);
        }
        let reopened = Library::open(&root)?;
        assert!(reopened.presets()?.contains(&preset));
        assert!(reopened.boards()?.contains(&board));
        Ok(())
    }

    #[test]
    fn original_collection_is_local_persistent_and_independent_of_names() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        let preset = Preset {
            id: "custom.generic".into(),
            name: "Original custom voice".into(),
            settings: DspSettings {
                gain_db: -3.0,
                ..Default::default()
            },
            original: true,
        };
        {
            let library = Library::open(&root)?;
            assert_eq!(
                library
                    .presets()?
                    .iter()
                    .filter(|p| p.is_original())
                    .count(),
                12
            );
            library.save_preset(&preset)?;
            let saved = library
                .presets()?
                .into_iter()
                .find(|p| p.id == preset.id)
                .unwrap();
            assert!(
                !saved.is_original(),
                "saving or naming cannot promote a preset"
            );
            let curated = library.set_preset_original(&preset.id, true)?;
            assert_eq!(curated.id, preset.id);
            assert_eq!(curated.name, preset.name);
            assert_eq!(curated.settings, preset.settings);
            assert!(curated.is_original());
            assert!(library.set_preset_original("builtin.clean", false).is_err());
            assert!(library.set_preset_original("custom.missing", true).is_err());
            // Updating the DSP definition does not reset the curated category.
            library.save_preset(&saved)?;
        }
        let library = Library::open(&root)?;
        let curated = library
            .presets()?
            .into_iter()
            .find(|p| p.id == preset.id)
            .unwrap();
        assert!(curated.is_original());
        assert_eq!(curated.settings, preset.settings);
        let personal = library.set_preset_original(&preset.id, false)?;
        assert!(!personal.is_original());
        assert_eq!(personal.settings, preset.settings);
        Ok(())
    }

    #[test]
    fn interchange_does_not_transfer_original_collection_membership() -> Result<()> {
        let temp = Temp::new();
        let library = Library::open(&temp.0.join("library"))?;
        let preset = Preset {
            id: "custom.curated".into(),
            name: "Curated".into(),
            settings: DspSettings::default(),
            original: false,
        };
        library.save_preset(&preset)?;
        library.set_preset_original(&preset.id, true)?;
        let path = temp.0.join("preset.json");
        library.export_preset(&preset.id, &path)?;
        let mut exported: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert!(exported["preset"].get("original").is_none());
        assert!(library.import_preset(&path)?.is_original());
        // Even a received JSON that advertises membership cannot promote a new ID.
        library.delete_preset(&preset.id)?;
        exported["preset"]["original"] = serde_json::json!(true);
        fs::write(&path, serde_json::to_vec(&exported)?)?;
        assert!(!library.import_preset(&path)?.is_original());
        library.set_preset_original(&preset.id, true)?;
        exported["preset"]["original"] = serde_json::json!(false);
        fs::write(&path, serde_json::to_vec(&exported)?)?;
        assert!(library.import_preset(&path)?.is_original());
        let other = Library::open(&temp.0.join("other-library"))?;
        assert!(!other.import_preset(&path)?.is_original());
        assert_eq!(
            other.presets()?.iter().filter(|p| p.is_original()).count(),
            12
        );
        Ok(())
    }

    #[test]
    fn existing_library_adds_collection_metadata_without_rewriting_definitions() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        fs::create_dir_all(&root)?;
        let settings = r#"{"gain_db":-4.0,"drive":0.2}"#;
        {
            let db = Connection::open(root.join("library.sqlite3"))?;
            db.execute_batch("CREATE TABLE presets(id TEXT PRIMARY KEY,name TEXT NOT NULL,settings TEXT NOT NULL); PRAGMA user_version=1;")?;
            db.execute(
                "INSERT INTO presets VALUES(?1,?2,?3)",
                params!["custom.existing", "Existing", settings],
            )?;
        }
        let library = Library::open(&root)?;
        let preset = library
            .presets()?
            .into_iter()
            .find(|p| p.id == "custom.existing")
            .unwrap();
        assert!(!preset.is_original());
        assert_eq!(preset.settings.gain_db, -4.0);
        assert_eq!(preset.settings.vocoder_mix, 0.0);
        library.set_preset_original(&preset.id, true)?;
        let stored: String = library.db.query_row(
            "SELECT settings FROM presets WHERE id=?1",
            [&preset.id],
            |r| r.get(0),
        )?;
        assert_eq!(
            stored, settings,
            "metadata must not reserialize existing DSP settings"
        );
        Ok(())
    }

    #[test]
    fn version_one_presets_without_effect_order_keep_the_original_chain() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        let path = temp.0.join("legacy.json");
        let legacy = serde_json::json!({
            "version": 1,
            "preset": {
                "id": "custom.legacy",
                "name": "Legacy voice",
                "settings": { "gain_db": 4.0, "drive": 0.25 }
            }
        });
        fs::write(&path, serde_json::to_vec(&legacy)?)?;
        {
            let library = Library::open(&root)?;
            let preset = library.import_preset(&path)?;
            assert_eq!(
                preset.settings.effect_order,
                voice_core::EffectOrder::default()
            );
            assert_eq!(preset.settings.gain_db, 4.0);
            // Existing SQLite rows use the same settings decoder as interchange files.
            library.db.execute(
                "UPDATE presets SET settings=?1 WHERE id=?2",
                params![legacy["preset"]["settings"].to_string(), preset.id],
            )?;
        }
        let reopened = Library::open(&root)?;
        let preset = reopened
            .presets()?
            .into_iter()
            .find(|preset| preset.id == "custom.legacy")
            .unwrap();
        assert_eq!(
            preset.settings.effect_order,
            voice_core::EffectOrder::default()
        );
        assert_eq!(preset.settings.drive, 0.25);
        Ok(())
    }

    #[test]
    fn invalid_effect_orders_never_overwrite_a_saved_preset_or_import_file() -> Result<()> {
        let temp = Temp::new();
        let library = Library::open(&temp.0.join("library"))?;
        let preset = Preset {
            original: false,
            id: "custom.ordered".into(),
            name: "Preserved voice".into(),
            settings: DspSettings {
                drive: 0.4,
                ..Default::default()
            },
        };
        library.save_preset(&preset)?;
        let mut invalid = preset.clone();
        invalid.settings.effect_order.0[0] = invalid.settings.effect_order.0[1];
        assert!(library.save_preset(&invalid).is_err());
        let path = temp.0.join("invalid-order.json");
        for failure in ["duplicate", "missing", "unknown"] {
            let mut envelope = serde_json::json!({"version":1,"preset":preset});
            let order = envelope["preset"]["settings"]["effect_order"]
                .as_array_mut()
                .unwrap();
            match failure {
                "duplicate" => order[0] = order[1].clone(),
                "missing" => {
                    order.pop();
                    order.pop();
                }
                "unknown" => order[0] = serde_json::json!("unknown_effect"),
                _ => unreachable!(),
            }
            let bytes = serde_json::to_vec_pretty(&envelope)?;
            fs::write(&path, &bytes)?;
            assert!(library.import_preset(&path).is_err(), "{failure}");
            assert_eq!(fs::read(&path)?, bytes);
            let saved = library
                .presets()?
                .into_iter()
                .find(|saved| saved.id == preset.id)
                .unwrap();
            assert_eq!(saved, preset, "{failure}");
        }
        Ok(())
    }

    #[test]
    fn malformed_newer_and_invalid_presets_do_not_change_library() -> Result<()> {
        let temp = Temp::new();
        let library = Library::open(&temp.0.join("library"))?;
        let path = temp.0.join("bad.json");
        fs::write(&path, b"not JSON")?;
        assert!(library.import_preset(&path).is_err());
        let mut preset = builtin_presets().remove(0);
        let json = serde_json::json!({"version": 999, "preset": preset});
        fs::write(&path, serde_json::to_vec(&json)?)?;
        assert!(library.import_preset(&path).is_err());
        preset.settings.echo_feedback = 1.5;
        assert!(library.save_preset(&preset).is_err());
        assert_eq!(library.presets()?.len(), 12);
        Ok(())
    }

    #[test]
    fn import_resample_copy_trim_and_limits() -> Result<()> {
        let temp = Temp::new();
        let source = temp.0.join("source.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 24_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&source, spec)?;
        for _ in 0..2400 {
            writer.write_sample(8192_i16)?;
            writer.write_sample(16384_i16)?;
        }
        writer.finalize()?;
        let library = Library::open(&temp.0.join("library"))?;
        let clip = library.import_clip(&source, DEFAULT_BOARD_ID, "Test sound")?;
        assert_eq!(clip.frames, 4800);
        fs::remove_file(&source)?;
        let samples = library.load_clip(&clip.id)?;
        assert!(samples.iter().all(|v| (*v - 0.375).abs() < 0.001));
        let trimmed = library.trim_clip(&clip.id, 0.025, 0.075, "Trimmed")?;
        assert_eq!(trimmed.frames, 2400);
        assert_eq!(library.load_clip(&clip.id)?.len(), 4800);
        assert_eq!(library.clips(DEFAULT_BOARD_ID)?.len(), 2);
        assert!(library.trim_clip(&clip.id, -1.0, 0.01, "Invalid").is_err());
        assert!(
            library
                .import_clip_with_limits(
                    &clip.path,
                    DEFAULT_BOARD_ID,
                    "Too big",
                    ImportLimits {
                        max_bytes: 2,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert!(
            library
                .import_clip_with_limits(
                    &clip.path,
                    DEFAULT_BOARD_ID,
                    "Too long",
                    ImportLimits {
                        max_duration_seconds: 0.01,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        fs::write(temp.0.join("bad.mp3"), b"corrupt MP3 audio")?;
        assert!(
            library
                .import_clip(&temp.0.join("bad.mp3"), DEFAULT_BOARD_ID, "Bad")
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn remove_clip_persists_metadata_only_and_retains_all_audio() -> Result<()> {
        let temp = Temp::new();
        let root = temp.0.join("library");
        let source = temp.0.join("original.wav");
        write_wav(&source, &[0.25; 32])?;
        let original_bytes = fs::read(&source)?;
        let library = Library::open(&root)?;
        let board = library.create_board("Other panel")?;
        let removed = library.import_clip(&source, DEFAULT_BOARD_ID, "Remove from panel")?;
        let other = library.import_clip(&source, &board.id, "Keep on other panel")?;
        let managed_bytes = fs::read(&removed.path)?;
        let other_bytes = fs::read(&other.path)?;
        library.remove_clip(&removed.id)?;
        assert!(library.clip(&removed.id).is_err());
        assert!(library.load_clip(&removed.id).is_err());
        assert!(library.clips(DEFAULT_BOARD_ID)?.is_empty());
        assert_eq!(library.clips(&board.id)?, vec![other.clone()]);
        assert_eq!(library.presets()?.len(), 12);
        assert_eq!(fs::read(&source)?, original_bytes);
        assert_eq!(fs::read(&removed.path)?, managed_bytes);
        assert_eq!(fs::read(&other.path)?, other_bytes);
        drop(library);

        let reopened = Library::open(&root)?;
        assert!(reopened.clip(&removed.id).is_err());
        assert_eq!(reopened.clips(&board.id)?, vec![other.clone()]);
        assert_eq!(reopened.load_clip(&other.id)?.len(), 32);
        assert_eq!(fs::read(&source)?, original_bytes);
        assert_eq!(fs::read(&removed.path)?, managed_bytes);
        assert_eq!(fs::read(&other.path)?, other_bytes);
        Ok(())
    }

    #[test]
    fn removal_rejects_unknown_ids_and_does_not_read_managed_audio() -> Result<()> {
        let temp = Temp::new();
        let library = Library::open(&temp.0.join("library"))?;
        let first = library.save_clip(&[0.1; 16], DEFAULT_BOARD_ID, "First")?;
        let second = library.save_clip(&[0.2; 16], DEFAULT_BOARD_ID, "Second")?;
        let first_bytes = fs::read(&first.path)?;
        let second_bytes = fs::read(&second.path)?;
        for id in ["", "../original.wav", "clip/invalid", "clip.missing"] {
            assert!(library.remove_clip(id).is_err(), "unexpected removal: {id}");
            assert_eq!(library.clips(DEFAULT_BOARD_ID)?.len(), 2);
        }
        assert_eq!(fs::read(&first.path)?, first_bytes);
        assert_eq!(fs::read(&second.path)?, second_bytes);
        // An unreadable/invalid audio payload must not prevent removal of its
        // panel entry, and retained bytes must not be modified as a side effect.
        fs::write(&first.path, b"not a WAV")?;
        library.remove_clip(&first.id)?;
        assert_eq!(fs::read(&first.path)?, b"not a WAV");
        assert!(library.remove_clip(&first.id).is_err());
        assert_eq!(library.clips(DEFAULT_BOARD_ID)?, vec![second]);
        Ok(())
    }

    #[test]
    fn corrupted_database_is_not_replaced() -> Result<()> {
        let temp = Temp::new();
        fs::write(temp.0.join("library.sqlite3"), b"not a sqlite database")?;
        assert!(Library::open(&temp.0).is_err());
        assert_eq!(
            fs::read(temp.0.join("library.sqlite3"))?,
            b"not a sqlite database"
        );
        Ok(())
    }

    #[test]
    fn imports_mp3_ogg_and_flac_fixtures() -> Result<()> {
        let temp = Temp::new();
        let library = Library::open(&temp.0.join("library"))?;
        for (extension, bytes) in [
            (
                "mp3",
                include_bytes!("../tests/fixtures/tone.mp3").as_slice(),
            ),
            (
                "ogg",
                include_bytes!("../tests/fixtures/tone.ogg").as_slice(),
            ),
            (
                "flac",
                include_bytes!("../tests/fixtures/tone.flac").as_slice(),
            ),
        ] {
            let source = temp.0.join(format!("tone.{extension}"));
            fs::write(&source, bytes)?;
            let clip = library.import_clip(&source, DEFAULT_BOARD_ID, extension)?;
            assert!(
                (0.08..0.2).contains(&clip.duration_seconds),
                "{extension}: {}",
                clip.duration_seconds
            );
            let pcm = library.load_clip(&clip.id)?;
            assert!(
                pcm.iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
            );
            let rms = (pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len() as f32).sqrt();
            assert!(rms > 0.02, "{extension}: signal missing ({rms})");
        }
        Ok(())
    }

    #[test]
    fn mixing_restart_overlap_loop_and_limit() -> Result<()> {
        let mut mixer = Mixer::new();
        let pcm = Arc::new(vec![0.2, 0.4]);
        mixer.play("a", pcm.clone(), 1.0, false, false)?;
        mixer.play("a", pcm.clone(), 1.0, false, false)?;
        assert_eq!(mixer.active_count(), 1);
        mixer.play("a", pcm.clone(), 1.0, true, true)?;
        let mut out = [99.0; 3];
        mixer.process(&mut out);
        assert_eq!(out, [0.4, 0.8, 0.2]);
        assert_eq!(mixer.active_count(), 1);
        mixer.stop("a");
        for _ in 0..MAX_PLAYING_CLIPS {
            mixer.play("a", pcm.clone(), 1.0, true, true)?;
        }
        assert!(mixer.play("a", pcm.clone(), 1.0, true, true).is_err());
        mixer.process(&mut out);
        assert!(out.iter().all(|v| v.abs() <= 1.0));
        mixer.stop_all();
        mixer.process(&mut out);
        assert_eq!(out, [0.0; 3]);
        assert!(
            mixer
                .play("empty", Arc::new(vec![]), 1.0, false, false)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn replay_is_bounded_chronological_and_sanitized() {
        let mut replay = ReplayBuffer {
            samples: vec![0.0; 3],
            next: 0,
            len: 0,
        };
        assert!(replay.snapshot().is_empty());
        replay.push(&[0.1, 0.2]);
        assert_eq!(replay.snapshot(), [0.1, 0.2]);
        replay.push(&[0.3, 0.4, f32::NAN]);
        assert_eq!(replay.snapshot(), [0.3, 0.4, 0.0]);
        replay.clear();
        assert!(replay.is_empty());
        let mut disabled = ReplayBuffer::new(0);
        disabled.push(&[1.0]);
        assert!(disabled.snapshot().is_empty());
    }

    #[test]
    fn downsampling_rejects_high_frequencies() {
        let high: Vec<f32> = (0..9600)
            .map(|i| (std::f32::consts::TAU * 30_000.0 * i as f32 / 96_000.0).sin())
            .collect();
        let low = resample(&high, 96_000, SAMPLE_RATE);
        let rms = (low[100..low.len() - 100].iter().map(|v| v * v).sum::<f32>()
            / (low.len() - 200) as f32)
            .sqrt();
        assert!(rms < 0.01, "alias rejection RMS: {rms}");
    }
}
