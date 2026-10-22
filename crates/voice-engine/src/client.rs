use crate::protocol::{Command, Request, Response};
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
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
    let mut stream = UnixStream::connect(socket).with_context(|| {
        format!(
            "cannot connect to {} (start voice-cli serve or voice-app)",
            socket.display()
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;
    let line = read_frame(BufReader::new(stream))?;
    let response: Response = serde_json::from_slice(&line)?;
    if response.id != request.id {
        bail!("response id does not match request");
    }
    Ok(response)
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
}
