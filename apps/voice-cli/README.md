# voice-cli

The CLI uses the same engine commands and local transport as the native interface: a Unix
socket on Linux and a user-restricted named pipe on Windows. JSON output
reports engine results. Control commands require a running `voice-cli serve` or `voice-app`.
`--data-dir` overrides storage and `--socket` overrides the socket independently; both are
global options. IPC paths are resolved relative to the CLI client's current directory;
on Windows the socket option is a logical path mapped to a local named pipe.

```
voice-cli serve                        # service only; microphone stays stopped
voice-cli doctor                       # probe the platform audio backend; no capture
voice-cli devices
voice-cli start --input NODE --output NODE
voice-cli voices
voice-cli select radio
voice-cli gain -3
voice-cli monitor on --gain 0.25
voice-cli mute off
voice-cli import ./sound.wav --board default --name "My sound"
voice-cli play CLIP_ID --preview
voice-cli play CLIP_ID --looped --overlap
voice-cli stop-sounds
voice-cli remove-clip CLIP_ID
voice-cli record ./recording.wav
voice-cli stop-recording
voice-cli replay ./last-30-seconds.wav
voice-cli desktop-start --output SINK_NODE_NAME
voice-cli record ./desktop.wav --source desktop
voice-cli stop-recording
voice-cli replay ./desktop-replay.wav --source desktop --seconds 10
voice-cli desktop-stop
voice-cli stop
voice-cli shutdown
```

`serve --start` explicitly starts capture. `start` creates the application's streams without
changing the system default microphone. Monitoring is off by default; preview plays locally.
`voices` lists implemented DSP presets, not neural voices.

`remove-clip` removes the soundboard entry persistently and stops only that clip's playback
and preview instances. Original imported files and normalized library WAVs are retained;
removal does not reclaim their disk space. Other entries and playing clips are preserved.
The shared GUI/CLI/MCP command is `{"op":"remove_clip","id":"CLIP_ID"}`.
The Windows preview has no virtual microphone driver; see `docs/WINDOWS.md` before testing calls.

`desktop-start --output` requires an explicit output from `devices`. Desktop capture works
independently of microphone capture and records that output in stereo at 48 kHz, including
any calls, monitoring or previews playing there. It feeds recording and its own 30-second
replay, with no live route to the virtual microphone. Desktop capture stays off on restart;
`desktop-stop` finalizes any desktop recording and keeps its captured replay available.
`record` and `replay` default to `--source processed-voice` for compatibility. Only one WAV
recording may run at a time. Long recordings stream to disk; replay memory is bounded.

The matching JSON commands are `{"op":"start_desktop_capture","output_device":"SINK_NODE_NAME"}`,
`{"op":"start_recording","path":"/absolute/desktop.wav","source":"desktop"}` and
`{"op":"save_replay","path":"/absolute/replay.wav","seconds":10,"source":"desktop"}`.
Use `{"op":"stop_desktop_capture"}` to stop that route. Omitting `source` uses `processed_voice`.

Offline commands need no running service and open no audio devices:

```
voice-cli tone ./test.wav --seconds 2 --hz 440
voice-cli process ./test.wav ./radio.wav --preset radio --gain-db -6
voice-cli bench --blocks 10000
voice-cli bench --advanced --blocks 10000
```

`tone` and `process` refuse to overwrite files. Process output has the decoded input's frame
count, compensating declared algorithm buffering but not extending echo/reverb tails. Imports use the library's byte/duration limits.
Bench reports only local DSP CPU time; use a release build for performance measurements.

`voice-cli ctl '{"op":"status"}'` forwards the canonical JSON `Command`; a full
`{"id":42,"command":{"op":"status"}}` Request preserves its ID. Engine validation handles
parameters consistently across clients.

## MCP

Launch `voice-cli --socket /absolute/path/control.sock mcp` as a stdio MCP server while the
engine is running. It supports protocol **2024-11-05**, newline-delimited JSON-RPC 2.0,
`initialize`, `ping`, `tools/list`, and `tools/call`. Notifications produce no response. Input
and output frames are limited to 1 MiB. Stdout contains protocol frames only.

Tools: `voice_status` with `{}`, and `voice_command` with
`{"command":{"op":"select_preset","id":"builtin.robot"}}`. Both forward to the same
engine registry; the MCP process neither owns a second session nor starts audio by itself.
The local engine has the same file authority as the user running it; MCP paths must be
absolute because they refer to that host. Neural conversion is not implemented.
