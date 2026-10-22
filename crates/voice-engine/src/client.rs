use crate::platform::{DeadlineReader, DeadlineWriter, LocalConnection};
use crate::protocol::{Command, Request, Response};
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::Duration;

pub const MAX_FRAME_BYTES: u64 = 1_048_576;

pub fn send(socket: &Path, command: Command) -> Result<serde_json::Value> {
    let response = send_request(socket, Request { id: 1, command })?;
    if !response.ok {
        bail!(
            "{}",
            response.error.unwrap_or_else(|| "command failed".into())
        );
    }
    Ok(response.result.unwrap_or(serde_json::Value::Null))
}

pub fn send_request(socket: &Path, request: Request) -> Result<Response> {
    let mut stream = LocalConnection::connect(socket).with_context(|| {
        format!(
            "cannot connect to {} (start voice-cli serve or voice-app)",
            socket.display()
        )
    })?;
    write_local_frame(&mut stream, &request)?;
    let line = read_local_frame(&mut stream, Duration::from_secs(20))?;
    let response: Response = serde_json::from_slice(&line)?;
    if response.id != request.id {
        bail!("response id does not match request");
    }
    Ok(response)
}

pub(crate) fn read_local_frame(stream: &mut LocalConnection, timeout: Duration) -> Result<Vec<u8>> {
    read_frame(BufReader::new(DeadlineReader::new(stream, timeout)))
}

pub(crate) fn write_local_frame(
    stream: &mut LocalConnection,
    value: &impl serde::Serialize,
) -> Result<()> {
    let mut writer = DeadlineWriter::new(stream, Duration::from_secs(3));
    serde_json::to_writer(&mut writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

pub(crate) fn read_frame(mut reader: impl BufRead) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            bail!("connection closed before newline");
        }
        let end = available.iter().position(|&b| b == b'\n');
        let count = end.map_or(available.len(), |i| i + 1);
        if line.len() + count > MAX_FRAME_BYTES as usize {
            bail!("request frame exceeds 1 MiB");
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_requires_delimiter_and_is_bounded() {
        assert!(read_frame(&b"{}"[..]).is_err());
        assert_eq!(read_frame(&b"{}\nnext\n"[..]).unwrap(), b"{}\n");
        let large = vec![b'x'; MAX_FRAME_BYTES as usize + 1];
        assert!(read_frame(&large[..]).is_err());
    }

    #[test]
    fn trickling_bytes_do_not_restart_the_frame_deadline() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Instant;
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "voice-ipc-deadline-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let endpoint = root.join("control.sock");
        let mut listener = crate::platform::LocalListener::bind(&endpoint)?;
        let server = std::thread::spawn(move || -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                if let Some(stream) = listener.try_accept()? {
                    break stream;
                }
                anyhow::ensure!(Instant::now() < deadline, "test IPC accept timed out");
                std::thread::sleep(Duration::from_millis(1));
            };
            stream.set_write_timeout(Some(Duration::from_millis(200)))?;
            // Each byte arrives before a per-read timeout would expire. A full
            // stream would take 400 ms, exceeding the one fixed 60 ms deadline.
            for _ in 0..20 {
                if stream.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        });
        let mut stream = LocalConnection::connect(&endpoint)?;
        let started = Instant::now();
        let result = read_local_frame(&mut stream, Duration::from_millis(60));
        let elapsed = started.elapsed();
        drop(stream);
        server.join().expect("test IPC writer panicked")?;
        if root.exists() {
            std::fs::remove_dir_all(root)?;
        }
        let error = result.expect_err("a dribbled incomplete frame must time out");
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::TimedOut)
        );
        assert!(
            elapsed < Duration::from_millis(350),
            "frame deadline was restarted: {elapsed:?}"
        );
        Ok(())
    }
}
