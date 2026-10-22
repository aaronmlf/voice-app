//! Reproducible project checks. This tool never changes the user's audio settings.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const PERMITTED_LICENSES: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "CC0-1.0",
    "Unlicense",
    "OFL-1.1",
    "Ubuntu-font-1.0",
    "MPL-2.0",
];

fn main() {
    if let Err(error) = execute() {
        eprintln!("xtask: {error:#}");
        std::process::exit(1);
    }
}
fn execute() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?;
    std::env::set_current_dir(root)?;
    let mut arguments = std::env::args().skip(1);
    let command = arguments.next().unwrap_or_else(|| "help".into());
    ensure!(
        arguments.next().is_none(),
        "unexpected argument; run cargo xtask help"
    );
    match command.as_str() {
        "check" => {
            dependency_boundaries(&metadata(true)?)?;
            run("cargo", &["fmt", "--all", "--", "--check"])?;
            let mut clippy = vec!["clippy", "--locked", "--workspace", "--all-targets"];
            let mut tests = vec!["test", "--locked", "--workspace", "--all-targets"];
            // PipeWire and its graph examples are Linux-only. Keep engine/IPC
            // and the Windows adapter in the native Windows checks.
            if cfg!(windows) {
                clippy.extend(["--exclude", "voice-pipewire"]);
                tests.extend(["--exclude", "voice-pipewire"]);
            }
            clippy.extend(["--", "-D", "warnings"]);
            run("cargo", &clippy)?;
            run("cargo", &tests)?;
            let mut python_tests = vec!["-m", "unittest", "discover", "-s", "scripts/tests"];
            if cfg!(windows) {
                // Other Python modules depend on Linux cgroups, fcntl and /proc.
                python_tests.extend(["-p", "test_pipewire_cli.py"]);
            }
            python_tests.push("-v");
            run(
                if cfg!(windows) { "python" } else { "python3" },
                &python_tests,
            )?;
            println!("Workspace formatting, Clippy, tests and dependency boundaries passed.");
        }
        "boundaries" => dependency_boundaries(&metadata(true)?)?,
        "bench" => run(
            "cargo",
            &[
                "run",
                "--locked",
                "--release",
                "-p",
                "voice-dsp",
                "--example",
                "bench",
            ],
        )?,
        "audio-check" => {
            ensure!(
                cfg!(target_os = "linux"),
                "audio-check requires Linux and an isolated PipeWire daemon; Windows offline validation is scripts/test-windows-offline.ps1"
            );
            ensure!(
                Command::new("bash")
                    .arg("scripts/test-pipewire.sh")
                    .env("VOICE_APP_ENGINE_TEST", "1")
                    .status()?
                    .success(),
                "isolated PipeWire and engine integration failed"
            );
        }
        "corpus" => {
            for test in [
                "imports_mp3_ogg_and_flac_fixtures",
                "import_resample_copy_trim_and_limits",
                "corrupted_database_is_not_replaced",
                "malformed_newer_and_invalid_presets_do_not_change_library",
            ] {
                run(
                    "cargo",
                    &[
                        "test",
                        "--locked",
                        "-p",
                        "voice-library",
                        test,
                        "--",
                        "--nocapture",
                    ],
                )?;
            }
            println!("Synthetic codec corpus and malformed-input regression checks passed.");
        }
        "license-check" => {
            license_report(&metadata(false)?, None)?;
        }
        "package" => package(root)?,
        "help" | "--help" | "-h" => println!(
            "cargo xtask <command>\n  check          fmt, Clippy, Rust/Python tests and dependency boundaries\n  boundaries     check architectural dependency allowlist\n  bench          release CPU-only DSP benchmark (not end-to-end latency)\n  audio-check    isolated PipeWire and engine integration (no hardware)\n  corpus         synthetic codec and malformed-input regressions\n  license-check  offline declared-SPDX allowlist report (not an audit)\n  package        native Linux tar.gz or Windows MSVC zip + notices + SHA256SUMS in dist/\n\nRun `cargo fetch --locked` first when using license-check offline.\nOptional fuller dependency checks: cargo deny check licenses bans sources advisories."
        ),
        _ => bail!("unknown command {command:?}; run cargo xtask help"),
    }
    Ok(())
}

