//! Generative sequencing: Euclidean rhythm generator, Bernoulli (coin-toss)
//! gate, and a Turing-machine shift register.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::{bernoulli as bp, euclid as ep, turing as tp};
use rack_dsp::volts::{AUDIO_PEAK, GATE_HIGH, GATE_THRESHOLD};

// ---------------------------------------------------------------------------
// Euclidean sequencer
// ---------------------------------------------------------------------------

pub struct Euclid {
    length: u32,
    fill: u32,
    rotate: u32,
    step: u32,
    clock_high: bool,
    reset_high: bool,
}

impl Euclid {
    pub fn new() -> Self {
        Self { length: 16, fill: 4, rotate: 0, step: 0, clock_high: false, reset_high: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            ep::LENGTH => self.length = (value as u32 + 1).clamp(1, 16),
            ep::FILL => self.fill = (value as u32).min(16),
            ep::ROTATE => self.rotate = value as u32,
            _ => {}
        }
    }

    /// Bjorklund-style test: is `step` a hit for `fill` pulses over `length`?
    /// Uses the standard even-distribution formula.
    #[inline]
    fn is_hit(&self, step: u32) -> bool {
        if self.length == 0 || self.fill == 0 {
            return false;
        }
        let fill = self.fill.min(self.length);
        let s = (step + self.rotate) % self.length;
        // A pulse falls on step s when floor(s*fill/length) changes.
        (s * fill) % self.length < fill
    }

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        gate: &mut PortBuffer,
        frames: usize,
    ) {
        gate.channels = 1;
        let clock_data = clock.map(|b| b.channel_or_broadcast(0));
        let reset_data = reset.map(|b| b.channel_or_broadcast(0));

        for i in 0..frames {
            if let Some(r) = reset_data {
                let high = r[i] >= GATE_THRESHOLD;
                if high && !self.reset_high {
                    self.step = 0;
                }
                self.reset_high = high;
            }
            let clock_now = clock_data.map_or(false, |c| c[i] >= GATE_THRESHOLD);
            if clock_now && !self.clock_high {
                self.step = (self.step + 1) % self.length;
            }
            self.clock_high = clock_now;
            // Gate is high (following the clock) only on hit steps.
            let hit = self.is_hit(self.step);
            gate.data[0][i] = if hit && clock_now { GATE_HIGH } else { 0.0 };
        }
    }
}

impl Default for Euclid {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Bernoulli gate (coin toss)
// ---------------------------------------------------------------------------

pub struct Bernoulli {
    prob: f32,
    rng: u32,
    trig_high: bool,
    route_a: bool,
}

impl Bernoulli {
    pub fn new() -> Self {
        Self { prob: 0.5, rng: 0x51ab_cd01, trig_high: false, route_a: true }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == bp::PROB {
            self.prob = value.clamp(0.0, 1.0);
        }
    }

    #[inline]
    fn coin(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f32 / u32::MAX as f32
    }

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        trig: Option<&PortBuffer>,
        prob_cv: Option<&PortBuffer>,
        a_out: &mut PortBuffer,
        b_out: &mut PortBuffer,
        frames: usize,
    ) {
        a_out.channels = 1;
        b_out.channels = 1;
        let trig_data = trig.map(|x| x.channel_or_broadcast(0));
        let cv = prob_cv.map(|x| x.channel_or_broadcast(0));

        for i in 0..frames {
            let high = trig_data.map_or(false, |t| t[i] >= GATE_THRESHOLD);
            if high && !self.trig_high {
                // Decide routing on the rising edge.
                let p = (self.prob + cv.map_or(0.0, |c| c[i] / 10.0)).clamp(0.0, 1.0);
                self.route_a = self.coin() < p;
            }
            self.trig_high = high;
            let pass = if high { GATE_HIGH } else { 0.0 };
            a_out.data[0][i] = if self.route_a { pass } else { 0.0 };
            b_out.data[0][i] = if self.route_a { 0.0 } else { pass };
        }
    }
}

impl Default for Bernoulli {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Turing machine (looping shift register)
// ---------------------------------------------------------------------------

pub struct Turing {
    bits: u16,
    length: u32,
    prob: f32,
    rng: u32,
    clock_high: bool,
}

impl Turing {
    pub fn new() -> Self {
        Self { bits: 0b1011_0010_1101_0110, length: 8, prob: 0.5, rng: 0x77, clock_high: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            tp::LENGTH => self.length = (value as u32 + 1).clamp(1, 16),
            tp::PROB => self.prob = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    #[inline]
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f32 / u32::MAX as f32
    }

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        gate: &mut PortBuffer,
        cv: &mut PortBuffer,
        frames: usize,
    ) {
        gate.channels = 1;
        cv.channels = 1;
        let clock_data = clock.map(|b| b.channel_or_broadcast(0));

        for i in 0..frames {
            let clock_now = clock_data.map_or(false, |c| c[i] >= GATE_THRESHOLD);
            if clock_now && !self.clock_high {
                // Rotate the register; the fed-back bit may flip per `prob`.
                let len = self.length;
                let top = (self.bits >> (len - 1)) & 1;
                // prob 0.5 = max randomness; 0 or 1 = locked loop.
                let mut new_bit = top;
                if self.rand() < self.prob {
                    new_bit ^= 1;
                }
                let mask = (1u16 << len) - 1;
                self.bits = (((self.bits << 1) | new_bit) & mask) | (self.bits & !mask);
            }
            self.clock_high = clock_now;

            // Gate = current top bit; CV = low 8 bits scaled to ±5 V.
            let top = (self.bits >> (self.length - 1)) & 1;
            gate.data[0][i] = if top == 1 && clock_now { GATE_HIGH } else { 0.0 };
            let byte = (self.bits & 0xFF) as f32 / 255.0;
            cv.data[0][i] = (byte * 2.0 - 1.0) * AUDIO_PEAK;
        }
    }
}

impl Default for Turing {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    /// Clock the module N times, return the gate state at each pulse.
    fn euclid_pattern(e: &mut Euclid, n: usize) -> Vec<bool> {
        let ctx = ProcessCtx::new(48_000.0);
        let mut gate = PortBuffer::silent();
        let mut hits = Vec::new();
        for _ in 0..n {
            let mut hi = PortBuffer::silent();
            hi.data[0] = [10.0; BLOCK];
            e.process(&ctx, Some(&hi), None, &mut gate, BLOCK);
            hits.push(gate.data[0][BLOCK - 1] > 1.0);
            let lo = PortBuffer::silent();
            e.process(&ctx, Some(&lo), None, &mut gate, BLOCK);
        }
        hits
    }

