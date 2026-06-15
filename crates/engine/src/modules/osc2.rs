//! Extra oscillators: 2-operator FM, additive (stacked harmonics), and a
//! trigger-driven drum voice (kick / snare / hi-hat).

use crate::buffer::{PortBuffer, BLOCK, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{additive as ap, drum as dp, fmop as fp};
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK, GATE_THRESHOLD};
use rack_dsp::Smoothed;

#[inline]
fn sin01(phase: f32) -> f32 {
    (core::f32::consts::TAU * phase).sin()
}

// ---------------------------------------------------------------------------
// 2-operator FM
// ---------------------------------------------------------------------------

pub struct FmOp {
    carrier: [f32; MAX_CHANNELS],
    modulator: [f32; MAX_CHANNELS],
    last_mod: [f32; MAX_CHANNELS],
    pitch: Smoothed,
    ratio: Smoothed,
    index: Smoothed,
    feedback: Smoothed,
}

impl FmOp {
    pub fn new() -> Self {
        Self {
            carrier: [0.0; MAX_CHANNELS],
            modulator: [0.0; MAX_CHANNELS],
            last_mod: [0.0; MAX_CHANNELS],
            pitch: Smoothed::new(0.75),
            ratio: Smoothed::new(2.0),
            index: Smoothed::new(2.0),
            feedback: Smoothed::new(0.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            fp::PITCH => self.pitch.set_target(value),
            fp::RATIO => self.ratio.set_target(value.clamp(0.5, 12.0)),
            fp::INDEX => self.index.set_target(value.clamp(0.0, 10.0)),
            fp::FEEDBACK => self.feedback.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        fm: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;
        let mut pitch = [0.0f32; BLOCK];
        let mut ratio = [0.0f32; BLOCK];
        let mut index = [0.0f32; BLOCK];
        let mut fb = [0.0f32; BLOCK];
        for i in 0..frames {
            pitch[i] = self.pitch.tick(ctx.smooth_k);
            ratio[i] = self.ratio.tick(ctx.smooth_k);
            index[i] = self.index.tick(ctx.smooth_k);
            fb[i] = self.feedback.tick(ctx.smooth_k);
        }

        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            let fmcv = fm.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                let v = pitch[i] + cv.map_or(0.0, |c| c[i]) + fmcv.map_or(0.0, |c| c[i]) * 0.1;
                let carrier_hz = voct_to_hz(v);
                let car_dt = (carrier_hz * ctx.inv_sample_rate).min(0.45);
                let mod_dt = (carrier_hz * ratio[i] * ctx.inv_sample_rate).min(0.49);

                let m = sin01(self.modulator[ch] + fb[i] * self.last_mod[ch] * 0.5);
                self.last_mod[ch] = m;
                let c = sin01(self.carrier[ch] + index[i] * m * 0.15);
                data[i] = c * AUDIO_PEAK;

                self.modulator[ch] += mod_dt;
                if self.modulator[ch] >= 1.0 {
                    self.modulator[ch] -= 1.0;
                }
                self.carrier[ch] += car_dt;
                if self.carrier[ch] >= 1.0 {
                    self.carrier[ch] -= 1.0;
                }
            }
        }
    }
}

