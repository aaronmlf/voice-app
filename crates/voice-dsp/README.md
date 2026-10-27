# voice-dsp

Prepared, mono, normalized `f32` PCM processing. `Processor::new(rate)` allocates the delay
buffers and algorithm state on a control thread. Processing reuses that storage without locks
or I/O. The caller must also construct and destroy processors off the audio callback. A test
moves the prepared processor to a fresh thread, enables every algorithm, disables/re-enables
them, and checks zero Rust allocator calls (including free/realloc). The C++ process path was
audited separately: its vectors remain within capacity allocated by configure().

Signal order: input sanitization → optional RNNoise → optional Signalsmith pitch/formants
→ soft gate and smoothed gain → first-order highpass/lowpass
→ optional 4:1 feed-forward compressor (−18 dBFS threshold) → normalized tanh drive → sine
ring modulation → optional filter-bank vocoder → optional granular processing → dual modulated chorus taps → fractional feedback echo → four damped parallel reverb combs → ±0.98 peak
ceiling. Control values approach their targets with a 10 ms time constant. Gate opening is
3 ms and closing is 25 ms; the level detector releases in 80 ms. These time constants are
not added transport buffers.

The traditional vocoder follows the Robot stage by default and can be reordered/mixed like
the other stages. Twenty matched constant-peak bandpass filters transfer rectified speech
envelopes onto a band-limited internal sawtooth carrier (55–440 Hz); a highpass speech path
preserves consonants. Envelope attack is 2 ms, with adjustable release (10–200 ms). Partial
wet mix retains the original articulation. This is a filter-bank effect, not a neural model
or a pitch tracker. It adds no audio-block/window/FIFO delay; filter response, envelope
settling, device queues and physical end-to-end delay remain separate considerations.
Its state is a fixed inline array under 4 KiB per chain, with no external dependencies or
model/audio buffer allocation. `vocoder_mix=0` is exact bypass and skips all vocoder filter
processing. Disabling it clears old vocoder history after the 10 ms control ramp reaches
zero, and the normal effect-opacity control also preserves a true bypass endpoint.

With pitch, formants, denoise and granular off, the graph adds no whole-stream buffering. At 48 kHz,
pitch/formants use a 4096-frame spectral window plus a 128-frame stream adapter: **4224 frames
(88 ms)**. RNNoise's 480-frame adapter and analysis window add **960 frames (20 ms)**.
Together they add **5184 frames (108 ms)**. `latency_frames(settings)` reports these algorithmic
buffers, not device/PipeWire/end-to-end latency. Enabling these stages restarts their buffered
streams, with initial silence. The 4096-frame pitch window was chosen after a 2048-frame
configuration showed a larger pitch error in the synthetic tone test.

Echo, chorus and reverb intentionally create delayed copies. The peak ceiling is a hard safety bound, not a transparent mastering limiter. The
filters are first order; the reverb is a small synthetic comb network. Echo-delay changes
produce a tape-like pitch glide. Delay/reverb buffers continue tracking dry input while their
wet mix is zero, so enabling them reveals the recent audio history.

Inputs and controls are sanitized defensively; non-finite input samples have silent output
positions. `reset()` clears history without allocation, but fills the prepared buffers, so call
it with the stream stopped. Processor construction bounds sample rates to 8–192 kHz
(0 means 48 kHz). The initial application's validated audio contract is 48 kHz.

Pitch/formants use `signalsmith-stretch =0.1.3`, a Rust wrapper around MIT-licensed native C++.
RNNoise uses the embedded BSD-3-Clause model from `nnnoiseless =0.5.1`, pinned deliberately:
0.5.2 moved FFT scratch into thread-local caches, which can allocate on the first callback
thread use. Version 0.5.1 owns prepared FFT plans/scratch, and passes the cross-thread allocator
test. No external model is downloaded or loaded. Denoise is enabled only at 48 kHz, the app's
validated audio format. The formant algorithm is an approximation and is not a guarantee of
natural-sounding voices. Neural voice conversion and adaptive echo cancellation are absent.

`cargo run --release -p voice-dsp --example bench` measures local processor CPU time for
256-sample blocks. It does not measure PipeWire scheduling, device latency or end-to-end
latency. Tests verify gain, gate, filtering, compression, generated drive harmonics, ring
modulation sidebands, decaying delay tails, chunk invariance, transitions, finite output,
octave transposition, formant-envelope movement, stationary-noise reduction and allocator
behavior. These signal tests do not establish perceptual parity with another voice product.
Add `--vocoder` to compare the same conventional/`--advanced`/`--blend` benchmark with the
vocoder enabled. The JSON reports fixed vocoder state bytes for both prepared chains;
that figure excludes the other algorithms, process RSS, allocator overhead and device buffers.


Granular processing uses four overlapping Hann-windowed time-domain grains, fractional
source cursors and a precomputed window table. Stylized mode supports ±12 semitones,
12–80 ms grains, deterministic backwards scatter, and feedback capped at 0.65. At zero
scatter its new grains receive bounded backwards-only correlation alignment (up to 13 ms)
to avoid phase cancellation and retain a coherent shifted voice. Nonzero scatter intentionally
fragments that timing. Natural
mode is experimental: it caps the effective pitch at ±4 semitones and grain length at
40 ms, ignores scatter/feedback, and aligns incoming grains with a bounded ±6.5 ms waveform
correlation search. It performs real resampling within each grain rather than calling a
delayed original a voice transformation. It does not preserve formants or guarantee a
natural result, especially on low fundamentals, rapid articulation and large changes.
The clarity control retains delayed high-frequency consonants and speech transients.

Stylized mode reports a nominal centre-alignment delay of half the rounded grain
length plus two samples: default 32 ms grains add 770 frames (16.04 ms) at 48 kHz;
12–80 ms gives 290–1922 frames (6.04–40.04 ms). Natural adds a 4 ms causal guard
for the wider alignment search: its default is 962 frames (20.04 ms), and its
12–40 ms grains give 482–1154 frames (10.04–24.04 ms). Waveform alignment can shift
grain timing by ±6.5 ms;
scatter deliberately spreads/repeats earlier audio. These figures describe the nominal
aligned timeline, not first nonzero output, every speech envelope or physical latency.
The clean dry path and outer effect-opacity path use the same prepared nominal delay,
independent of feedback history. Neutral pitch/scatter/feedback is sample-exact aligned
dry at all intensities; disabled granular is sample-exact zero-latency bypass.

Prepared history storage has fixed capacity for the maximum supported grain configuration.
Four grain records and correlation loops are bounded; construction performs all allocation.
Reset invalidates the history in constant time, preventing stale speech from escaping.
Mode, grain-delay and active-route changes fade down, reset, wait for the delayed stream,
and fade in, including a mode switch whose nominal latency stays the same. Both prepared
voices have their own state, and branch alignment accounts for granular plus the existing
pitch/denoise delays. `granular_state_bytes()` includes inline state, history, clean dry
alignment and the complete shared dual-voice alignment buffers; it excludes allocator
metadata, the other algorithms and process RSS. `--granular` and `--granular-natural`
benchmark both modes; either can be combined with `--blend` and `--advanced`.
`--granular-clear` measures Stylized with zero scatter/feedback, exercising the
bounded grain alignment used when preserving a coherent pitch.
