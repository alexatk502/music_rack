//! Distortion and dynamics: a multi-mode drive, a transient shaper, a
//! sidechain ducker, and a noise gate.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_dsp::volts::AUDIO_PEAK;
use rack_dsp::{tanh_pade, Smoothed};

// ---------------------------------------------------------------------------
// Drive: tube / diode / fuzz / foldback distortion with tone and mix.
// params: 0 type, 1 drive, 2 tone, 3 mix
// ---------------------------------------------------------------------------

pub struct Drive {
    lp: [f32; MAX_CHANNELS],
    kind: u32,
    drive: Smoothed,
    tone: Smoothed,
    mix: Smoothed,
}

impl Drive {
    pub fn new() -> Self {
        Self {
            lp: [0.0; MAX_CHANNELS],
            kind: 0,
            drive: Smoothed::new(4.0),
            tone: Smoothed::new(0.5),
            mix: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.kind = (value as u32).min(3),
            1 => self.drive.set_target(value.clamp(1.0, 30.0)),
            2 => self.tone.set_target(value.clamp(0.0, 1.0)),
            3 => self.mix.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    #[inline]
    fn shape(kind: u32, x: f32) -> f32 {
        match kind {
            // Diode: asymmetric clip.
            1 => {
                if x >= 0.0 {
                    tanh_pade(x)
                } else {
                    tanh_pade(x * 0.6) * 0.8
                }
            }
            // Fuzz: hard sigmoid with a little squared grit.
            2 => {
                let y = x / (1.0 + x.abs());
                (y * 1.3 + y * y * y * 0.2).clamp(-1.0, 1.0)
            }
            // Foldback.
            3 => {
                let mut v = x;
                for _ in 0..4 {
                    if v > 1.0 {
                        v = 2.0 - v;
                    } else if v < -1.0 {
                        v = -2.0 - v;
                    } else {
                        break;
                    }
                }
                v
            }
            // Tube: symmetric soft clip.
            _ => tanh_pade(x),
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
        let kind = self.kind;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let drive = self.drive.tick(ctx.smooth_k);
                let tone = self.tone.tick(ctx.smooth_k);
                let mix = self.mix.tick(ctx.smooth_k);
                let dry = in_data.map_or(0.0, |d| d[i]);
                let shaped = Self::shape(kind, dry / AUDIO_PEAK * drive);
                // Tone: blend full signal with a low-passed copy.
                let lp_k = 0.05 + tone * 0.9;
                self.lp[ch] += lp_k * (shaped - self.lp[ch]);
                let toned = self.lp[ch] + (shaped - self.lp[ch]) * tone;
                let wet = toned * AUDIO_PEAK;
                out.data[ch][i] = dry + (wet - dry) * mix;
            }
        }
    }
}

impl Default for Drive {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Transient Shaper: independent attack / sustain gain from a fast vs slow
// envelope difference.
// params: 0 attack (-1..1), 1 sustain (-1..1)
// ---------------------------------------------------------------------------

pub struct Transient {
    fast: [f32; MAX_CHANNELS],
    slow: [f32; MAX_CHANNELS],
    attack: Smoothed,
    sustain: Smoothed,
}

impl Transient {
    pub fn new() -> Self {
        Self {
            fast: [0.0; MAX_CHANNELS],
            slow: [0.0; MAX_CHANNELS],
            attack: Smoothed::new(0.0),
            sustain: Smoothed::new(0.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.attack.set_target(value.clamp(-1.0, 1.0)),
            1 => self.sustain.set_target(value.clamp(-1.0, 1.0)),
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
        let kf = 1.0 - (-1.0 / (0.002 * ctx.sample_rate)).exp(); // fast ~2 ms
        let ks = 1.0 - (-1.0 / (0.050 * ctx.sample_rate)).exp(); // slow ~50 ms
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let attack = self.attack.tick(ctx.smooth_k);
                let sustain = self.sustain.tick(ctx.smooth_k);
                let x = in_data.map_or(0.0, |d| d[i]);
                let rect = x.abs();
                self.fast[ch] += kf * (rect - self.fast[ch]);
                self.slow[ch] += ks * (rect - self.slow[ch]);
                let d = self.fast[ch] - self.slow[ch];
                // Positive d = onset; negative d = the decaying body.
                let onset = (d / AUDIO_PEAK * 6.0).clamp(0.0, 1.0);
                let body = (-d / AUDIO_PEAK * 6.0).clamp(0.0, 1.0);
                let g = (1.0 + attack * onset + sustain * body).clamp(0.0, 4.0);
                out.data[ch][i] = x * g;
            }
        }
    }
}

impl Default for Transient {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Sidechain Ducker: the key input's level reduces the main signal's gain.
// params: 0 amount, 1 attack, 2 release
// ---------------------------------------------------------------------------

pub struct Ducker {
    env: [f32; MAX_CHANNELS],
    amount: Smoothed,
    attack: Smoothed,
    release: Smoothed,
}

impl Ducker {
    pub fn new() -> Self {
        Self {
            env: [0.0; MAX_CHANNELS],
            amount: Smoothed::new(0.8),
            attack: Smoothed::new(0.01),
            release: Smoothed::new(0.2),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.amount.set_target(value.clamp(0.0, 1.0)),
            1 => self.attack.set_target(value.clamp(0.001, 0.1)),
            2 => self.release.set_target(value.clamp(0.02, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        key: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, key]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let key_data = key.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let amount = self.amount.tick(ctx.smooth_k);
                let attack = self.attack.tick(ctx.smooth_k);
                let release = self.release.tick(ctx.smooth_k);
                let atk = 1.0 - (-1.0 / (attack * ctx.sample_rate)).exp();
                let rel = 1.0 - (-1.0 / (release * ctx.sample_rate)).exp();
                let k = (key_data.map_or(0.0, |d| d[i]) / AUDIO_PEAK).abs();
                let c = if k > self.env[ch] { atk } else { rel };
                self.env[ch] += c * (k - self.env[ch]);
                let gain = (1.0 - amount * self.env[ch].min(1.0)).max(0.0);
                out.data[ch][i] = in_data.map_or(0.0, |d| d[i]) * gain;
            }
        }
    }
}

impl Default for Ducker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Noise Gate: opens above a threshold, closes (after a hold) below it.
// params: 0 threshold (dB), 1 attack, 2 release, 3 hold
// ---------------------------------------------------------------------------

pub struct Gate {
    env: [f32; MAX_CHANNELS],
    gain: [f32; MAX_CHANNELS],
    held: [f32; MAX_CHANNELS],
    thresh: Smoothed,
    attack: Smoothed,
    release: Smoothed,
    hold: Smoothed,
}

impl Gate {
    pub fn new() -> Self {
        Self {
            env: [0.0; MAX_CHANNELS],
            gain: [0.0; MAX_CHANNELS],
            held: [0.0; MAX_CHANNELS],
            thresh: Smoothed::new(-40.0),
            attack: Smoothed::new(0.002),
            release: Smoothed::new(0.1),
            hold: Smoothed::new(0.05),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.thresh.set_target(value.clamp(-60.0, 0.0)),
            1 => self.attack.set_target(value.clamp(0.0005, 0.05)),
            2 => self.release.set_target(value.clamp(0.01, 0.5)),
            3 => self.hold.set_target(value.clamp(0.0, 0.5)),
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
        let det = 1.0 - (-1.0 / (0.003 * ctx.sample_rate)).exp();
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let thresh = self.thresh.tick(ctx.smooth_k);
                let attack = self.attack.tick(ctx.smooth_k);
                let release = self.release.tick(ctx.smooth_k);
                let hold = self.hold.tick(ctx.smooth_k);
                let x = in_data.map_or(0.0, |d| d[i]);
                let level = (x / AUDIO_PEAK).abs();
                self.env[ch] += det * (level - self.env[ch]);
                let thr = 10.0f32.powf(thresh / 20.0);
                let open = self.env[ch] >= thr;
                if open {
                    self.held[ch] = hold;
                } else if self.held[ch] > 0.0 {
                    self.held[ch] -= ctx.inv_sample_rate;
                }
                let want = if open || self.held[ch] > 0.0 { 1.0 } else { 0.0 };
                let k = if want > self.gain[ch] {
                    1.0 - (-1.0 / (attack * ctx.sample_rate)).exp()
                } else {
                    1.0 - (-1.0 / (release * ctx.sample_rate)).exp()
                };
                self.gain[ch] += k * (want - self.gain[ch]);
                out.data[ch][i] = x * self.gain[ch];
            }
        }
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn ramp_buf(level: f32) -> PortBuffer {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        for (i, s) in b.data[0].iter_mut().enumerate() {
            *s = ((i as f32 / BLOCK as f32) - 0.5) * 2.0 * level;
        }
        b
    }

