//! Arpeggiator: collects held notes (fed the same note events as the note
//! input), then steps through them on each clock pulse — up / down / up-down
//! / random — across a configurable octave range.

use crate::buffer::PortBuffer;
use crate::ProcessCtx;
use rack_core::modules::params::arp as p;
use rack_dsp::volts::{GATE_HIGH, GATE_THRESHOLD};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Up,
    Down,
    UpDown,
    Random,
}

pub struct Arp {
    /// Bitmask of currently-held MIDI notes (0..128).
    held: u128,
    mode: Mode,
    octaves: u32,
    gate_len: f32,
    /// Flattened sequence of notes for the current pattern (rebuilt on edit).
    step: usize,
    up: bool,
    rng: u32,
    cur_voct: f32,
    clock_high: bool,
    reset_high: bool,
    // Period measurement for staccato gates.
    since_edge: f32,
    period: f32,
    gate_timer: f32,
}

impl Arp {
    pub fn new() -> Self {
        Self {
            held: 0,
            mode: Mode::Up,
            octaves: 1,
            gate_len: 0.5,
            step: 0,
            up: true,
            rng: 0x2545_f491,
            cur_voct: 0.0,
            clock_high: false,
            reset_high: false,
            since_edge: 0.0,
            period: 12_000.0,
            gate_timer: 0.0,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            p::MODE => {
                self.mode = match value as u32 {
                    1 => Mode::Down,
                    2 => Mode::UpDown,
                    3 => Mode::Random,
                    _ => Mode::Up,
                }
            }
            p::OCTAVES => self.octaves = (value as u32 + 1).clamp(1, 4),
            p::GATE_LEN => self.gate_len = value.clamp(0.05, 0.95),
            _ => {}
        }
    }

    pub fn note_on(&mut self, note: u8) {
        if note < 128 {
            self.held |= 1u128 << note;
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if note < 128 {
            self.held &= !(1u128 << note);
        }
    }

    pub fn all_notes_off(&mut self) {
        self.held = 0;
    }

    /// Number of held notes.
    fn count(&self) -> u32 {
        self.held.count_ones()
    }

    /// The `idx`-th held note (ascending), or None.
    fn nth_note(&self, idx: usize) -> Option<u8> {
        let mut seen = 0;
        for n in 0..128u8 {
            if self.held & (1u128 << n) != 0 {
                if seen == idx {
                    return Some(n);
                }
                seen += 1;
            }
        }
        None
    }

    #[inline]
    fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// Advance to the next note in the pattern and latch its v/oct.
    fn advance(&mut self) {
        let n = self.count() as usize;
        if n == 0 {
            return;
        }
        let total = n * self.octaves as usize;
        let pos = match self.mode {
            Mode::Up => {
                let p = self.step % total;
                self.step = self.step.wrapping_add(1);
                p
            }
            Mode::Down => {
                let p = total - 1 - (self.step % total);
                self.step = self.step.wrapping_add(1);
                p
            }
            Mode::UpDown => {
                // Triangle traversal across the full range.
                let span = (total * 2 - 2).max(1);
                let t = self.step % span;
                self.step = self.step.wrapping_add(1);
                if t < total {
                    t
                } else {
                    span - t
                }
            }
            Mode::Random => (self.rand() as usize) % total,
        };
        let octave = pos / n;
        let within = pos % n;
        if let Some(note) = self.nth_note(within) {
            self.cur_voct = (note as f32 - 60.0) / 12.0 + octave as f32;
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
            self.since_edge += 1.0;
            if let Some(r) = reset_data {
                let high = r[i] >= GATE_THRESHOLD;
                if high && !self.reset_high {
                    self.step = 0;
                    self.up = true;
                }
                self.reset_high = high;
            }
            if let Some(c) = clock_data {
                let high = c[i] >= GATE_THRESHOLD;
                if high && !self.clock_high {
                    if self.since_edge > 1.0 {
                        self.period = self.since_edge;
                    }
                    self.since_edge = 0.0;
                    if self.count() > 0 {
                        self.advance();
                        self.gate_timer = self.period * self.gate_len;
                    }
                }
                self.clock_high = high;
            }

            voct.data[0][i] = self.cur_voct;
            gate.data[0][i] = if self.gate_timer > 0.0 && self.count() > 0 { GATE_HIGH } else { 0.0 };
            self.gate_timer = (self.gate_timer - 1.0).max(0.0);
        }
    }
}

impl Default for Arp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    /// Clock the arp once (high block then low block) and return the latched
    /// v/oct as a rounded semitone offset from C4.
    fn clock_once(arp: &mut Arp) -> i32 {
        let ctx = ProcessCtx::new(48_000.0);
        let mut voct = PortBuffer::silent();
        let mut gate = PortBuffer::silent();
        let mut hi = PortBuffer::silent();
        hi.data[0] = [10.0; BLOCK];
        arp.process(&ctx, Some(&hi), None, &mut voct, &mut gate, BLOCK);
        let semis = (voct.data[0][BLOCK - 1] * 12.0).round() as i32;
        let lo = PortBuffer::silent();
        arp.process(&ctx, Some(&lo), None, &mut voct, &mut gate, BLOCK);
        semis
    }

    #[test]
    fn up_mode_ascends_held_notes() {
        let mut arp = Arp::new();
        arp.set_param(p::MODE, 0.0);
        arp.set_param(p::OCTAVES, 0.0); // 1 octave
        // Hold C(60), E(64), G(67) → semitone offsets 0, 4, 7.
        arp.note_on(60);
        arp.note_on(64);
        arp.note_on(67);
        let seq: Vec<i32> = (0..4).map(|_| clock_once(&mut arp)).collect();
        assert_eq!(seq, vec![0, 4, 7, 0], "up arp wrong: {seq:?}");
    }

    #[test]
    fn down_mode_descends() {
        let mut arp = Arp::new();
        arp.set_param(p::MODE, 1.0);
        arp.set_param(p::OCTAVES, 0.0);
        arp.note_on(60);
        arp.note_on(64);
        arp.note_on(67);
        let seq: Vec<i32> = (0..3).map(|_| clock_once(&mut arp)).collect();
        assert_eq!(seq, vec![7, 4, 0], "down arp wrong: {seq:?}");
    }

    #[test]
    fn octaves_extend_range() {
        let mut arp = Arp::new();
        arp.set_param(p::MODE, 0.0);
        arp.set_param(p::OCTAVES, 1.0); // 2 octaves
        arp.note_on(60);
        arp.note_on(64);
        let seq: Vec<i32> = (0..5).map(|_| clock_once(&mut arp)).collect();
        // C, E, then the same an octave up, then wrap.
        assert_eq!(seq, vec![0, 4, 12, 16, 0], "octave arp wrong: {seq:?}");
    }

    #[test]
    fn no_notes_means_no_gate() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut arp = Arp::new();
        let mut voct = PortBuffer::silent();
        let mut gate = PortBuffer::silent();
        let mut hi = PortBuffer::silent();
        hi.data[0] = [10.0; BLOCK];
        arp.process(&ctx, Some(&hi), None, &mut voct, &mut gate, BLOCK);
        assert_eq!(gate.data[0][BLOCK - 1], 0.0, "gate fired with no held notes");
    }
}
