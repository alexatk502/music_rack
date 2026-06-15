//! Routing, CV processing, and dynamics: sequential switch, crossfader, VCA
//! bank, CV mixer (precision adder), octave/transpose, panner, compressor.

use crate::buffer::{propagate_channels, PortBuffer, BLOCK, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{
    compressor as comp, crossfade as xp, octave as op, pan as pp, seqswitch as sp, vcabank as vp,
};
use rack_dsp::volts::{AUDIO_PEAK, GATE_HIGH, GATE_THRESHOLD};
use rack_dsp::Smoothed;

// ---------------------------------------------------------------------------
// Sequential switch (4 inputs → 1 output, advanced by clock)
// ---------------------------------------------------------------------------

pub struct SeqSwitch {
    steps: u32,
    step: u32,
    clock_high: bool,
    reset_high: bool,
}

impl SeqSwitch {
    pub fn new() -> Self {
        Self { steps: 4, step: 0, clock_high: false, reset_high: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == sp::STEPS {
            self.steps = (value as u32 + 1).clamp(2, 4);
        }
    }

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        ins: [Option<&PortBuffer>; 4],
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let clock_data = clock.map(|b| b.channel_or_broadcast(0));
        let reset_data = reset.map(|b| b.channel_or_broadcast(0));
        // Output channel count follows the currently-selected input.
        let selected = ins[self.step as usize % 4];
        out.channels = selected.map(|b| b.channels.max(1)).unwrap_or(1);

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
                self.step = (self.step + 1) % self.steps;
            }
            self.clock_high = clock_now;

            let src = ins[self.step as usize % 4];
            for ch in 0..out.channels as usize {
                out.data[ch][i] = src.map_or(0.0, |b| b.channel_or_broadcast(ch)[i]);
            }
        }
    }
}

