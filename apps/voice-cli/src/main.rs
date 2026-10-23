mod mcp;

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use voice_core::{AudioConfig, DspSettings, RecordingSource, SAMPLE_RATE};
use voice_engine::{
    Paths,
    client::{send, send_request},
    protocol::{Command, Request},
};
use voice_library::{ImportLimits, Library, decode_audio, write_wav};

#[derive(Parser)]
#[command(
    name = "voice-cli",
    version,
    about = "Native voice engine control and offline audio tools"
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Run the service and restore its saved connection intent. --start requests capture now.
    Serve {
        #[arg(long)]
        start: bool,
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        output: Option<String>,
        /// Explicit validated virtual render ID from doctor/status (Windows, e.g. VB-CABLE).
        #[arg(long, requires = "start")]
        virtual_output: Option<String>,
        #[arg(long, default_value_t = 256)]
        quantum: u32,
    },
    /// Probe the native audio backend without opening capture.
    Doctor,
    Status,
    Devices,
    /// Start voice streams; physical IDs come from devices, virtual IDs from doctor/status.
    Start {
        #[arg(long)]
        input: Option<String>,
        #[arg(long)]
        output: Option<String>,
        /// Send final voice and soundboard to this validated virtual render ID (e.g. VB-CABLE).
        #[arg(long)]
        virtual_output: Option<String>,
        #[arg(long, default_value_t = 256)]
        quantum: u32,
    },
    Stop,
    /// Capture one explicitly selected output in stereo, independently of the microphone.
    DesktopStart {
        #[arg(long)]
        output: String,
    },
    /// Stop desktop capture and finalize its recording, retaining its replay.
    DesktopStop,
    /// List implemented conventional DSP presets.
    Voices,
    Select {
        id: String,
    },
    Gain {
        #[arg(allow_hyphen_values = true)]
        db: f32,
    },
    Monitor {
        #[arg(action = clap::ArgAction::Set, value_parser = parse_switch)]
        enabled: bool,
        #[arg(long)]
        gain: Option<f32>,
    },
    Mute {
        #[arg(action = clap::ArgAction::Set, value_parser = parse_switch)]
        enabled: bool,
    },
    Bypass {
        #[arg(action = clap::ArgAction::Set, value_parser = parse_switch)]
        enabled: bool,
    },
    Boards,
    CreateBoard {
        name: String,
    },
    Clips {
        #[arg(default_value = "default")]
        board_id: String,
    },
    Import {
        path: PathBuf,
        #[arg(long, default_value = "default")]
        board: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a soundboard entry and stop that sound; keep all audio files.
    RemoveClip {
        id: String,
    },
    Play {
        id: String,
        #[arg(long)]
        preview: bool,
        #[arg(long)]
        looped: bool,
        #[arg(long)]
        overlap: bool,
        #[arg(long, default_value_t = 1.0)]
        gain: f32,
    },
    StopSounds,
    Record {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = CaptureSource::ProcessedVoice)]
        source: CaptureSource,
    },
    StopRecording,
    Replay {
        path: PathBuf,
        #[arg(long, default_value_t = 30)]
        seconds: u32,
        #[arg(long, value_enum, default_value_t = CaptureSource::ProcessedVoice)]
        source: CaptureSource,
    },
    ImportPreset {
        path: PathBuf,
    },
    ExportPreset {
        id: String,
        path: PathBuf,
    },
    /// Forward a JSON Command or {id,command} Request to the engine registry.
    Ctl {
        json: String,
    },
    Shutdown,
    /// Decode/resample, apply DSP and write a new WAV offline.
    Process {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "builtin.clean")]
        preset: String,
        #[arg(long, allow_hyphen_values = true)]
        gain_db: Option<f32>,
    },
    /// Generate a quiet test WAV offline, without playback or capture.
    Tone {
        output: PathBuf,
        #[arg(long, default_value_t = 2.0)]
        seconds: f64,
        #[arg(long, default_value_t = 440.0)]
        hz: f64,
    },
    /// Measure local DSP execution; not end-to-end audio latency.
    Bench {
        #[arg(long, default_value_t = 10_000)]
        blocks: usize,
        #[arg(long)]
        advanced: bool,
    },
    /// MCP 2024-11-05 over newline-delimited JSON-RPC stdin/stdout.
    Mcp,
}