fn run(program: &str, arguments: &[&str]) -> Result<()> {
    eprintln!("$ {program} {}", arguments.join(" "));
    ensure!(
        Command::new(program)
            .args(arguments)
            .status()
            .with_context(|| format!("start {program}"))?
            .success(),
        "{program} {} failed",
        arguments.join(" ")
    );
    Ok(())
}
fn capture(program: &str, arguments: &[&str]) -> Result<Output> {
    let result = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("start {program}"))?;
    ensure!(
        result.status.success(),
        "{program} {} failed:\n{}",
        arguments.join(" "),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(result)
}
fn host_triple() -> Result<String> {
    let output = capture("rustc", &["-vV"])?;
    String::from_utf8(output.stdout)?
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc host triple")
}
fn metadata(no_deps: bool) -> Result<Value> {
    let host = host_triple()?;
    let mut args = vec!["metadata", "--format-version", "1", "--locked", "--offline"];
    if no_deps {
        args.push("--no-deps");
    } else {
        args.extend(["--filter-platform", host.as_str()]);
    }
    let output = capture("cargo", &args).context("cargo metadata needs the locked dependencies in Cargo's local cache; run cargo fetch --locked if necessary")?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn boundary_policy() -> BTreeMap<&'static str, &'static [&'static str]> {
    BTreeMap::from([
        ("voice-core", &["serde", "serde_json"][..]),
        (
            "voice-dsp",
            &["voice-core", "signalsmith-stretch", "nnnoiseless"][..],
        ),
        (
            "voice-library",
            &[
                "voice-core",
                "anyhow",
                "serde",
                "serde_json",
                "hound",
                "image",
                "rusqlite",
                "symphonia",
            ][..],
        ),
        (
            "voice-pipewire",
            &[
                "voice-core",
                "voice-dsp",
                "anyhow",
                "pipewire",
                "serde_json",
            ][..],
        ),
        (
            "voice-wasapi",
            &["voice-core", "voice-dsp", "anyhow", "cpal"][..],
        ),
        (
            "voice-engine",
            &[
                "voice-core",
                "voice-dsp",
                "voice-library",
                "voice-pipewire",
                "voice-wasapi",
                "anyhow",
                "hound",
                "libc",
                "windows-sys",
                "serde",
                "serde_json",
            ][..],
        ),
        (
            "voice-app",
            &[
                "voice-core",
                "voice-engine",
                "voice-library",
                "anyhow",
                "serde_json",
                "eframe",
                "rfd",
                "image",
            ][..],
        ),
        (
            "voice-cli",
            &[
                "voice-core",
                "voice-dsp",
                "voice-engine",
                "voice-library",
                "anyhow",
                "serde_json",
                "clap",
                "hound",
                "ctrlc",
            ][..],
        ),
        ("xtask", &["anyhow", "serde_json"][..]),
    ])
}
fn boundary_errors(meta: &Value) -> Result<Vec<String>> {
    let policy = boundary_policy();
    let members: BTreeSet<&str> = meta["workspace_members"]
        .as_array()
        .context("workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut errors = Vec::new();
    for package in meta["packages"].as_array().context("packages")? {
        if !members.contains(package["id"].as_str().context("package id")?) {
            continue;
        }
        let name = package["name"].as_str().context("package name")?;
        let Some(permitted) = policy.get(name) else {
            errors.push(format!(
                "new workspace package {name}: define its architectural dependency policy"
            ));
            continue;
        };
        for dependency in package["dependencies"]
            .as_array()
            .context("package dependencies")?
        {
            let dependency = dependency["name"].as_str().context("dependency name")?;
            if !permitted.contains(&dependency) {
                errors.push(format!(
                    "{name} -> {dependency} is outside the architectural allowlist"
                ));
            }
        }
    }
    Ok(errors)
}
fn dependency_boundaries(meta: &Value) -> Result<()> {
    let errors = boundary_errors(meta)?;
    ensure!(
        errors.is_empty(),
        "dependency boundary violations:\n{}",
        errors.join("\n")
    );
    println!(
        "Dependency boundaries passed (all normal/build/dev/target-specific direct dependencies)."
    );
    Ok(())
}

/// Small fail-closed evaluator for SPDX identifiers, AND, OR, WITH, and parentheses.
/// It checks the explicit project allowlist, not legal compatibility or embedded assets.
fn permitted_expression(expression: &str) -> bool {
    // Cargo historically accepted slash-separated alternatives (e.g. MIT/Apache-2.0).
    let separated = expression
        .replace('/', " OR ")
        .replace('(', " ( ")
        .replace(')', " ) ");
    let tokens: Vec<&str> = separated.split_whitespace().collect();
    struct Parser<'a> {
        tokens: &'a [&'a str],
        next: usize,
    }
    impl Parser<'_> {
        fn consume(&mut self, token: &str) -> bool {
            if self.tokens.get(self.next) == Some(&token) {
                self.next += 1;
                true
            } else {
                false
            }
        }
        fn or(&mut self) -> Option<bool> {
            let mut result = self.and()?;
            while self.consume("OR") {
                let rhs = self.and()?;
                result |= rhs;
            }
            Some(result)
        }
        fn and(&mut self) -> Option<bool> {
            let mut result = self.atom()?;
            while self.consume("AND") {
                let rhs = self.atom()?;
                result &= rhs;
            }
            Some(result)
        }
        fn atom(&mut self) -> Option<bool> {
            if self.consume("(") {
                let value = self.or()?;
                return self.consume(")").then_some(value);
            }
            let license = *self.tokens.get(self.next)?;
            if matches!(license, "(" | ")" | "AND" | "OR" | "WITH") {
                return None;
            }
            if !license
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
            {
                return None;
            }
            self.next += 1;
            let mut identifier = license.to_owned();
            if self.consume("WITH") {
                let exception = *self.tokens.get(self.next)?;
                if matches!(exception, "(" | ")" | "AND" | "OR" | "WITH")
                    || !exception
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
                {
                    return None;
                }
                identifier.push_str(" WITH ");
                identifier.push_str(exception);
                self.next += 1;
            }
            Some(PERMITTED_LICENSES.contains(&identifier.as_str()))
        }
    }
    if tokens.is_empty() {
        return false;
    }
    let mut parser = Parser {
        tokens: &tokens,
        next: 0,
    };
    parser.or() == Some(true) && parser.next == tokens.len()
}

