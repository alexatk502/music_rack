//! Delay: mono echo with feedback and wet/dry mix. The 2-second ring buffer
//! lives on the heap (boxed at construction — the one allowed allocation,
//! during plan application between quanta, never in process()).
//!
//! Mono by design (poly inputs read channel 0): a 16-lane poly delay would
//! cost 16× buffer memory for marginal musical benefit in an MVP.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::delay as p;
use rack_dsp::{tanh_pade, undenorm, Smoothed};

const MAX_SECONDS: f32 = 2.0;
/// Sized for 48 kHz; clamped against the actual buffer at runtime so higher
/// sample rates just cap the maximum delay a little below 2 s.
const BUF_LEN: usize = (48_000.0 * MAX_SECONDS) as usize + 4;

pub struct Delay {
    buf: Box<[f32; BUF_LEN]>,
    write: usize,
    /// Heavily smoothed so dragging the time knob tape-warbles instead of
    /// crackling.
    time: Smoothed,
    feedback: Smoothed,
    mix: Smoothed,
}

impl Delay {
    pub fn new() -> Self {
        Self {
            buf: Box::new([0.0; BUF_LEN]),
            write: 0,
            time: Smoothed::new(0.4),
            feedback: Smoothed::new(0.4),
            mix: Smoothed::new(0.4),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::TIME => self.time.set_target(value.clamp(0.02, MAX_SECONDS)),
            p::FEEDBACK => self.feedback.set_target(value.clamp(0.0, 0.95)),
            p::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        // ~80 ms time constant for the time knob: slow enough to warble.
        let time_k = Smoothed::coeff(0.080, ctx.sample_rate);
        let len = BUF_LEN as f32;

        for i in 0..frames {
            let time = self.time.tick(time_k);
            let feedback = self.feedback.tick(ctx.smooth_k);
            let mix = self.mix.tick(ctx.smooth_k);

            let delay_samples = (time * ctx.sample_rate).clamp(1.0, len - 3.0);
            // Linear-interpolated read behind the write head.
            let read_pos = self.write as f32 - delay_samples;
            let read_pos = if read_pos < 0.0 { read_pos + len } else { read_pos };
            let idx = read_pos as usize;
            let frac = read_pos - idx as f32;
            let a = self.buf[idx % BUF_LEN];
            let b = self.buf[(idx + 1) % BUF_LEN];
            let delayed = a + (b - a) * frac;

            let dry = input.map_or(0.0, |b| b.mono(i));
            // Feedback path soft-clipped so self-oscillation stays bounded.
            let writeback = tanh_pade((dry + delayed * feedback) / 10.0) * 10.0;
            self.buf[self.write] = undenorm(writeback);
            self.write = (self.write + 1) % BUF_LEN;

            out.data[0][i] = dry + (delayed - dry) * mix;
        }
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn echoes_an_impulse_at_the_set_time() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut delay = Delay::new();
        delay.time.set_immediate(0.1); // 4800 samples
        delay.mix.set_immediate(1.0); // wet only
        delay.feedback.set_immediate(0.0);

        let mut input = PortBuffer::silent();
        input.data[0][0] = 5.0; // impulse in the first block
        let mut out = PortBuffer::silent();
        let mut first_echo: Option<usize> = None;
        let silent = PortBuffer::silent();
        for block in 0..400 {
            let inp = if block == 0 { &input } else { &silent };
            delay.process(&ctx, Some(inp), &mut out, BLOCK);
            if first_echo.is_none() {
                if let Some(i) = out.data[0][..BLOCK].iter().position(|&s| s.abs() > 1.0) {
                    first_echo = Some(block * BLOCK + i);
                }
            }
        }
        let at = first_echo.expect("echo never arrived");
        assert!(
            (at as i64 - 4800).unsigned_abs() <= 2,
            "echo at sample {at}, expected ~4800"
        );
    }

    #[test]
    fn feedback_repeats_decay() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut delay = Delay::new();
        delay.time.set_immediate(0.05);
        delay.mix.set_immediate(1.0);
        delay.feedback.set_immediate(0.5);

        let mut input = PortBuffer::silent();
        input.data[0][0] = 5.0;
        let mut out = PortBuffer::silent();
        let silent = PortBuffer::silent();
        let mut peaks = Vec::new();
        let mut block_peak = 0.0f32;
        for block in 0..600 {
            let inp = if block == 0 { &input } else { &silent };
            delay.process(&ctx, Some(inp), &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                block_peak = block_peak.max(s.abs());
            }
            // 0.05 s = 75 blocks per repeat; sample the peak per window.
            if block % 75 == 74 {
                peaks.push(block_peak);
                block_peak = 0.0;
            }
        }
        // The echo lands exactly on the second window (0.05 s = 75 blocks);
        // repeats decay by ~the feedback ratio and never grow.
        let peaks = &peaks[1..];
        assert!(peaks[0] > 1.0, "first echo missing: {peaks:?}");
        for pair in peaks.windows(2) {
            assert!(pair[1] <= pair[0] + 1e-3, "feedback grew: {peaks:?}");
        }
        assert!(peaks[3] < peaks[0] * 0.3, "not decaying: {peaks:?}");
    }
}