    #[test]
    fn euclid_distributes_fills_over_length() {
        let mut e = Euclid::new();
        e.set_param(ep::LENGTH, 7.0); // length = 8
        e.set_param(ep::FILL, 4.0);
        e.set_param(ep::ROTATE, 0.0);
        let pat = euclid_pattern(&mut e, 8);
        let hits = pat.iter().filter(|&&h| h).count();
        assert_eq!(hits, 4, "expected 4 hits over 8 steps, got {hits}: {pat:?}");
    }

    #[test]
    fn euclid_zero_fill_is_silent() {
        let mut e = Euclid::new();
        e.set_param(ep::LENGTH, 7.0);
        e.set_param(ep::FILL, 0.0);
        assert!(euclid_pattern(&mut e, 8).iter().all(|&h| !h));
    }

    #[test]
    fn bernoulli_routes_by_probability() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut coin = Bernoulli::new();
        coin.set_param(bp::PROB, 1.0); // always route to A
        let mut a = PortBuffer::silent();
        let mut b = PortBuffer::silent();
        let (mut a_hits, mut b_hits) = (0, 0);
        for _ in 0..100 {
            let mut hi = PortBuffer::silent();
            hi.data[0][0] = 10.0;
            coin.process(&ctx, Some(&hi), None, &mut a, &mut b, BLOCK);
            if a.data[0][0] > 1.0 { a_hits += 1; }
            if b.data[0][0] > 1.0 { b_hits += 1; }
            let lo = PortBuffer::silent();
            coin.process(&ctx, Some(&lo), None, &mut a, &mut b, BLOCK);
        }
        assert_eq!(a_hits, 100);
        assert_eq!(b_hits, 0);

        // Probability 0 → always B.
        coin.set_param(bp::PROB, 0.0);
        let (mut a2, mut b2) = (0, 0);
        for _ in 0..100 {
            let mut hi = PortBuffer::silent();
            hi.data[0][0] = 10.0;
            coin.process(&ctx, Some(&hi), None, &mut a, &mut b, BLOCK);
            if a.data[0][0] > 1.0 { a2 += 1; }
            if b.data[0][0] > 1.0 { b2 += 1; }
            let lo = PortBuffer::silent();
            coin.process(&ctx, Some(&lo), None, &mut a, &mut b, BLOCK);
        }
        assert_eq!(a2, 0);
        assert_eq!(b2, 100);
    }

    #[test]
    fn turing_locked_loop_repeats() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut t = Turing::new();
        t.set_param(tp::LENGTH, 7.0); // length 8
        t.set_param(tp::PROB, 0.0); // locked: pattern must repeat every 8 clocks
        let mut gate = PortBuffer::silent();
        let mut cv = PortBuffer::silent();

        let clock = |t: &mut Turing, gate: &mut PortBuffer, cv: &mut PortBuffer| -> f32 {
            let mut hi = PortBuffer::silent();
            hi.data[0] = [10.0; BLOCK];
            t.process(&ctx, Some(&hi), gate, cv, BLOCK);
            let v = cv.data[0][BLOCK - 1];
            let lo = PortBuffer::silent();
            t.process(&ctx, Some(&lo), gate, cv, BLOCK);
            v
        };
        let first: Vec<f32> = (0..8).map(|_| clock(&mut t, &mut gate, &mut cv)).collect();
        let second: Vec<f32> = (0..8).map(|_| clock(&mut t, &mut gate, &mut cv)).collect();
        assert_eq!(first, second, "locked Turing machine should loop");
    }

    #[test]
    fn turing_full_randomness_changes_pattern() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut t = Turing::new();
        t.set_param(tp::LENGTH, 7.0);
        t.set_param(tp::PROB, 0.5);
        let mut gate = PortBuffer::silent();
        let mut cv = PortBuffer::silent();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let mut hi = PortBuffer::silent();
            hi.data[0] = [10.0; BLOCK];
            t.process(&ctx, Some(&hi), &mut gate, &mut cv, BLOCK);
            seen.insert(cv.data[0][BLOCK - 1].to_bits());
            let lo = PortBuffer::silent();
            t.process(&ctx, Some(&lo), &mut gate, &mut cv, BLOCK);
        }
        assert!(seen.len() > 4, "randomized Turing too static: {} values", seen.len());
    }
}
