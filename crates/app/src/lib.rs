//! egui application: the rack UI editing a rack_graph::Patch; every topology
//! change re-plans and re-syncs the engine, param tweaks stream SetParam.

mod audio;
mod engine_link;
mod keyboard;
mod midi;
mod patch_io;
mod ui;

use eframe::egui;
use engine_link::{EngineLink, LinkState};
use rack_core::messages::Msg;
use rack_core::modules::ModuleKindId;
use rack_graph::{Patch, PortRef};
use std::cell::RefCell;
use std::rc::Rc;
use ui::rack::RackState;
use wasm_bindgen::prelude::*;

/// The boot patch: a complete polyphonic subtractive voice playable from the
/// computer keyboard.
fn demo_patch() -> Patch {
    let mut p = Patch::new();
    let notes = p.add_module(ModuleKindId::NoteIn, (0, 2));
    let vco = p.add_module(ModuleKindId::Vco, (0, 16));
    let vcf = p.add_module(ModuleKindId::Vcf, (0, 30));
    let adsr = p.add_module(ModuleKindId::Adsr, (0, 44));
    let vca = p.add_module(ModuleKindId::Vca, (0, 58));
    let out = p.add_module(ModuleKindId::Output, (0, 72));
    let conn = |p: &mut Patch, fm, fp, tm, tp| {
        p.connect(PortRef { module: fm, port: fp }, PortRef { module: tm, port: tp });
    };
    conn(&mut p, notes, 0, vco, 0); // v/oct → VCO
    conn(&mut p, notes, 1, adsr, 0); // gate → ADSR
    conn(&mut p, notes, 3, adsr, 1); // retrig → ADSR
    conn(&mut p, vco, 0, vcf, 0);
    conn(&mut p, vcf, 0, vca, 0);
    conn(&mut p, adsr, 0, vca, 1); // env → VCA cv
    conn(&mut p, vca, 0, out, 0);
    // 8 voices out of the box.
    p.set_param(notes, 0, 8.0);
    p
}

pub struct RackApp {
    link: Rc<RefCell<EngineLink>>,
    rack: RackState,
    piano: keyboard::KeyboardPiano,
    plan_sent: bool,
    /// Patch edited since the last autosave.
    dirty: bool,
    last_save: f64,
    midi_state: Rc<RefCell<midi::MidiState>>,
    midi: Rc<RefCell<Option<midi::MidiSystem>>>,
    /// Last MIDI CC values forwarded to the engine (change detection).
    cc_cache: [u8; 128],
    /// Whether the `?` keyboard-shortcut overlay is showing.
    show_help: bool,
    /// Saved custom modules (named subpatches) shown in the Custom add-menu.
    customs: Vec<rack_graph::subpatch::CustomModule>,
    /// "Save selection as custom" panel state.
    show_save_custom: bool,
    custom_name: String,
    /// Active UI theme (dark / light / high-contrast variants).
    theme: crate::ui::theme::Theme,
}

impl RackApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let patch = patch_io::load_from_local_storage().unwrap_or_else(demo_patch);
        Self {
            link: Rc::new(RefCell::new(EngineLink::default())),
            rack: RackState::new(patch),
            piano: keyboard::KeyboardPiano::default(),
            plan_sent: false,
            dirty: false,
            last_save: 0.0,
            midi_state: Rc::new(RefCell::new(midi::MidiState::default())),
            midi: Rc::new(RefCell::new(None)),
            cc_cache: [0; 128],
            show_help: false,
            customs: patch_io::load_customs(),
            show_save_custom: false,
            custom_name: String::new(),
            theme: patch_io::load_theme()
                .map(|k| crate::ui::theme::Theme::from_key(&k))
                .unwrap_or(crate::ui::theme::Theme::Dark),
        }
    }

    /// Re-plan and send the full patch plus every param value (param
    /// messages must follow the plan that creates their slots).
    fn sync_engine(&mut self) {
        let blob = self.rack.planner.build(&self.rack.patch);
        let mut link = self.link.borrow_mut();
        link.send_plan(&blob);
        for (id, inst) in &self.rack.patch.modules {
            if let Some(slot) = self.rack.planner.slot_of(*id) {
                for (i, &v) in inst.params.iter().enumerate() {
                    link.send(Msg::set_param(slot as u32, i as u32, v));
                }
            }
        }
    }

    fn power_button(&mut self, ui: &mut egui::Ui) {
        let state = self.link.borrow().state();
        let label = match state {
            LinkState::Off => "⏻ Power on",
            LinkState::Starting => "starting…",
            LinkState::Running => "⏻ Power off",
        };
        if ui.button(label).clicked() {
            match state {
                LinkState::Off => {
                    let link = self.link.clone();
                    let meters = self.link.borrow().meters();
                    wasm_bindgen_futures::spawn_local(async move {
                        match audio::start(meters).await {
                            Ok(sys) => link.borrow_mut().attach(sys),
                            Err(e) => link.borrow_mut().fail(e),
                        }
                    });
                }
                _ => {
                    self.link.borrow_mut().power_off();
                    self.plan_sent = false;
                }
            }
        }
    }
}

