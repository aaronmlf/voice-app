use serde_json::{Value, json};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    server: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "voice-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, server: None }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_voice-cli"));
        command
            .arg("--data-dir")
            .arg(self.root.join("data"))
            .arg("--socket")
            .arg(self.root.join("runtime/control.sock"));
        command
    }
    fn start_service(&mut self) {
        self.server = Some(
            self.command()
                .arg("serve")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        for _ in 0..250 {
            if voice_engine::client::send(
                &self.root.join("runtime/control.sock"),
                voice_engine::protocol::Command::Status,
            )
            .is_ok()
            {
                return;
            }
            if let Some(status) = self.server.as_mut().unwrap().try_wait().unwrap() {
                panic!("service exited: {status}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("service socket did not become ready");
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
    fn stop_service(&mut self) {
        self.json(&["shutdown"]);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while self.server.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline, "service did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
        self.server = None;
    }
    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.server.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn virtual_start_forwards_exact_render_ids_without_changing_the_headphones() {
    let fixture = Fixture::new();
    let runtime = fixture.root.join("runtime");
    fs::create_dir(&runtime).unwrap();
    let listener = UnixListener::bind(runtime.join("control.sock")).unwrap();
    let server = std::thread::spawn(move || {
        let mut commands = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            commands.push(request["command"].clone());
            writeln!(
                stream,
                "{}",
                json!({"id":request["id"],"ok":true,"result":{}})
            )
            .unwrap();
        }
        commands
    });
    let ids = ["owned-id", "{0.0.0.00000000}.{vb-cable-render}"];
    for id in ids {
        fixture.json(&[
            "start",
            "--input",
            "physical-microphone",
            "--output",
            "physical-headphones",
            "--virtual-output",
            id,
        ]);
    }
    for (command, id) in server.join().unwrap().iter().zip(ids) {
        assert_eq!(command["op"], "start");
        assert_eq!(command["config"]["input_device"], "physical-microphone");
        assert_eq!(command["config"]["output_device"], "physical-headphones");
        assert_eq!(command["config"]["virtual_output_device"], id);
    }
}

#[cfg(unix)]
#[test]
fn desktop_commands_forward_explicit_targets_sources_and_absolute_paths() {
    let fixture = Fixture::new();
    let runtime = fixture.root.join("runtime");
    fs::create_dir(&runtime).unwrap();
    let listener = UnixListener::bind(runtime.join("control.sock")).unwrap();
    let server = std::thread::spawn(move || {
        let mut commands = Vec::new();
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            commands.push(request["command"].clone());
            writeln!(
                stream,
                "{}",
                json!({"id": request["id"], "ok": true, "result": {}})
            )
            .unwrap();
        }
        commands
    });
    for args in [
        vec!["desktop-start", "--output", "test.output"],
        vec!["record", "desktop.wav", "--source", "desktop"],
        vec![
            "replay",
            "desktop-replay.wav",
            "--source",
            "desktop",
            "--seconds",
            "10",
        ],
        vec!["desktop-stop"],
        vec!["record", "voice.wav"],
        vec!["replay", "voice-replay.wav"],
    ] {
        let output = fixture
            .command()
            .current_dir(&fixture.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let commands = server.join().unwrap();
    assert_eq!(
        commands[0],
        json!({"op":"start_desktop_capture","output_device":"test.output"})
    );
    assert_eq!(
        commands[1],
        json!({"op":"start_recording","path":fixture.root.join("desktop.wav"),"source":"desktop"})
    );
    assert_eq!(
        commands[2],
        json!({"op":"save_replay","path":fixture.root.join("desktop-replay.wav"),"source":"desktop","seconds":10})
    );
    assert_eq!(commands[3], json!({"op":"stop_desktop_capture"}));
    assert_eq!(commands[4]["source"], "processed_voice");
    assert_eq!(commands[5]["source"], "processed_voice");
}

#[test]
fn service_commands_use_one_session_without_implicit_capture() {
    let mut fixture = Fixture::new();
    fixture.start_service();
    let status = fixture.json(&["status"]);
    assert_eq!(status["audio_running"], false);
    let envelope = fixture.json(&["ctl", r#"{"id":42,"command":{"op":"status"}}"#]);
    assert_eq!(envelope["id"], 42);
    let export = fixture
        .command()
        .current_dir(&fixture.root)
        .args([
            "ctl",
            r#"{"op":"export_preset","id":"builtin.radio","path":"from-client.json"}"#,
        ])
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(fixture.root.join("from-client.json").exists());
    assert_eq!(status["monitor"], false);
    assert_eq!(status["capabilities"]["neural_voice"], false);
    assert!(
        fixture
            .json(&["voices"])
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == "builtin.robot")
    );
    fixture.json(&["select", "radio"]);
    fixture.json(&["gain", "-3"]);
    fixture.json(&["monitor", "on", "--gain", "0.2"]);
    fixture.json(&["mute", "true"]);
    fixture.json(&["bypass", "off"]);
    let status = fixture.json(&["status"]);
    assert_eq!(status["settings"]["gain_db"], -3.0);
    assert_eq!(status["settings"]["highpass_hz"], 400.0);
    assert_eq!(status["monitor"], true);
    assert_eq!(status["muted"], true);
    assert_eq!(status["audio_running"], false);
    let board = fixture.json(&["create-board", "CLI soundboard"]);
    let board_id = board["id"].as_str().unwrap();
    assert!(
        fixture
            .json(&["boards"])
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["id"] == board_id)
    );
    let tone = fixture.root.join("tone.wav");
    fixture.json(&["tone", tone.to_str().unwrap(), "--seconds", "0.1"]);
    let clip = fixture.json(&["import", tone.to_str().unwrap(), "--board", board_id]);
    assert_eq!(
        fixture.json(&["clips", board_id]).as_array().unwrap().len(),
        1
    );
    assert!(
        !fixture
            .run(&["play", clip["id"].as_str().unwrap(), "--preview"])
            .status
            .success()
    );
    assert!(!fixture.run(&["gain", "NaN"]).status.success());
    assert_eq!(fixture.json(&["status"])["settings"]["gain_db"], -3.0);
    fixture.json(&["shutdown"]);
}

#[test]
fn remove_clip_persists_keeps_audio_files_and_preserves_other_entries() {
    let mut fixture = Fixture::new();
    fixture.start_service();
    let source = fixture.root.join("source.wav");
    fixture.json(&["tone", source.to_str().unwrap(), "--seconds", "0.1"]);
    let source_bytes = fs::read(&source).unwrap();
    let removed = fixture.json(&["import", source.to_str().unwrap(), "--name", "Remove me"]);
    let retained = fixture.json(&["import", source.to_str().unwrap(), "--name", "Keep me"]);
    let removed_id = removed["id"].as_str().unwrap();
    let retained_id = retained["id"].as_str().unwrap();
    assert_ne!(removed_id, retained_id);
    let managed_audio = PathBuf::from(removed["path"].as_str().unwrap());
    let managed_bytes = fs::read(&managed_audio).unwrap();
    assert!(
        !fixture
            .run(&["remove-clip", "missing-clip"])
            .status
            .success()
    );
    assert_eq!(fixture.json(&["clips"]).as_array().unwrap().len(), 2);
    assert_eq!(
        fixture.json(&["remove-clip", removed_id]),
        json!({"removed":true,"id":removed_id})
    );
    assert!(!fixture.run(&["remove-clip", removed_id]).status.success());
    let clips = fixture.json(&["clips"]);
    assert_eq!(clips.as_array().unwrap().len(), 1);
    assert_eq!(clips[0]["id"], retained_id);
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(fs::read(&managed_audio).unwrap(), managed_bytes);
    fixture.stop_service();
    fixture.start_service();
    let clips = fixture.json(&["clips"]);
    assert_eq!(clips.as_array().unwrap().len(), 1);
    assert_eq!(clips[0]["id"], retained_id);
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(fs::read(&managed_audio).unwrap(), managed_bytes);
    assert_eq!(fixture.json(&["status"])["audio_running"], false);
    fixture.stop_service();
}

#[test]
fn offline_render_is_real_audio_and_existing_output_is_preserved() {
    let fixture = Fixture::new();
    let input = fixture.root.join("input.wav");
    let output = fixture.root.join("radio.wav");
    fixture.json(&[
        "tone",
        input.to_str().unwrap(),
        "--seconds",
        "0.2",
        "--hz",
        "440",
    ]);
    let result = fixture.json(&[
        "process",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--preset",
        "radio",
        "--gain-db",
        "-6",
    ]);
    assert_eq!(result["offline"], true);
    assert_eq!(result["frames"], 9_600);
    assert!(!fixture.root.join("runtime/control.sock").exists());
    let mut reader = hound::WavReader::open(&output).unwrap();
    assert_eq!(reader.spec().sample_rate, 48_000);
    assert_eq!(reader.spec().channels, 1);
    let samples = reader
        .samples::<f32>()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 0.98));
    assert!(samples.iter().any(|s| s.abs() > 0.01));
    let original_output = fs::read(&output).unwrap();
    assert!(
        !fixture
            .run(&["process", input.to_str().unwrap(), output.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(fs::read(&output).unwrap(), original_output);
    assert_ne!(fs::read(&input).unwrap(), original_output);
    {
        let library = voice_library::Library::open(&fixture.root.join("data/library")).unwrap();
        library
            .save_preset(&voice_library::Preset {
                id: "test.pitch".into(),
                name: "Test octave".into(),
                original: false,
                settings: voice_core::DspSettings {
                    pitch_semitones: 12.0,
                    ..Default::default()
                },
            })
            .unwrap();
    }
    let shifted = fixture.root.join("shifted.wav");
    let result = fixture.json(&[
        "process",
        input.to_str().unwrap(),
        shifted.to_str().unwrap(),
        "--preset",
        "test.pitch",
    ]);
    assert_eq!(result["frames"], 9_600);
    assert_eq!(result["compensated_latency_frames"], 4_224);
    let shifted_samples: Vec<f32> = hound::WavReader::open(shifted)
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(shifted_samples.len(), 9_600);
    assert!(
        shifted_samples[..2_400]
            .iter()
            .any(|sample| sample.abs() > 0.01),
        "offline renderer must remove the leading algorithm buffer"
    );
}

#[test]
fn offline_render_uses_the_saved_effect_order_instead_of_only_preset_parameters() {
    let fixture = Fixture::new();
    let input = fixture.root.join("input.wav");
    fixture.json(&[
        "tone",
        input.to_str().unwrap(),
        "--seconds",
        "1",
        "--hz",
        "440",
    ]);
    let data = fixture.root.join("data");
    voice_engine::Paths {
        data: data.clone(),
        socket: fixture.root.join("runtime/control.sock"),
    }
    .prepare_data()
    .unwrap();
    let settings = voice_core::DspSettings {
        gain_db: -6.0,
        drive: 0.8,
        ..Default::default()
    };
    let mut reordered = settings;
    reordered.effect_order.0.swap(
        voice_core::EffectKind::Gain as usize,
        voice_core::EffectKind::Distortion as usize,
    );
    {
        let library = voice_library::Library::open(&data.join("library")).unwrap();
        for (id, settings) in [
            ("custom.gain-first", settings),
            ("custom.drive-first", reordered),
        ] {
            library
                .save_preset(&voice_library::Preset {
                    id: id.into(),
                    name: id.into(),
                    original: false,
                    settings,
                })
                .unwrap();
        }
    }
    let mut rendered = Vec::new();
    for id in ["custom.gain-first", "custom.drive-first"] {
        let output = fixture.root.join(format!("{id}.wav"));
        let result = fixture.json(&[
            "process",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "--preset",
            id,
        ]);
        assert_eq!(result["frames"], 48_000);
        assert_eq!(result["compensated_latency_frames"], 0);
        let samples: Vec<f32> = hound::WavReader::open(output)
            .unwrap()
            .samples::<f32>()
            .map(Result::unwrap)
            .collect();
        assert_eq!(samples.len(), 48_000);
        assert!(
            samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 0.98)
        );
        rendered.push(samples);
    }
    // Ignore startup smoothing and the tone's edge fades: changing two
    // noncommuting stages must change the actual stationary rendered waveform.
    let difference_rms = (rendered[0][12_000..36_000]
        .iter()
        .zip(&rendered[1][12_000..36_000])
        .map(|(a, b)| f64::from(a - b).powi(2))
        .sum::<f64>()
        / 24_000.)
        .sqrt();
    assert!(
        difference_rms > 0.025,
        "effect order was ignored: waveform difference RMS={difference_rms}"
    );
    assert!(!fixture.root.join("runtime/control.sock").exists());
}

#[test]
fn mcp_commands_reach_the_cli_service_and_stdout_contains_only_jsonrpc() {
    let mut fixture = Fixture::new();
    fixture.start_service();
    let mut mcp = fixture
        .command()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"integration","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"voice_command","arguments":{"command":{"op":"create_board","name":"From MCP"}}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"voice_status","arguments":{}}}),
    ];
    let mut input = mcp.stdin.take().unwrap();
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = mcp.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let values: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(values.len(), 3);
    assert_eq!(values[1]["result"]["isError"], false);
    let status: Value =
        serde_json::from_str(values[2]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(status["audio_running"], false);
    assert!(
        fixture
            .json(&["boards"])
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["name"] == "From MCP")
    );
    fixture.json(&["shutdown"]);
}
