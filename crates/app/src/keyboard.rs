//! Computer-keyboard piano: AWSEDFTGYHUJK map (A = C, white keys on the home
//! row), Z/X shift octaves. Only active when egui isn't using the keyboard
//! for text; releases everything on focus loss so notes can't stick.

use crate::engine_link::EngineLink;
use eframe::egui::{Context, Event, Key};
use rack_core::messages::Msg;
use std::collections::HashMap;

/// Semitone offsets from the base note for each mapped key.
const KEYS: &[(Key, i32)] = &[
    (Key::A, 0),  // C
    (Key::W, 1),
    (Key::S, 2),
    (Key::E, 3),
    (Key::D, 4),
    (Key::F, 5),
    (Key::T, 6),
    (Key::G, 7),
    (Key::Y, 8),
    (Key::H, 9),
    (Key::U, 10),
    (Key::J, 11),
    (Key::K, 12), // C above
];

pub struct KeyboardPiano {
    /// Note sounded per held key (notes survive octave shifts mid-hold).
    held: HashMap<Key, u8>,
    /// Octave offset from middle C.
    pub octave: i32,
}

impl Default for KeyboardPiano {
    fn default() -> Self {
        Self { held: HashMap::new(), octave: 0 }
    }
}

impl KeyboardPiano {
    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    pub fn update(&mut self, ctx: &Context, link: &mut EngineLink) {
        let focused = ctx.input(|i| i.focused);
        if !focused || ctx.egui_wants_keyboard_input() {
            if !self.held.is_empty() {
                link.send(Msg::all_notes_off());
                self.held.clear();
            }
            return;
        }

        let events = ctx.input(|i| i.events.clone());
        for event in &events {
            let Event::Key { key, pressed, repeat, modifiers, .. } = event else { continue };
            if *repeat {
                continue; // key repeat must not retrigger notes
            }
            // Let Ctrl/Cmd shortcuts (copy/paste/undo/…) through untouched —
            // don't play a note or shift the octave on Cmd+Z, Cmd+D, etc.
            if modifiers.command || modifiers.ctrl {
                continue;
            }
            match key {
                Key::Z if *pressed => self.octave = (self.octave - 1).max(-3),
                Key::X if *pressed => self.octave = (self.octave + 1).min(3),
                _ => {
                    let Some(&(_, semis)) = KEYS.iter().find(|(k, _)| k == key) else {
                        continue;
                    };
                    if *pressed {
                        if !self.held.contains_key(key) {
                            let note = (60 + self.octave * 12 + semis).clamp(0, 127) as u8;
                            self.held.insert(*key, note);
                            link.send(Msg::note_on(note, 100, 0));
                        }
                    } else if let Some(note) = self.held.remove(key) {
                        link.send(Msg::note_off(note, 0));
                    }
                }
            }
        }
    }
}
