# voice-library

Offline operations for the engine's single background worker. No library method is
intended for an audio callback: decoding, SQLite, allocation and PCM destruction
remain on the worker. The callback consumes a bounded sample queue populated by
the engine.

`Library::open(root)` manages `root/library.sqlite3` and `root/clips/`. It seeds
six DSP presets and the `default` soundboard. Presets and boards/clips are returned
as serializable structs. `save_preset`, `create_board`, `import_clip`, `load_clip`,
`save_clip`, and `trim_clip` support the offline workflow. `load_clip` returns
`Arc<Vec<f32>>`; every clip is mono 48 kHz. Trimming creates a separate clip.

WAV, MP3, Ogg Vorbis and FLAC import use Symphonia and an offline windowed-sinc
resampler. Default source limits are 20 MiB and eight minutes; callers can provide
`ImportLimits` to `import_clip_with_limits`. Managed normalized WAV files may be
larger than their source. The library validates names, DSP parameters and preset
JSON (64 KiB, version 1), and errors on corrupt or newer databases. Reopening
restores missing built-in presets; user presets persist. JSON imports replace an
existing preset of the same ID only after validation.

`Mixer::play(id, pcm, gain, looped, overlap)` supports up to 16 simultaneous voices.
`overlap=false` restarts that ID, while `true` adds another instance. `process`
replaces its output buffer, sums/clamps samples and releases finished clips.
`stop` stops an ID and `stop_all` is the panic action. Non-finite samples become
silence. Gain is linear, 0–4. This is a worker mixer, not a callback mixer.

`ReplayBuffer::default()` retains the last 30 seconds at 48 kHz. `push` captures
worker-owned samples, `snapshot` returns chronological captured audio and `clear`
resets it. The engine chooses the captured bus and publishes recordings to the
library through `save_clip` or to a file through `write_wav`.

## Test fixtures

`tests/fixtures/tone.{mp3,ogg,flac}` contain a synthetic 220 Hz sine wave generated
for this project, not third-party recordings, and carry the project's
MIT OR Apache-2.0 license. Each was generated with FFmpeg from:

```text
-f lavfi -i sine=frequency=220:sample_rate=44100:duration=0.1 -ac 2
```

The codecs were `libmp3lame`, `libvorbis`, and `flac`. Tests embed these small
fixtures; FFmpeg is not a build or runtime requirement.
