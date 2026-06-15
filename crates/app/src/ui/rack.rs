//! The rack: module panels on a scrollable grid, ports, and bezier cables.
//!
//! All persistent state lives in [`RackState`]; egui repaints it every frame.
//! `show` returns true when the patch topology changed (the caller re-plans
//! and re-syncs the engine).

use crate::audio::MeterMap;
use crate::engine_link::EngineLink;
use crate::ui::knob::knob;
use crate::ui::theme::{Palette, Theme};
use rack_core::meters::MeterEntry;
use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use rack_core::messages::Msg;
use rack_core::modules::{ModuleKindId, ParamDesc};
use rack_graph::plan::Planner;
use rack_graph::subpatch::{CustomModule, SubPatch};
use rack_graph::{GroupId, ModuleId, ModuleInst, ParamRef, Patch, PortRef};
use std::collections::{HashMap, HashSet};

const MODULE_W: f32 = 110.0;
const MODULE_H: f32 = 300.0;
/// Vertical unit for collapsed group boxes: a quarter of a module height, so a
/// box can be 1/4, 1/2, or a full module tall and several tile into one slot.
/// A group's `pos.row` is measured in these units.
const GROUP_V: f32 = MODULE_H / 4.0;
const GRID_X: f32 = 10.0;
const TITLE_H: f32 = 22.0;
const PORT_R: f32 = 7.0;

/// A leaf group of modules inside a category submenu.
struct AddGroup {
    name: &'static str,
    kinds: &'static [ModuleKindId],
}

/// A top-level add-menu category. A category with one group lists its modules
/// directly; with several, each group becomes its own submenu — so crowded
/// categories stay tidy and there's room to grow.
struct AddCategory {
    name: &'static str,
    groups: &'static [AddGroup],
}

const fn group(name: &'static str, kinds: &'static [ModuleKindId]) -> AddGroup {
    AddGroup { name, kinds }
}

/// The add menu, grouped by role then by sub-role (mirrors the module catalog).
const ADD_MENU: &[AddCategory] = &[
    AddCategory {
        name: "Sources",
        groups: &[
            group(
                "Oscillators",
                &[
                    ModuleKindId::Vco,
                    ModuleKindId::MacroOsc,
                    ModuleKindId::WtVco,
                    ModuleKindId::FmOp,
                    ModuleKindId::Additive,
                    ModuleKindId::SubOsc,
                    ModuleKindId::Supersaw,
                    ModuleKindId::ChordOsc,
                ],
            ),
            group(
                "Physical & string",
                &[ModuleKindId::Pluck, ModuleKindId::Resonator, ModuleKindId::Wavefold],
            ),
            group("Drums & noise", &[ModuleKindId::Drum, ModuleKindId::Noise]),
            group("Input", &[ModuleKindId::NoteIn, ModuleKindId::MidiCc]),
        ],
    },
    AddCategory {
        name: "Shapers & timbre",
        groups: &[
            group(
                "Filters",
                &[
                    ModuleKindId::Vcf,
                    ModuleKindId::Ladder,
                    ModuleKindId::Lpg,
                    ModuleKindId::SvfMulti,
                    ModuleKindId::AutoWah,
                    ModuleKindId::DualFilter,
                ],
            ),
            group(
                "Shaping",
                &[
                    ModuleKindId::Vca,
                    ModuleKindId::Waveshaper,
                    ModuleKindId::Saturator,
                    ModuleKindId::RingMod,
                    ModuleKindId::BitCrush,
                ],
            ),
            group("Physical", &[ModuleKindId::Comb, ModuleKindId::Formant]),
        ],
    },
    AddCategory {
        name: "Modulation",
        groups: &[
            group("Envelopes", &[ModuleKindId::Adsr, ModuleKindId::Maths]),
            group("LFOs", &[ModuleKindId::Lfo, ModuleKindId::ComplexLfo]),
            group(
                "Random & follow",
                &[ModuleKindId::SampleHold, ModuleKindId::Random, ModuleKindId::EnvFollow],
            ),
        ],
    },
    AddCategory {
        name: "Sequencing & timing",
        groups: &[
            group(
                "Clocks",
                &[ModuleKindId::Clock, ModuleKindId::ClockDiv, ModuleKindId::ClockMult],
            ),
            group(
                "Sequencers",
                &[
                    ModuleKindId::Seq8,
                    ModuleKindId::Arp,
                    ModuleKindId::Euclid,
                    ModuleKindId::Beats,
                    ModuleKindId::Ratchet,
                    ModuleKindId::Turing,
                    ModuleKindId::Bernoulli,
                    ModuleKindId::Burst,
                ],
            ),
            group("Pitch", &[ModuleKindId::Quantizer]),
        ],
    },
    AddCategory {
        name: "Pitch & CV",
        groups: &[
            group(
                "Pitch & CV",
                &[
                    ModuleKindId::CvMix,
                    ModuleKindId::Octave,
                    ModuleKindId::Quantizer,
                    ModuleKindId::Slew,
                    ModuleKindId::Comparator,
                    ModuleKindId::Rectify,
                ],
            ),
            group(
                "CV tools",
                &[
                    ModuleKindId::Offset,
                    ModuleKindId::TrackHold,
                    ModuleKindId::Phasor,
                    ModuleKindId::MinMax,
                ],
            ),
        ],
    },
    AddCategory {
        name: "Utilities & routing",
        groups: &[
            group(
                "Mixing",
                &[ModuleKindId::Mixer, ModuleKindId::VcaBank, ModuleKindId::Crossfade],
            ),
            group("Routing", &[ModuleKindId::Mult, ModuleKindId::SeqSwitch]),
            group(
                "CV tools",
                &[
                    ModuleKindId::Attenuverter,
                    ModuleKindId::Logic,
                    ModuleKindId::GateDelay,
                    ModuleKindId::TrigTool,
                ],
            ),
        ],
    },
    AddCategory {
        name: "Effects & output",
        groups: &[
            group(
                "Effects",
                &[
                    ModuleKindId::Delay,
                    ModuleKindId::TapeDelay,
                    ModuleKindId::PingPong,
                    ModuleKindId::Reverb,
                    ModuleKindId::Shimmer,
                    ModuleKindId::Chorus,
                    ModuleKindId::Flanger,
                    ModuleKindId::Phaser,
                    ModuleKindId::Tremolo,
                    ModuleKindId::Vibrato,
                    ModuleKindId::ParamEq,
                ],
            ),
            group(
                "Pitch & spectral",
                &[
                    ModuleKindId::PitchShift,
                    ModuleKindId::FreqShift,
                    ModuleKindId::Vocoder,
                ],
            ),
            group(
                "Drive & dynamics",
                &[
                    ModuleKindId::Drive,
                    ModuleKindId::Saturator,
                    ModuleKindId::Compressor,
                    ModuleKindId::Limiter,
                    ModuleKindId::Transient,
                    ModuleKindId::Ducker,
                    ModuleKindId::Gate,
                ],
            ),
            group("Stereo", &[ModuleKindId::Pan, ModuleKindId::Stereo]),
            group("Output", &[ModuleKindId::Output]),
        ],
    },
    AddCategory {
        name: "Meters",
        groups: &[group(
            "Meters",
            &[
                ModuleKindId::Scope,
                ModuleKindId::Spectrum,
                ModuleKindId::Voltmeter,
                ModuleKindId::Tuner,
            ],
        )],
    },
];

/// Panel width per kind: knob-heavy modules get more columns.
fn module_width(kind: ModuleKindId) -> f32 {
    match kind {
        ModuleKindId::Seq8 => 230.0,
        // Three knobs per row so the level knobs leave room for the scope.
        ModuleKindId::Mixer => 180.0,
        // Many ports along the bottom row need horizontal room.
        ModuleKindId::ClockDiv | ModuleKindId::Mult | ModuleKindId::Logic => 150.0,
        ModuleKindId::ComplexLfo | ModuleKindId::CvMix => 140.0,
        ModuleKindId::SeqSwitch => 200.0,
        ModuleKindId::VcaBank => 300.0,
        // Long param labels (harmonics/timbre/morph) + 5 ports need room.
        ModuleKindId::MacroOsc => 190.0,
        ModuleKindId::Rectify | ModuleKindId::Comparator | ModuleKindId::Arp => 140.0,
        // Multi-output filter: 4 outs + 2 ins along the bottom.
        ModuleKindId::SvfMulti => 160.0,
        ModuleKindId::DualFilter => 130.0,
        // 4 trigger outs + 2 ins, plus a 16-step clickable grid.
        ModuleKindId::Beats => 240.0,
        ModuleKindId::ParamEq | ModuleKindId::Flanger => 160.0,
        ModuleKindId::Voltmeter | ModuleKindId::Tuner => 130.0,
        // Visualizers want a wide display.
        ModuleKindId::Scope | ModuleKindId::Spectrum => 220.0,
        _ => MODULE_W,
    }
}