fn resolved_packages(meta: &Value) -> Result<Vec<&Value>> {
    let resolved: BTreeSet<&str> = meta["resolve"]["nodes"]
        .as_array()
        .context("resolved dependency nodes")?
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    let mut packages: Vec<&Value> = meta["packages"]
        .as_array()
        .context("packages")?
        .iter()
        .filter(|p| p["id"].as_str().is_some_and(|id| resolved.contains(id)))
        .collect();
    packages.sort_by(|a, b| {
        a["name"]
            .as_str()
            .cmp(&b["name"].as_str())
            .then_with(|| a["version"].as_str().cmp(&b["version"].as_str()))
    });
    Ok(packages)
}
fn license_report(meta: &Value, notices: Option<&Path>) -> Result<PathBuf> {
    let mut rows = Vec::new();
    let mut rejected = Vec::new();
    for package in resolved_packages(meta)? {
        let name = package["name"].as_str().context("package name")?;
        let version = package["version"].as_str().context("package version")?;
        let expression = package["license"].as_str().unwrap_or("");
        let permitted = permitted_expression(expression);
        let archive_hash = source_archive(package)
            .map(|path| sha256_file(&path))
            .transpose()?;
        let mut models = Vec::new();
        if name == "nnnoiseless" {
            let root = Path::new(
                package["manifest_path"]
                    .as_str()
                    .context("denoiser manifest")?,
            )
            .parent()
            .context("denoiser source directory")?;
            models.push(json!({"path":"src/weights.rnn","sha256":sha256_file(&root.join("src/weights.rnn"))?,"declared_license":"BSD-3-Clause","notice":"Default denoising weights shipped with this nnnoiseless crate; retain its complete COPYING notice. No external model is loaded."}));
        }
        rows.push(json!({"name":name,"version":version,"source":package["source"],"repository":package["repository"],"source_archive_sha256":archive_hash,"declared_spdx":expression,"normalized_expression":expression.replace('/', " OR "),"accepted_by_allowlist":permitted,"embedded_models":models}));
        if !permitted {
            rejected.push(format!(
                "{name} {version}: {}",
                if expression.is_empty() {
                    "no declared SPDX license"
                } else {
                    expression
                }
            ));
        }
        if let Some(destination) = notices.filter(|_| package["source"].is_string()) {
            copy_package_notices(package, &destination.join(format!("{name}-{version}")))?;
            // Ship the complete, unmodified covered source alongside the binary;
            // keep MPL obligations and notices separate from our own dual license.
            if expression.contains("MPL-2.0") {
                let source = Path::new(
                    package["manifest_path"]
                        .as_str()
                        .context("MPL source manifest")?,
                )
                .parent()
                .context("MPL source directory")?;
                let target = destination
                    .parent()
                    .context("package root")?
                    .join("THIRD_PARTY_SOURCES")
                    .join(format!("{name}-{version}"));
                copy_directory(source, &target)?;
            }
        }
    }
    let directory = PathBuf::from(
        meta["target_directory"]
            .as_str()
            .context("target directory")?,
    )
    .join("xtask");
    fs::create_dir_all(&directory)?;
    let path = directory.join("THIRD_PARTY_LICENSES.json");
    let report = json!({
        "report_version":1,
        "target":host_triple()?,
        "scope":"Resolved Cargo packages, including build/dev dependencies, declared SPDX metadata and cached source archive hashes. The default nnnoiseless denoising weights are fingerprinted separately. No legal, source-code, embedded-asset or security audit is implied. System libraries and external/future model weights are outside this report.",
        "permitted_licenses":PERMITTED_LICENSES,
        "packages":rows
    });
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "Declared SPDX report: {} ({} packages)",
        path.display(),
        report["packages"].as_array().map_or(0, Vec::len)
    );
    ensure!(
        rejected.is_empty(),
        "licenses outside project policy require review:\n{}",
        rejected.join("\n")
    );
    println!("Declared SPDX allowlist passed; this is not a security or legal audit.");
    Ok(path)
}
fn sha256_file(path: &Path) -> Result<String> {
    let result = if cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; $stream = [System.IO.File]::OpenRead($env:VOICE_APP_HASH_PATH); try { $algorithm = [System.Security.Cryptography.SHA256]::Create(); try { [System.BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() } finally { $algorithm.Dispose() } } finally { $stream.Dispose() }"])
            .env("VOICE_APP_HASH_PATH", path)
            .output()?
    } else {
        Command::new("sha256sum").arg(path).output()?
    };
    ensure!(
        result.status.success(),
        "cannot hash {}: {}",
        path.display(),
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout)?;
    let hash = if cfg!(windows) {
        output.trim()
    } else {
        output.split_whitespace().next().context("SHA-256 output")?
    };
    ensure!(
        hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid SHA-256 output"
    );
    Ok(hash.to_owned())
}
fn source_archive(package: &Value) -> Option<PathBuf> {
    if !package["source"].as_str()?.starts_with("registry+") {
        return None;
    }
    let source = Path::new(package["manifest_path"].as_str()?).parent()?;
    let index = source.parent()?;
    let registry = index.parent()?.parent()?;
    let archive = registry
        .join("cache")
        .join(index.file_name()?)
        .join(format!(
            "{}-{}.crate",
            package["name"].as_str()?,
            package["version"].as_str()?
        ));
    archive.is_file().then_some(archive)
}
fn copy_package_notices(package: &Value, destination: &Path) -> Result<()> {
    let manifest = Path::new(package["manifest_path"].as_str().context("manifest path")?);
    let root = manifest.parent().context("crate directory")?;
    let mut files = Vec::new();
    collect_notice_files(root, root, 0, &mut files)?;
    if let Some(path) = package["license_file"].as_str() {
        let path = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            root.join(path)
        };
        if path.is_file() && path.starts_with(root) && !files.contains(&path) {
            files.push(path);
        }
    }
    fs::create_dir_all(destination)?;
    for path in files {
        let relative = path.strip_prefix(root)?;
        let target = destination.join(relative);
        fs::create_dir_all(target.parent().context("notice target parent")?)?;
        fs::copy(path, target)?;
    }
    fs::write(
        destination.join("DECLARED-LICENSE.txt"),
        format!(
            "{} {}\nDeclared SPDX: {}\nSource: {}\n",
            package["name"].as_str().unwrap_or(""),
            package["version"].as_str().unwrap_or(""),
            package["license"].as_str().unwrap_or("UNKNOWN"),
            package["source"].as_str().unwrap_or("")
        ),
    )?;
    Ok(())
}
fn collect_notice_files(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > 5 {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
        if kind.is_dir() && !matches!(name.as_str(), ".GIT" | "TARGET" | "NODE_MODULES") {
            collect_notice_files(root, &entry.path(), depth + 1, files)?;
        } else if kind.is_file() && is_notice_name(&name) {
            let path = entry.path();
            ensure!(
                path.starts_with(root),
                "notice path escapes dependency directory"
            );
            files.push(path);
        }
    }
    Ok(())
}
fn is_notice_name(name: &str) -> bool {
    name.starts_with("LICENSE")
        || name.starts_with("LICENCE")
        || name.starts_with("COPYING")
        || name.starts_with("NOTICE")
        || matches!(name, "OFL.TXT" | "UFL.TXT" | "UNLICENSE" | "COPYRIGHT")
}

