//! CLI compatibility for isolated fixtures, outside the application's audio callbacks.
use anyhow::{Context, Result, ensure};
use std::process::Command;

/// Old pw-cat treats `-` as raw PCM; newer versions require `--raw` as well.
/// Always pass `-` and connect the corresponding stdin/stdout in the caller.
pub fn raw_stdio_command(program: &str) -> Result<Command> {
    let help = Command::new(program)
        .arg("--help")
        .output()
        .with_context(|| format!("read {program} CLI capabilities"))?;
    ensure!(
        help.status.success(),
        "cannot read {program} CLI capabilities"
    );
    let supports_raw = [&help.stdout[..], &help.stderr[..]]
        .into_iter()
        .any(|output| output.windows(5).any(|window| window == b"--raw"));
    let mut command = Command::new(program);
    if supports_raw {
        command.arg("--raw");
    }
    Ok(command)
}