/// A module panel's screen rectangle from its grid position.
fn module_rect(origin: Pos2, inst: &ModuleInst) -> Rect {
    let (row, col) = inst.pos;
    Rect::from_min_size(
        origin + Vec2::new(col as f32 * GRID_X, row as f32 * MODULE_H),
        Vec2::new(module_width(inst.kind), MODULE_H),
    )
}

#[derive(Clone, Copy)]
struct CableDrag {
    /// Cables are always dragged by their input end; the output end stays
    /// anchored (dragging from a connected input detaches that cable).
    from_output: PortRef,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PortSide {
    In,
    Out,
}

/// How many patch snapshots the undo/redo history keeps.
const HISTORY_CAP: usize = 64;

pub struct RackState {
    pub patch: Patch,
    pub planner: Planner,
    /// Set when a param changed this frame (autosave tracking).
    pub param_changed: bool,
    /// Currently-selected modules. Click selects one; Shift/Ctrl-click toggles;
    /// drag on empty rack box-selects. Copy/delete/save act on the whole set.
    selected: HashSet<ModuleId>,
    drag: Option<CableDrag>,
    /// In-progress rubber-band box selection (anchor in screen space).
    box_select: Option<Pos2>,
    /// Port screen rects from this frame's panel painting (cables and drop
    /// hit-testing read them after modules paint).
    port_rects: HashMap<(ModuleId, PortSide, usize), Rect>,
    module_drag: Option<(ModuleId, Vec2)>,
    /// In-progress drag of a collapsed group box (moves all its members).
    group_drag: Option<(rack_graph::GroupId, Vec2)>,
    /// Copy/paste buffer: a fragment (modules + internal cables).
    clipboard: Option<SubPatch>,
    /// Undo/redo history of whole-patch snapshots (structural edits only).
    undo: Vec<Patch>,
    redo: Vec<Patch>,
    /// Active theme palette for the hand-painted canvas (set each frame).
    palette: Palette,
    /// In-progress group rename: (group, edit buffer). Shows a small dialog.
    group_rename: Option<(GroupId, String)>,
}

impl RackState {
    pub fn new(patch: Patch) -> Self {
        Self {
            patch,
            planner: Planner::new(),
            param_changed: false,
            selected: HashSet::new(),
            drag: None,
            box_select: None,
            port_rects: HashMap::new(),
            module_drag: None,
            group_drag: None,
            clipboard: None,
            undo: Vec::new(),
            redo: Vec::new(),
            palette: Theme::Dark.palette(),
            group_rename: None,
        }
    }

