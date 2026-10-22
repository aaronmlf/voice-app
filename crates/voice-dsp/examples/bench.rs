//! CPU-only benchmark; intentionally independent of audio hardware and routing.
use std::{hint::black_box, time::Instant};
use voice_core::{DspSettings, VoiceBlend, VoiceLayer};
use voice_dsp::Processor;

fn main() {
    let blend = std::env::args().any(|arg| arg == "--blend");
    let advanced = blend || std::env::args().any(|arg| arg == "--advanced");
    const BLOCK: usize = 256;
    const ITERATIONS: usize = 10_000;
    let template: [f32; BLOCK] =
        std::array::from_fn(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.25);
    let mut settings = DspSettings {
        effect_order: Default::default(),
        gain_db: 6.0,
        gate_db: -50.0,
        highpass_hz: 100.0,
        lowpass_hz: 9_000.0,
        compressor: true,
        drive: 0.3,
        robot_mix: 0.5,
        robot_hz: 75.0,
        echo_mix: 0.4,
        echo_delay_ms: 180.0,
        echo_feedback: 0.4,
        reverb_mix: 0.3,
        pitch_semitones: if advanced { 7.0 } else { 0.0 },
        formant_semitones: if advanced { -3.0 } else { 0.0 },
        chorus_mix: if advanced { 0.5 } else { 0.0 },
        denoise: advanced,
        ..Default::default()
    };
    if blend {
        settings.blend = Some(VoiceBlend {
            secondary: VoiceLayer::from(DspSettings {
                compressor: true,
                drive: 0.3,
                robot_mix: 0.5,
                robot_hz: 84.0,
                echo_mix: 0.1,
                echo_delay_ms: 15.0,
                reverb_mix: 0.05,
                chorus_mix: 0.2,
                ..Default::default()
            }),
            primary_mix: 0.7,
            secondary_mix: 0.3,
        });
    }
    let mut processor = Processor::new(48_000);
    let mut block = template;
    for _ in 0..1_000 {
        block.copy_from_slice(&template);
        processor.process(&mut block, &settings);
        black_box(&block);
    }
    let mut timings = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        block.copy_from_slice(&template);
        let start = Instant::now();
        processor.process(black_box(&mut block), black_box(&settings));
        timings.push(start.elapsed().as_nanos() as u64);
        black_box(&block);
    }
    let total = timings.iter().sum::<u64>();
    timings.sort_unstable();
    let percentile =
        |fraction: f64| timings[((ITERATIONS - 1) as f64 * fraction) as usize] as f64 / 1_000.0;
    let period_us = BLOCK as f64 / 48_000.0 * 1_000_000.0;
    println!(
        "{{\"benchmark\":\"dsp_cpu_only\",\"advanced\":{advanced},\"blend\":{blend},\"sample_rate\":48000,\"block_frames\":{BLOCK},\"iterations\":{ITERATIONS},\"period_us\":{period_us:.3},\"p50_us\":{:.3},\"p95_us\":{:.3},\"p99_us\":{:.3},\"max_us\":{:.3},\"mean_cpu_ratio\":{:.6},\"stream_lookahead_frames\":{}}}",
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        percentile(1.0),
        total as f64 / ITERATIONS as f64 / 1_000.0 / period_us,
        processor.latency_frames(&settings)
    );
}