#[derive(Clone, Copy, ValueEnum)]
enum CaptureSource {
    ProcessedVoice,
    Desktop,
}
impl From<CaptureSource> for RecordingSource {
    fn from(source: CaptureSource) -> Self {
        match source {
            CaptureSource::ProcessedVoice => Self::ProcessedVoice,
            CaptureSource::Desktop => Self::Desktop,
        }
    }
}

fn parse_switch(value: &str) -> std::result::Result<bool, String> {
    match value {
        "true" | "on" | "1" => Ok(true),
        "false" | "off" | "0" => Ok(false),
        _ => Err("expected on/off or true/false".into()),
    }
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("voice-cli: {error:#}");
        std::process::exit(1);
    }
}

fn emit(value: &Value) -> Result<()> {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    out.write_all(b"\n")?;
    Ok(())
}

fn absolute(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn normalize_paths(mut command: Command) -> Result<Command> {
    match &mut command {
        Command::ImportPreset { path }
        | Command::SetPresetImage { path, .. }
        | Command::ExportPreset { path, .. }
        | Command::ImportClip { path, .. }
        | Command::StartRecording { path, .. }
        | Command::SaveReplay { path, .. } => *path = absolute(path.clone())?,
        _ => {}
    }
    Ok(command)
}

fn controls(
    bypass: Option<bool>,
    muted: Option<bool>,
    monitor: Option<bool>,
    monitor_gain: Option<f32>,
) -> Command {
    Command::SetControls {
        bypass,
        muted,
        monitor,
        monitor_gain,
        soundboard_gain: None,
    }
}

fn preset_id(id: String) -> String {
    if ["clean", "radio", "robot", "echo", "space", "megaphone"].contains(&id.as_str()) {
        format!("builtin.{id}")
    } else {
        id
    }
}

fn run(cli: Cli) -> Result<()> {
    let mut paths = Paths::discover()?;
    if let Some(path) = cli.data_dir {
        paths.data = absolute(path)?;
    }
    if let Some(path) = cli.socket {
        paths.socket = absolute(path)?;
    }
    let command = match cli.command {
        Action::Serve {
            start,
            input,
            output,
            virtual_output,
            quantum,
        } => {
            ensure!(
                start || (input.is_none() && output.is_none() && virtual_output.is_none()),
                "--input/--output/--virtual-output require serve --start"
            );
            let mut service = voice_engine::spawn_service(paths.clone())?;
            let stopping = Arc::new(AtomicBool::new(false));
            let signal = stopping.clone();
            ctrlc::set_handler(move || signal.store(true, Ordering::Release))
                .context("install Ctrl-C handler")?;
            if start {
                let config = AudioConfig {
                    input_device: input,
                    output_device: output,
                    virtual_output_device: virtual_output,
                    quantum,
                    ..Default::default()
                };
                if let Err(error) = send(&paths.socket, Command::Start { config }) {
                    service.stop()?;
                    return Err(error);
                }
            }
            let status = send(&paths.socket, Command::Status)?;
            emit(
                &json!({"service":"started", "socket":paths.socket,"capture_requested":status["audio_requested"]}),
            )?;
            while !stopping.load(Ordering::Acquire) && !service.is_finished() {
                std::thread::sleep(Duration::from_millis(50));
            }
            service.stop()?;
            return Ok(());
        }
        Action::Doctor => {
            let virtual_microphone =
                voice_engine::probe_virtual_microphone().unwrap_or_else(|error| {
                    voice_core::VirtualMicrophoneStatus {
                        diagnostic: Some(format!("Virtual microphone discovery failed: {error:#}")),
                        ..Default::default()
                    }
                });
            return match voice_engine::probe_devices() {
                Ok(devices) => emit(
                    &json!({"backend":voice_engine::BACKEND_NAME,"backend_available":true,"pipewire_available":voice_engine::BACKEND_NAME == "pipewire","virtual_microphone":virtual_microphone.available,"virtual_microphone_status":virtual_microphone,"capture_started":false,"devices":devices}),
                ),
                Err(error) => {
                    emit(
                        &json!({"backend":voice_engine::BACKEND_NAME,"backend_available":false,"pipewire_available":false,"virtual_microphone":virtual_microphone.available,"virtual_microphone_status":virtual_microphone,"capture_started":false,"error":error.to_string()}),
                    )?;
                    Err(error)
                }
            };
        }
        Action::Mcp => {
            return mcp::serve(
                std::io::stdin().lock(),
                std::io::stdout().lock(),
                &paths.socket,
            );
        }
        Action::Process {
            input,
            output,
            preset,
            gain_db,
        } => return offline_process(&paths, &input, &output, preset, gain_db),
        Action::Tone {
            output,
            seconds,
            hz,
        } => return tone(&output, seconds, hz),
        Action::Bench { blocks, advanced } => return benchmark(blocks, advanced),
        Action::Ctl { json } => {
            ensure!(json.len() <= 1_048_576, "request exceeds 1 MiB");
            let value: Value = serde_json::from_str(&json).context("parse JSON command")?;
            let mut request = if value.get("command").is_some() {
                serde_json::from_value::<Request>(value)?
            } else {
                Request {
                    id: 1,
                    command: serde_json::from_value::<Command>(value)?,
                }
            };
            request.command = normalize_paths(request.command)?;
            let response = send_request(&paths.socket, request)?;
            emit(&serde_json::to_value(&response)?)?;
            if !response.ok {
                bail!(
                    "{}",
                    response.error.unwrap_or_else(|| "command failed".into())
                );
            }
            return Ok(());
        }
        Action::Status => Command::Status,
        Action::Devices => Command::Devices,
        Action::Start {
            input,
            output,
            virtual_output,
            quantum,
        } => Command::Start {
            config: AudioConfig {
                input_device: input,
                output_device: output,
                virtual_output_device: virtual_output,
                quantum,
                ..Default::default()
            },
        },
        Action::Stop => Command::Stop,
        Action::DesktopStart { output } => Command::StartDesktopCapture {
            output_device: output,
        },
        Action::DesktopStop => Command::StopDesktopCapture,
        Action::Voices => Command::Presets,
        Action::Select { id } => Command::SelectPreset { id: preset_id(id) },
        Action::Gain { db } => {
            let status = send(&paths.socket, Command::Status)?;
            let mut settings: DspSettings = serde_json::from_value(
                status
                    .get("settings")
                    .cloned()
                    .context("status missing settings")?,
            )?;
            settings.gain_db = db;
            Command::Configure { settings }
        }
        Action::Monitor { enabled, gain } => controls(None, None, Some(enabled), gain),
        Action::Mute { enabled } => controls(None, Some(enabled), None, None),
        Action::Bypass { enabled } => controls(Some(enabled), None, None, None),
        Action::Boards => Command::Boards,
        Action::CreateBoard { name } => Command::CreateBoard { name },
        Action::Clips { board_id } => Command::Clips { board_id },
        Action::Import { path, board, name } => {
            let name = name.unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Audio clip")
                    .to_string()
            });
            Command::ImportClip {
                path: absolute(path)?,
                board_id: board,
                name,
            }
        }
        Action::Play {
            id,
            preview,
            looped,
            overlap,
            gain,
        } => Command::Play {
            id,
            preview,
            looped,
            overlap,
            gain,
        },
        Action::RemoveClip { id } => Command::RemoveClip { id },
        Action::StopSounds => Command::StopSounds,
        Action::Record { path, source } => Command::StartRecording {
            path: absolute(path)?,
            source: source.into(),
        },
        Action::StopRecording => Command::StopRecording,
        Action::Replay {
            path,
            seconds,
            source,
        } => Command::SaveReplay {
            path: absolute(path)?,
            seconds,
            source: source.into(),
        },
        Action::ImportPreset { path } => Command::ImportPreset {
            path: absolute(path)?,
        },
        Action::ExportPreset { id, path } => Command::ExportPreset {
            id: preset_id(id),
            path: absolute(path)?,
        },
        Action::Shutdown => Command::Shutdown,
    };
    emit(&send(&paths.socket, command)?)
}