    #[test]
    fn dynamics_stay_finite() {
        let ctx = ProcessCtx::new(48_000.0);
        let inp = ramp_buf(AUDIO_PEAK);
        let key = ramp_buf(AUDIO_PEAK * 0.8);
        for kind in 0..4 {
            let mut drive = Drive::new();
            drive.set_param(0, kind as f32);
            drive.set_param(1, 30.0);
            let mut out = PortBuffer::silent();
            for _ in 0..50 {
                drive.process(&ctx, Some(&inp), &mut out, BLOCK);
            }
            for &s in out.data[0][..BLOCK].iter() {
                assert!(s.is_finite() && s.abs() < 100.0, "drive {kind} blew up: {s}");
            }
        }
        let mut tr = Transient::new();
        let mut duck = Ducker::new();
        let mut gate = Gate::new();
        tr.set_param(0, 1.0);
        for _ in 0..50 {
            let mut out = PortBuffer::silent();
            tr.process(&ctx, Some(&inp), &mut out, BLOCK);
            duck.process(&ctx, Some(&inp), Some(&key), &mut out, BLOCK);
            gate.process(&ctx, Some(&inp), &mut out, BLOCK);
            for &s in out.data[0][..BLOCK].iter() {
                assert!(s.is_finite() && s.abs() < 100.0, "dynamics blew up: {s}");
            }
        }
    }
}
