//! Small signal-mangling utilities: sample & hold, attenuverter, waveshaper,
//! mult/splitter, gate logic, and slew limiter.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{attenuverter as ap, slew as sp, waveshaper as wp};
use rack_dsp::volts::{AUDIO_PEAK, GATE_HIGH, GATE_THRESHOLD};
use rack_dsp::{tanh_pade, Smoothed};

/// Sample & hold: captures the input on each trigger rising edge.
pub struct SampleHold {
    held: [f32; MAX_CHANNELS],
    trig_high: [bool; MAX_CHANNELS],
}

impl SampleHold {
    pub fn new() -> Self {
        Self { held: [0.0; MAX_CHANNELS], trig_high: [false; MAX_CHANNELS] }
    }

    pub fn set_param(&mut self, _param: u32, _value: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        trig: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, trig]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let trig_data = trig.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                if let Some(t) = trig_data {
                    let high = t[i] >= GATE_THRESHOLD;
                    if high && !self.trig_high[ch] {
                        self.held[ch] = in_data.map_or(0.0, |d| d[i]);
                    }
                    self.trig_high[ch] = high;
                }
                data[i] = self.held[ch];
            }
        }
    }
}

impl Default for SampleHold {
    fn default() -> Self {
        Self::new()
    }
}

/// Attenuverter: out = in × gain + offset. Gain spans -2..2 (inversion),
/// offset ±10 V. The workhorse for scaling/shifting CV.
pub struct Attenuverter {
    gain: Smoothed,
    offset: Smoothed,
}

impl Attenuverter {
    pub fn new() -> Self {
        Self { gain: Smoothed::new(1.0), offset: Smoothed::new(0.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            ap::GAIN => self.gain.set_target(value.clamp(-2.0, 2.0)),
            ap::OFFSET => self.offset.set_target(value.clamp(-10.0, 10.0)),
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
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        let mut gain = [0.0f32; crate::buffer::BLOCK];
        let mut offset = [0.0f32; crate::buffer::BLOCK];
        for i in 0..frames {
            gain[i] = self.gain.tick(ctx.smooth_k);
            offset[i] = self.offset.tick(ctx.smooth_k);
        }
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                let x = in_data.map_or(0.0, |d| d[i]);
                data[i] = (x * gain[i] + offset[i]).clamp(-12.0, 12.0);
            }
        }
    }
}

impl Default for Attenuverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Waveshaper: tanh saturation or sine wavefolding, with dry/wet mix.
pub struct Waveshaper {
    drive: Smoothed,
    mix: Smoothed,
    fold: bool,
}

impl Waveshaper {
    pub fn new() -> Self {
        Self { drive: Smoothed::new(2.0), mix: Smoothed::new(1.0), fold: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            wp::DRIVE => self.drive.set_target(value.clamp(1.0, 20.0)),
            wp::MODE => self.fold = (value as u32) == 1,
            wp::MIX => self.mix.set_target(value.clamp(0.0, 1.0)),
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
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        let mut drive = [0.0f32; crate::buffer::BLOCK];
        let mut mix = [0.0f32; crate::buffer::BLOCK];
        for i in 0..frames {
            drive[i] = self.drive.tick(ctx.smooth_k);
            mix[i] = self.mix.tick(ctx.smooth_k);
        }
        let fold = self.fold;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                let x = in_data.map_or(0.0, |d| d[i]);
                let norm = x / AUDIO_PEAK;
                let shaped = if fold {
                    // Sine folder: overdriven signal wraps back on itself.
                    (norm * drive[i] * core::f32::consts::FRAC_PI_2).sin()
                } else {
                    tanh_pade(norm * drive[i])
                } * AUDIO_PEAK;
                data[i] = x + (shaped - x) * mix[i];
            }
        }
    }
}

impl Default for Waveshaper {
    fn default() -> Self {
        Self::new()
    }
}

/// Mult / splitter: copies one input to four buffered outputs (poly
/// pass-through). Cables can already fan out from a single output port, but a
/// dedicated mult is the familiar idiom.
pub struct Mult;

impl Mult {
    pub fn new() -> Self {
        Self
    }

    pub fn set_param(&mut self, _param: u32, _value: f32) {}

