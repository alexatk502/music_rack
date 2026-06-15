//! Clock divider: counts rising edges on the clock input and emits divided
//! gates at ÷2, ÷4, ÷8, ÷16. A reset edge restarts the count. Each output is
//! high for the first half of its divided period (50% duty).

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_dsp::volts::{GATE_HIGH, GATE_THRESHOLD};

const DIVS: [u32; 4] = [2, 4, 8, 16];

pub struct ClockDiv {
    count: u32,
    clock_high: bool,
    reset_high: bool,
}

impl ClockDiv {
    pub fn new() -> Self {
        Self { count: 0, clock_high: false, reset_high: false }
    }

    pub fn set_param(&mut self, _param: u32, _value: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        outs: [&mut PortBuffer; 4],
        frames: usize,
    ) {
        let clock_data = clock.map(|b| b.channel_or_broadcast(0));
        let reset_data = reset.map(|b| b.channel_or_broadcast(0));
        let [o2, o4, o8, o16] = outs;
        o2.channels = 1;
        o4.channels = 1;
        o8.channels = 1;
        o16.channels = 1;

        for i in 0..frames {
            if let Some(r) = reset_data {
                let high = r[i] >= GATE_THRESHOLD;
                if high && !self.reset_high {
                    self.count = 0;
                }
                self.reset_high = high;
            }
            if let Some(c) = clock_data {
                let high = c[i] >= GATE_THRESHOLD;
                if high && !self.clock_high {
                    self.count = self.count.wrapping_add(1);
                }
                self.clock_high = high;
            }
            // Output n is high during the first half of its divided period.
            let level = |div: u32| {
                if (self.count.wrapping_sub(1)) % div < div / 2 {
                    GATE_HIGH
                } else {
                    0.0
                }
            };
            // Before the first edge (count == 0) everything is low.
            let lvl = |div: u32| if self.count == 0 { 0.0 } else { level(div) };
            o2.data[0][i] = lvl(DIVS[0]);
            o4.data[0][i] = lvl(DIVS[1]);
            o8.data[0][i] = lvl(DIVS[2]);
            o16.data[0][i] = lvl(DIVS[3]);
        }
    }
}

impl Default for ClockDiv {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn divides_clock_edges() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut div = ClockDiv::new();
        let mut o2 = PortBuffer::silent();
        let mut o4 = PortBuffer::silent();
        let mut o8 = PortBuffer::silent();
        let mut o16 = PortBuffer::silent();

        // Count rising edges on each output over many input pulses.
        let mut rises = [0u32; 4];
        let mut last = [0.0f32; 4];
        let pulses = 64;
        for n in 0..pulses {
            for level in [10.0f32, 0.0] {
                let mut clk = PortBuffer::silent();
                clk.data[0] = [level; BLOCK];
                div.process(&ctx, Some(&clk), None, [&mut o2, &mut o4, &mut o8, &mut o16], BLOCK);
                for (j, o) in [&o2, &o4, &o8, &o16].iter().enumerate() {
                    let s = o.data[0][BLOCK - 1];
                    if last[j] < 1.0 && s >= 1.0 {
                        rises[j] += 1;
                    }
                    last[j] = s;
                }
            }
            let _ = n;
        }
        // 64 input pulses → ÷2 fires 32, ÷4 → 16, ÷8 → 8, ÷16 → 4 (±1).
        assert!((rises[0] as i32 - 32).abs() <= 1, "÷2 {rises:?}");
        assert!((rises[1] as i32 - 16).abs() <= 1, "÷4 {rises:?}");
        assert!((rises[2] as i32 - 8).abs() <= 1, "÷8 {rises:?}");
        assert!((rises[3] as i32 - 4).abs() <= 1, "÷16 {rises:?}");
    }
}