fn offline_process(
    paths: &Paths,
    input: &Path,
    output: &Path,
    preset: String,
    gain_db: Option<f32>,
) -> Result<()> {
    ensure!(
        !output.exists(),
        "output already exists: {}",
        output.display()
    );
    paths.prepare_data()?;
    let library = Library::open(&paths.data.join("library"))?;
    let preset = preset_id(preset);
    let mut settings = library
        .presets()?
        .into_iter()
        .find(|p| p.id == preset)
        .with_context(|| format!("unknown preset: {preset}"))?
        .settings;
    if let Some(gain) = gain_db {
        settings.gain_db = gain;
    }
    settings.validate().map_err(anyhow::Error::msg)?;
    let mut samples = decode_audio(input, &ImportLimits::default())?;
    let mut processor = voice_dsp::Processor::new(SAMPLE_RATE);
    let frames = samples.len();
    let latency = processor.latency_frames(&settings);
    samples.resize(frames + latency, 0.0);
    for block in samples.chunks_mut(256) {
        processor.process(block, &settings);
    }
    write_wav(output, &samples[latency..])?;
    emit(
        &json!({"offline":true,"output":output,"frames":frames,"sample_rate":SAMPLE_RATE,"preset":preset,"compensated_latency_frames":latency,"tail_extended":false}),
    )
}

