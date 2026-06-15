//! polyBLEP anti-aliasing residual for discontinuous oscillator waveforms.

/// Two-sample polyBLEP residual. `t` is the phase in [0, 1), `dt` is the
/// per-sample phase increment. Add the residual at falling edges, subtract
/// at rising edges (or fold the sign into the caller's waveform math).
#[inline]
pub fn polyblep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// Naive saw in [-1, 1] with polyBLEP correction at the wrap discontinuity.
#[inline]
pub fn saw_blep(phase: f32, dt: f32) -> f32 {
    2.0 * phase - 1.0 - polyblep(phase, dt)
}

/// Square/pulse in [-1, 1] with polyBLEP at both edges. `pw` is pulse width
/// in (0, 1); callers should clamp to something like [0.05, 0.95].
#[inline]
pub fn square_blep(phase: f32, dt: f32, pw: f32) -> f32 {
    let raw = if phase < pw { 1.0 } else { -1.0 };
    // Rising edge at phase 0, falling edge at phase pw.
    let falling = {
        let shifted = phase - pw;
        let shifted = if shifted < 0.0 { shifted + 1.0 } else { shifted };
        polyblep(shifted, dt)
    };
    raw + polyblep(phase, dt) - falling
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustfft::{num_complex::Complex, FftPlanner};

    /// Worst alias level (dB relative to the fundamental) among bins below
    /// `band_hz`. Two-point polyBLEP leaves folded partials parked near
    /// Nyquist at modest attenuation — that's expected and inaudible; what
    /// must be clean is the audible band well below Nyquist.
    fn alias_floor_db(samples: &[f32], f0: f32, sr: f32, band_hz: f32) -> f32 {
        let n = samples.len();
        // FFT with a Hann window; compare the strongest non-harmonic bin
        // against the fundamental.
        let mut buf: Vec<Complex<f32>> = (0..n)
            .map(|i| {
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
                Complex::new(samples[i] * w, 0.0)
            })
            .collect();
        FftPlanner::new().plan_fft_forward(n).process(&mut buf);
        let mag: Vec<f32> = buf[..n / 2].iter().map(|c| c.norm()).collect();

        let bin_hz = sr / n as f32;
        let fund_bin = (f0 / bin_hz).round() as usize;
        let fund = mag[fund_bin - 2..=fund_bin + 2].iter().cloned().fold(0.0, f32::max);

        let mut worst_alias: f32 = 0.0;
        for (i, &m) in mag.iter().enumerate().skip(4) {
            let hz = i as f32 * bin_hz;
            if hz > band_hz {
                break;
            }
            let harmonic_index = hz / f0;
            let near_harmonic = (harmonic_index - harmonic_index.round()).abs() < 0.05;
            if !near_harmonic && m > worst_alias {
                worst_alias = m;
            }
        }
        20.0 * (worst_alias / fund).log10()
    }

    fn render(f0: f32, sr: f32, wave: impl Fn(f32, f32) -> f32) -> Vec<f32> {
        let dt = f0 / sr;
        let mut phase = 0.0f32;
        (0..16384)
            .map(|_| {
                let s = wave(phase, dt);
                phase += dt;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                s
            })
            .collect()
    }

    #[test]
    fn saw_in_band_aliases_below_minus_50db() {
        let sr = 48_000.0;
        // Awkward non-bin-aligned high pitch (C6-ish).
        let f0 = 1_046.5;
        let samples = render(f0, sr, saw_blep);
        let floor = alias_floor_db(&samples, f0, sr, 10_000.0);
        assert!(floor < -50.0, "in-band alias floor was {floor} dB");
    }

    #[test]
    fn square_in_band_aliases_below_minus_50db() {
        let sr = 48_000.0;
        let f0 = 1_046.5;
        let samples = render(f0, sr, |p, dt| square_blep(p, dt, 0.5));
        let floor = alias_floor_db(&samples, f0, sr, 10_000.0);
        assert!(floor < -50.0, "in-band alias floor was {floor} dB");
    }

    #[test]
    fn naive_saw_fails_where_blep_passes() {
        // Sanity-check the measurement itself: the uncorrected saw must be
        // dramatically dirtier in the same band.
        let sr = 48_000.0;
        let f0 = 1_046.5;
        let naive = render(f0, sr, |p, _| 2.0 * p - 1.0);
        let floor = alias_floor_db(&naive, f0, sr, 10_000.0);
        assert!(floor > -40.0, "naive saw unexpectedly clean: {floor} dB");
    }

    #[test]
    fn saw_is_bounded() {
        let dt = 440.0 / 48_000.0;
        let mut phase = 0.0f32;
        for _ in 0..48_000 {
            let s = saw_blep(phase, dt);
            assert!(s.abs() <= 1.3, "saw out of range: {s}");
            phase += dt;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
    }
}