fn package(root: &Path) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux") || cfg!(windows),
        "package currently supports native Linux and Windows MSVC builds"
    );
    ensure!(
        std::env::var_os("CARGO_BUILD_TARGET").is_none(),
        "package requires a native build; unset CARGO_BUILD_TARGET"
    );
    let host = host_triple()?;
    ensure!(
        !cfg!(windows) || host == "x86_64-pc-windows-msvc",
        "Windows packaging requires the native x86_64-pc-windows-msvc toolchain"
    );
    run(
        "cargo",
        &[
            "build",
            "--locked",
            "--release",
            "-p",
            "voice-app",
            "-p",
            "voice-cli",
        ],
    )?;
    let meta = metadata(false)?;
    let target = PathBuf::from(
        meta["target_directory"]
            .as_str()
            .context("target directory")?,
    );
    let version = meta["packages"]
        .as_array()
        .context("packages")?
        .iter()
        .find(|p| p["name"] == "voice-app")
        .and_then(|p| p["version"].as_str())
        .context("app version")?;
    let stem = format!("voice-app-{version}-{host}");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let staging_parent = target.join("xtask-package").join(stamp.to_string());
    let staging = staging_parent.join(&stem);
    fs::create_dir_all(&staging)?;
    for executable in ["voice-app", "voice-cli"] {
        let executable = format!("{executable}{}", std::env::consts::EXE_SUFFIX);
        fs::copy(
            target.join("release").join(&executable),
            staging.join(&executable),
        )?;
    }
    for name in ["README.md", "LICENSE-MIT", "LICENSE-APACHE"] {
        fs::copy(root.join(name), staging.join(name))
            .with_context(|| format!("package requires {name}"))?;
    }
    if cfg!(windows) {
        // Explicit package allowlist: the complete local DEVLOG can contain
        // private presets. Windows releases include no local history or assets.
        let documents = staging.join("docs");
        fs::create_dir_all(documents.join("images"))?;
        for name in [
            "ARCHITECTURE.md",
            "CONTROL.md",
            "FEATURES.md",
            "RELEASE-NOTES.md",
            "RELEASING.md",
            "ROADMAP.md",
            "VALIDATION.md",
            "VOICE-LAB.md",
            "WINDOWS.md",
            "images/desktop-capture.png",
            "images/lab.png",
            "images/voices.png",
        ] {
            fs::copy(root.join("docs").join(name), documents.join(name))?;
        }
        fs::copy(
            root.join("packaging/windows/README.md"),
            staging.join("RUNNING.txt"),
        )?;
        fs::create_dir_all(staging.join("scripts"))?;
        fs::copy(
            root.join("scripts/test-windows-offline.ps1"),
            staging.join("scripts/test-windows-offline.ps1"),
        )?;
    } else {
        copy_directory(&root.join("docs"), &staging.join("docs"))?;
        let validation = staging.join("scripts");
        fs::create_dir_all(validation.join("fixtures"))?;
        for script in [
            "validation_common.py",
            "test-engine.py",
            "validate-latency.py",
            "validate-stability.py",
            "validate-workflow.py",
            "validate-obs.py",
            "run-soak-batch.py",
        ] {
            fs::copy(root.join("scripts").join(script), validation.join(script))?;
        }
        fs::copy(
            root.join("scripts/fixtures/pipewire-validation.conf"),
            validation.join("fixtures/pipewire-validation.conf"),
        )?;
    }
    let report = license_report(&meta, Some(&staging.join("THIRD_PARTY_LICENSES")))?;
    fs::copy(report, staging.join("THIRD_PARTY_LICENSES.json"))?;
    fs::write(
        staging.join("SOURCE-NOTICE.txt"),
        "This package includes unmodified Symphonia decoder source under MPL-2.0 in THIRD_PARTY_SOURCES/. Those source files remain under their original licenses and notices, included in each source directory and THIRD_PARTY_LICENSES/. The project's MIT OR Apache-2.0 grant applies to our original code; it does not replace third-party code or font licenses. Fonts include OFL-1.1 and Ubuntu-font-1.0 notices. Declared package licenses are listed in THIRD_PARTY_LICENSES.json.\n",
    )?;
    if !cfg!(windows) {
        fs::write(
            staging.join("RUNNING.txt"),
            "Linux native binaries. This archive is not a self-contained AppImage. Install PipeWire, libpipewire-0.3 and the display/windowing runtime required by your distribution. Run ./voice-app or ./voice-cli --help. Verify the outer archive with sha256sum -c SHA256SUMS.txt. No drivers, services, default microphone changes, or system installs are performed by unpacking this archive.\n",
        )?;
    }
    let output_directory = root.join("dist");
    fs::create_dir_all(&output_directory)?;
    let archive = output_directory.join(format!(
        "{stem}.{}",
        if cfg!(windows) { "zip" } else { "tar.gz" }
    ));
    let status = if cfg!(windows) {
        ensure!(
            !archive.exists(),
            "refusing to overwrite {}",
            archive.display()
        );
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::CreateFromDirectory($env:VOICE_APP_ZIP_SOURCE, $env:VOICE_APP_ZIP_TARGET, [System.IO.Compression.CompressionLevel]::Optimal, $true)"])
            .env("VOICE_APP_ZIP_SOURCE", &staging)
            .env("VOICE_APP_ZIP_TARGET", &archive)
            .status()?
    } else {
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging_parent)
            .arg(&stem)
            .status()?
    };
    ensure!(status.success(), "archive packaging failed");
    let file_name = archive
        .file_name()
        .context("archive file name")?
        .to_string_lossy();
    fs::write(
        output_directory.join("SHA256SUMS.txt"),
        format!("{}  {file_name}\n", sha256_file(&archive)?),
    )?;
    println!(
        "Package: {}\nChecksum: {}",
        archive.display(),
        output_directory.join("SHA256SUMS.txt").display()
    );
    Ok(())
}
fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        source.is_dir(),
        "required package directory missing: {}",
        source.display()
    );
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            bail!(
                "unsupported symlink/special file in package documents: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spdx_expressions_respect_alternatives_and_obligations() {
        assert!(permitted_expression("MIT OR GPL-3.0-only"));
        assert!(permitted_expression("MIT/Apache-2.0"));
        assert!(permitted_expression("(MIT OR Apache-2.0) AND Unicode-3.0"));
        assert!(permitted_expression("Apache-2.0 WITH LLVM-exception"));
        assert!(!permitted_expression("MIT AND GPL-3.0-only"));
        assert!(!permitted_expression("MIT WITH Unknown-exception"));
        assert!(!permitted_expression("MIT WITH OR OR MIT"));
        assert!(!permitted_expression("MIT OR"));
        assert!(!permitted_expression("(MIT"));
        assert!(!permitted_expression("MIT)"));
        assert!(!permitted_expression("MIT garbage"));
        assert!(!permitted_expression(""));
    }
    #[test]
    fn architecture_rejects_ui_in_core_and_engine_in_dsp() -> Result<()> {
        let meta = json!({"workspace_members":["core","dsp"],"packages":[
            {"id":"core","name":"voice-core","dependencies":[{"name":"serde"},{"name":"eframe"}]},
            {"id":"dsp","name":"voice-dsp","dependencies":[{"name":"voice-core"},{"name":"voice-engine"}]}
        ]});
        let errors = boundary_errors(&meta)?;
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.contains("voice-core -> eframe")));
        assert!(
            errors
                .iter()
                .any(|e| e.contains("voice-dsp -> voice-engine"))
        );
        Ok(())
    }
    #[test]
    fn dependency_notices_recognize_embedded_font_licenses() {
        for name in [
            "LICENSE",
            "LICENSE-MIT",
            "COPYING.LESSER",
            "NOTICE.txt",
            "OFL.txt",
            "UFL.txt",
        ] {
            assert!(is_notice_name(&name.to_ascii_uppercase()));
        }
        assert!(!is_notice_name("main.rs"));
    }
}
