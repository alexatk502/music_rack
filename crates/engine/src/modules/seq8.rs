//! SEQ-8: 8-step pitch sequencer. Advances on the clock input's rising edge
//! (the first edge plays step 1 rather than skipping it), passes the clock
//! through as the gate (clock width = gate length), and returns to step 1 on
//! a reset rising edge.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::seq8 as p;
use rack_dsp::volts::GATE_THRESHOLD;

pub struct Seq8 {
    pitches: [f32; 8],
    steps: u8,
    current: u8,
    /// False until the first clock edge: that edge plays step 1 in place.
    started: bool,
    clock_high: bool,
    reset_high: bool,
}

impl Seq8 {
    pub fn new() -> Self {
        Self {
            pitches: [0.0; 8],
            steps: 8,
            current: 0,
            started: false,
            clock_high: false,
            reset_high: false,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == p::STEPS {
            self.steps = (value.round() as u8).clamp(1, 8);
        } else if (p::PITCH_BASE..p::PITCH_BASE + 8).contains(&param) {
            self.pitches[(param - p::PITCH_BASE) as usize] = value.clamp(-2.0, 2.0);
        }
    }

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        voct: &mut PortBuffer,
        gate: &mut PortBuffer,
        frames: usize,
    ) {
        voct.channels = 1;
        gate.channels = 1;
        let clock_data = clock.map(|b| b.channel_or_broadcast(0));
        let reset_data = reset.map(|b| b.channel_or_broadcast(0));

        for i in 0..frames {
            if let Some(r) = reset_data {
                let high = r[i] >= GATE_THRESHOLD;
                if high && !self.reset_high {
                    self.current = 0;
                    self.started = false;
                }
                self.reset_high = high;
            }
            let clock_now = clock_data.map_or(false, |c| c[i] >= GATE_THRESHOLD);
            if clock_now && !self.clock_high {
                if self.started {
                    self.current = (self.current + 1) % self.steps.max(1);
                } else {
                    self.started = true; // first edge plays step 1 in place
                }
            }
            self.clock_high = clock_now;
            self.current %= self.steps.max(1);
            voct.data[0][i] = self.pitches[self.current as usize];
            gate.data[0][i] = clock_data.map_or(0.0, |c| c[i]);
        }
    }
}

impl Default for Seq8 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    /// One block with the given clock/reset levels; returns (pitch, gate).
    fn step(seq: &mut Seq8, clock_v: f32, reset_v: f32) -> (f32, f32) {
        let ctx = ProcessCtx::new(48_000.0);
        let mut clock = PortBuffer::silent();
        clock.data[0] = [clock_v; BLOCK];
        let mut reset = PortBuffer::silent();
        reset.data[0] = [reset_v; BLOCK];
        let mut voct = PortBuffer::silent();
        let mut gate = PortBuffer::silent();
        seq.process(&ctx, Some(&clock), Some(&reset), &mut voct, &mut gate, BLOCK);
        (voct.data[0][BLOCK - 1], gate.data[0][BLOCK - 1])
    }

    #[test]
    fn sequences_and_wraps() {
        let mut seq = Seq8::new();
        seq.set_param(p::STEPS, 3.0);
        for (i, pitch) in [0.5, 1.0, 1.5].iter().enumerate() {
            seq.set_param(p::PITCH_BASE + i as u32, *pitch);
        }
        let mut seen = Vec::new();
        for _ in 0..5 {
            let (pitch, gate) = step(&mut seq, 10.0, 0.0); // clock high
            assert_eq!(gate, 10.0, "gate follows clock");
            seen.push(pitch);
            step(&mut seq, 0.0, 0.0); // clock low
        }
        // First edge plays step 1, then advances, wrapping after 3.
        assert_eq!(seen, vec![0.5, 1.0, 1.5, 0.5, 1.0]);
    }

    #[test]
    fn reset_returns_to_step_one() {
        let mut seq = Seq8::new();
        for i in 0..8 {
            seq.set_param(p::PITCH_BASE + i, i as f32 * 0.1);
        }
        for _ in 0..3 {
            step(&mut seq, 10.0, 0.0);
            step(&mut seq, 0.0, 0.0);
        }
        // Reset, then next clock plays step 1 again.
        step(&mut seq, 0.0, 10.0);
        let (pitch, _) = step(&mut seq, 10.0, 10.0);
        assert_eq!(pitch, 0.0);
    }
}
