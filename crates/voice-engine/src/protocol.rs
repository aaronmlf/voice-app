use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use voice_core::{AudioConfig, DspSettings, RecordingSource};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub id: u64,
    pub command: Command,
}

/// The only command registry. All clients use this wire contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status,
    Devices,
    Start {
        config: AudioConfig,
    },
    Stop,
    Configure {
        settings: DspSettings,
    },
    SetControls {
        bypass: Option<bool>,
        muted: Option<bool>,
        monitor: Option<bool>,
        monitor_gain: Option<f32>,
        soundboard_gain: Option<f32>,
    },
    Presets,
    PresetImage {
        id: String,
    },
    SetPresetImage {
        id: String,
        path: PathBuf,
    },
    ClearPresetImage {
        id: String,
    },
    SelectPreset {
        id: String,
    },
    SavePreset {
        id: String,
        name: String,
        settings: DspSettings,
    },
    ImportPreset {
        path: PathBuf,
    },
    ExportPreset {
        id: String,
        path: PathBuf,
    },
    Boards,
    CreateBoard {
        name: String,
    },
    Clips {
        board_id: String,
    },
    ImportClip {
        path: PathBuf,
        board_id: String,
        name: String,
    },
    TrimClip {
        id: String,
        start_seconds: f64,
        end_seconds: f64,
        name: String,
    },
    Play {
        id: String,
        preview: bool,
        looped: bool,
        overlap: bool,
        gain: f32,
    },
    StopSounds,
    StartDesktopCapture {
        output_device: String,
    },
    StopDesktopCapture,
    StartRecording {
        path: PathBuf,
        #[serde(default)]
        source: RecordingSource,
    },
    StopRecording,
    SaveReplay {
        path: PathBuf,
        seconds: u32,
        #[serde(default)]
        source: RecordingSource,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
impl Response {
    pub fn success(id: u64, result: impl Serialize) -> Self {
        match serde_json::to_value(result) {
            Ok(result) => Self {
                id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => Self::failure(id, error.to_string()),
        }
    }
    pub fn failure(id: u64, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}
