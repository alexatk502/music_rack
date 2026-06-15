//! VCF: trapezoidal SVF (Simper) with LP/BP/HP modes, V/oct cutoff CV, and
//! tanh input drive. Cutoff knob is stored in volts relative to C4 so the CV
//! input adds directly (keyboard tracking is one cable).

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::vcf as p;
use rack_dsp::svf::{Svf, SvfCoeffs, SvfOut};
use rack_dsp::tanh_pade;
use rack_dsp::volts::{voct_to_hz, AUDIO_PEAK};
use rack_dsp::Smoothed;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    LowPass,
    BandPass,
    HighPass,
}

pub struct Vcf {
    svf: [Svf; MAX_CHANNELS],
    cutoff_v: Smoothed,
    res: Smoothed,
    drive: Smoothed,
    mode: Mode,
}

impl Vcf {
    pub fn new() -> Self {
        Self {
            svf: [Svf::default(); MAX_CHANNELS],
            cutoff_v: Smoothed::new(3.0), // C4 + 3 oct ≈ 2.1 kHz
            res: Smoothed::new(0.7071),
            drive: Smoothed::new(1.0),
            mode: Mode::LowPass,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::CUTOFF => self.cutoff_v.set_target(value),
            p::RES => self.res.set_target(value.clamp(0.5, 10.0)),
            p::MODE => {
                self.mode = match value as u32 {
                    1 => Mode::BandPass,
                    2 => Mode::HighPass,
                    _ => Mode::LowPass,
                }
            }
            p::DRIVE => self.drive.set_target(value.clamp(0.2, 4.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        cutoff_cv: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, cutoff_cv]);
        out.channels = channels;

        // Knobs smooth once per block; coefficients update per block per
        // channel (one tan each — audio-rate cutoff FM is quantized to
        // blocks, which is fine for an MVP filter).
        let mut cutoff_v = self.cutoff_v.current();
        let mut res = self.res.current();
        let mut drive = self.drive.current();
        for _ in 0..frames {
            cutoff_v = self.cutoff_v.tick(ctx.smooth_k);
            res = self.res.tick(ctx.smooth_k);
            drive = self.drive.tick(ctx.smooth_k);
        }
        let mode = self.mode;
        let inv_peak_drive = drive / AUDIO_PEAK;

        for ch in 0..channels as usize {
            let cv = cutoff_cv.map_or(0.0, |b| b.channel_or_broadcast(ch)[0]);
            let coeffs = SvfCoeffs::new(voct_to_hz(cutoff_v + cv), res, ctx.sample_rate);
            let svf = &mut self.svf[ch];
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                let x = in_data.map_or(0.0, |d| d[i]);
                let v0 = tanh_pade(x * inv_peak_drive) * AUDIO_PEAK;
                let SvfOut { lp, bp, hp } = svf.tick(v0, &coeffs);
                data[i] = match mode {
                    Mode::LowPass => lp,
                    Mode::BandPass => bp,
                    Mode::HighPass => hp,
                };
            }
            svf.undenorm();
        }
    }
}

impl Default for Vcf {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    #[test]
    fn lowpass_attenuates_a_bright_saw() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut vcf = Vcf::new();
        vcf.set_param(p::CUTOFF, -2.0); // ~65 Hz cutoff: kill most of the saw
        vcf.cutoff_v.set_immediate(-2.0);

        let mut input = PortBuffer::silent();
        let mut out = PortBuffer::silent();
        let mut phase = 0.0f32;
        let dt = 440.0 / 48_000.0;

        let mut in_energy = 0.0f64;
        let mut out_energy = 0.0f64;
        for block in 0..1000 {
            for i in 0..BLOCK {
                input.data[0][i] = (2.0 * phase - 1.0) * AUDIO_PEAK;
                phase += dt;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
            }
            vcf.process(&ctx, Some(&input), None, &mut out, BLOCK);
            if block > 100 {
                for i in 0..BLOCK {
                    in_energy += (input.data[0][i] as f64).powi(2);
                    out_energy += (out.data[0][i] as f64).powi(2);
                }
            }
        }
        let ratio = (out_energy / in_energy).sqrt();
        assert!(ratio < 0.15, "lowpass passed too much: {ratio}");
    }

    #[test]
    fn output_stays_finite_at_high_resonance() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut vcf = Vcf::new();
        vcf.set_param(p::RES, 10.0);
        vcf.set_param(p::DRIVE, 4.0);

        let mut input = PortBuffer::silent();
        input.data[0] = [AUDIO_PEAK; BLOCK];
        let mut out = PortBuffer::silent();
        for _ in 0..2000 {
            vcf.process(&ctx, Some(&input), None, &mut out, BLOCK);
            for &s in out.data[0].iter() {
                assert!(s.is_finite());
                assert!(s.abs() < 100.0, "filter blew up: {s}");
            }
        }
    }
}