impl Default for FmOp {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Additive
// ---------------------------------------------------------------------------

pub struct Additive {
    phase: [f32; MAX_CHANNELS],
    pitch: Smoothed,
    partials: Smoothed,
    rolloff: Smoothed,
    odd_even: Smoothed,
}

impl Additive {
    pub fn new() -> Self {
        Self {
            phase: [0.0; MAX_CHANNELS],
            pitch: Smoothed::new(0.75),
            partials: Smoothed::new(8.0),
            rolloff: Smoothed::new(1.0),
            odd_even: Smoothed::new(0.5),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            ap::PITCH => self.pitch.set_target(value),
            ap::PARTIALS => self.partials.set_target(value.clamp(1.0, 16.0)),
            ap::ROLLOFF => self.rolloff.set_target(value.clamp(0.2, 3.0)),
            ap::ODD_EVEN => self.odd_even.set_target(value.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        voct: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = voct.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;
        let pitch = self.pitch.current();
        let n = self.partials.current().round() as usize;
        let rolloff = self.rolloff.current();
        let odd_even = self.odd_even.current();
        for _ in 0..frames {
            self.pitch.tick(ctx.smooth_k);
            self.partials.tick(ctx.smooth_k);
            self.rolloff.tick(ctx.smooth_k);
            self.odd_even.tick(ctx.smooth_k);
        }

        for ch in 0..channels as usize {
            let cv = voct.map(|b| b.channel_or_broadcast(ch));
            let phase = &mut self.phase[ch];
            let data = &mut out.data[ch];
            for i in 0..frames {
                let v = pitch + cv.map_or(0.0, |c| c[i]);
                let f0 = voct_to_hz(v);
                let dt = f0 * ctx.inv_sample_rate;
                let mut sum = 0.0;
                let mut norm = 0.0;
                for k in 1..=n.max(1) {
                    let hz = f0 * k as f32;
                    if hz > ctx.sample_rate * 0.45 {
                        break;
                    }
                    // Amplitude rolloff, with an odd/even harmonic balance.
                    let mut amp = 1.0 / (k as f32).powf(rolloff);
                    if k % 2 == 0 {
                        amp *= odd_even * 2.0;
                    } else {
                        amp *= (1.0 - odd_even) * 2.0;
                    }
                    sum += amp * sin01(*phase * k as f32);
                    norm += amp;
                }
                data[i] = if norm > 0.0 { sum / norm } else { 0.0 } * AUDIO_PEAK;
                *phase += dt;
                if *phase >= 1.0 {
                    *phase -= 1.0;
                }
            }
        }
    }
}

impl Default for Additive {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Drum voice
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrumKind {
    Kick,
    Snare,
    Hat,
}

pub struct Drum {
    kind: DrumKind,
    tune: f32,
    decay: f32,
    // Per-voice state (mono).
    env: f32,
    pitch_env: f32,
    phase: f32,
    rng: u32,
    trig_high: bool,
    accent_level: f32,
    hp_state: f32,
}

impl Drum {
    pub fn new() -> Self {
        Self {
            kind: DrumKind::Kick,
            tune: 0.0,
            decay: 0.3,
            env: 0.0,
            pitch_env: 0.0,
            phase: 0.0,
            rng: 0x1234_5678,
            trig_high: false,
            accent_level: 1.0,
            hp_state: 0.0,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            dp::KIND => {
                self.kind = match value as u32 {
                    1 => DrumKind::Snare,
                    2 => DrumKind::Hat,
                    _ => DrumKind::Kick,
                }
            }
            dp::TUNE => self.tune = value.clamp(-2.0, 2.0),
            dp::DECAY => self.decay = value.clamp(0.02, 2.0),
            _ => {}
        }
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        trig: Option<&PortBuffer>,
        accent: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let trig_data = trig.map(|b| b.channel_or_broadcast(0));
        let acc_data = accent.map(|b| b.channel_or_broadcast(0));
        // Envelope decay coefficient from the decay time.
        let amp_k = 1.0 - (-1.0 / (self.decay * ctx.sample_rate)).exp();
        // Kick pitch sweep is fast regardless of amp decay.
        let pitch_k = 1.0 - (-1.0 / (0.03 * ctx.sample_rate)).exp();

        for i in 0..frames {
            if let Some(t) = trig_data {
                let high = t[i] >= GATE_THRESHOLD;
                if high && !self.trig_high {
                    self.env = 1.0;
                    self.pitch_env = 1.0;
                    self.phase = 0.0;
                    self.accent_level =
                        0.6 + acc_data.map_or(0.4, |a| (a[i] / 10.0).clamp(0.0, 1.0) * 0.4);
                }
                self.trig_high = high;
            }

            let s = match self.kind {
                DrumKind::Kick => {
                    // Sine whose pitch drops from a tuned high to a low thud.
                    let base = 55.0 * 2f32.powf(self.tune);
                    let hz = base * (1.0 + self.pitch_env * 6.0);
                    let dt = hz * ctx.inv_sample_rate;
                    self.phase += dt;
                    if self.phase >= 1.0 {
                        self.phase -= 1.0;
                    }
                    self.pitch_env -= pitch_k * self.pitch_env;
                    sin01(self.phase) * self.env
                }
                DrumKind::Snare => {
                    // Noise + a tuned tone body.
                    let body_hz = 180.0 * 2f32.powf(self.tune);
                    self.phase += body_hz * ctx.inv_sample_rate;
                    if self.phase >= 1.0 {
                        self.phase -= 1.0;
                    }
                    let tone = sin01(self.phase) * 0.5;
                    let noise = self.noise();
                    (tone + noise) * self.env
                }
                DrumKind::Hat => {
                    // High-passed noise with a very short envelope.
                    let n = self.noise();
                    self.hp_state += 0.4 * (n - self.hp_state);
                    (n - self.hp_state) * self.env
                }
            };
            // Snare/hat decay faster than the amp_k base for snap.
            let decay_mul = match self.kind {
                DrumKind::Kick => 1.0,
                DrumKind::Snare => 1.5,
                DrumKind::Hat => 4.0,
            };
            self.env -= (amp_k * decay_mul).min(1.0) * self.env;

            out.data[0][i] = s * self.accent_level * AUDIO_PEAK;
        }
    }
}

impl Default for Drum {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|&s| (s * s) as f64).sum::<f64>() / buf.len() as f64).sqrt() as f32
    }