impl eframe::App for RackApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        // Apply the active theme's chrome visuals (cheap; egui dedups no-ops).
        ctx.set_visuals(self.theme.visuals());
        egui::Panel::top("toolbar").show_inside(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Music Rack");
                ui.separator();
                self.power_button(ui);
                {
                    let link = self.link.borrow();
                    if let Some(err) = link.failure() {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    } else if link.state() == LinkState::Running {
                        ui.colored_label(egui::Color32::LIGHT_GREEN, "engine running");
                    }
                }
                ui.separator();
                if ui.button("Export").clicked() {
                    patch_io::export_download(&self.rack.patch);
                }
                if ui.button("New").clicked() {
                    self.rack.patch = demo_patch();
                    self.dirty = true;
                    if self.plan_sent {
                        self.sync_engine();
                    }
                }
                ui.separator();
                if ui.add_enabled(self.rack.can_undo(), egui::Button::new("↶ Undo")).clicked()
                    && self.rack.undo()
                {
                    self.dirty = true;
                    if self.plan_sent {
                        self.sync_engine();
                    }
                }
                if ui.add_enabled(self.rack.can_redo(), egui::Button::new("↷ Redo")).clicked()
                    && self.rack.redo()
                {
                    self.dirty = true;
                    if self.plan_sent {
                        self.sync_engine();
                    }
                }
                ui.separator();
                let n_sel = self.rack.selection_count();
                if ui
                    .add_enabled(n_sel >= 2, egui::Button::new("⊟ Group"))
                    .on_hover_text("Collapse the selected modules into one box (Ctrl/⌘+G)")
                    .clicked()
                {
                    self.rack.group_selection();
                }
                let save_label = if n_sel > 1 { format!("Save {n_sel} as custom…") } else { "Save as custom…".to_owned() };
                if ui.add_enabled(n_sel > 0, egui::Button::new(save_label)).clicked() {
                    self.show_save_custom = true;
                }
                if midi::supported() {
                    let midi_on = self.midi.borrow().is_some();
                    let count = self.midi_state.borrow().device_count;
                    let label = if midi_on {
                        format!("MIDI ({count})")
                    } else {
                        "MIDI".to_owned()
                    };
                    if ui.button(label).clicked() && !midi_on {
                        let state = self.midi_state.clone();
                        let slot = self.midi.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            match midi::start(state.clone()).await {
                                Ok(sys) => *slot.borrow_mut() = Some(sys),
                                Err(e) => {
                                    state.borrow_mut().error = Some(format!("{e:?}"));
                                }
                            }
                        });
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("?").on_hover_text("Keyboard shortcuts").clicked() {
                        self.show_help = !self.show_help;
                    }
                    // Theme picker.
                    ui.menu_button(format!("Theme: {}", self.theme.label()), |ui| {
                        for t in crate::ui::theme::Theme::ALL {
                            if ui.selectable_label(self.theme == t, t.label()).clicked() {
                                self.theme = t;
                                patch_io::save_theme(t.key());
                                ui.close();
                            }
                        }
                    });
                    ui.weak(format!(
                        "play: A–K · octave: Z/X (C{}) · ? for shortcuts",
                        4 + self.piano.octave
                    ));
                });
            });
        });

        // `?` toggles the shortcut overlay; Esc closes it.
        if !ctx.egui_wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Questionmark))
        {
            self.show_help = !self.show_help;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_help = false;
        }
        let mut help_open = self.show_help;
        egui::Window::new("Keyboard & mouse")
            .open(&mut help_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(&ctx, help_overlay);
        self.show_help = help_open;

        // "Save selection as custom module" panel + library manager.
        let mut save_open = self.show_save_custom;
        egui::Window::new("Custom modules")
            .open(&mut save_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(&ctx, |ui| {
                let n = self.rack.selection_count();
                ui.label(format!("Save the {n} selected module(s) as a reusable custom module:"));
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.custom_name);
                    let can_save = n > 0 && !self.custom_name.trim().is_empty();
                    if ui.add_enabled(can_save, egui::Button::new("Save")).clicked() {
                        if let Some(sub) = self.rack.extract_selection() {
                            let name = self.custom_name.trim().to_owned();
                            // Replace an existing custom of the same name.
                            self.customs.retain(|c| c.name != name);
                            self.customs.push(rack_graph::subpatch::CustomModule { name, sub });
                            patch_io::save_customs(&self.customs);
                            self.custom_name.clear();
                        }
                    }
                });
                if n == 0 {
                    ui.weak("Select one or more modules in the rack first (click, Shift-click, or box-drag).");
                }
                if !self.customs.is_empty() {
                    ui.separator();
                    ui.label("Saved custom modules (edit a name to rename):");
                    let mut remove: Option<usize> = None;
                    let mut renamed = false;
                    for i in 0..self.customs.len() {
                        ui.horizontal(|ui| {
                            let count = self.customs[i].sub.len();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.customs[i].name)
                                    .desired_width(150.0),
                            );
                            if resp.changed() {
                                renamed = true;
                            }
                            ui.weak(format!("({count} modules)"));
                            if ui.small_button("🗑").on_hover_text("Delete").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if renamed {
                        patch_io::save_customs(&self.customs);
                    }
                    if let Some(i) = remove {
                        self.customs.remove(i);
                        patch_io::save_customs(&self.customs);
                    }
                }
            });
        self.show_save_custom = save_open;

        // Ship the patch as soon as the engine is up.
        let running = self.link.borrow().state() == LinkState::Running;
        if running && !self.plan_sent {
            self.sync_engine();
            self.plan_sent = true;
        }

        let meters_rc = self.link.borrow().meters();
        let palette = self.theme.palette();
        let topology_changed = egui::CentralPanel::default()
            .show_inside(root, |ui| {
                let mut link = self.link.borrow_mut();
                let meters = meters_rc.borrow();
                self.rack.show(ui, &mut link, &meters, &self.customs, palette)
            })
            .inner;
        if topology_changed {
            self.dirty = true;
            if running {
                self.sync_engine();
            }
        }
        if self.rack.param_changed {
            self.rack.param_changed = false;
            self.dirty = true;
        }

        // Import: a .json file dropped anywhere on the window.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let Some(bytes) = file.bytes else { continue };
            match patch_io::import_bytes(&bytes) {
                Ok(patch) => {
                    self.rack.patch = patch;
                    self.dirty = true;
                    if running {
                        self.sync_engine();
                    }
                }
                Err(e) => {
                    web_sys::console::error_1(&format!("patch import failed: {e}").into())
                }
            }
        }

        // Hardware MIDI notes join the same queue as the keyboard.
        for msg in self.midi_state.borrow_mut().drain() {
            self.link.borrow_mut().send(msg);
        }

        // MIDI CC → CV: when a CC value changes, push it to every MIDI-CC
        // module listening on that CC number (via its hidden VALUE param).
        let cc = self.midi_state.borrow().cc;
        for n in 0..128usize {
            if cc[n] == self.cc_cache[n] {
                continue;
            }
            self.cc_cache[n] = cc[n];
            let volts = cc[n] as f32 / 127.0 * 10.0;
            for (id, inst) in &self.rack.patch.modules {
                if inst.kind == ModuleKindId::MidiCc
                    && inst.params.first().map(|c| *c as usize) == Some(n)
                {
                    if let Some(slot) = self.rack.planner.slot_of(*id) {
                        self.link.borrow_mut().send(Msg::set_param(
                            slot as u32,
                            rack_core::modules::params::midi_cc::VALUE,
                            volts,
                        ));
                    }
                }
            }
        }

        self.piano.update(&ctx, &mut self.link.borrow_mut());
        self.link.borrow_mut().flush();

        // Debounced autosave.
        let now = ctx.input(|i| i.time);
        if self.dirty && now - self.last_save > 1.0 {
            patch_io::save_to_local_storage(&self.rack.patch);
            self.dirty = false;
            self.last_save = now;
        }

        // Expose state for headless smoke tests.
        let state_str = match self.link.borrow().state() {
            LinkState::Off => "off",
            LinkState::Starting => "starting",
            LinkState::Running => {
                if self.plan_sent {
                    "patched"
                } else {
                    "running"
                }
            }
        };
        if let Some(window) = web_sys::window() {
            let _ = js_sys::Reflect::set(&window, &"__rackState".into(), &state_str.into());
            let _ = js_sys::Reflect::set(
                &window,
                &"__rackCables".into(),
                &(self.rack.patch.cables.len() as f64).into(),
            );
            let _ = js_sys::Reflect::set(
                &window,
                &"__rackModules".into(),
                &(self.rack.patch.modules.len() as f64).into(),
            );
            let _ = js_sys::Reflect::set(
                &window,
                &"__rackNotes".into(),
                &(self.piano.held_count() as f64).into(),
            );
        }

        if self.link.borrow().state() != LinkState::Off {
            // Animate scopes/meters at ~30 fps. request_repaint_after (a timed
            // single repaint) is the correct pattern for animation; the bare
            // request_repaint() tight-loops at the display's max rate, which
            // both wastes cycles and stresses eframe's web event loop.
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }
}