fn tone(output: &Path, seconds: f64, hz: f64) -> Result<()> {
    ensure!(
        !output.exists(),
        "output already exists: {}",
        output.display()
    );
    ensure!(
        seconds.is_finite() && seconds > 0.0 && seconds <= 120.0,
        "seconds must be finite and between 0 and 120"
    );
    ensure!(
        hz.is_finite() && (20.0..=20_000.0).contains(&hz),
        "hz must be between 20 and 20000"
    );
    let frames = (seconds * SAMPLE_RATE as f64).round() as usize;
    ensure!(frames > 0, "duration is shorter than one sample");
    let samples: Vec<f32> = (0..frames)
        .map(|i| {
            let fade = (i as f64 / 240.0)
                .min((frames - 1 - i) as f64 / 240.0)
                .min(1.0);
            (std::f64::consts::TAU * hz * i as f64 / SAMPLE_RATE as f64).sin() as f32
                * 0.15
                * fade as f32
        })
        .collect();
    write_wav(output, &samples)?;
    emit(&json!({"offline":true,"output":output,"frames":frames,"sample_rate":SAMPLE_RATE,"hz":hz}))
}

fn benchmark(blocks: usize, advanced: bool) -> Result<()> {
    ensure!(
        (100..=100_000).contains(&blocks),
        "blocks must be between 100 and 100000"
    );
    let template: [f32; 256] = std::array::from_fn(|i| {
        (std::f32::consts::TAU * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.25
    });
    let settings = DspSettings {
        compressor: true,
        drive: 0.3,
        robot_mix: 0.5,
        echo_mix: 0.4,
        reverb_mix: 0.3,
        pitch_semitones: if advanced { 7.0 } else { 0.0 },
        formant_semitones: if advanced { -3.0 } else { 0.0 },
        chorus_mix: if advanced { 0.5 } else { 0.0 },
        denoise: advanced,
        ..Default::default()
    };
    let mut processor = voice_dsp::Processor::new(SAMPLE_RATE);
    let mut block = template;
    for _ in 0..1_000 {
        block.copy_from_slice(&template);
        processor.process(&mut block, &settings);
    }
    let mut timings = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        block.copy_from_slice(&template);
        let now = Instant::now();
        processor.process(
            std::hint::black_box(&mut block),
            std::hint::black_box(&settings),
        );
        timings.push(now.elapsed().as_nanos() as u64);
        std::hint::black_box(&block);
    }
    let sum: u64 = timings.iter().sum();
    timings.sort_unstable();
    let percentile = |q: f64| timings[((blocks - 1) as f64 * q) as usize] as f64 / 1_000.0;
    let period = 256.0 / SAMPLE_RATE as f64 * 1_000_000.0;
    emit(
        &json!({"benchmark":"dsp_cpu_only","build":if cfg!(debug_assertions) {"debug"} else {"release"},
        "sample_rate":SAMPLE_RATE,"block_frames":256,"blocks":blocks,"period_us":period,
        "p50_us":percentile(0.5),"p95_us":percentile(0.95),"p99_us":percentile(0.99),"max_us":percentile(1.0),
        "mean_cpu_ratio":sum as f64 / blocks as f64 / 1_000.0 / period,
        "advanced":advanced,"algorithmic_latency_frames":processor.latency_frames(&settings),
        "end_to_end_latency_measured":false,"audio_devices_opened":false}),
    )
}