    /// Snapshot the patch before a structural edit so it can be undone.
    /// Clears the redo stack (a new edit forks the history).
    fn push_undo(&mut self) {
        if self.undo.len() >= HISTORY_CAP {
            self.undo.remove(0);
        }
        self.undo.push(self.patch.clone());
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Restore the previous snapshot. Returns true if anything changed.
    pub fn undo(&mut self) -> bool {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.patch, prev));
            self.selected.clear();
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.patch, next));
            self.selected.clear();
            true
        } else {
            false
        }
    }

    /// A clickable "add <module>" menu entry. Places the module at the
    /// pointer (grid-snapped) and records an undo snapshot.
    fn add_button(&mut self, ui: &mut Ui, origin: Pos2, kind: ModuleKindId, changed: &mut bool) {
        if ui.button(kind.desc().name).clicked() {
            let pos = ui.ctx().pointer_interact_pos().unwrap_or(origin + Vec2::splat(40.0));
            let col = ((pos.x - origin.x) / GRID_X).round() as i32;
            let row = ((pos.y - origin.y) / MODULE_H).floor() as i32;
            self.push_undo();
            self.patch.add_module(kind, (row.max(0), col.max(0)));
            *changed = true;
            ui.close();
        }
    }

    /// Number of currently-selected modules.
    pub fn selection_count(&self) -> usize {
        self.selected.len()
    }

    /// Capture the current selection as a fragment (for "save as custom…").
    /// If the selection is exactly one group, its *designed* interface is
    /// captured; otherwise the boundary ports are used as the default.
    pub fn extract_selection(&self) -> Option<SubPatch> {
        if self.selected.is_empty() {
            return None;
        }
        if let Some(gid) = self.selection_group() {
            return Some(self.patch.extract_group(gid));
        }
        let ids: Vec<ModuleId> = self.selected.iter().copied().collect();
        Some(self.patch.extract(&ids))
    }

    /// The group whose members are exactly the current selection, if any.
    fn selection_group(&self) -> Option<GroupId> {
        self.patch
            .groups
            .iter()
            .find(|(_, g)| {
                g.members.len() == self.selected.len()
                    && g.members.iter().all(|m| self.selected.contains(m))
            })
            .map(|(id, _)| *id)
    }

    /// Set or toggle the selection. `additive` (Shift/Ctrl) toggles membership;
    /// otherwise the click replaces the whole selection.
    fn select(&mut self, id: ModuleId, additive: bool) {
        if additive {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else if !self.selected.contains(&id) {
            // Plain click on an unselected module: select just it. Clicking a
            // module that's already part of a group leaves the group intact.
            self.selected.clear();
            self.selected.insert(id);
        }
    }

    /// Collapse the current selection into a group (≥ 2 modules). Returns true
    /// if a group was made. Doesn't change audio, so the caller only needs to
    /// mark the patch dirty (not resync the engine).
    pub fn group_selection(&mut self) -> bool {
        if self.selected.len() < 2 {
            return false;
        }
        let ids: Vec<ModuleId> = self.selected.iter().copied().collect();
        let min_row = ids.iter().filter_map(|id| self.patch.modules.get(id)).map(|m| m.pos.0).min().unwrap_or(0);
        let min_col = ids.iter().filter_map(|id| self.patch.modules.get(id)).map(|m| m.pos.1).min().unwrap_or(0);
        let name = format!("Group {}", self.patch.groups.len() + 1);
        self.push_undo();
        // Group row is in quarter-module units; a module row r sits at r*4.
        if self.patch.create_group(&ids, name, (min_row * 4, min_col)).is_some() {
            self.selected.clear();
            self.param_changed = true;
            true
        } else {
            self.undo.pop();
            false
        }
    }

    /// Copy the whole selection (with internal cables) to the clipboard.
    fn copy_selected(&mut self) {
        self.clipboard = self.extract_selection();
    }

    /// Paste the clipboard fragment at a grid position, selecting the copy.
    fn paste_at(&mut self, pos: (i32, i32)) -> bool {
        let Some(sub) = self.clipboard.clone() else { return false };
        self.push_undo();
        let new_ids = self.patch.insert(&sub, pos);
        if new_ids.is_empty() {
            self.undo.pop(); // nothing inserted — drop the snapshot
            return false;
        }
        self.selected = new_ids.into_iter().collect();
        true
    }

    /// Stamp a saved custom module into the patch as a single collapsed group
    /// box (its designed interface becomes the box's ports). Records undo and
    /// selects the new modules.
    pub fn instantiate(&mut self, sub: &SubPatch, pos: (i32, i32), name: &str) -> bool {
        self.push_undo();
        match self.patch.insert_as_group(sub, pos, name.to_owned()) {
            Some(gid) => {
                self.selected = self.patch.groups[&gid].members.iter().copied().collect();
                true
            }
            None => {
                self.undo.pop();
                false
            }
        }
    }

    /// Copy the selection and immediately paste it, offset to the right of
    /// the original group.
    fn duplicate_selected(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let ids: Vec<ModuleId> = self.selected.iter().copied().collect();
        let min_row = ids.iter().filter_map(|id| self.patch.modules.get(id)).map(|m| m.pos.0).min().unwrap_or(0);
        let min_col = ids.iter().filter_map(|id| self.patch.modules.get(id)).map(|m| m.pos.1).min().unwrap_or(0);
        let sub = self.patch.extract(&ids);
        // Column span of the group (rightmost panel edge, in grid columns).
        let span = sub
            .modules
            .iter()
            .map(|m| {
                let w = ModuleKindId::from_type_name(&m.type_name).map_or(MODULE_W, module_width);
                m.pos.1 + (w / GRID_X).round() as i32
            })
            .max()
            .unwrap_or(8)
            + 1;
        self.clipboard = Some(sub.clone());
        self.push_undo();
        let new_ids = self.patch.insert(&sub, (min_row, min_col + span));
        if new_ids.is_empty() {
            self.undo.pop();
            return false;
        }
        self.selected = new_ids.into_iter().collect();
        true
    }

    /// Paint and interact. Returns true if the topology changed.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        link: &mut EngineLink,
        meters: &MeterMap,
        customs: &[CustomModule],
        palette: Palette,
    ) -> bool {
        let mut topology_changed = false;
        self.palette = palette;
        self.port_rects.clear();

        egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
            // A generous canvas so there's room to scroll into.
            let canvas = ui.allocate_rect(
                Rect::from_min_size(ui.max_rect().min, Vec2::new(2400.0, 1000.0)),
                Sense::click_and_drag(),
            );
            let origin = canvas.rect.min;

            // Add-module menu on rack background right-click: category →
            // (sub-group →) module. Single-group categories list directly.
            canvas.context_menu(|ui| {
                ui.label("Add module");
                ui.separator();
                for cat in ADD_MENU {
                    ui.menu_button(cat.name, |ui| {
                        if let [only] = cat.groups {
                            for &kind in only.kinds {
                                self.add_button(ui, origin, kind, &mut topology_changed);
                            }
                        } else {
                            for grp in cat.groups {
                                ui.menu_button(grp.name, |ui| {
                                    for &kind in grp.kinds {
                                        self.add_button(ui, origin, kind, &mut topology_changed);
                                    }
                                });
                            }
                        }
                    });
                }
                // Saved custom modules (subpatches).
                if !customs.is_empty() {
                    ui.separator();
                    ui.menu_button("Custom", |ui| {
                        for cm in customs {
                            if ui.button(&cm.name).clicked() {
                                let pos = ui
                                    .ctx()
                                    .pointer_interact_pos()
                                    .unwrap_or(origin + Vec2::splat(40.0));
                                let col = ((pos.x - origin.x) / GRID_X).round() as i32;
                                // Custom modules drop in as a collapsed group, whose
                                // row is in quarter-module (GROUP_V) units.
                                let row = ((pos.y - origin.y) / GROUP_V).floor() as i32;
                                if self.instantiate(&cm.sub, (row.max(0), col.max(0)), &cm.name) {
                                    topology_changed = true;
                                }
                                ui.close();
                            }
                        }
                    });
                }
            });

            // Click on empty rack clears the selection.
            if canvas.clicked() {
                self.selected.clear();
            }

            // Rubber-band box select: drag on empty rack.
            if canvas.drag_started() {
                self.box_select = ui.ctx().pointer_interact_pos();
            }
            if let (Some(start), true) = (self.box_select, canvas.dragged()) {
                if let Some(cur) = ui.ctx().pointer_interact_pos() {
                    let rect = Rect::from_two_pos(start, cur);
                    let a = self.palette.accent;
                    ui.painter().rect(
                        rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 20),
                        Stroke::new(1.0, a),
                        StrokeKind::Inside,
                    );
                }
            }
            // Members of collapsed groups are folded away into one box each.
            let hidden: HashSet<ModuleId> = self
                .patch
                .groups
                .values()
                .filter(|g| g.collapsed)
                .flat_map(|g| g.members.iter().copied())
                .collect();

            if canvas.drag_stopped() {
                if let (Some(start), Some(end)) = (self.box_select.take(), ui.ctx().pointer_interact_pos()) {
                    let sel_rect = Rect::from_two_pos(start, end);
                    let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                    if !additive {
                        self.selected.clear();
                    }
                    let hits: Vec<ModuleId> = self
                        .patch
                        .modules
                        .iter()
                        .filter(|(id, _)| !hidden.contains(id))
                        .filter(|(_, inst)| module_rect(origin, inst).intersects(sel_rect))
                        .map(|(id, _)| *id)
                        .collect();
                    self.selected.extend(hits);
                }
            }

            // Collapsed group boxes (register their exposed-port rects so the
            // existing cable drawing/patching works against the box).
            let collapsed_gids: Vec<rack_graph::GroupId> = self
                .patch
                .groups
                .iter()
                .filter(|(_, g)| g.collapsed)
                .map(|(id, _)| *id)
                .collect();
            for gid in collapsed_gids {
                if self.collapsed_group(ui, origin, gid, link, meters) {
                    topology_changed = true;
                }
            }

            // Paint visible modules (fills port_rects).
            let ids: Vec<ModuleId> =
                self.patch.modules.keys().copied().filter(|id| !hidden.contains(id)).collect();
            for id in ids {
                if self.module_panel(ui, origin, id, link, meters) {
                    topology_changed = true;
                }
            }

            // Keyboard shortcuts (only when egui isn't capturing text).
            if !ui.ctx().egui_wants_keyboard_input() {
                let (del, copy, paste, dup, undo, redo) = ui.input(|i| {
                    let cmd = i.modifiers.command;
                    let shift = i.modifiers.shift;
                    // On web, Ctrl/Cmd+C and +V arrive as high-level clipboard
                    // events rather than key presses — accept both forms.
                    let copy_evt = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                    let paste_evt = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
                    (
                        i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                        copy_evt || (cmd && i.key_pressed(egui::Key::C)),
                        paste_evt || (cmd && i.key_pressed(egui::Key::V)),
                        cmd && i.key_pressed(egui::Key::D),
                        cmd && !shift && i.key_pressed(egui::Key::Z),
                        cmd && (i.key_pressed(egui::Key::Y) || (shift && i.key_pressed(egui::Key::Z))),
                    )
                });
                let group = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::G));
                if group {
                    self.group_selection();
                }

                if del && !self.selected.is_empty() {
                    self.push_undo();
                    for id in std::mem::take(&mut self.selected) {
                        self.patch.remove_module(id);
                    }
                    topology_changed = true;
                }
                if copy {
                    self.copy_selected();
                }
                if dup && self.duplicate_selected() {
                    topology_changed = true;
                }
                if paste {
                    // Paste at the pointer if it's over the rack, else offset.
                    let pos = ui.ctx().pointer_interact_pos().map_or((0, 2), |p| {
                        let col = ((p.x - origin.x) / GRID_X).round() as i32;
                        let row = ((p.y - origin.y) / MODULE_H).floor() as i32;
                        (row.max(0), col.max(0))
                    });
                    if self.paste_at(pos) {
                        topology_changed = true;
                    }
                }
                if undo && self.undo() {
                    topology_changed = true;
                }
                if redo && self.redo() {
                    topology_changed = true;
                }
            }

            // Cable drag interaction.
            if let Some(drag) = self.drag {
                if ui.input(|i| i.pointer.any_released()) {
                    let drop = ui.ctx().pointer_interact_pos().and_then(|p| self.input_port_at(p));
                    if let Some(to) = drop {
                        self.push_undo();
                        self.patch.connect(drag.from_output, to);
                    }
                    self.drag = None;
                    topology_changed = true;
                } else if !ui.input(|i| i.pointer.any_down()) {
                    self.drag = None; // released off-window
                    topology_changed = true;
                }
            }

            self.paint_cables(ui);
        });

        self.rename_dialog(ui);

        topology_changed
    }

    /// Floating "rename group" dialog, shown while `group_rename` is set.
    fn rename_dialog(&mut self, ui: &mut Ui) {
        let Some((gid, mut buf)) = self.group_rename.take() else { return };
        let mut commit = false;
        let mut cancel = false;
        egui::Window::new("Rename group")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                let resp = ui.text_edit_singleline(&mut buf);
                resp.request_focus();
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() {
                        commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if commit {
            let name = buf.trim();
            if !name.is_empty() {
                if let Some(g) = self.patch.groups.get_mut(&gid) {
                    g.name = name.to_owned();
                }
                self.param_changed = true; // triggers autosave
            }
        } else if !cancel {
            // Not committed or cancelled yet — keep the dialog open next frame.
            self.group_rename = Some((gid, buf));
        }
    }

    /// Draw a collapsed group as a single box exposing its designed interface.
    /// Registers each exposed port under its real (member, port) key so the
    /// shared cable-drawing and patching code routes to the box for free.
    fn collapsed_group(
        &mut self,
        ui: &mut Ui,
        origin: Pos2,
        gid: GroupId,
        link: &mut EngineLink,
        meters: &MeterMap,
    ) -> bool {
        let mut changed = false;
        let Some(g) = self.patch.groups.get(&gid) else { return false };
        let name = g.name.clone();
        let members = g.members.clone();
        let (row, col) = g.pos;
        let exposed_in = g.exposed_in.clone();
        let exposed_out = g.exposed_out.clone();
        let exposed_params = g.exposed_params.clone();

        // Exposed ports + their names (collected up front so we can borrow
        // self mutably for port_widget afterwards). Skip any whose member/port
        // no longer resolves.
        let port_name = |p: &PortRef, input: bool| -> Option<&'static str> {
            let desc = self.patch.modules.get(&p.module)?.kind.desc();
            let list = if input { desc.inputs } else { desc.outputs };
            list.get(p.port).map(|pd| pd.name)
        };
        let ins: Vec<(PortRef, &'static str)> =
            exposed_in.iter().filter_map(|p| Some((*p, port_name(p, true)?))).collect();
        let outs: Vec<(PortRef, &'static str)> =
            exposed_out.iter().filter_map(|p| Some((*p, port_name(p, false)?))).collect();

        // Exposed knobs/switches: (module, param index, static descriptor).
        // A Beats module's exposure means "show its grid" — collected here and
        // rendered as the full step grid instead of as knobs.
        let mut grid_mods: Vec<ModuleId> = Vec::new();
        let knobs: Vec<(ModuleId, u32, &'static ParamDesc)> = exposed_params
            .iter()
            .filter_map(|p| {
                let kind = self.patch.modules.get(&p.module)?.kind;
                if kind == ModuleKindId::Beats {
                    if !grid_mods.contains(&p.module) {
                        grid_mods.push(p.module);
                    }
                    return None;
                }
                let desc = kind.desc().params.get(p.param)?;
                Some((p.module, p.param as u32, desc))
            })
            .collect();

        // --- Size like a standard module: the SAME fixed height, and a width
        // that grows to hold the ports and knobs so nothing overlaps. Extra
        // knobs add columns (widen), never rows past what a module's height
        // holds, so the box is never taller than a normal module. ---
        // Just a thin pad below the title bar so content sits right at the top.
        let caption_h = 4.0;
        let param_top = TITLE_H + caption_h;

        // Inputs sit on the left, outputs on the right: the bottom row needs
        // room for both sides plus a gap.
        let port_width = if ins.is_empty() && outs.is_empty() {
            0.0
        } else {
            (ins.len() + outs.len()) as f32 * 24.0 + 48.0
        };

        // Reserve room for a step grid and the bottom port row, so the knobs
        // widen (add columns) rather than stack — keeping all the content within
        // one module height with the ports tucked along the bottom.
        const GRID_H: f32 = 4.0 * (12.0 + 4.0) + 6.0;
        let grids_total = grid_mods.len() as f32 * (GRID_H + 4.0);
        let ports_h = if ins.is_empty() && outs.is_empty() { 8.0 } else { 42.0 };

        // Pick the column count so every knob row fits in the height a module
        // has (default 2 like a module; add columns only when needed).
        let n_switch = knobs.iter().filter(|(_, _, p)| p.steps.is_some()).count();
        let n_knob = knobs.len() - n_switch;
        let avail = (MODULE_H - param_top - ports_h - grids_total).max(64.0);
        let knob_rows_budget =
            (((avail - n_switch as f32 * 26.0) / 64.0).floor() as usize).max(1);
        let cols = if n_knob == 0 {
            1
        } else {
            // ceil(n_knob / budget), but at least 2 (module-like).
            n_knob.div_ceil(knob_rows_budget).max(2)
        };
        let knob_width = if knobs.is_empty() { 0.0 } else { cols as f32 * 54.0 + 12.0 };
        // A step grid needs room for 16 columns.
        let grid_width = if grid_mods.is_empty() { 0.0 } else { 240.0 };

        let width = MODULE_W.max(port_width).max(knob_width).max(grid_width);

        // Pack params into rows: knobs fill rows of `cols`, switches own a row.
        let mut knob_rows: Vec<Vec<(ModuleId, u32, &'static ParamDesc)>> = Vec::new();
        let mut current: Vec<(ModuleId, u32, &'static ParamDesc)> = Vec::new();
        for &(mid, pi, pdesc) in &knobs {
            if pdesc.steps.is_some() {
                if !current.is_empty() {
                    knob_rows.push(std::mem::take(&mut current));
                }
                knob_rows.push(vec![(mid, pi, pdesc)]);
            } else {
                current.push((mid, pi, pdesc));
                if current.len() == cols {
                    knob_rows.push(std::mem::take(&mut current));
                }
            }
        }
        if !current.is_empty() {
            knob_rows.push(current);
        }
        let params_height: f32 = knob_rows
            .iter()
            .map(|r| if r.first().is_some_and(|(_, _, p)| p.steps.is_some()) { 26.0 } else { 64.0 })
            .sum();

        // Snap to the smallest of quarter / half / full module height that fits
        // the content, capped at a full module so a box is never taller than a
        // normal one. When content is tall the ports stay pinned to the box's
        // bottom edge (below) instead of growing the panel past a module.
        let knob_pad = if params_height > 0.0 { 2.0 } else { 0.0 };
        let base_h = param_top + params_height + knob_pad + grids_total + ports_h + 6.0;
        let height = if base_h <= GROUP_V {
            GROUP_V
        } else if base_h <= GROUP_V * 2.0 {
            GROUP_V * 2.0
        } else {
            MODULE_H
        };
        let show_scope = !outs.is_empty() && base_h + 50.0 <= height;
        let scope_h = if show_scope { 50.0 } else { 0.0 };
        let rect = Rect::from_min_size(
            origin + Vec2::new(col as f32 * GRID_X, row as f32 * GROUP_V),
            Vec2::new(width, height),
        );

        let all_selected = !members.is_empty() && members.iter().all(|m| self.selected.contains(m));

        // The box's "output" for the scope/LED: the first exposed output's
        // source module (the signal the box presents to the outside world).
        let scope_src = outs.first().map(|(pr, _)| pr.module);
        let scope_kind = scope_src.and_then(|m| self.patch.modules.get(&m)).map(|i| i.kind);
        let entry = scope_src.and_then(|m| self.planner.slot_of(m)).and_then(|s| meters.get(&s));
        let full_scale = if scope_kind == Some(ModuleKindId::Output) { 1.0 } else { 6.0 };

        let pal = self.palette;
        // Box body — same panel palette as a module so it reads as one.
        ui.painter().rect(
            rect,
            4.0,
            pal.panel,
            if all_selected {
                Stroke::new(2.0, pal.accent)
            } else {
                Stroke::new(1.0, pal.border)
            },
            StrokeKind::Inside,
        );

        // Title bar: click selects members, double-click expands, drag moves
        // the whole group, right-click for group actions. A faint slate tint +
        // ▣ glyph is the only cue that this panel is a group, not a module.
        let title_rect = Rect::from_min_size(rect.min, Vec2::new(width, TITLE_H));
        let title_resp =
            ui.interact(title_rect, ui.id().with(("group", gid)), Sense::click_and_drag());
        ui.painter().rect(title_rect, 4.0, pal.title_group, Stroke::NONE, StrokeKind::Inside);
        ui.painter().text(
            title_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("▣ {name}"),
            egui::FontId::proportional(12.0),
            pal.title_text,
        );

        // Peak LED, exactly like a module's.
        let peak = entry.map_or(0.0, |e| e.peak);
        let led = peak / full_scale;
        let led_color = if led < 0.01 {
            pal.led_off
        } else if led < 0.6 {
            Color32::from_rgb(80, 220, 100)
        } else if led < 0.85 {
            Color32::from_rgb(235, 200, 60)
        } else {
            Color32::from_rgb(240, 80, 60)
        };
        ui.painter().circle(
            Pos2::new(title_rect.max.x - 11.0, title_rect.center().y),
            4.0,
            led_color,
            Stroke::new(1.0, pal.outline),
        );

        if title_resp.double_clicked() {
            if let Some(gg) = self.patch.groups.get_mut(&gid) {
                gg.collapsed = false;
            }
            self.param_changed = true;
        } else if title_resp.clicked() {
            self.selected = members.iter().copied().collect();
        }
        if title_resp.drag_started() {
            self.push_undo();
        }
        if title_resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let mut acc = self.group_drag.map_or(Vec2::ZERO, |(_, d)| d) + title_resp.drag_delta();
            let dx = (acc.x / GRID_X).round() as i32;
            // Vertical step is a quarter module, so boxes tile at finer than
            // module granularity.
            let dy = (acc.y / GROUP_V).round() as i32;
            acc.x -= dx as f32 * GRID_X;
            acc.y -= dy as f32 * GROUP_V;
            if dx != 0 || dy != 0 {
                // Move only the box; members keep their own positions (they're
                // shown only when the group is expanded for editing).
                if let Some(gg) = self.patch.groups.get_mut(&gid) {
                    gg.pos = ((row + dy).max(0), (col + dx).max(0));
                }
                self.param_changed = true;
            }
            self.group_drag = Some((gid, acc));
        } else if matches!(self.group_drag, Some((d, _)) if d == gid) {
            self.group_drag = None;
        }
        title_resp.context_menu(|ui| {
            if ui.button("Rename…").clicked() {
                self.group_rename = Some((gid, name.clone()));
                ui.close();
            }
            if ui.button("Expand").clicked() {
                if let Some(gg) = self.patch.groups.get_mut(&gid) {
                    gg.collapsed = false;
                }
                self.param_changed = true;
                ui.close();
            }
            if ui.button("Ungroup").clicked() {
                self.push_undo();
                self.patch.ungroup(gid);
                self.param_changed = true;
                ui.close();
            }
            if ui.button("Remove group").clicked() {
                self.push_undo();
                for m in &members {
                    self.patch.remove_module(*m);
                }
                changed = true;
                ui.close();
            }
        });

        // Content flows top-down from directly under the title; the box was
        // sized to fit it, so everything packs up to the top with no dead gap.
        let knob_top = rect.min.y + param_top;

        // Exposed knobs/switches: live, interactive, laid out in rows just like
        // a real module's param area. group=None means no nested expose menu
        // here (you manage exposure from the expanded view); the knob still
        // drives the member's param.
        if !knob_rows.is_empty() {
            let mut knob_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(
                        Pos2::new(rect.min.x + 6.0, knob_top),
                        Pos2::new(rect.max.x - 6.0, knob_top + params_height),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            for r in &knob_rows {
                // Spread the row's knobs evenly across the full box width:
                // equal columns, each control centered in its column.
                knob_ui.columns(r.len().max(1), |cols| {
                    for (i, &(mid, pi, pdesc)) in r.iter().enumerate() {
                        cols[i].vertical_centered(|ui| {
                            self.param_widget(ui, mid, pi, pdesc, link, None);
                        });
                    }
                });
            }
        }

        // Exposed step grids, stacked directly below the knobs (interactive —
        // they drive the member Beats module, same as on its own panel).
        let mut y = knob_top + params_height + knob_pad;
        for gm in &grid_mods {
            let gh = self.beats_grid(ui, *gm, rect.min.x, y, width, link);
            y += gh + 4.0;
        }

        // Wave viewer directly below the content (only when there's an output
        // to show and it fits without making the box too tall).
        if show_scope {
            let scope_rect = Rect::from_min_max(
                Pos2::new(rect.min.x + 6.0, y + 2.0),
                Pos2::new(rect.max.x - 6.0, y + 46.0),
            );
            paint_scope(ui, scope_rect, entry, full_scale, &pal);
            y += scope_h;
        }

        // Exposed ports sit just below the content, but are pinned to the box's
        // bottom if the content is tall — so they're always inside the panel,
        // never hanging off the bottom edge. (Reuses the normal port widget,
        // which registers the rect under the real member/port key.)
        let port_top = y.min(rect.max.y - ports_h);
        let label_y = port_top + 8.0;
        let port_y = port_top + 26.0;
        for (i, (pr, pname)) in ins.iter().enumerate() {
            let center = Pos2::new(rect.min.x + 16.0 + i as f32 * 24.0, port_y);
            if self.port_widget(ui, pr.module, PortSide::In, pr.port, center, pname, label_y, None) {
                changed = true;
            }
        }
        for (i, (pr, pname)) in outs.iter().enumerate() {
            let center = Pos2::new(rect.max.x - 16.0 - i as f32 * 24.0, port_y);
            if self.port_widget(ui, pr.module, PortSide::Out, pr.port, center, pname, label_y, None) {
                changed = true;
            }
        }
        changed
    }

    /// One module panel. Returns true if topology changed (module removed or
    /// cable detached via its input port).
    fn module_panel(
        &mut self,
        ui: &mut Ui,
        origin: Pos2,
        id: ModuleId,
        link: &mut EngineLink,
        meters: &MeterMap,
    ) -> bool {
        let mut topology_changed = false;
        let Some(inst) = self.patch.modules.get(&id) else { return false };
        let kind = inst.kind;
        let desc = kind.desc();
        let (row, col) = inst.pos;
        let width = module_width(kind);
        // Group this module belongs to (it's expanded, since collapsed members
        // aren't drawn here). Shown with a tinted title + group menu items.
        let group = self.patch.group_of(id);
        let rect = Rect::from_min_size(
            origin + Vec2::new(col as f32 * GRID_X, row as f32 * MODULE_H),
            Vec2::new(width, MODULE_H),
        );

        // Whole-panel click selects (registered before knobs/ports, so those
        // widgets still win their own clicks). Shift/Ctrl toggles membership.
        let panel_resp = ui.interact(rect, ui.id().with(("panel", id)), Sense::click());
        if panel_resp.clicked() {
            let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
            self.select(id, additive);
        }

        let pal = self.palette;
        let selected = self.selected.contains(&id);
        let painter = ui.painter();
        painter.rect(
            rect,
            4.0,
            pal.panel,
            if selected {
                Stroke::new(2.0, pal.accent)
            } else {
                Stroke::new(1.0, pal.border)
            },
            StrokeKind::Inside,
        );

        // Title bar: click to select, drag to move, right-click to remove.
        let title_rect = Rect::from_min_size(rect.min, Vec2::new(width, TITLE_H));
        let title_resp = ui.interact(
            title_rect,
            ui.id().with(("title", id)),
            Sense::click_and_drag(),
        );
        if title_resp.clicked() {
            let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);
            self.select(id, additive);
        }
        if title_resp.drag_started() {
            // Dragging a module that isn't selected selects just it.
            if !self.selected.contains(&id) {
                self.select(id, false);
            }
            self.push_undo(); // snapshot once at the start of a move
        }
        // Grouped (but expanded) modules get a slate title tint as a cue.
        let title_bg = if group.is_some() { pal.title_group } else { pal.title };
        ui.painter().rect(title_rect, 4.0, title_bg, Stroke::NONE, StrokeKind::Inside);
        ui.painter().text(
            title_rect.center(),
            egui::Align2::CENTER_CENTER,
            desc.name,
            egui::FontId::proportional(12.0),
            pal.title_text,
        );
        if title_resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let mut acc = self.module_drag.map_or(Vec2::ZERO, |(_, d)| d) + title_resp.drag_delta();
            // Consume whole grid steps per axis independently, keeping the
            // remainder so neither axis loses accumulated motion.
            let dx = (acc.x / GRID_X).round() as i32;
            let dy = (acc.y / MODULE_H).round() as i32;
            acc.x -= dx as f32 * GRID_X;
            acc.y -= dy as f32 * MODULE_H;
            if dx != 0 || dy != 0 {
                if let Some(inst) = self.patch.modules.get_mut(&id) {
                    inst.pos = ((row + dy).max(0), (col + dx).max(0));
                }
            }
            self.module_drag = Some((id, acc));
        } else if matches!(self.module_drag, Some((d, _)) if d == id) {
            self.module_drag = None;
        }
        title_resp.context_menu(|ui| {
            // Right-clicking a module outside the current selection acts on
            // just it; inside the selection, acts on the whole group.
            if !self.selected.contains(&id) {
                self.select(id, false);
            }
            let n = self.selected.len();
            let label = |verb: &str| if n > 1 { format!("{verb} {n} modules") } else { verb.to_owned() };
            if ui.button(label("Copy")).clicked() {
                self.copy_selected();
                ui.close();
            }
            if ui.button(label("Duplicate")).clicked() {
                if self.duplicate_selected() {
                    topology_changed = true;
                }
                ui.close();
            }
            // Group actions: collapse this module's group back into a box, or
            // group the current multi-selection.
            if let Some(gid) = group {
                ui.separator();
                if ui.button("Rename group…").clicked() {
                    let nm = self.patch.groups.get(&gid).map(|g| g.name.clone()).unwrap_or_default();
                    self.group_rename = Some((gid, nm));
                    ui.close();
                }
                if ui.button("Collapse group").clicked() {
                    if let Some(gg) = self.patch.groups.get_mut(&gid) {
                        gg.collapsed = true;
                    }
                    self.param_changed = true;
                    ui.close();
                }
                if ui.button("Ungroup").clicked() {
                    self.push_undo();
                    self.patch.ungroup(gid);
                    self.param_changed = true;
                    ui.close();
                }
                // The BEATS grid is exposed as a whole (its track-mask params);
                // param 0 doubles as the "grid is exposed" marker.
                if kind == ModuleKindId::Beats {
                    let marker = ParamRef { module: id, param: 0 };
                    let shown = self
                        .patch
                        .groups
                        .get(&gid)
                        .is_some_and(|gg| gg.exposed_params.contains(&marker));
                    let lbl = if shown { "Hide grid from group" } else { "Show grid on group" };
                    if ui.button(lbl).clicked() {
                        self.patch.toggle_exposed_param(marker);
                        self.param_changed = true;
                        ui.close();
                    }
                }
            } else if self.selected.len() >= 2 && self.selected.contains(&id) {
                ui.separator();
                if ui.button("Group selection").clicked() {
                    self.group_selection();
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Remove module").clicked() {
                self.push_undo();
                self.patch.remove_module(id);
                topology_changed = true;
                ui.close();
            }
        });
        if topology_changed {
            return true;
        }

        // The drum grid replaces the normal knob panel with a clickable
        // 4×16 step matrix; every other module lays out knobs/switches.
        let params_height: f32 = if kind == ModuleKindId::Beats {
            self.beats_grid(ui, id, rect.min.x, rect.min.y + TITLE_H + 8.0, width, link)
        } else {
            // Params: knobs in rows of 2; switches as small selectors.
            let params: Vec<(usize, &'static ParamDesc)> = desc.params.iter().enumerate().collect();
            let mut param_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(
                        rect.min + Vec2::new(6.0, TITLE_H + 4.0),
                        rect.max - Vec2::new(6.0, 70.0),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            // Knobs fill rows by available width; switches get their own
            // full-width row (their text buttons don't fit beside a knob).
            let knobs_per_row = (((width - 12.0) / 54.0).floor() as usize).max(2);
            let mut rows: Vec<Vec<(usize, &'static ParamDesc)>> = Vec::new();
            let mut current: Vec<(usize, &'static ParamDesc)> = Vec::new();
            for (i, p) in params {
                if p.steps.is_some() {
                    if !current.is_empty() {
                        rows.push(std::mem::take(&mut current));
                    }
                    rows.push(vec![(i, p)]);
                } else {
                    current.push((i, p));
                    if current.len() == knobs_per_row {
                        rows.push(std::mem::take(&mut current));
                    }
                }
            }
            if !current.is_empty() {
                rows.push(current);
            }
            // Knob rows are ~64 px tall (label + knob + value), switch rows ~26.
            let h: f32 = rows
                .iter()
                .map(|r| if r.first().is_some_and(|(_, p)| p.steps.is_some()) { 26.0 } else { 64.0 })
                .sum();
            for row in rows {
                param_ui.horizontal(|ui| {
                    for (i, p) in row {
                        self.param_widget(ui, id, i as u32, p, link, group);
                    }
                });
            }
            h
        };

        // Live signal feedback. The Output module's tap is the post-scale
        // master bus (±1), everything else is in volts (±5 nominal).
        let entry = self.planner.slot_of(id).and_then(|s| meters.get(&s));
        let full_scale = if kind == ModuleKindId::Output { 1.0 } else { 6.0 };

        // Peak LED in the title bar.
        let peak = entry.map_or(0.0, |e| e.peak);
        let led = peak / full_scale;
        let led_color = if led < 0.01 {
            pal.led_off
        } else if led < 0.6 {
            Color32::from_rgb(80, 220, 100)
        } else if led < 0.85 {
            Color32::from_rgb(235, 200, 60)
        } else {
            Color32::from_rgb(240, 80, 60)
        };
        ui.painter().circle(
            Pos2::new(title_rect.max.x - 11.0, title_rect.center().y),
            4.0,
            led_color,
            Stroke::new(1.0, pal.outline),
        );

        // Scope strip above the port row, when params leave room for it.
        let scope_rect = Rect::from_min_max(
            Pos2::new(rect.min.x + 6.0, rect.max.y - 102.0),
            Pos2::new(rect.max.x - 6.0, rect.max.y - 54.0),
        );
        let params_bottom = rect.min.y + TITLE_H + 8.0 + params_height;
        // A large display that fills the panel (scope / spectrum visualizers).
        let big_rect = Rect::from_min_max(
            Pos2::new(rect.min.x + 6.0, rect.min.y + TITLE_H + 6.0),
            Pos2::new(rect.max.x - 6.0, rect.max.y - 50.0),
        );
        // Meter readout modules show a number/note/visualization instead of the
        // small scope strip.
        match kind {
            ModuleKindId::Voltmeter => paint_readout(ui, scope_rect, readout_volts(entry), &pal),
            ModuleKindId::Tuner => paint_readout(ui, scope_rect, readout_note(entry), &pal),
            ModuleKindId::Scope => paint_scope_big(ui, big_rect, entry, &pal),
            ModuleKindId::Spectrum => paint_spectrum(ui, big_rect, entry, &pal),
            _ if params_bottom < scope_rect.min.y - 2.0 => {
                paint_scope(ui, scope_rect, entry, full_scale, &pal);
            }
            _ => {}
        }

        // Ports: inputs along the bottom-left, outputs bottom-right.
        let port_y = rect.max.y - 26.0;
        let label_y = rect.max.y - 44.0;
        for (i, port) in desc.inputs.iter().enumerate() {
            let center = Pos2::new(rect.min.x + 16.0 + i as f32 * 24.0, port_y);
            if self.port_widget(ui, id, PortSide::In, i, center, port.name, label_y, group) {
                topology_changed = true;
            }
        }
        for (i, port) in desc.outputs.iter().enumerate() {
            let center = Pos2::new(rect.max.x - 16.0 - i as f32 * 24.0, port_y);
            if self.port_widget(ui, id, PortSide::Out, i, center, port.name, label_y, group) {
                topology_changed = true;
            }
        }

        topology_changed
    }

    /// The BEATS drum grid: a 4-track × 16-step clickable matrix. Each track's
    /// pattern is a 16-bit mask param; clicking a cell flips its bit and streams
    /// the new mask to the engine. Returns the grid's pixel height.
    /// Draws the 4×16 BEATS grid for module `id` with its top-left at
    /// (`panel_left`, `top`), sized to `width`. Used both on the module panel
    /// and, when exposed, on a collapsed group box. Returns its pixel height.
    fn beats_grid(
        &mut self,
        ui: &mut Ui,
        id: ModuleId,
        panel_left: f32,
        top: f32,
        width: f32,
        link: &mut EngineLink,
    ) -> f32 {
        const STEPS: usize = 16;
        const TRACKS: usize = 4;
        // Per-track accent colours, matching the four trigger outputs.
        const COLORS: [Color32; TRACKS] = [
            Color32::from_rgb(240, 110, 90),
            Color32::from_rgb(240, 200, 80),
            Color32::from_rgb(110, 210, 120),
            Color32::from_rgb(120, 170, 240),
        ];
        let pal = self.palette;
        let Some(inst) = self.patch.modules.get(&id) else { return 0.0 };
        let masks: [u32; TRACKS] =
            std::array::from_fn(|t| inst.params[t] as u32 & 0xFFFF);

        let label_w = 12.0;
        let gap = 2.0;
        let row_gap = 4.0;
        let cell_h = 12.0;
        let left = panel_left + 8.0 + label_w;
        let avail = width - 8.0 - label_w - 8.0;
        let cell_w = ((avail - (STEPS as f32 - 1.0) * gap) / STEPS as f32).max(4.0);

        for t in 0..TRACKS {
            let mut mask = masks[t];
            let y = top + t as f32 * (cell_h + row_gap);
            ui.painter().text(
                Pos2::new(panel_left + 8.0, y + cell_h * 0.5),
                egui::Align2::LEFT_CENTER,
                format!("{}", t + 1),
                egui::FontId::proportional(9.0),
                pal.text_dim,
            );
            for s in 0..STEPS {
                let x = left + s as f32 * (cell_w + gap);
                let cell = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, cell_h));
                let resp = ui.interact(cell, ui.id().with(("beat", id, t, s)), Sense::click());
                let on = mask & (1 << s) != 0;
                // Group beats in fours: every fourth column reads brighter.
                let off_bg = if s % 4 == 0 { pal.cell_off4 } else { pal.cell_off };
                let fill = if on { COLORS[t] } else { off_bg };
                let stroke = if resp.hovered() {
                    Stroke::new(1.0, pal.hover)
                } else {
                    Stroke::new(1.0, pal.cell_border)
                };
                ui.painter().rect(cell, 1.5, fill, stroke, StrokeKind::Inside);
                if resp.clicked() {
                    mask ^= 1 << s;
                    let v = (mask & 0xFFFF) as f32;
                    self.patch.set_param(id, t, v);
                    self.param_changed = true;
                    if let Some(slot) = self.planner.slot_of(id) {
                        link.send(Msg::set_param(slot as u32, t as u32, v));
                    }
                }
            }
        }
        TRACKS as f32 * (cell_h + row_gap) + 6.0
    }

    fn param_widget(
        &mut self,
        ui: &mut Ui,
        id: ModuleId,
        param: u32,
        desc: &ParamDesc,
        link: &mut EngineLink,
        group: Option<GroupId>,
    ) {
        let Some(inst) = self.patch.modules.get(&id) else { return };
        let mut value = inst.params[param as usize];
        let (resp, changed) = if let Some(steps) = desc.steps {
            // Switch: click cycles through positions.
            let mut v = value as u32;
            let label = format!("{}: {}", desc.name, v);
            let resp = ui.small_button(label);
            if resp.clicked() {
                v = (v + 1) % steps;
                value = v as f32;
            }
            let clicked = resp.clicked();
            (resp, clicked)
        } else {
            let resp = knob(ui, desc.name, &mut value, desc.min..=desc.max, desc.default);
            let changed = resp.changed();
            (resp, changed)
        };
        if changed {
            self.patch.set_param(id, param as usize, value);
            self.param_changed = true;
            if let Some(slot) = self.planner.slot_of(id) {
                link.send(Msg::set_param(slot as u32, param, value));
            }
        }

        // Right-click a grouped member's knob/switch to surface it on the
        // collapsed box (mirrors port exposure). Exposed params get a blue tag.
        let exposed = group
            .and_then(|gid| self.patch.groups.get(&gid))
            .map(|g| g.exposed_params.contains(&ParamRef { module: id, param: param as usize }))
            .unwrap_or(false);
        if group.is_some() {
            let pref = ParamRef { module: id, param: param as usize };
            resp.context_menu(|ui| {
                let label = if exposed { "Hide from group" } else { "Expose on group" };
                if ui.button(label).clicked() {
                    self.patch.toggle_exposed_param(pref);
                    self.param_changed = true;
                    ui.close();
                }
            });
        }
        if exposed {
            ui.painter().circle_filled(
                resp.rect.right_top() + Vec2::new(-3.0, 3.0),
                3.0,
                self.palette.accent2,
            );
        }
    }

    /// Returns true if topology changed (cable detached by grabbing an input).
    fn port_widget(
        &mut self,
        ui: &mut Ui,
        id: ModuleId,
        side: PortSide,
        index: usize,
        center: Pos2,
        name: &str,
        label_y: f32,
        group: Option<GroupId>,
    ) -> bool {
        let mut topology_changed = false;
        let rect = Rect::from_center_size(center, Vec2::splat(PORT_R * 2.0 + 4.0));
        self.port_rects.insert((id, side, index), rect);

        // click_and_drag (not drag alone): drag starts a cable, while click
        // sensing is what lets the right-click "Expose on group" menu fire.
        let resp =
            ui.interact(rect, ui.id().with(("port", id, side, index)), Sense::click_and_drag());
        if resp.drag_started() {
            match side {
                PortSide::Out => {
                    self.drag = Some(CableDrag {
                        from_output: PortRef { module: id, port: index },
                    });
                }
                PortSide::In => {
                    // Detach the existing cable and keep dragging its output end.
                    let to = PortRef { module: id, port: index };
                    if let Some(cable) = self.patch.cable_into(to).copied() {
                        self.push_undo();
                        self.patch.disconnect(cable.id);
                        self.drag = Some(CableDrag { from_output: cable.from });
                        topology_changed = true;
                    }
                }
            }
        }

        // Is this port part of its group's exposed interface?
        let exposed = group
            .and_then(|gid| self.patch.groups.get(&gid))
            .map(|g| {
                let list = if side == PortSide::In { &g.exposed_in } else { &g.exposed_out };
                list.contains(&PortRef { module: id, port: index })
            })
            .unwrap_or(false);

        // Right-click a grouped member's port to add/remove it from the group
        // interface (the ports the collapsed box will present).
        if group.is_some() {
            let pref = PortRef { module: id, port: index };
            resp.context_menu(|ui| {
                let label = if exposed { "Remove from group I/O" } else { "Expose on group" };
                if ui.button(label).clicked() {
                    self.patch.toggle_exposed(pref, side == PortSide::In);
                    self.param_changed = true;
                    ui.close();
                }
            });
        }

        let pal = self.palette;
        let painter = ui.painter();
        let fill = match side {
            PortSide::In => pal.port_in,
            PortSide::Out => pal.port_out,
        };
        let ring = if resp.hovered() || self.drag.is_some() {
            pal.accent
        } else if exposed {
            pal.accent2 // exposed ports glow blue
        } else {
            pal.port_ring
        };
        painter.circle(center, PORT_R, fill, Stroke::new(if exposed { 2.0 } else { 1.5 }, ring));
        painter.circle(center, PORT_R * 0.45, pal.port_dot, Stroke::NONE);
        painter.text(
            Pos2::new(center.x, label_y),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(8.0),
            pal.text_dim,
        );
        topology_changed
    }

    fn input_port_at(&self, pos: Pos2) -> Option<PortRef> {
        self.port_rects.iter().find_map(|(&(id, side, index), rect)| {
            (side == PortSide::In && rect.expand(4.0).contains(pos))
                .then_some(PortRef { module: id, port: index })
        })
    }

    fn port_center(&self, id: ModuleId, side: PortSide, index: usize) -> Option<Pos2> {
        self.port_rects.get(&(id, side, index)).map(|r| r.center())
    }

    fn paint_cables(&self, ui: &Ui) {
        // Paint in the rack's own layer (called after the modules, so cables
        // draw over them) rather than a foreground layer — that way menus,
        // dialogs and the toolbar still render on top of the cables.
        let painter = ui.painter().clone();
        for cable in &self.patch.cables {
            let (Some(a), Some(b)) = (
                self.port_center(cable.from.module, PortSide::Out, cable.from.port),
                self.port_center(cable.to.module, PortSide::In, cable.to.port),
            ) else {
                continue;
            };
            paint_cable(&painter, a, b, cable_color(cable.id));
        }
        // In-flight drag: cable from the anchored output to the pointer.
        if let Some(drag) = self.drag {
            if let (Some(a), Some(p)) = (
                self.port_center(drag.from_output.module, PortSide::Out, drag.from_output.port),
                ui.ctx().pointer_interact_pos(),
            ) {
                paint_cable(&painter, a, p, self.palette.accent);
            }
        }
    }
}

fn cable_color(id: u64) -> Color32 {
    const COLORS: [Color32; 5] = [
        Color32::from_rgb(220, 80, 80),
        Color32::from_rgb(80, 160, 220),
        Color32::from_rgb(90, 200, 120),
        Color32::from_rgb(230, 180, 60),
        Color32::from_rgb(180, 110, 220),
    ];
    COLORS[(id % COLORS.len() as u64) as usize]
}

/// Mini oscilloscope: the module's recent output waveform on a dark strip.
fn paint_scope(ui: &Ui, rect: Rect, entry: Option<&MeterEntry>, full_scale: f32, pal: &Palette) {
    let painter = ui.painter();
    painter.rect(
        rect,
        2.0,
        pal.scope_bg,
        Stroke::new(1.0, pal.scope_border),
        StrokeKind::Inside,
    );
    let mid = rect.center().y;
    painter.line_segment(
        [Pos2::new(rect.min.x, mid), Pos2::new(rect.max.x, mid)],
        Stroke::new(1.0, pal.scope_grid),
    );
    let Some(entry) = entry else { return };
    let half = rect.height() * 0.5 - 2.0;
    let n = entry.scope.len();
    let points: Vec<Pos2> = entry
        .scope
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let x = rect.min.x + rect.width() * i as f32 / (n - 1) as f32;
            let y = mid - (s / full_scale).clamp(-1.0, 1.0) * half;
            Pos2::new(x, y)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, pal.scope_line)));
}

/// Large oscilloscope display: the wire's waveform, auto-scaled to its own
/// peak so both quiet CV and full-level audio stay readable, with a grid.
fn paint_scope_big(ui: &Ui, rect: Rect, entry: Option<&MeterEntry>, pal: &Palette) {
    let painter = ui.painter();
    painter.rect(
        rect,
        2.0,
        pal.scope_bg,
        Stroke::new(1.0, pal.scope_border),
        StrokeKind::Inside,
    );
    // Grid: centre line plus a couple of divisions.
    for f in [0.25, 0.5, 0.75] {
        let y = rect.min.y + rect.height() * f;
        painter.line_segment(
            [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
            Stroke::new(1.0, if f == 0.5 { pal.scope_mid } else { pal.scope_grid }),
        );
    }
    for f in [0.25, 0.5, 0.75] {
        let x = rect.min.x + rect.width() * f;
        painter.line_segment(
            [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
            Stroke::new(1.0, pal.scope_grid),
        );
    }
    let Some(entry) = entry else { return };
    // Auto-range to the peak (floor at ~1 V so a flat line doesn't blow up).
    let scale = entry.scope.iter().fold(0.0_f32, |m, &s| m.max(s.abs())).max(1.0) * 1.1;
    let mid = rect.center().y;
    let half = rect.height() * 0.5 - 3.0;
    let n = entry.scope.len();
    let points: Vec<Pos2> = entry
        .scope
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let x = rect.min.x + rect.width() * i as f32 / (n - 1) as f32;
            let y = mid - (s / scale).clamp(-1.0, 1.0) * half;
            Pos2::new(x, y)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, pal.scope_line)));
    painter.text(
        rect.right_top() + Vec2::new(-4.0, 4.0),
        egui::Align2::RIGHT_TOP,
        format!("±{:.1}V", scale / 1.1),
        egui::FontId::monospace(8.0),
        pal.text_dim,
    );
}

/// Spectrum display: magnitude of a direct DFT of the waveform tap, drawn as
/// bars. Computed in the UI from the 128-sample scope buffer (no engine FFT).
fn paint_spectrum(ui: &Ui, rect: Rect, entry: Option<&MeterEntry>, pal: &Palette) {
    let painter = ui.painter();
    painter.rect(
        rect,
        2.0,
        pal.scope_bg,
        Stroke::new(1.0, pal.scope_border),
        StrokeKind::Inside,
    );
    let Some(entry) = entry else { return };
    let s = &entry.scope;
    let n = s.len();
    // Remove DC so a CV offset doesn't dominate bin 0.
    let mean: f32 = s.iter().sum::<f32>() / n as f32;
    let bins = n / 2;
    let mut mags = vec![0.0f32; bins];
    let mut peak = 1e-6f32;
    for (k, mag) in mags.iter_mut().enumerate().skip(1) {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        let w = std::f32::consts::TAU * k as f32 / n as f32;
        for (i, &v) in s.iter().enumerate() {
            let a = w * i as f32;
            re += (v - mean) * a.cos();
            im -= (v - mean) * a.sin();
        }
        *mag = (re * re + im * im).sqrt();
        peak = peak.max(*mag);
    }
    // Log-ish bars, normalised to the loudest bin.
    let bw = rect.width() / bins as f32;
    for (k, &m) in mags.iter().enumerate().skip(1) {
        let norm = (m / peak).clamp(0.0, 1.0);
        let h = norm.sqrt() * (rect.height() - 4.0);
        let x = rect.min.x + k as f32 * bw;
        let bar = Rect::from_min_max(
            Pos2::new(x + 0.5, rect.max.y - 2.0 - h),
            Pos2::new(x + bw - 0.5, rect.max.y - 2.0),
        );
        painter.rect_filled(bar, 0.0, pal.bar);
    }
}

/// Latest sampled value from a module's meter tap (most recent scope sample).
fn latest_value(entry: Option<&MeterEntry>) -> Option<f32> {
    entry.map(|e| *e.scope.last().unwrap_or(&0.0))
}

fn readout_volts(entry: Option<&MeterEntry>) -> String {
    match latest_value(entry) {
        Some(v) => format!("{v:+.2} V"),
        None => "– V".to_owned(),
    }
}

fn readout_note(entry: Option<&MeterEntry>) -> String {
    // The tuner outputs detected pitch as V/oct (0 V = C4); show the note.
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    match latest_value(entry) {
        Some(v) => {
            let midi = (v * 12.0).round() as i32 + 60; // 0 V = C4 = MIDI 60
            let name = NAMES[midi.rem_euclid(12) as usize];
            let octave = midi.div_euclid(12) - 1;
            let cents = ((v * 12.0) - (v * 12.0).round()) * 100.0;
            format!("{name}{octave}  {cents:+.0}¢")
        }
        None => "–".to_owned(),
    }
}

/// Big centered text readout (for voltmeter/tuner panels).
fn paint_readout(ui: &Ui, rect: Rect, text: String, pal: &Palette) {
    let painter = ui.painter();
    painter.rect(
        rect,
        2.0,
        pal.scope_bg,
        Stroke::new(1.0, pal.scope_border),
        StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::monospace(16.0),
        pal.readout,
    );
}

fn paint_cable(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32) {
    let dist = a.distance(b);
    let sag = Vec2::new(0.0, 30.0 + dist * 0.15);
    let shape = egui::epaint::CubicBezierShape::from_points_stroke(
        [a, a + sag, b + sag, b],
        false,
        Color32::TRANSPARENT,
        Stroke::new(3.5, color),
    );
    painter.add(shape);
    painter.circle_filled(a, 4.5, color);
    painter.circle_filled(b, 4.5, color);
}
