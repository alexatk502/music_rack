//! Quantizer: snaps each channel's V/oct input to the nearest note of a
//! chosen scale (chromatic / major / minor / pentatonic), relative to a root.
//! Per-octave masks make this a simple search over the 12 semitones.

use crate::buffer::{propagate_channels, PortBuffer};
use crate::ProcessCtx;
use rack_core::modules::params::quantizer as p;

/// Semitone membership masks (bit i set = semitone i is in the scale).
/// Written most-significant-nibble first: bits 11..8 | 7..4 | 3..0.
const SCALES: [u16; 4] = [
    0b1111_1111_1111, // chromatic
    0b1010_1011_0101, // major (0,2,4,5,7,9,11)
    0b0101_1010_1101, // natural minor (0,2,3,5,7,8,10)
    0b0100_1010_1001, // minor pentatonic (0,3,5,7,10)
];

pub struct Quantizer {
    scale: usize,
    root: i32,
}

impl Quantizer {
    pub fn new() -> Self {
        Self { scale: 1, root: 0 }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::SCALE => self.scale = (value as usize).min(SCALES.len() - 1),
            p::ROOT => self.root = (value as i32).rem_euclid(12),
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
        let mask = SCALES[self.scale];

        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let data = &mut out.data[ch];
            for i in 0..frames {
                let v = in_data.map_or(0.0, |d| d[i]);
                data[i] = quantize(v, mask, self.root);
            }
        }
    }
}

impl Default for Quantizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Snap a V/oct value to the nearest in-scale semitone. 1 V = 12 semitones.
#[inline]
fn quantize(volts: f32, mask: u16, root: i32) -> f32 {
    let semis = volts * 12.0;
    let center = semis.round() as i32;
    // Scan a window wide enough to span the largest scale gap (3 semitones),
    // and keep the candidate genuinely closest to the input — not just the
    // first found, which biases low when the input sits between two tones.
    let mut best: Option<i32> = None;
    let mut best_dist = f32::INFINITY;
    for s in (center - 3)..=(center + 3) {
        let pc = (s - root).rem_euclid(12) as u16;
        if mask & (1 << pc) != 0 {
            let dist = (semis - s as f32).abs();
            if dist < best_dist {
                best_dist = dist;
                best = Some(s);
            }
        }
    }
    best.map_or(volts, |s| s as f32 / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn run(scale: f32, root: f32, input_v: f32) -> f32 {
        let ctx = ProcessCtx::new(48_000.0);
        let mut q = Quantizer::new();
        q.set_param(p::SCALE, scale);
        q.set_param(p::ROOT, root);
        let mut inb = PortBuffer::silent();
        inb.data[0] = [input_v; BLOCK];
        let mut out = PortBuffer::silent();
        q.process(&ctx, Some(&inb), &mut out, BLOCK);
        out.data[0][BLOCK - 1]
    }

    #[test]
    fn major_scale_snaps_to_scale_tones() {
        // C major, input near D# (3 semitones = 0.25 V) snaps to E (4) or D (2).
        let out = run(1.0, 0.0, 3.4 / 12.0);
        let semis = (out * 12.0).round() as i32;
        assert!(semis == 4, "expected E(4), got {semis}");
        // Input near F (5 semis = exact scale tone) stays.
        let out = run(1.0, 0.0, 5.0 / 12.0);
        assert_eq!((out * 12.0).round() as i32, 5);
    }

    #[test]
    fn chromatic_rounds_to_nearest_semitone() {
        let out = run(0.0, 0.0, 3.4 / 12.0);
        assert_eq!((out * 12.0).round() as i32, 3);
    }

    #[test]
    fn output_is_always_in_scale() {
        // Sweep a range; every output semitone must be a major-scale degree.
        let major = [0, 2, 4, 5, 7, 9, 11];
        for milli in -1200..1200 {
            let out = run(1.0, 0.0, milli as f32 / 1200.0);
            let pc = ((out * 12.0).round() as i32).rem_euclid(12);
            assert!(major.contains(&pc), "out pitch class {pc} not in C major");
        }
    }
}
