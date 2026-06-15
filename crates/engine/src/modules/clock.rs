//! Clock: BPM-driven gate with /2 and /4 division outputs. The width knob
//! sets the gate's duty cycle for all three outputs.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::clock as p;
use rack_dsp::volts::GATE_HIGH;
use rack_dsp::Smoothed;

pub struct Clock {
    /// Beat phase in [0, 4): one full /4 cycle (the slowest division).
    phase: f32,
    bpm: Smoothed,
    width: Smoothed,
}

impl Clock {
    pub fn new() -> Self {
        Self { phase: 0.0, bpm: Smoothed::new(120.0), width: Smoothed::new(0.5) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::BPM => self.bpm.set_target(value.clamp(30.0, 300.0)),
            p::WIDTH => self.width.set_target(value.clamp(0.05, 0.95)),
            _ => {}
        }
    }

    pub fn process(&mut self, ctx: &ProcessCtx, outputs: [&mut PortBuffer; 3], frames: usize) {
        let [out, div2, div4] = outputs;
        out.channels = 1;
        div2.channels = 1;
        div4.channels = 1;
        for i in 0..frames {
            let bpm = self.bpm.tick(ctx.smooth_k);
            let width = self.width.tick(ctx.smooth_k);
            let beat = self.phase.fract(); // position within the current beat
            let half = (self.phase / 2.0).fract();
            let quarter = (self.phase / 4.0).fract();
            out.data[0][i] = if beat < width { GATE_HIGH } else { 0.0 };
            div2.data[0][i] = if half < width { GATE_HIGH } else { 0.0 };
            div4.data[0][i] = if quarter < width { GATE_HIGH } else { 0.0 };
            self.phase += bpm / 60.0 * ctx.inv_sample_rate;
            if self.phase >= 4.0 {
                self.phase -= 4.0;
            }
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn pulses_at_bpm_with_divisions() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut clock = Clock::new();
        clock.bpm.set_immediate(240.0); // 4 Hz: easy to count

        let mut out = PortBuffer::silent();
        let mut d2 = PortBuffer::silent();
        let mut d4 = PortBuffer::silent();
        let mut rising = [0u32; 3];
        let mut last = [0.0f32; 3];
        // 2 s → expect 8 beats, 4 halves, 2 quarters.
        for _ in 0..3000 {
            clock.process(&ctx, [&mut out, &mut d2, &mut d4], BLOCK);
            for (j, buf) in [&out, &d2, &d4].iter().enumerate() {
                for &s in &buf.data[0][..BLOCK] {
                    if last[j] < 1.0 && s >= 1.0 {
                        rising[j] += 1;
                    }
                    last[j] = s;
                }
            }
        }
        // ±1 tolerance: f32 phase accumulation can slip one edge into or out
        // of the 2 s window.
        assert!((rising[0] as i32 - 8).abs() <= 1, "beats {rising:?}");
        assert!((rising[1] as i32 - 4).abs() <= 1, "halves {rising:?}");
        assert!((rising[2] as i32 - 2).abs() <= 1, "quarters {rising:?}");
    }
}
