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
ring modulation → dual modulated chorus taps → fractional feedback echo → four damped parallel reverb combs → ±0.98 peak
ceiling. Control values approach their targets with a 10 ms time constant. Gate opening is
3 ms and closing is 25 ms; the level detector releases in 80 ms. These time constants are
not added transport buffers.

With pitch, formants and denoise off, the graph adds no whole-stream buffering. At 48 kHz,
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
