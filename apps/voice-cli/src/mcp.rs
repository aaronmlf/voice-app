//! MCP transport only. Product actions use the same command service as every other client.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::Path;
use voice_engine::{client::send, protocol::Command};

const MAX_FRAME: usize = 1_048_576;
const VERSION: &str = "2024-11-05";

enum Frame {
    End,
    Data(Vec<u8>),
    Invalid(&'static str),
}

fn read_frame(reader: &mut impl BufRead) -> Result<Frame> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if oversized {
                Frame::Invalid("frame exceeds 1 MiB")
            } else if line.is_empty() {
                Frame::End
            } else {
                Frame::Invalid("frame must end with newline")
            });
        }
        let end = available.iter().position(|&b| b == b'\n');
        let count = end.map_or(available.len(), |i| i + 1);
        if line.len() + count > MAX_FRAME {
            oversized = true;
        }
        if !oversized {
            line.extend_from_slice(&available[..count]);
        }
        reader.consume(count);
        if end.is_some() {
            return Ok(if oversized {
                Frame::Invalid("frame exceeds 1 MiB")
            } else {
                Frame::Data(line)
            });
        }
    }
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.into()}})
}

fn tools() -> Value {
    json!({"tools":[
        {"name":"voice_status","description":"Inspect the local voice engine without starting audio capture.",
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"voice_command","description":"Forward one engine Command. Use an op field: status, devices, start, stop, start_desktop_capture, stop_desktop_capture, configure, set_controls, presets, set_preset_original, select_preset, save_preset, import_preset, export_preset, preset_image, set_preset_image, clear_preset_image, boards, create_board, clips, import_clip, trim_clip, remove_clip, play, stop_sounds, start_recording, stop_recording, save_replay, shutdown. remove_clip takes id, removes only that catalog entry, stops its playback/preview and keeps all audio files. start_desktop_capture requires output_device and enables stereo recording/replay of that output only. start_recording/save_replay accept source: processed_voice (default) or desktop. preset_image reads the managed thumbnail path for id; set_preset_image takes id/path (PNG/JPEG/WebP); clear_preset_image takes id. Only custom presets accept images, including locally curated originals. set_preset_original takes id/original and changes only local collection membership; it does not publish content and is not transferred by export/import. configure.settings supports vocoder_mix (0..1), vocoder_carrier_hz (55..440), vocoder_unvoiced (0..1), vocoder_release_ms (10..200), granular_mode (stylized or natural), granular_mix (0..1, default 0), granular_pitch_semitones (-12..12; natural limited to -4..4), granular_grain_ms (12..80; natural limited to 40), granular_scatter (0..1, stylized only), granular_feedback (0..0.65, stylized only), granular_clarity (0..1), effect_mix (13 intensities from 0 to 1, indexed by EffectKind) and blend (secondary snapshot, primary_mix, secondary_mix); blends have exactly two chains. Natural granular mode is experimental; speech quality requires listening tests. Paths refer to the local engine host. JSON is identical to voice-cli ctl.",
            "inputSchema":{"type":"object","properties":{"command":{"type":"object","properties":{"op":{"type":"string"}},"required":["op"],"additionalProperties":true}},"required":["command"],"additionalProperties":false}}
    ]})
}

fn call(params: &Value, socket: &Path) -> Value {
    let result = (|| -> Result<Value> {
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let object = arguments
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("arguments must be an object"))?;
        let command = match params.get("name").and_then(Value::as_str) {
            Some("voice_status") => {
                if !object.is_empty() {
                    bail!("voice_status takes no arguments");
                }
                Command::Status
            }
            Some("voice_command") => {
                if object.len() != 1 {
                    bail!("voice_command requires only a command object");
                }
                serde_json::from_value(
                    arguments
                        .get("command")
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("command is required"))?,
                )?
            }
            _ => bail!("unknown tool"),
        };
        send(socket, command)
    })();
    match result {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}],"isError":false}),
        Err(error) => {
            json!({"content":[{"type":"text","text":format!("{error:#}")}],"isError":true})
        }
    }
}

fn handle(request: Value, initialized: &mut bool, socket: &Path) -> Option<Value> {
    if !request.is_object() {
        return Some(error(Value::Null, -32600, "request must be an object"));
    }
    let id = request.get("id")?.clone(); // Notifications never produce a response.
    if !id.is_string() && !id.is_number() && !id.is_null() {
        return Some(error(Value::Null, -32600, "invalid request id"));
    }
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error(id, -32600, "jsonrpc must be 2.0"));
    }
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Some(error(id, -32600, "method is required"));
    };
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            if params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .is_none()
            {
                return Some(error(id, -32602, "protocolVersion is required"));
            }
            *initialized = true;
            json!({"protocolVersion":VERSION,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"voice-cli","version":env!("CARGO_PKG_VERSION")},"instructions":"Tools connect to the same local engine as the GUI and CLI. The service must already be running. Desktop capture requires an explicit start_desktop_capture with output_device and does not feed the virtual microphone. Neural voice conversion is not implemented."})
        }
        "ping" => json!({}),
        _ if !*initialized => return Some(error(id, -32002, "initialize first")),
        "tools/list" => tools(),
        "tools/call" => call(&params, socket),
        _ => return Some(error(id, -32601, "method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

pub fn serve(mut reader: impl BufRead, mut writer: impl Write, socket: &Path) -> Result<()> {
    let mut initialized = false;
    loop {
        let response = match read_frame(&mut reader)? {
            Frame::End => break,
            Frame::Invalid(message) => Some(error(Value::Null, -32700, message)),
            Frame::Data(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(request) => handle(request, &mut initialized, socket),
                Err(_) => Some(error(Value::Null, -32700, "invalid JSON")),
            },
        };
        if let Some(response) = response {
            let mut bytes = serde_json::to_vec(&response)?;
            if bytes.len() + 1 > MAX_FRAME {
                bytes = serde_json::to_vec(&error(
                    response.get("id").cloned().unwrap_or(Value::Null),
                    -32000,
                    "response exceeds 1 MiB",
                ))?;
                if bytes.len() + 1 > MAX_FRAME {
                    bytes =
                        serde_json::to_vec(&error(Value::Null, -32000, "response exceeds 1 MiB"))?;
                }
            }
            writer.write_all(&bytes)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negotiates_lists_tools_and_ignores_notifications_without_service() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n"
        );
        let mut output = Vec::new();
        serve(
            input.as_bytes(),
            &mut output,
            Path::new("/nonexistent/voice-cli-test.sock"),
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        let responses: Vec<Value> = text
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["result"]["protocolVersion"], VERSION);
        assert_eq!(responses[1]["result"]["tools"][0]["name"], "voice_status");
        assert_eq!(responses[1]["result"]["tools"][1]["name"], "voice_command");
    }

    #[test]
    fn framing_is_bounded_and_recovers_after_bad_json() {
        let mut data = vec![b'x'; MAX_FRAME + 10];
        data.extend_from_slice(b"\ninvalid\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n");
        let mut output = Vec::new();
        serve(data.as_slice(), &mut output, Path::new("/nonexistent")).unwrap();
        let text = String::from_utf8(output).unwrap();
        let values: Vec<Value> = text
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0]["error"]["code"], -32700);
        assert_eq!(values[2]["id"], 3);
        assert_eq!(values[2]["result"], json!({}));
    }
}