/// Contents of the `?` keyboard/mouse shortcut overlay.
fn help_overlay(ui: &mut egui::Ui) {
    let row = |ui: &mut egui::Ui, keys: &str, desc: &str| {
        ui.horizontal(|ui| {
            ui.add_sized([130.0, 16.0], egui::Label::new(egui::RichText::new(keys).monospace().strong()));
            ui.label(desc);
        });
    };
    ui.heading("Play");
    row(ui, "A S D F G H J K", "white keys (C–C)");
    row(ui, "W E T Y U", "black keys (sharps)");
    row(ui, "Z / X", "octave down / up");
    ui.separator();
    ui.heading("Select");
    row(ui, "click panel", "select one module");
    row(ui, "Shift/Ctrl-click", "add / remove from selection");
    row(ui, "drag empty rack", "box-select a group");
    row(ui, "Ctrl/⌘ + G", "collapse selection into a group box");
    row(ui, "double-click box", "expand a group");
    row(ui, "right-click port (in group)", "expose / hide on the box");
    row(ui, "right-click knob (in group)", "expose / hide on the box");
    ui.separator();
    ui.heading("Edit");
    row(ui, "Ctrl/⌘ + C", "copy selection");
    row(ui, "Ctrl/⌘ + V", "paste (at the pointer)");
    row(ui, "Ctrl/⌘ + D", "duplicate selection");
    row(ui, "Delete / ⌫", "remove selection");
    row(ui, "Ctrl/⌘ + Z", "undo");
    row(ui, "Ctrl/⌘ + Y", "redo (or Ctrl/⌘ + Shift + Z)");
    ui.separator();
    ui.heading("Mouse");
    row(ui, "right-click rack", "add a module / custom");
    row(ui, "drag title bar", "move a module");
    row(ui, "drag a port", "patch a cable");
    row(ui, "drag an input", "detach its cable");
    row(ui, "drag a knob", "adjust (Shift = fine)");
    row(ui, "double-click knob", "reset to default");
    row(ui, "right-click module", "copy / duplicate / remove");
    ui.separator();
    ui.heading("Custom modules & files");
    row(ui, "Save as custom…", "save selection as a reusable module");
    row(ui, "Add ▸ Custom", "place a saved custom module");
    row(ui, "drop patch.json", "import a patch");
    row(ui, "Export / New", "toolbar buttons");
    ui.add_space(4.0);
    ui.weak("Press ? or Esc to close.");
}

#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        // Surface Rust panics in the browser console instead of the opaque
        // "unreachable executed" / closure errors.
        console_error_panic_hook::set_once();
        Self { runner: eframe::WebRunner::new() }
    }

    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(RackApp::new(cc)))),
            )
            .await
    }
}