impl Default for SeqSwitch {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Crossfader
// ---------------------------------------------------------------------------

pub struct Crossfade {
    mix: Smoothed,
}

impl Crossfade {
    pub fn new() -> Self {
        Self { mix: Smoothed::new(0.5) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == xp::MIX {
            self.mix.set_target(value.clamp(0.0, 1.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        a: Option<&PortBuffer>,
        b: Option<&PortBuffer>,
        cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[a, b, cv]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let ad = a.map(|x| x.channel_or_broadcast(ch));
            let bd = b.map(|x| x.channel_or_broadcast(ch));
            let cvd = cv.map(|x| x.channel_or_broadcast(ch));
            for i in 0..frames {
                let m = (self.mix.tick_for(ch, ctx.smooth_k) + cvd.map_or(0.0, |c| c[i] / 10.0))
                    .clamp(0.0, 1.0);
                let av = ad.map_or(0.0, |d| d[i]);
                let bv = bd.map_or(0.0, |d| d[i]);
                out.data[ch][i] = av * (1.0 - m) + bv * m;
            }
        }
    }
}

impl Default for Crossfade {
    fn default() -> Self {
        Self::new()
    }
}

// Helper so a single Smoothed can drive multiple channels without ticking
// per channel (we only need its current value mid-block here).
trait SmoothedChannel {
    fn tick_for(&mut self, ch: usize, k: f32) -> f32;
}
impl SmoothedChannel for Smoothed {
    #[inline]
    fn tick_for(&mut self, ch: usize, k: f32) -> f32 {
        // Only advance on channel 0 so the time constant is per-sample, not
        // per-sample-times-channels.
        if ch == 0 {
            self.tick(k)
        } else {
            self.current()
        }
    }
}

// ---------------------------------------------------------------------------
// VCA bank (4 independent VCAs)
// ---------------------------------------------------------------------------

pub struct VcaBank {
    levels: [Smoothed; 4],
}

impl VcaBank {
    pub fn new() -> Self {
        Self { levels: [Smoothed::new(1.0); 4] }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        let v = value.clamp(0.0, 1.0);
        match param {
            vp::LEVEL1 => self.levels[0].set_target(v),
            vp::LEVEL2 => self.levels[1].set_target(v),
            vp::LEVEL3 => self.levels[2].set_target(v),
            vp::LEVEL4 => self.levels[3].set_target(v),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        ins: [Option<&PortBuffer>; 4],
        cvs: [Option<&PortBuffer>; 4],
        outs: [&mut PortBuffer; 4],
        frames: usize,
    ) {
        for (k, out) in outs.into_iter().enumerate() {
            let input = ins[k];
            let cv = cvs[k];
            let channels = propagate_channels(&[input, cv]);
            out.channels = channels;
            // Smooth this VCA's level once per frame (channel 0).
            let mut gain = [0.0f32; BLOCK];
            for g in gain.iter_mut().take(frames) {
                *g = self.levels[k].tick(ctx.smooth_k);
            }
            for ch in 0..channels as usize {
                let ind = input.map(|b| b.channel_or_broadcast(ch));
                let cvd = cv.map(|b| b.channel_or_broadcast(ch));
                for i in 0..frames {
                    let x = ind.map_or(0.0, |d| d[i]);
                    let c = cvd.map_or(1.0, |d| (d[i] / GATE_HIGH).clamp(0.0, 1.0));
                    out.data[ch][i] = x * gain[i] * c;
                }
            }
        }
    }
}

impl Default for VcaBank {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CV mixer / precision adder
// ---------------------------------------------------------------------------

pub struct CvMix;

impl CvMix {
    pub fn new() -> Self {
        Self
    }
    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        ins: [Option<&PortBuffer>; 4],
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&ins);
        out.channels = channels;
        for ch in 0..channels as usize {
            for i in 0..frames {
                let mut sum = 0.0;
                for input in ins.iter() {
                    if let Some(b) = input {
                        sum += b.channel_or_broadcast(ch)[i];
                    }
                }
                // Unity-sum (precision adder): exact for pitch CV addition.
                out.data[ch][i] = sum;
            }
        }
    }
}

impl Default for CvMix {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Octave / transpose
// ---------------------------------------------------------------------------

pub struct Octave {
    octaves: i32,
    semis: i32,
}

impl Octave {
    pub fn new() -> Self {
        Self { octaves: 0, semis: 0 }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            op::OCTAVES => self.octaves = value as i32 - 4, // switch 0..8 → -4..4
            op::SEMIS => self.semis = value as i32 - 12,    // switch 0..24 → -12..12
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        let offset = self.octaves as f32 + self.semis as f32 / 12.0;
        for ch in 0..channels as usize {
            let ind = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                out.data[ch][i] = ind.map_or(0.0, |d| d[i]) + offset;
            }
        }
    }
}

impl Default for Octave {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Panner (constant-power)
// ---------------------------------------------------------------------------

pub struct Pan {
    pan: Smoothed,
}

impl Pan {
    pub fn new() -> Self {
        Self { pan: Smoothed::new(0.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == pp::PAN {
            self.pan.set_target(value.clamp(-1.0, 1.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        pan_cv: Option<&PortBuffer>,
        left: &mut PortBuffer,
        right: &mut PortBuffer,
        frames: usize,
    ) {
        left.channels = 1;
        right.channels = 1;
        let cv = pan_cv.map(|b| b.channel_or_broadcast(0));
        for i in 0..frames {
            let p = (self.pan.tick(ctx.smooth_k) + cv.map_or(0.0, |c| c[i] / 5.0)).clamp(-1.0, 1.0);
            // Constant-power: map -1..1 to 0..pi/2.
            let angle = (p * 0.5 + 0.5) * core::f32::consts::FRAC_PI_2;
            // Sum poly voices to mono so every voice is panned, not just ch 0.
            let x = input.map_or(0.0, |b| b.mono(i));
            left.data[0][i] = x * angle.cos();
            right.data[0][i] = x * angle.sin();
        }
    }
}

impl Default for Pan {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Compressor
// ---------------------------------------------------------------------------

pub struct Compressor {
    env: [f32; MAX_CHANNELS],
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup_db: f32,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            env: [0.0; MAX_CHANNELS],
            threshold_db: -18.0,
            ratio: 4.0,
            attack: 0.01,
            release: 0.15,
            makeup_db: 0.0,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            comp::THRESHOLD => self.threshold_db = value.clamp(-40.0, 0.0),
            comp::RATIO => self.ratio = value.clamp(1.0, 20.0),
            comp::ATTACK => self.attack = value.clamp(0.001, 0.2),
            comp::RELEASE => self.release = value.clamp(0.01, 1.0),
            comp::MAKEUP => self.makeup_db = value.clamp(0.0, 24.0),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        sidechain: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, sidechain]);
        out.channels = channels;
        let atk_k = 1.0 - (-1.0 / (self.attack * ctx.sample_rate)).exp();
        let rel_k = 1.0 - (-1.0 / (self.release * ctx.sample_rate)).exp();
        let makeup = 10f32.powf(self.makeup_db / 20.0);
        let inv_ratio = 1.0 / self.ratio;

        for ch in 0..channels as usize {
            let ind = input.map(|b| b.channel_or_broadcast(ch));
            // Sidechain drives detection if patched, else the input itself.
            let det = sidechain.map(|b| b.channel_or_broadcast(ch)).or(ind);
            let env = &mut self.env[ch];
            for i in 0..frames {
                let key = det.map_or(0.0, |d| d[i]).abs() / AUDIO_PEAK; // 0..1 ≈ full scale
                let k = if key > *env { atk_k } else { rel_k };
                *env += k * (key - *env);
                // Level in dBFS (full scale = 0 dB).
                let level_db = 20.0 * (env.max(1e-6)).log10();
                let gain_db = if level_db > self.threshold_db {
                    (self.threshold_db - level_db) * (1.0 - inv_ratio)
                } else {
                    0.0
                };
                let gain = 10f32.powf(gain_db / 20.0) * makeup;
                out.data[ch][i] = ind.map_or(0.0, |d| d[i]) * gain;
            }
        }
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn const_buf(v: f32) -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.data[0] = [v; BLOCK];
        b
    }

    #[test]
    fn seq_switch_cycles_inputs() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut sw = SeqSwitch::new();
        sw.set_param(sp::STEPS, 3.0); // 4 steps
        let i1 = const_buf(1.0);
        let i2 = const_buf(2.0);
        let i3 = const_buf(3.0);
        let i4 = const_buf(4.0);
        let ins = [Some(&i1), Some(&i2), Some(&i3), Some(&i4)];
        let mut out = PortBuffer::silent();

        let mut seen = Vec::new();
        for _ in 0..5 {
            let mut hi = PortBuffer::silent();
            hi.data[0] = [10.0; BLOCK];
            sw.process(&ctx, Some(&hi), None, ins, &mut out, BLOCK);
            seen.push(out.data[0][BLOCK - 1]);
            let lo = PortBuffer::silent();
            sw.process(&ctx, Some(&lo), None, ins, &mut out, BLOCK);
        }
        // Starts on input 1, advances each clock, wraps after 4.
        assert_eq!(seen, vec![2.0, 3.0, 4.0, 1.0, 2.0]);
    }

    #[test]
    fn crossfade_blends() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut xf = Crossfade::new();
        xf.mix.set_immediate(0.25);
        let a = const_buf(4.0);
        let b = const_buf(8.0);
        let mut out = PortBuffer::silent();
        xf.process(&ctx, Some(&a), Some(&b), None, &mut out, BLOCK);
        // 4*0.75 + 8*0.25 = 5.
        assert!((out.data[0][BLOCK - 1] - 5.0).abs() < 1e-4);
    }

    #[test]
    fn cvmix_sums_inputs_exactly() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut m = CvMix::new();
        let a = const_buf(1.0);
        let b = const_buf(0.5);
        let c = const_buf(0.25);
        let mut out = PortBuffer::silent();
        m.process(&ctx, [Some(&a), Some(&b), Some(&c), None], &mut out, BLOCK);
        assert!((out.data[0][BLOCK - 1] - 1.75).abs() < 1e-6);
    }

    #[test]
    fn octave_transposes_by_volts() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut o = Octave::new();
        o.set_param(op::OCTAVES, 4.0 + 1.0); // +1 octave
        o.set_param(op::SEMIS, 12.0 + 7.0); // +7 semitones
        let input = const_buf(0.0);
        let mut out = PortBuffer::silent();
        o.process(&ctx, Some(&input), &mut out, BLOCK);
        // +1 V (octave) + 7/12 V.
        assert!((out.data[0][BLOCK - 1] - (1.0 + 7.0 / 12.0)).abs() < 1e-5);
    }

    #[test]
    fn vca_bank_scales_each_independently() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut bank = VcaBank::new();
        bank.levels[0].set_immediate(1.0);
        bank.levels[1].set_immediate(0.5);
        let i1 = const_buf(4.0);
        let i2 = const_buf(4.0);
        let ins = [Some(&i1), Some(&i2), None, None];
        let cvs = [None, None, None, None];
        let mut o = [PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent()];
        let [a, b, c, d] = &mut o;
        bank.process(&ctx, ins, cvs, [a, b, c, d], BLOCK);
        assert!((o[0].data[0][BLOCK - 1] - 4.0).abs() < 1e-4);
        assert!((o[1].data[0][BLOCK - 1] - 2.0).abs() < 1e-4);
    }

    #[test]
    fn pan_is_constant_power() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut pan = Pan::new();
        pan.pan.set_immediate(0.0); // center
        let input = const_buf(5.0);
        let mut l = PortBuffer::silent();
        let mut r = PortBuffer::silent();
        pan.process(&ctx, Some(&input), None, &mut l, &mut r, BLOCK);
        // Center: both at 5 * cos(45°) ≈ 3.54, and L²+R² ≈ input².
        let (lv, rv) = (l.data[0][BLOCK - 1], r.data[0][BLOCK - 1]);
        assert!((lv - rv).abs() < 1e-4, "center not balanced");
        assert!((lv * lv + rv * rv - 25.0).abs() < 0.1, "not constant power");

        // Hard left: right ≈ 0.
        pan.pan.set_immediate(-1.0);
        pan.process(&ctx, Some(&input), None, &mut l, &mut r, BLOCK);
        assert!(r.data[0][BLOCK - 1].abs() < 0.01, "hard left leaked to right");
    }

    #[test]
    fn compressor_reduces_loud_signals() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut comp_m = Compressor::new();
        comp_m.set_param(comp::THRESHOLD, -20.0);
        comp_m.set_param(comp::RATIO, 8.0);
        comp_m.set_param(comp::ATTACK, 0.001);
        comp_m.set_param(comp::RELEASE, 0.01);

        // Quiet signal (below threshold) passes ~unchanged.
        let quiet = const_buf(0.2); // ~ -28 dBFS
        let mut out = PortBuffer::silent();
        for _ in 0..200 {
            comp_m.process(&ctx, Some(&quiet), None, &mut out, BLOCK);
        }
        assert!((out.data[0][BLOCK - 1] - 0.2).abs() < 0.03, "quiet altered too much");

        // Loud signal (above threshold) gets pulled down.
        let loud = const_buf(4.0); // near full scale
        for _ in 0..400 {
            comp_m.process(&ctx, Some(&loud), None, &mut out, BLOCK);
        }
        let compressed = out.data[0][BLOCK - 1];
        assert!(compressed < 4.0 * 0.9, "loud signal not compressed: {compressed}");
        assert!(compressed > 0.5, "compressor over-attenuated: {compressed}");
    }
}