    #[test]
    fn fm_index_adds_harmonics() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut fm = FmOp::new();
        fm.pitch.set_immediate(0.0);
        let mut out = PortBuffer::silent();

        // Helper: measure first-difference energy (a crude brightness proxy).
        let brightness = |fm: &mut FmOp, out: &mut PortBuffer| {
            let mut diff = 0.0f64;
            let mut total = 0.0f64;
            let mut last = 0.0f32;
            for _ in 0..500 {
                fm.process(&ctx, None, None, out, BLOCK);
                for &s in &out.data[0][..BLOCK] {
                    assert!(s.is_finite());
                    diff += ((s - last) as f64).powi(2);
                    total += (s as f64).powi(2);
                    last = s;
                }
            }
            diff / total.max(1e-9)
        };
        fm.index.set_immediate(0.0);
        let clean = brightness(&mut fm, &mut out);
        fm.index.set_immediate(8.0);
        let bright = brightness(&mut fm, &mut out);
        assert!(bright > clean * 1.3, "FM index didn't add harmonics: {clean} -> {bright}");
    }

    #[test]
    fn additive_partials_increase_brightness() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut add = Additive::new();
        add.pitch.set_immediate(-1.0); // low so many partials fit
        let mut out = PortBuffer::silent();
        let bright = |add: &mut Additive, out: &mut PortBuffer| {
            let mut diff = 0.0f64;
            let mut total = 0.0f64;
            let mut last = 0.0f32;
            for _ in 0..500 {
                add.process(&ctx, None, out, BLOCK);
                for &s in &out.data[0][..BLOCK] {
                    assert!(s.is_finite() && s.abs() <= AUDIO_PEAK + 0.1);
                    diff += ((s - last) as f64).powi(2);
                    total += (s as f64).powi(2);
                    last = s;
                }
            }
            diff / total.max(1e-9)
        };
        add.partials.set_immediate(1.0);
        let one = bright(&mut add, &mut out);
        add.partials.set_immediate(16.0);
        let many = bright(&mut add, &mut out);
        assert!(many > one * 1.3, "more partials should be brighter: {one} -> {many}");
    }

    #[test]
    fn drum_triggers_and_decays() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut drum = Drum::new();
        drum.set_param(dp::DECAY, 0.1);
        let mut out = PortBuffer::silent();

        // Silent until triggered.
        let silent = PortBuffer::silent();
        drum.process(&ctx, Some(&silent), None, &mut out, BLOCK);
        assert!(rms(&out.data[0][..BLOCK]) < 1e-4);

        // Trigger: loud onset, then decays toward silence.
        let mut trig = PortBuffer::silent();
        trig.data[0][0] = 10.0;
        drum.process(&ctx, Some(&trig), None, &mut out, BLOCK);
        let onset = rms(&out.data[0][..BLOCK]);
        assert!(onset > 0.1, "drum silent on trigger: {onset}");

        let zero = PortBuffer::silent();
        // 0.1 s decay time constant → ~0.4 s (≈4 τ) to fall below 5%.
        for _ in 0..600 {
            drum.process(&ctx, Some(&zero), None, &mut out, BLOCK);
        }
        assert!(rms(&out.data[0][..BLOCK]) < onset * 0.05, "drum didn't decay");
    }

    #[test]
    fn drum_kinds_differ() {
        let ctx = ProcessCtx::new(48_000.0);
        let render = |kind: u32| {
            let mut drum = Drum::new();
            drum.set_param(dp::KIND, kind as f32);
            let mut out = PortBuffer::silent();
            let mut trig = PortBuffer::silent();
            trig.data[0][0] = 10.0;
            drum.process(&ctx, Some(&trig), None, &mut out, BLOCK);
            // High-frequency content proxy.
            let mut diff = 0.0f64;
            let mut last = 0.0f32;
            for &s in &out.data[0][..BLOCK] {
                diff += ((s - last) as f64).abs();
                last = s;
            }
            diff
        };
        // Hi-hat is far brighter (noisier) than kick.
        assert!(render(2) > render(0) * 2.0, "hat not brighter than kick");
    }
}