    pub fn process(&mut self, _ctx: &ProcessCtx, input: Option<&PortBuffer>, outs: [&mut PortBuffer; 4], frames: usize) {
        let channels = propagate_channels(&[input]);
        for out in outs {
            out.channels = channels;
            for ch in 0..channels as usize {
                let src = input.map(|b| b.channel_or_broadcast(ch));
                for i in 0..frames {
                    out.data[ch][i] = src.map_or(0.0, |s| s[i]);
                }
            }
        }
    }
}

impl Default for Mult {
    fn default() -> Self {
        Self::new()
    }
}

/// Gate logic: AND / OR / XOR of two gate inputs (threshold at 1 V), each
/// output 10 V when true. Mono — logic is a control-rate operation.
pub struct Logic;

impl Logic {
    pub fn new() -> Self {
        Self
    }

    pub fn set_param(&mut self, _param: u32, _value: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        a: Option<&PortBuffer>,
        b: Option<&PortBuffer>,
        and_out: &mut PortBuffer,
        or_out: &mut PortBuffer,
        xor_out: &mut PortBuffer,
        frames: usize,
    ) {
        and_out.channels = 1;
        or_out.channels = 1;
        xor_out.channels = 1;
        let ad = a.map(|x| x.channel_or_broadcast(0));
        let bd = b.map(|x| x.channel_or_broadcast(0));
        for i in 0..frames {
            let ah = ad.map_or(false, |d| d[i] >= GATE_THRESHOLD);
            let bh = bd.map_or(false, |d| d[i] >= GATE_THRESHOLD);
            and_out.data[0][i] = if ah && bh { GATE_HIGH } else { 0.0 };
            or_out.data[0][i] = if ah || bh { GATE_HIGH } else { 0.0 };
            xor_out.data[0][i] = if ah != bh { GATE_HIGH } else { 0.0 };
        }
    }
}

impl Default for Logic {
    fn default() -> Self {
        Self::new()
    }
}

/// Slew limiter: rate-limits how fast the output can rise/fall toward the
/// input (portamento, envelope smoothing). Rise/fall in volts-per... we use
/// one-pole time constants for a musical feel. Per-channel.
pub struct Slew {
    state: [f32; MAX_CHANNELS],
    rise: Smoothed,
    fall: Smoothed,
}

impl Slew {
    pub fn new() -> Self {
        Self { state: [0.0; MAX_CHANNELS], rise: Smoothed::new(0.1), fall: Smoothed::new(0.1) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            sp::RISE => self.rise.set_target(value.clamp(0.0, 2.0)),
            sp::FALL => self.fall.set_target(value.clamp(0.0, 2.0)),
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
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        // Per-block time constants (seconds → one-pole coefficient).
        let mut rise = self.rise.current();
        let mut fall = self.fall.current();
        for _ in 0..frames {
            rise = self.rise.tick(ctx.smooth_k);
            fall = self.fall.tick(ctx.smooth_k);
        }
        let rise_k = if rise <= 0.0 { 1.0 } else { Smoothed::coeff(rise, ctx.sample_rate) };
        let fall_k = if fall <= 0.0 { 1.0 } else { Smoothed::coeff(fall, ctx.sample_rate) };

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let s = &mut self.state[ch];
            let data = &mut out.data[ch];
            for i in 0..frames {
                let target = in_data.map_or(0.0, |d| d[i]);
                let k = if target > *s { rise_k } else { fall_k };
                *s += k * (target - *s);
                data[i] = *s;
            }
        }
    }
}

