//! Rhythm modules: a 4-track euclidean beat generator and a 4-step ratchet
//! sequencer. Both advance on a clock input and emit short triggers (mono).

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_dsp::volts::{GATE_HIGH, GATE_THRESHOLD};

// ---------------------------------------------------------------------------
// Beats: a four-track, 16-step trigger grid sharing one clock. Each track's
// pattern is a 16-bit mask param (bit s = step s on), so the whole grid is
// just four numbers that serialize and stream to the engine like any param.
// inputs: clock, reset
// outputs: t1, t2, t3, t4
// params: 0..3 track masks
// ---------------------------------------------------------------------------

pub const BEATS_STEPS: usize = 16;
const TRIG_SECS: f32 = 0.005;

pub struct Beats {
    step: usize,
    prev_clock: bool,
    prev_reset: bool,
    timers: [f32; 4],
    masks: [u32; 4],
}

impl Beats {
    pub fn new() -> Self {
        Self {
            step: 0,
            prev_clock: false,
            prev_reset: false,
            timers: [0.0; 4],
            // Default groove: kick on 1/5/9/13, snare on 5/13, hats on evens.
            masks: [0x1111, 0x1010, 0x5555, 0x0000],
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param < 4 {
            // Mask is a 16-bit integer carried in an f32 (exactly representable).
            self.masks[param as usize] = (value.max(0.0) as u32) & 0xFFFF;
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        outs: &mut [&mut PortBuffer; 4],
        frames: usize,
    ) {
        for o in outs.iter_mut() {
            o.channels = 1;
        }
        let trig = TRIG_SECS * ctx.sample_rate;
        for i in 0..frames {
            let rs = reset.map_or(false, |b| b.mono(i) >= GATE_THRESHOLD);
            if rs && !self.prev_reset {
                self.step = 0;
            }
            self.prev_reset = rs;

            let ck = clock.map_or(false, |b| b.mono(i) >= GATE_THRESHOLD);
            if ck && !self.prev_clock {
                for t in 0..4 {
                    if self.masks[t] & (1 << self.step) != 0 {
                        self.timers[t] = trig;
                    }
                }
                self.step = (self.step + 1) % BEATS_STEPS;
            }
            self.prev_clock = ck;

            for t in 0..4 {
                let high = self.timers[t] > 0.0;
                if high {
                    self.timers[t] -= 1.0;
                }
                outs[t].data[0][i] = if high { GATE_HIGH } else { 0.0 };
            }
        }
    }
}

impl Default for Beats {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Ratchet: a 4-step sequencer where each step fires N evenly-spaced sub-
// triggers across the measured clock period (rolls/stutters).
// inputs: clock, reset
// outputs: gate
// params: 0..3 ratchet count per step
// ---------------------------------------------------------------------------

pub struct Ratchet {
    step: usize,
    prev_clock: bool,
    prev_reset: bool,
    period: f32,
    since: f32,
    sub_interval: f32,
    next_sub: f32,
    subs_left: u32,
    timer: f32,
    counts: [u32; 4],
}

impl Ratchet {
    pub fn new() -> Self {
        Self {
            step: 0,
            prev_clock: false,
            prev_reset: false,
            period: 12000.0,
            since: 0.0,
            sub_interval: 0.0,
            next_sub: 0.0,
            subs_left: 0,
            timer: 0.0,
            counts: [1, 2, 1, 4],
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param < 4 {
            // Switch positions 0..7 map to 1..8 sub-triggers.
            self.counts[param as usize] = (value as u32 + 1).clamp(1, 8);
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let trig = 0.002 * ctx.sample_rate;
        for i in 0..frames {
            self.since += 1.0;

            let rs = reset.map_or(false, |b| b.mono(i) >= GATE_THRESHOLD);
            if rs && !self.prev_reset {
                self.step = 0;
            }
            self.prev_reset = rs;

            let ck = clock.map_or(false, |b| b.mono(i) >= GATE_THRESHOLD);
            if ck && !self.prev_clock {
                self.period = self.since.clamp(2.0, ctx.sample_rate);
                self.since = 0.0;
                let n = self.counts[self.step].max(1);
                self.subs_left = n;
                self.sub_interval = self.period / n as f32;
                self.next_sub = 0.0; // fire the first sub immediately
                self.step = (self.step + 1) % 4;
            }
            self.prev_clock = ck;

            // Emit the scheduled sub-triggers across the step.
            if self.subs_left > 0 {
                if self.next_sub <= 0.0 {
                    self.timer = trig;
                    self.subs_left -= 1;
                    self.next_sub += self.sub_interval;
                }
                self.next_sub -= 1.0;
            }

            let high = self.timer > 0.0;
            if high {
                self.timer -= 1.0;
            }
            out.data[0][i] = if high { GATE_HIGH } else { 0.0 };
        }
    }
}

impl Default for Ratchet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn high() -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        b.data[0] = [GATE_HIGH; BLOCK];
        b
    }
    fn low() -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        b
    }

    #[test]
    fn beats_plays_only_the_masked_steps() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut beats = Beats::new();
        // Track 1 fires on step 0 only (bit 0); track 2 on step 1 only (bit 1).
        beats.set_param(0, 1.0);
        beats.set_param(1, 2.0);
        beats.set_param(2, 0.0);
        beats.set_param(3, 0.0);

        let clk = high();
        let rest = low();
        // Run one rising edge and capture the peak of each track on that block.
        // Long gaps between edges let the ~5 ms triggers fully decay.
        let mut edge = |beats: &mut Beats| -> [f32; 4] {
            let (mut a, mut b, mut c, mut d) = (low(), low(), low(), low());
            {
                let mut outs = [&mut a, &mut b, &mut c, &mut d];
                beats.process(&ctx, Some(&clk), Some(&rest), &mut outs, BLOCK);
            }
            let peaks = std::array::from_fn(|t| {
                [&a, &b, &c, &d][t].data[0][..BLOCK].iter().cloned().fold(0.0f32, f32::max)
            });
            // Clock-low gap, long enough for the trigger pulse to expire.
            for _ in 0..16 {
                let (mut a, mut b, mut c, mut d) = (low(), low(), low(), low());
                let mut outs = [&mut a, &mut b, &mut c, &mut d];
                beats.process(&ctx, Some(&rest), Some(&rest), &mut outs, BLOCK);
            }
            peaks
        };

        let e0 = edge(&mut beats); // plays step 0
        let e1 = edge(&mut beats); // plays step 1
        assert!(e0[0] > 1.0, "track1 should fire on step 0");
        assert_eq!(e0[1], 0.0, "track2 silent on step 0");
        assert!(e1[1] > 1.0, "track2 should fire on step 1");
        assert_eq!(e1[0], 0.0, "track1 silent on step 1");
        assert_eq!(e0[2] + e0[3] + e1[2] + e1[3], 0.0, "empty tracks never fire");
    }
}