#[cfg(test)]
mod virtual_output_tests {
    use super::*;

    #[test]
    fn serve_requires_explicit_start_for_a_virtual_route_before_starting_a_service() {
        for id in ["owned-id", "{0.0.0.00000000}.{vb-cable-render}"] {
            assert!(Cli::try_parse_from(["voice-cli", "serve", "--virtual-output", id]).is_err());
            let cli = Cli::try_parse_from([
                "voice-cli",
                "serve",
                "--start",
                "--output",
                "headphones",
                "--virtual-output",
                id,
            ])
            .unwrap();
            assert!(matches!(cli.command, Action::Serve {
                start: true,
                output: Some(ref output),
                virtual_output: Some(ref virtual_output),
                ..
            } if output == "headphones" && virtual_output == id));
        }
    }

    #[test]
    fn start_accepts_an_explicit_virtual_id_separately_from_headphones() {
        for id in ["owned-id", "{0.0.0.00000000}.{vb-cable-render}"] {
            let cli = Cli::try_parse_from([
                "voice-cli",
                "start",
                "--input",
                "microphone",
                "--output",
                "headphones",
                "--virtual-output",
                id,
            ])
            .unwrap();
            assert!(matches!(cli.command, Action::Start {
                input: Some(ref input),
                output: Some(ref output),
                virtual_output: Some(ref virtual_output),
                ..
            } if input == "microphone" && output == "headphones" && virtual_output == id));
        }
        let legacy = Cli::try_parse_from(["voice-cli", "start"]).unwrap();
        assert!(matches!(
            legacy.command,
            Action::Start {
                virtual_output: None,
                ..
            }
        ));
    }

    #[test]
    fn shared_start_registry_accepts_the_same_config_as_raw_cli_and_mcp() {
        for id in ["owned-id", "{0.0.0.00000000}.{vb-cable-render}"] {
            let wire = json!({"op":"start","config":{
                "output_device":"headphones", "virtual_output_device":id
            }});
            let command: Command = serde_json::from_value(wire).unwrap();
            let Command::Start { config } = &command else {
                panic!("wrong command");
            };
            assert_eq!(config.output_device.as_deref(), Some("headphones"));
            assert_eq!(config.virtual_output_device.as_deref(), Some(id));
            assert_eq!(
                serde_json::to_value(command).unwrap()["config"]["virtual_output_device"],
                id
            );
        }
    }
}