impl Default for Slew {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn mult_copies_to_all_outputs() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut m = Mult::new();
        let mut input = PortBuffer::silent();
        input.data[0] = [3.0; BLOCK];
        let mut o = [PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent()];
        let [a, b, c, d] = &mut o;
        m.process(&ctx, Some(&input), [a, b, c, d], BLOCK);
        for out in &o {
            assert_eq!(out.data[0][BLOCK - 1], 3.0);
        }
    }

    #[test]
    fn logic_gates() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut l = Logic::new();
        let mut a = PortBuffer::silent();
        let mut b = PortBuffer::silent();
        let (mut and, mut or, mut xor) = (PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent());

        a.data[0] = [10.0; BLOCK];
        b.data[0] = [0.0; BLOCK];
        l.process(&ctx, Some(&a), Some(&b), &mut and, &mut or, &mut xor, BLOCK);
        assert_eq!(and.data[0][0], 0.0);
        assert_eq!(or.data[0][0], GATE_HIGH);
        assert_eq!(xor.data[0][0], GATE_HIGH);

        b.data[0] = [10.0; BLOCK];
        l.process(&ctx, Some(&a), Some(&b), &mut and, &mut or, &mut xor, BLOCK);
        assert_eq!(and.data[0][0], GATE_HIGH);
        assert_eq!(or.data[0][0], GATE_HIGH);
        assert_eq!(xor.data[0][0], 0.0);
    }

    #[test]
    fn slew_limits_rate_of_change() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut slew = Slew::new();
        slew.rise.set_immediate(0.1);
        slew.fall.set_immediate(0.1);
        let mut input = PortBuffer::silent();
        input.data[0] = [5.0; BLOCK]; // step from 0 to 5
        let mut out = PortBuffer::silent();
        // First block: output must lag well below the target.
        slew.process(&ctx, Some(&input), &mut out, BLOCK);
        assert!(out.data[0][BLOCK - 1] < 4.0, "slew didn't limit: {}", out.data[0][BLOCK - 1]);
        // After enough time it reaches the target.
        for _ in 0..2000 {
            slew.process(&ctx, Some(&input), &mut out, BLOCK);
        }
        assert!((out.data[0][BLOCK - 1] - 5.0).abs() < 0.05);
    }

    #[test]
    fn sample_hold_captures_on_edges_only() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut snh = SampleHold::new();
        let mut input = PortBuffer::silent();
        let mut trig = PortBuffer::silent();
        let mut out = PortBuffer::silent();

        input.data[0] = [3.3; BLOCK];
        trig.data[0][4] = 10.0; // single-sample trigger
        snh.process(&ctx, Some(&input), Some(&trig), &mut out, BLOCK);
        assert_eq!(out.data[0][3], 0.0, "held before trigger");
        assert_eq!(out.data[0][5], 3.3, "captured at trigger");

        // Input changes but no new edge → value holds.
        input.data[0] = [-1.0; BLOCK];
        trig.data[0] = [0.0; BLOCK];
        snh.process(&ctx, Some(&input), Some(&trig), &mut out, BLOCK);
        assert_eq!(out.data[0][BLOCK - 1], 3.3);
    }

    #[test]
    fn attenuverter_scales_inverts_offsets() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut attn = Attenuverter::new();
        attn.gain.set_immediate(-0.5);
        attn.offset.set_immediate(2.0);
        let mut input = PortBuffer::silent();
        input.data[0] = [4.0; BLOCK];
        let mut out = PortBuffer::silent();
        attn.process(&ctx, Some(&input), &mut out, BLOCK);
        // 4 × -0.5 + 2 = 0.
        assert!((out.data[0][BLOCK - 1]).abs() < 1e-5, "got {}", out.data[0][BLOCK - 1]);
    }

    #[test]
    fn waveshaper_fold_brings_peaks_back_down() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut shaper = Waveshaper::new();
        shaper.set_param(wp::MODE, 1.0); // fold
        shaper.drive.set_immediate(3.0);
        shaper.mix.set_immediate(1.0);
        let mut input = PortBuffer::silent();
        input.data[0] = [AUDIO_PEAK; BLOCK]; // full-scale DC
        let mut out = PortBuffer::silent();
        shaper.process(&ctx, Some(&input), &mut out, BLOCK);
        // sin(3·π/2) = -1 → folded all the way to -5 V.
        assert!((out.data[0][BLOCK - 1] + AUDIO_PEAK).abs() < 0.01, "got {}", out.data[0][BLOCK - 1]);

        // Saturation mode stays bounded and same-signed.
        shaper.set_param(wp::MODE, 0.0);
        shaper.process(&ctx, Some(&input), &mut out, BLOCK);
        let y = out.data[0][BLOCK - 1];
        assert!(y > 4.0 && y <= AUDIO_PEAK + 0.01, "got {y}");
    }
}
