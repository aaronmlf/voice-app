//! Prepared elastic reader between independently clocked native-rate streams.
//! The queue still has exactly one consumer. No waiting, allocation or device calls
//! occur here; every loop is bounded by the output slice or FIFO capacity.
use voice_core::SampleQueue;

const CHUNK: usize = 1024;
const SOURCE: usize = CHUNK + 4;
const MAX_RATIO_ERROR: f64 = 0.001; // +/-1000 ppm, never an unrestricted rate change.

#[derive(Default, Debug)]
pub(crate) struct BridgeRead {
    pub(crate) missing_frames: usize,
    pub(crate) priming_frames: usize,
    pub(crate) trimmed_frames: usize,
    pub(crate) non_finite_samples: u64,
}

pub(crate) struct ClockBridge {
    source: Box<[f32]>,
    buffered: usize,
    phase: f64,
    ratio: f64,
    filtered_error: f64,
    rate: u32,
    capacity: usize,
    primed: bool,
    immediate: bool,
}

impl ClockBridge {
    pub(crate) fn new(capacity: usize, rate: u32) -> Self {
        assert!(capacity >= 2 && rate > 0);
        Self {
            source: vec![0.; SOURCE].into_boxed_slice(),
            buffered: 0,
            phase: 0.,
            ratio: 1.,
            filtered_error: 0.,
            rate,
            capacity,
            primed: false,
            immediate: false,
        }
    }

    /// Tiny sample-by-sample DSP tests bypass the transport's latency policy.
    #[cfg(test)]
    pub(crate) fn immediate(capacity: usize, rate: u32) -> Self {
        let mut bridge = Self::new(capacity, rate);
        bridge.primed = true;
        bridge.immediate = true;
        bridge
    }

