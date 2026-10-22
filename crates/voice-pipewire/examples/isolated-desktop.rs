//! Hardware-free desktop capture tests. Run only inside scripts/test-pipewire.sh.
mod support;

use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};
use voice_core::DesktopShared;

fn main() -> Result<()> {
    ensure!(
        std::env::var("VOICE_APP_ISOLATED_TEST").as_deref() == Ok("1"),
        "run scripts/test-pipewire.sh; never connect this test to user audio"
    );
    let fixture = PathBuf::from(std::env::var("VOICE_APP_TEST_DIR")?);
    let shared = Arc::new(DesktopShared::default());
    for target in [
        "",
        "voice-app.virtual-microphone",
        "voice-test.input",
        "nonexistent-sink",
    ] {
        ensure!(
            voice_pipewire::start_desktop(target.into(), shared.clone()).is_err(),
            "unsafe or missing desktop target accepted: {target}"
        );
    }
    let output = voice_pipewire::list_devices()?
        .into_iter()
        .find(|device| device.name == "voice-test.output")
        .context("fixture output missing")?;
    let mut handle = voice_pipewire::start_desktop(output.id.to_string(), shared.clone())?;
    ensure!(
        handle.target() == "voice-test.output",
        "target ID was not canonicalized"
    );
    for channel in ["FL", "FR"] {
        link(
            &port("-o", "voice-test.output:", &format!("monitor_{channel}"))?,
            &port(
                "-i",
                "voice-app.desktop-capture:",
                &format!("input_{channel}"),
            )?,
        )?;
    }
    // A quiet sink is healthy: heartbeat checks must not confuse silence with server failure.
    std::thread::sleep(Duration::from_millis(2300));
    ensure!(
        shared.active.load(Ordering::Acquire),
        "quiet desktop capture was stopped"
    );
    let mut scratch = [0.0; 4096];
    while shared.samples.pop(&mut scratch) > 0 {}
    let path = fixture.join("desktop-stereo.raw");
    let raw: Vec<u8> = (0..48_000)
        .flat_map(|i| {
            let time = i as f32 / 48_000.0;
            [
                0.2 * (std::f32::consts::TAU * 330.0 * time).sin(),
                0.4 * (std::f32::consts::TAU * 770.0 * time).sin(),
            ]
        })
        .flat_map(f32::to_le_bytes)
        .collect();
    std::fs::write(&path, raw)?;
    let _player = ChildGuard(
        support::raw_stdio_command("pw-play")?
            .args([
                "--format",
                "f32",
                "--channels",
                "2",
                "--rate",
                "48000",
                "--target",
                "0",
                "--latency",
                "256",
                "--properties",
                "node.name=voice-test.desktop-player adapter.auto-port-config={mode=dsp position=preserve}",
            ])
            .arg("-")
            .stdin(File::open(path)?)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    for channel in ["FL", "FR"] {
        link(
            &port(
                "-o",
                "voice-test.desktop-player:",
                &format!("output_{channel}"),
            )?,
            &port("-i", "voice-test.output:", &format!("playback_{channel}"))?,
        )?;
    }
    let mut peaks = [0.0_f32; 2];
    let mut energy = [0.0_f64; 2];
    let mut product = 0.0_f64;
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        let count = shared.samples.pop(&mut scratch);
        ensure!(count % 2 == 0, "partial stereo frame in desktop queue");
        for frame in scratch[..count].as_chunks::<2>().0 {
            ensure!(
                frame.iter().all(|sample| sample.is_finite()),
                "non-finite captured PCM"
            );
            for channel in 0..2 {
                peaks[channel] = peaks[channel].max(frame[channel].abs());
                energy[channel] += f64::from(frame[channel]).powi(2);
            }
            product += f64::from(frame[0]) * f64::from(frame[1]);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    ensure!(
        (0.19..0.21).contains(&peaks[0]) && (0.39..0.41).contains(&peaks[1]),
        "desktop channels/levels changed: {peaks:?}"
    );
    ensure!(
        energy.iter().all(|e| *e > 1.0) && product.abs() / (energy[0] * energy[1]).sqrt() < 0.05,
        "independent stereo signals were remixed: energy={energy:?} product={product}"
    );
    ensure!(
        !nodes()?.contains("voice-app.virtual-microphone"),
        "desktop-only capture unexpectedly created a microphone route"
    );
    ensure!(
        shared.samples.len() <= shared.samples.capacity(),
        "desktop queue grew"
    );
    handle.stop();
    ensure!(
        !shared.active.load(Ordering::Acquire),
        "stop left desktop capture active"
    );
    ensure!(
        !nodes()?.contains("voice-app.desktop-capture"),
        "capture node survived stop"
    );

    let mut stopped = voice_pipewire::start_desktop("voice-test.output".into(), shared.clone())?;
    let frozen = FrozenDaemon::new()?;
    let start = Instant::now();
    stopped.stop();
    let stop_elapsed = start.elapsed();
    drop(frozen);
    ensure!(
        stop_elapsed < Duration::from_secs(2),
        "desktop stop stalled: {stop_elapsed:?}"
    );
    let mut stalled = voice_pipewire::start_desktop("voice-test.output".into(), shared.clone())?;
    let frozen = FrozenDaemon::new()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while shared.active.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let detected = !shared.active.load(Ordering::Acquire);
    drop(frozen);
    stalled.stop();
    ensure!(
        detected,
        "desktop control heartbeat did not detect frozen server"
    );
    println!(
        "PASS: explicit desktop sink capture; independent stereo peaks {peaks:?}; bounded queue; idle capture; owned/missing target rejection; node cleanup; frozen-server stop {stop_elapsed:?} and heartbeat"
    );
    Ok(())
}

fn port(direction: &str, prefix: &str, suffix: &str) -> Result<String> {
    for _ in 0..100 {
        let output = Command::new("pw-link").arg(direction).output()?;
        if let Some(name) = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|name| name.starts_with(prefix) && name.ends_with(suffix))
        {
            return Ok(name.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::bail!("missing isolated port: {prefix} {suffix}")
}
fn link(source: &str, sink: &str) -> Result<()> {
    ensure!(
        Command::new("pw-link")
            .args([source, sink])
            .status()?
            .success(),
        "cannot link {source} -> {sink}"
    );
    Ok(())
}
fn nodes() -> Result<String> {
    Ok(String::from_utf8(
        Command::new("pw-cli").args(["ls", "Node"]).output()?.stdout,
    )?)
}
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct FrozenDaemon(String);
impl FrozenDaemon {
    fn new() -> Result<Self> {
        let pid = std::env::var("VOICE_APP_TEST_DAEMON_PID")?;
        ensure!(
            Command::new("kill")
                .args(["-STOP", &pid])
                .status()?
                .success(),
            "could not suspend isolated daemon"
        );
        Ok(Self(pid))
    }
}
impl Drop for FrozenDaemon {
    fn drop(&mut self) {
        let _ = Command::new("kill").args(["-CONT", &self.0]).status();
    }
}
