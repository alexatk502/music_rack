//! Note input: turns note on/off events into polyphonic V/oct, gate,
//! velocity, and retrigger outputs. The polyphony param sets the channel
//! count at the source of the poly chain.

use crate::buffer::{PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::note_in as p;
use rack_dsp::volts::GATE_HIGH;

/// Retrigger pulse length (~1 ms at 48 kHz, in frames).
const RETRIG_FRAMES: u32 = 48;

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    velocity: f32,
    gate: bool,
    /// Monotonic press order; lowest = oldest = steal target.
    order: u64,
    retrig_remaining: u32,
}

pub struct NoteIn {
    polyphony: u8,
    voices: [Voice; MAX_CHANNELS],
    counter: u64,
}

impl NoteIn {
    pub fn new() -> Self {
        Self { polyphony: 1, voices: [Voice::default(); MAX_CHANNELS], counter: 0 }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == p::POLYPHONY {
            let new = (value as u8).clamp(1, MAX_CHANNELS as u8);
            if new != self.polyphony {
                self.polyphony = new;
                // Lanes beyond the new count fall silent.
                for v in &mut self.voices[new as usize..] {
                    v.gate = false;
                }
            }
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        self.counter += 1;
        let lanes = &mut self.voices[..self.polyphony as usize];

        // Same note retriggers its existing lane.
        let lane = if let Some(v) = lanes.iter_mut().find(|v| v.note == note && v.gate) {
            v
        } else if let Some(v) = lanes
            .iter_mut()
            .filter(|v| !v.gate)
            .min_by_key(|v| v.order)
        {
            // Oldest released lane.
            v
        } else {
            // Steal the oldest held note.
            lanes.iter_mut().min_by_key(|v| v.order).expect("polyphony >= 1")
        };
        lane.note = note;
        lane.velocity = velocity as f32 / 127.0;
        lane.gate = true;
        lane.order = self.counter;
        lane.retrig_remaining = RETRIG_FRAMES;
    }

    pub fn note_off(&mut self, note: u8) {
        for v in &mut self.voices[..self.polyphony as usize] {
            if v.note == note && v.gate {
                v.gate = false;
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for v in &mut self.voices {
            v.gate = false;
            v.retrig_remaining = 0;
        }
    }

    /// Outputs: [v/oct, gate, velocity, retrig].
    pub fn process(&mut self, _ctx: &ProcessCtx, outputs: [&mut PortBuffer; 4], frames: usize) {
        let [voct, gate, vel, retrig] = outputs;
        let channels = self.polyphony;
        voct.channels = channels;
        gate.channels = channels;
        vel.channels = channels;
        retrig.channels = channels;

        for ch in 0..channels as usize {
            let v = &mut self.voices[ch];
            // 0 V = C4 = MIDI 60.
            let pitch = (v.note as f32 - 60.0) / 12.0;
            let gate_v = if v.gate { GATE_HIGH } else { 0.0 };
            let vel_v = v.velocity * GATE_HIGH;
            voct.data[ch][..frames].fill(pitch);
            gate.data[ch][..frames].fill(gate_v);
            vel.data[ch][..frames].fill(vel_v);
            // Retrig pulse: high for the first `retrig_remaining` frames.
            let pulse = (v.retrig_remaining as usize).min(frames);
            retrig.data[ch][..pulse].fill(GATE_HIGH);
            retrig.data[ch][pulse..frames].fill(0.0);
            v.retrig_remaining = v.retrig_remaining.saturating_sub(frames as u32);
        }
    }
}

impl Default for NoteIn {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;

    fn render(n: &mut NoteIn) -> [PortBuffer; 4] {
        let ctx = ProcessCtx::new(48_000.0);
        let mut bufs = [PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent()];
        let [a, b, c, d] = &mut bufs;
        n.process(&ctx, [a, b, c, d], BLOCK);
        bufs
    }

    #[test]
    fn mono_last_note_priority() {
        let mut n = NoteIn::new();
        n.note_on(60, 100);
        n.note_on(64, 100); // steals the single lane
        let [voct, gate, ..] = render(&mut n);
        assert_eq!(voct.channels, 1);
        assert!((voct.data[0][0] - 4.0 / 12.0).abs() < 1e-6);
        assert_eq!(gate.data[0][0], GATE_HIGH);
    }

    #[test]
    fn poly_allocates_separate_lanes() {
        let mut n = NoteIn::new();
        n.set_param(p::POLYPHONY, 4.0);
        n.note_on(60, 100);
        n.note_on(64, 100);
        n.note_on(67, 100);
        let [voct, gate, ..] = render(&mut n);
        assert_eq!(voct.channels, 4);
        let mut pitches: Vec<f32> = (0..3).map(|ch| voct.data[ch][0]).collect();
        pitches.sort_by(f32::total_cmp);
        assert!((pitches[0] - 0.0).abs() < 1e-6);
        assert!((pitches[1] - 4.0 / 12.0).abs() < 1e-6);
        assert!((pitches[2] - 7.0 / 12.0).abs() < 1e-6);
        assert_eq!((0..3).filter(|&ch| gate.data[ch][0] > 1.0).count(), 3);
    }

    #[test]
    fn steals_oldest_when_full() {
        let mut n = NoteIn::new();
        n.set_param(p::POLYPHONY, 2.0);
        n.note_on(60, 100);
        n.note_on(64, 100);
        n.note_on(67, 100); // steals lane holding 60 (oldest)
        let [voct, gate, _, retrig] = render(&mut n);
        let held: Vec<u8> = (0..2)
            .filter(|&ch| gate.data[ch][0] > 1.0)
            .map(|ch| (voct.data[ch][0] * 12.0 + 60.0).round() as u8)
            .collect();
        assert_eq!(held.len(), 2);
        assert!(held.contains(&64));
        assert!(held.contains(&67));
        assert!(!held.contains(&60));
        // The stolen lane re-attacks: retrig pulse present somewhere.
        assert!((0..2).any(|ch| retrig.data[ch][0] > 1.0));
    }

    #[test]
    fn note_off_releases_gate_keeps_pitch() {
        let mut n = NoteIn::new();
        n.note_on(60, 100);
        n.note_off(60);
        let [voct, gate, ..] = render(&mut n);
        assert_eq!(gate.data[0][0], 0.0);
        // Pitch holds during release so envelopes tail on the right note.
        assert!((voct.data[0][0] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn retrig_pulse_expires() {
        let mut n = NoteIn::new();
        n.note_on(60, 100);
        let [_, _, _, retrig] = render(&mut n);
        assert_eq!(retrig.data[0][0], GATE_HIGH);
        // 48-frame pulse: after two 32-frame blocks it must be over.
        let [_, _, _, retrig] = render(&mut n);
        assert_eq!(retrig.data[0][16], 0.0);
        let [_, _, _, retrig] = render(&mut n);
        assert!(retrig.data[0].iter().all(|&s| s == 0.0));
    }
}