    pub(crate) fn read(
        &mut self,
        queue: &SampleQueue,
        output: &mut [f32],
        producer_batch: usize,
    ) -> BridgeRead {
        output.fill(0.);
        let mut result = BridgeRead::default();
        if output.is_empty() {
            return result;
        }
        // Such a request cannot leave any jitter reserve in this FIFO. Report a
        // real failed read, rather than remaining silently in priming forever.
        if output.len() >= self.capacity {
            self.primed = false;
            result.missing_frames = output.len();
            return result;
        }

        // 20 ms minimum reserve, increased for one observed producer burst or
        // consumer request. At most half the FIFO (85.3 ms at 48 kHz/8192).
        let reserve = if self.immediate {
            0
        } else {
            (self.rate as usize / 50)
                .max(producer_batch)
                .max(output.len())
                .min(self.capacity / 2)
        };
        // Priming must leave room for an entire producer burst, rather than
        // requiring a full FIFO and forcing its producer to drop a partial push.
        // Extreme profiles outside that budget fail visibly just like a read
        // shortage; they cannot spend forever emitting unreported prime silence.
        if !self.immediate
            && reserve
                .saturating_add(output.len())
                .saturating_add(producer_batch)
                > self.capacity
        {
            self.primed = false;
            result.missing_frames = output.len();
            return result;
        }
        let desired = reserve.saturating_add(output.len()).min(self.capacity);
        // A fixed 128 ms recovery budget, expanded for actual unequal batches.
        // A short callback must never turn an ordinary producer burst into loss.
        let high = (self.rate as usize)
            .saturating_mul(128)
            .saturating_div(1000)
            .max(desired.saturating_add(producer_batch))
            .min(self.capacity);
        let occupancy = queue.len().saturating_add(self.buffered);
        if occupancy > high {
            let mut remaining = occupancy.saturating_sub(desired);
            let cached = remaining.min(self.buffered);
            self.source.copy_within(cached..self.buffered, 0);
            self.buffered -= cached;
            remaining -= cached;
            result.trimmed_frames = cached;
            while remaining > 0 {
                let count = remaining.min(self.source.len());
                let consumed = queue.pop(&mut self.source[..count]);
                if consumed == 0 {
                    break;
                }
                remaining -= consumed;
                result.trimmed_frames += consumed;
            }
            self.phase = 0.;
            self.filtered_error = 0.;
            self.primed = false;
        }

        let occupancy = queue.len().saturating_add(self.buffered);
        if !self.primed {
            if !self.immediate && occupancy < desired {
                result.priming_frames = output.len();
                return result;
            }
            self.primed = true;
            self.filtered_error = 0.;
            self.ratio = 1.;
        }
        if !self.immediate {
            // Proportional occupancy correction with a 0.5 s low-pass rejects
            // callback-batch jitter. +/-250 ppm settles inside the reserve;
            // correction is always capped to +/-1000 ppm.
            let alpha = (output.len() as f64 / (self.rate as f64 * 0.5)).min(1.);
            let error = occupancy as f64 - desired as f64;
            self.filtered_error += alpha * (error - self.filtered_error);
            let correction = (self.filtered_error / reserve.max(1) as f64 * MAX_RATIO_ERROR)
                .clamp(-MAX_RATIO_ERROR, MAX_RATIO_ERROR);
            self.ratio = 1. + correction;
        }

        let output_frames = output.len();
        let mut written = 0;
        for destination in output.chunks_mut(CHUNK) {
            let last = self.phase + (destination.len() - 1) as f64 * self.ratio;
            let needed = last.ceil() as usize + 1;
            debug_assert!(needed <= self.source.len());
            let old = self.buffered;
            if needed > old {
                self.buffered += queue.pop(&mut self.source[old..needed]);
                for sample in &mut self.source[old..self.buffered] {
                    if !sample.is_finite() {
                        *sample = 0.;
                        result.non_finite_samples += 1;
                    }
                }
            }
            let mut produced = 0;
            for (index, sample) in destination.iter_mut().enumerate() {
                let position = self.phase + index as f64 * self.ratio;
                let whole = position.floor() as usize;
                let fraction = position - whole as f64;
                if whole >= self.buffered || (fraction > 0. && whole + 1 >= self.buffered) {
                    break;
                }
                let left = self.source[whole] as f64;
                *sample = if fraction == 0. {
                    left as f32
                } else {
                    let right = self.source[whole + 1] as f64;
                    (left * (1. - fraction) + right * fraction) as f32
                };
                produced += 1;
            }
            let position = self.phase + produced as f64 * self.ratio;
            let consumed = (position.floor() as usize).min(self.buffered);
            self.source.copy_within(consumed..self.buffered, 0);
            self.buffered -= consumed;
            self.phase = position - consumed as f64;
            written += produced;
            if produced < destination.len() {
                result.missing_frames = output_frames - written;
                self.primed = self.immediate;
                break;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn producer_burst_is_not_dropped_by_a_short_consumer() {
        let queue = SampleQueue::new(8192);
        let mut bridge = ClockBridge::new(8192, 48_000);
        queue.push(&[0.25; 1536]);
        let mut out = [1.; 240];
        let first = bridge.read(&queue, &mut out, 1536);
        assert_eq!(first.priming_frames, 240);
        assert_eq!(queue.len(), 1536);
        queue.push(&[0.25; 1536]);
        let ready = bridge.read(&queue, &mut out, 1536);
        assert_eq!(ready.trimmed_frames, 0);
        assert_eq!(ready.missing_frames, 0);
        assert!(out.iter().all(|sample| (*sample - 0.25).abs() < 1e-6));
    }

    #[test]
    fn startup_and_repriming_are_distinct_from_real_starvation() {
        let queue = SampleQueue::new(8192);
        let mut bridge = ClockBridge::new(8192, 48_000);
        let mut out = [1.; 480];
        let initial = bridge.read(&queue, &mut out, 480);
        assert_eq!(initial.priming_frames, 480);
        assert_eq!(initial.missing_frames, 0);
        queue.push(&[0.5; 1920]);
        assert_eq!(bridge.read(&queue, &mut out, 480).missing_frames, 0);
        let mut missing = 0;
        for _ in 0..6 {
            missing += bridge.read(&queue, &mut out, 480).missing_frames;
        }
        assert!(missing > 0);
        let recovery = bridge.read(&queue, &mut out, 480);
        assert_eq!(recovery.missing_frames, 0);
        assert_eq!(recovery.priming_frames, 480);
        assert!(out.iter().all(|sample| *sample == 0.));
    }

    #[test]
    fn fractional_crossings_keep_every_source_position_across_calls() {
        let queue = SampleQueue::new(8192);
        let mut bridge = ClockBridge::immediate(8192, 48_000);
        bridge.ratio = 1.00025;
        let input: Vec<_> = (0..4096).map(|i| i as f32).collect();
        queue.push(&input);
        let mut out = [0.; 137];
        let mut position = 0.;
        for _ in 0..20 {
            let read = bridge.read(&queue, &mut out, 480);
            assert_eq!(read.missing_frames, 0);
            for &sample in &out {
                assert!((sample as f64 - position).abs() < 0.001);
                position += 1.00025;
            }
        }
    }

    #[test]
    fn impossible_requests_fail_visibly_and_input_is_sanitized() {
        let queue = SampleQueue::new(8);
        let mut bridge = ClockBridge::new(8, 48_000);
        let mut out = [1.; 8];
        let read = bridge.read(&queue, &mut out, 480);
        assert_eq!(read.missing_frames, 8);
        assert_eq!(read.priming_frames, 0);
        assert!(out.iter().all(|sample| *sample == 0.));
        let queue = SampleQueue::new(8192);
        let mut bridge = ClockBridge::immediate(8192, 48_000);
        bridge.ratio = 0.99975;
        queue.push(&[f32::NAN, f32::INFINITY, f32::MAX, -f32::MAX]);
        let mut out = [0.; 4];
        let read = bridge.read(&queue, &mut out, 480);
        assert_eq!(read.non_finite_samples, 2);
        assert!(out.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn requests_that_cannot_leave_burst_headroom_fail_without_eating_the_fifo() {
        let queue = SampleQueue::new(8192);
        let mut bridge = ClockBridge::new(8192, 48_000);
        queue.push(&[0.25; 7680]);
        let mut output = [1.; 6000];
        let read = bridge.read(&queue, &mut output, 480);
        assert_eq!(read.missing_frames, output.len());
        assert_eq!(read.priming_frames, 0);
        assert_eq!(read.trimmed_frames, 0);
        assert_eq!(queue.len(), 7680);
        assert!(output.iter().all(|sample| *sample == 0.));
    }

    #[test]
    fn thirty_minutes_of_signed_250ppm_drift_and_batched_jitter_stay_bounded() {
        // The real FIFO/resampler/controller runs for 30 minutes of simulated
        // time at a 1 kHz clock. Durations and ppm are unchanged; scaling avoids
        // processing 172 million mostly-identical 48 kHz samples in debug tests.
        for ppm in [-250.0_f64, 250.0] {
            let queue = SampleQueue::new(256);
            let mut bridge = ClockBridge::new(256, 1000);
            let input = [0.25; 32];
            let mut output = [0.; 10];
            let mut accumulated = 0.0_f64;
            let mut delayed = 0;
            let mut started = false;
            let mut corrected = false;
            for tick in 0..180_000 {
                accumulated += 10. * (1. + ppm / 1_000_000.);
                let produced = accumulated.floor() as usize;
                accumulated -= produced as f64;
                delayed += produced;
                // Deterministically coalesce two 10 ms producer periods.
                if tick % 97 != 0 {
                    assert_eq!(queue.push(&input[..delayed]), delayed);
                    delayed = 0;
                }
                let read = bridge.read(&queue, &mut output, 21);
                assert_eq!(read.trimmed_frames, 0, "ppm={ppm}, tick={tick}");
                assert_eq!(read.missing_frames, 0, "ppm={ppm}, tick={tick}");
                if started {
                    assert_eq!(read.priming_frames, 0, "ppm={ppm}, tick={tick}");
                } else if read.priming_frames == 0 {
                    started = true;
                }
                if started {
                    assert!(output.iter().all(|sample| (*sample - 0.25).abs() < 1e-6));
                }
                assert!((0.999..=1.001).contains(&bridge.ratio));
                corrected |= (bridge.ratio - 1.).abs() > 0.0001;
                assert!(queue.len() < 128);
            }
            assert!(started && corrected);
        }
    }
}
