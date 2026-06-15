//! UI-side patch model: the authoritative document the rack UI edits, and the
//! planner that flattens it into an execution plan for the audio thread.

pub mod persist;
pub mod plan;
pub mod subpatch;

use rack_core::modules::ModuleKindId;
use std::collections::BTreeMap;

pub type ModuleId = u64;
pub type CableId = u64;
pub type GroupId = u64;

/// A UI-level grouping of modules that can be collapsed into one box. Members
/// stay normal flat modules in the patch (the planner ignores groups); the UI
/// draws a collapsed group as a single panel exposing its boundary-crossing
/// ports. Lives in the patch so save/load and undo carry it automatically.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub members: Vec<ModuleId>,
    pub collapsed: bool,
    /// Box position (row, hp column) when collapsed.
    pub pos: (i32, i32),
    /// The interface: which member ports the collapsed box exposes. Defaults
    /// to the ports that cross the group boundary, but is hand-editable — you
    /// can expose unconnected ports too, like designing a real module's I/O.
    pub exposed_in: Vec<PortRef>,
    pub exposed_out: Vec<PortRef>,
    /// Member params (knobs/switches) surfaced on the collapsed box so they
    /// stay tweakable without expanding. Opt-in: empty by default.
    pub exposed_params: Vec<ParamRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleInst {
    pub kind: ModuleKindId,
    /// Rack position (row, hp column) — UI concern, carried for save/load.
    pub pos: (i32, i32),
    /// Current param values, indexed like the descriptor's params array.
    pub params: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortRef {
    pub module: ModuleId,
    pub port: usize,
}

/// A reference to one param of a member module, for group knob exposure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamRef {
    pub module: ModuleId,
    pub param: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cable {
    pub id: CableId,
    /// Output port of the producing module.
    pub from: PortRef,
    /// Input port of the consuming module.
    pub to: PortRef,
}

/// The patch document. IDs are monotonic and never reused so save/load and
/// undo stay stable.
#[derive(Clone, Debug, Default)]
pub struct Patch {
    pub modules: BTreeMap<ModuleId, ModuleInst>,
    pub cables: Vec<Cable>,
    pub groups: BTreeMap<GroupId, Group>,
    next_id: u64,
}

impl Patch {
    pub fn new() -> Self {
        Self {
            modules: BTreeMap::new(),
            cables: Vec::new(),
            groups: BTreeMap::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// For deserialization: restore the persisted counter.
    pub fn set_next_id(&mut self, next: u64) {
        self.next_id = self.next_id.max(next);
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    pub fn add_module(&mut self, kind: ModuleKindId, pos: (i32, i32)) -> ModuleId {
        let id = self.alloc_id();
        let params = kind.desc().params.iter().map(|p| p.default).collect();
        self.modules.insert(id, ModuleInst { kind, pos, params });
        id
    }

    /// Remove a module and every cable touching it, and drop it from any
    /// group (removing the group if it becomes empty).
    pub fn remove_module(&mut self, id: ModuleId) {
        self.modules.remove(&id);
        self.cables.retain(|c| c.from.module != id && c.to.module != id);
        for g in self.groups.values_mut() {
            g.members.retain(|m| *m != id);
            g.exposed_in.retain(|p| p.module != id);
            g.exposed_out.retain(|p| p.module != id);
            g.exposed_params.retain(|p| p.module != id);
        }
        self.groups.retain(|_, g| !g.members.is_empty());
    }

    /// Ports of a member set that connect to non-members — (inputs, outputs).
    /// The default exposed interface for a group of those members.
    pub fn boundary_ports_for(&self, members: &[ModuleId]) -> (Vec<PortRef>, Vec<PortRef>) {
        let is_member = |m: ModuleId| members.contains(&m);
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for c in &self.cables {
            let from_in = is_member(c.from.module);
            let to_in = is_member(c.to.module);
            if to_in && !from_in && !inputs.contains(&c.to) {
                inputs.push(c.to);
            }
            if from_in && !to_in && !outputs.contains(&c.from) {
                outputs.push(c.from);
            }
        }
        (inputs, outputs)
    }

    /// Collapse a set of modules into a new group (collapsed by default).
    /// Members are removed from any prior group first; empty groups are
    /// pruned. Returns the new group id, or None if nothing valid was given.
    pub fn create_group(&mut self, members: &[ModuleId], name: String, pos: (i32, i32)) -> Option<GroupId> {
        let members: Vec<ModuleId> =
            members.iter().copied().filter(|id| self.modules.contains_key(id)).collect();
        if members.is_empty() {
            return None;
        }
        for g in self.groups.values_mut() {
            g.members.retain(|m| !members.contains(m));
        }
        self.groups.retain(|_, g| !g.members.is_empty());
        let (exposed_in, exposed_out) = self.boundary_ports_for(&members);
        let id = self.alloc_id();
        self.groups.insert(
            id,
            Group { name, members, collapsed: true, pos, exposed_in, exposed_out, exposed_params: Vec::new() },
        );
        Some(id)
    }

    pub fn ungroup(&mut self, gid: GroupId) {
        self.groups.remove(&gid);
    }

    /// The group a module belongs to, if any.
    pub fn group_of(&self, module: ModuleId) -> Option<GroupId> {
        self.groups.iter().find(|(_, g)| g.members.contains(&module)).map(|(id, _)| *id)
    }

    /// Toggle whether a member port is part of its group's exposed interface.
    /// `input` selects the input vs output list. No-op if the module isn't in
    /// a group. Returns true if a change was made.
    pub fn toggle_exposed(&mut self, port: PortRef, input: bool) -> bool {
        let Some(gid) = self.group_of(port.module) else { return false };
        let Some(g) = self.groups.get_mut(&gid) else { return false };
        let list = if input { &mut g.exposed_in } else { &mut g.exposed_out };
        if let Some(i) = list.iter().position(|p| *p == port) {
            list.remove(i);
        } else {
            list.push(port);
        }
        true
    }

    /// Toggle whether a member param (knob/switch) appears on its group's
    /// collapsed box. No-op if the module isn't in a group. Returns true if a
    /// change was made.
    pub fn toggle_exposed_param(&mut self, param: ParamRef) -> bool {
        let Some(gid) = self.group_of(param.module) else { return false };
        let Some(g) = self.groups.get_mut(&gid) else { return false };
        if let Some(i) = g.exposed_params.iter().position(|p| *p == param) {
            g.exposed_params.remove(i);
        } else {
            g.exposed_params.push(param);
        }
        true
    }

    /// Connect an output port to an input port. Replaces any existing cable
    /// on the input (one cable per input, VCV-style). Returns None if the
    /// ports are invalid.
    pub fn connect(&mut self, from: PortRef, to: PortRef) -> Option<CableId> {
        let from_mod = self.modules.get(&from.module)?;
        let to_mod = self.modules.get(&to.module)?;
        if from.port >= from_mod.kind.desc().outputs.len()
            || to.port >= to_mod.kind.desc().inputs.len()
        {
            return None;
        }
        self.cables.retain(|c| c.to != to);
        let id = self.alloc_id();
        self.cables.push(Cable { id, from, to });
        Some(id)
    }

    pub fn disconnect(&mut self, cable: CableId) {
        self.cables.retain(|c| c.id != cable);
    }

    /// The cable feeding an input port, if any.
    pub fn cable_into(&self, to: PortRef) -> Option<&Cable> {
        self.cables.iter().find(|c| c.to == to)
    }

    pub fn set_param(&mut self, id: ModuleId, param: usize, value: f32) {
        if let Some(m) = self.modules.get_mut(&id) {
            if let Some(p) = m.params.get_mut(param) {
                *p = value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_replaces_existing_input_cable() {
        let mut p = Patch::new();
        let vco1 = p.add_module(ModuleKindId::Vco, (0, 0));
        let vco2 = p.add_module(ModuleKindId::Vco, (0, 10));
        let out = p.add_module(ModuleKindId::Output, (0, 20));

        let to = PortRef { module: out, port: 0 };
        p.connect(PortRef { module: vco1, port: 0 }, to).unwrap();
        p.connect(PortRef { module: vco2, port: 0 }, to).unwrap();
        assert_eq!(p.cables.len(), 1);
        assert_eq!(p.cable_into(to).unwrap().from.module, vco2);
    }

    #[test]
    fn invalid_ports_rejected() {
        let mut p = Patch::new();
        let vco = p.add_module(ModuleKindId::Vco, (0, 0));
        let out = p.add_module(ModuleKindId::Output, (0, 20));
        // VCO has 1 output (index 0); port 5 is invalid.
        assert!(p
            .connect(PortRef { module: vco, port: 5 }, PortRef { module: out, port: 0 })
            .is_none());
        // Output module has no outputs.
        assert!(p
            .connect(PortRef { module: out, port: 0 }, PortRef { module: vco, port: 0 })
            .is_none());
    }

    #[test]
    fn remove_module_drops_cables() {
        let mut p = Patch::new();
        let vco = p.add_module(ModuleKindId::Vco, (0, 0));
        let out = p.add_module(ModuleKindId::Output, (0, 20));
        p.connect(PortRef { module: vco, port: 0 }, PortRef { module: out, port: 0 })
            .unwrap();
        p.remove_module(vco);
        assert!(p.cables.is_empty());
    }

    #[test]
    fn ids_never_reused() {
        let mut p = Patch::new();
        let a = p.add_module(ModuleKindId::Vco, (0, 0));
        p.remove_module(a);
        let b = p.add_module(ModuleKindId::Vco, (0, 0));
        assert_ne!(a, b);
    }

    fn voice() -> (Patch, [ModuleId; 4]) {
        // NoteIn → VCO → VCF → Output.
        let mut p = Patch::new();
        let notes = p.add_module(ModuleKindId::NoteIn, (0, 0));
        let vco = p.add_module(ModuleKindId::Vco, (0, 12));
        let vcf = p.add_module(ModuleKindId::Vcf, (0, 24));
        let out = p.add_module(ModuleKindId::Output, (0, 36));
        p.connect(PortRef { module: notes, port: 0 }, PortRef { module: vco, port: 0 });
        p.connect(PortRef { module: vco, port: 0 }, PortRef { module: vcf, port: 0 });
        p.connect(PortRef { module: vcf, port: 0 }, PortRef { module: out, port: 0 });
        (p, [notes, vco, vcf, out])
    }

    #[test]
    fn group_membership_and_ungroup() {
        let (mut p, [_notes, vco, vcf, _out]) = voice();
        let g = p.create_group(&[vco, vcf], "tone".into(), (0, 12)).unwrap();
        assert_eq!(p.group_of(vco), Some(g));
        assert_eq!(p.group_of(vcf), Some(g));
        assert_eq!(p.group_of(_out), None);
        p.ungroup(g);
        assert_eq!(p.group_of(vco), None);
    }

    #[test]
    fn boundary_ports_are_the_crossing_connections() {
        let (mut p, [_notes, vco, vcf, _out]) = voice();
        // Group VCO+VCF. The VCO→VCF cable is internal. The NoteIn→VCO cable
        // crosses in (exposed input on VCO), and VCF→Output crosses out
        // (exposed output on VCF).
        let g = p.create_group(&[vco, vcf], "tone".into(), (0, 12)).unwrap();
        // The group's default exposed interface = the boundary-crossing ports.
        let grp = &p.groups[&g];
        assert_eq!(grp.exposed_in, vec![PortRef { module: vco, port: 0 }]);
        assert_eq!(grp.exposed_out, vec![PortRef { module: vcf, port: 0 }]);
    }

    #[test]
    fn removing_a_member_prunes_the_group() {
        let (mut p, [_notes, vco, vcf, _out]) = voice();
        let g = p.create_group(&[vco, vcf], "tone".into(), (0, 12)).unwrap();
        p.remove_module(vco);
        // Group still exists with one member.
        assert_eq!(p.groups[&g].members, vec![vcf]);
        p.remove_module(vcf);
        // Now empty → pruned.
        assert!(p.groups.is_empty());
    }

    #[test]
    fn regrouping_moves_members_out_of_old_group() {
        let (mut p, [notes, vco, vcf, _out]) = voice();
        let g1 = p.create_group(&[vco, vcf], "a".into(), (0, 12)).unwrap();
        // New group claims vcf — it leaves g1.
        let _g2 = p.create_group(&[vcf, notes], "b".into(), (0, 0)).unwrap();
        assert_eq!(p.groups[&g1].members, vec![vco]);
        assert_eq!(p.group_of(vcf), Some(_g2));
    }
}
