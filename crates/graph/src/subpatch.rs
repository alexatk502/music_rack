//! A reusable fragment of a patch: a set of modules plus the cables wholly
//! between them, with positions normalized to the group's top-left. This is
//! the shared representation behind group copy/paste *and* saved custom
//! modules. Like the patch format, it references kinds/ports/params by name
//! so fragments stay loadable as the module set evolves.

use crate::{Group, GroupId, ModuleId, ParamRef, Patch, PortRef};
use rack_core::modules::ModuleKindId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const SUBPATCH_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubModule {
    #[serde(rename = "type")]
    pub type_name: String,
    /// Position relative to the group's top-left (row, hp column).
    pub pos: (i32, i32),
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubCable {
    /// Indices into `SubPatch::modules`.
    pub from_mod: usize,
    pub from_port: String,
    pub to_mod: usize,
    pub to_port: String,
}

/// An exposed port of the fragment's interface: a member port (by module index
/// + port name) that becomes an input/output of the collapsed box.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubExposed {
    pub module: usize,
    pub port: String,
}

/// An exposed param of the fragment: a member knob/switch (by module index +
/// param name) surfaced on the collapsed box.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubExposedParam {
    pub module: usize,
    pub param: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubPatch {
    pub version: u32,
    pub modules: Vec<SubModule>,
    pub cables: Vec<SubCable>,
    /// Designed interface (empty in older fragments → falls back to boundary).
    #[serde(default)]
    pub exposed_in: Vec<SubExposed>,
    #[serde(default)]
    pub exposed_out: Vec<SubExposed>,
    /// Knobs/switches surfaced on the collapsed box (empty in older fragments).
    #[serde(default)]
    pub exposed_params: Vec<SubExposedParam>,
}

/// A named, saved fragment shown in the "Custom" add-menu.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomModule {
    pub name: String,
    pub sub: SubPatch,
}

/// Serialize the whole custom-module library (for localStorage / export).
pub fn customs_to_json(list: &[CustomModule]) -> String {
    serde_json::to_string(list).unwrap_or_else(|_| "[]".to_owned())
}

/// Load a custom-module library, tolerating corruption (returns empty).
pub fn customs_from_json(json: &str) -> Vec<CustomModule> {
    serde_json::from_str(json).unwrap_or_default()
}

impl SubPatch {
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("subpatch serialization is infallible")
    }

    pub fn from_json(json: &str) -> Result<SubPatch, String> {
        let sub: SubPatch = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if sub.version > SUBPATCH_VERSION {
            return Err(format!("subpatch version {} is newer than supported", sub.version));
        }
        Ok(sub)
    }
}

impl Patch {
    /// Capture the given modules (and only the cables whose *both* ends are in
    /// the set) as a reusable fragment. Positions are made relative to the
    /// group's top-left so it can be stamped down anywhere.
    pub fn extract(&self, ids: &[ModuleId]) -> SubPatch {
        let mut ids: Vec<ModuleId> =
            ids.iter().copied().filter(|id| self.modules.contains_key(id)).collect();
        ids.sort_unstable();
        let index: HashMap<ModuleId, usize> =
            ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let min_row = ids.iter().map(|id| self.modules[id].pos.0).min().unwrap_or(0);
        let min_col = ids.iter().map(|id| self.modules[id].pos.1).min().unwrap_or(0);

        let modules = ids
            .iter()
            .map(|id| {
                let inst = &self.modules[id];
                let desc = inst.kind.desc();
                SubModule {
                    type_name: inst.kind.type_name().to_owned(),
                    pos: (inst.pos.0 - min_row, inst.pos.1 - min_col),
                    params: desc
                        .params
                        .iter()
                        .zip(&inst.params)
                        .map(|(p, &v)| (p.name.to_owned(), v))
                        .collect(),
                }
            })
            .collect();

        let cables = self
            .cables
            .iter()
            .filter_map(|c| {
                let fi = *index.get(&c.from.module)?;
                let ti = *index.get(&c.to.module)?;
                let from_desc = self.modules[&c.from.module].kind.desc();
                let to_desc = self.modules[&c.to.module].kind.desc();
                Some(SubCable {
                    from_mod: fi,
                    from_port: from_desc.outputs.get(c.from.port)?.name.to_owned(),
                    to_mod: ti,
                    to_port: to_desc.inputs.get(c.to.port)?.name.to_owned(),
                })
            })
            .collect();

        // Default interface = the ports crossing the selection boundary.
        let (bin, bout) = self.boundary_ports_for(&ids);
        let to_sub = |ports: &[PortRef], input: bool| -> Vec<SubExposed> {
            ports
                .iter()
                .filter_map(|p| {
                    let mi = *index.get(&p.module)?;
                    let desc = self.modules[&p.module].kind.desc();
                    let name = if input { desc.inputs.get(p.port) } else { desc.outputs.get(p.port) };
                    Some(SubExposed { module: mi, port: name?.name.to_owned() })
                })
                .collect()
        };
        let exposed_in = to_sub(&bin, true);
        let exposed_out = to_sub(&bout, false);

        // A bare selection has no group, so no exposed knobs yet.
        SubPatch {
            version: SUBPATCH_VERSION,
            modules,
            cables,
            exposed_in,
            exposed_out,
            exposed_params: Vec::new(),
        }
    }

    /// Like [`Patch::extract`] but uses a group's *designed* exposed interface
    /// (which may include unconnected ports) instead of boundary detection.
    pub fn extract_group(&self, gid: GroupId) -> SubPatch {
        let Some(g) = self.groups.get(&gid) else {
            return SubPatch {
                version: SUBPATCH_VERSION,
                modules: Vec::new(),
                cables: Vec::new(),
                exposed_in: Vec::new(),
                exposed_out: Vec::new(),
                exposed_params: Vec::new(),
            };
        };
        let mut sub = self.extract(&g.members);
        // Local index of a module within the (sorted) fragment.
        let mut ids: Vec<ModuleId> = g.members.clone();
        ids.sort_unstable();
        let local = |m: ModuleId| ids.iter().position(|x| *x == m);
        let map_ports = |ports: &[PortRef], input: bool| -> Vec<SubExposed> {
            ports
                .iter()
                .filter_map(|p| {
                    let mi = local(p.module)?;
                    let desc = self.modules.get(&p.module)?.kind.desc();
                    let name = if input { desc.inputs.get(p.port) } else { desc.outputs.get(p.port) };
                    Some(SubExposed { module: mi, port: name?.name.to_owned() })
                })
                .collect()
        };
        sub.exposed_in = map_ports(&g.exposed_in, true);
        sub.exposed_out = map_ports(&g.exposed_out, false);
        sub.exposed_params = g
            .exposed_params
            .iter()
            .filter_map(|p| {
                let mi = local(p.module)?;
                let desc = self.modules.get(&p.module)?.kind.desc();
                Some(SubExposedParam { module: mi, param: desc.params.get(p.param)?.name.to_owned() })
            })
            .collect();
        sub
    }

    /// Stamp a fragment into this patch at `at`. Returns the new module ids
    /// (fragment order) and the per-local-index id map (None = unknown kind).
    fn insert_mapped(&mut self, sub: &SubPatch, at: (i32, i32)) -> (Vec<ModuleId>, Vec<Option<ModuleId>>) {
        let mut new_ids = Vec::with_capacity(sub.modules.len());
        let mut map: Vec<Option<ModuleId>> = Vec::with_capacity(sub.modules.len());

        for m in &sub.modules {
            match ModuleKindId::from_type_name(&m.type_name) {
                Some(kind) => {
                    let pos = ((at.0 + m.pos.0).max(0), (at.1 + m.pos.1).max(0));
                    let id = self.add_module(kind, pos);
                    let defaults = kind.desc().params;
                    if let Some(inst) = self.modules.get_mut(&id) {
                        inst.params = defaults
                            .iter()
                            .map(|p| m.params.get(p.name).copied().unwrap_or(p.default))
                            .collect();
                    }
                    map.push(Some(id));
                    new_ids.push(id);
                }
                None => map.push(None),
            }
        }

        for c in &sub.cables {
            let (Some(Some(from_id)), Some(Some(to_id))) =
                (map.get(c.from_mod), map.get(c.to_mod))
            else {
                continue;
            };
            let from_kind = self.modules[from_id].kind;
            let to_kind = self.modules[to_id].kind;
            let fp = from_kind.desc().outputs.iter().position(|p| p.name == c.from_port);
            let tp = to_kind.desc().inputs.iter().position(|p| p.name == c.to_port);
            if let (Some(fp), Some(tp)) = (fp, tp) {
                self.connect(
                    PortRef { module: *from_id, port: fp },
                    PortRef { module: *to_id, port: tp },
                );
            }
        }
        (new_ids, map)
    }

    /// Stamp a fragment in as flat modules; returns the new module ids.
    pub fn insert(&mut self, sub: &SubPatch, at: (i32, i32)) -> Vec<ModuleId> {
        self.insert_mapped(sub, at).0
    }

    /// Stamp a fragment in and immediately fold it into a collapsed group,
    /// carrying the fragment's designed interface. Returns the new group id.
    pub fn insert_as_group(&mut self, sub: &SubPatch, at: (i32, i32), name: String) -> Option<GroupId> {
        let (new_ids, map) = self.insert_mapped(sub, at);
        if new_ids.is_empty() {
            return None;
        }
        let resolve = |specs: &[SubExposed], input: bool, patch: &Patch| -> Vec<PortRef> {
            specs
                .iter()
                .filter_map(|e| {
                    let id = (*map.get(e.module)?)?;
                    let desc = patch.modules.get(&id)?.kind.desc();
                    let pi = if input {
                        desc.inputs.iter().position(|p| p.name == e.port)
                    } else {
                        desc.outputs.iter().position(|p| p.name == e.port)
                    }?;
                    Some(PortRef { module: id, port: pi })
                })
                .collect()
        };
        let exposed_in = resolve(&sub.exposed_in, true, self);
        let exposed_out = resolve(&sub.exposed_out, false, self);
        let exposed_params: Vec<ParamRef> = sub
            .exposed_params
            .iter()
            .filter_map(|e| {
                let id = (*map.get(e.module)?)?;
                let pi = self.modules.get(&id)?.kind.desc().params.iter().position(|p| p.name == e.param)?;
                Some(ParamRef { module: id, param: pi })
            })
            .collect();
        let gid = self.alloc_id();
        self.groups.insert(
            gid,
            Group { name, members: new_ids, collapsed: true, pos: at, exposed_in, exposed_out, exposed_params },
        );
        Some(gid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VCO → VCF → Output chain plus a stray LFO; returns (patch, ids).
    fn sample() -> (Patch, [ModuleId; 4]) {
        let mut p = Patch::new();
        let vco = p.add_module(ModuleKindId::Vco, (0, 4));
        let vcf = p.add_module(ModuleKindId::Vcf, (0, 16));
        let out = p.add_module(ModuleKindId::Output, (0, 28));
        let lfo = p.add_module(ModuleKindId::Lfo, (1, 4));
        p.connect(PortRef { module: vco, port: 0 }, PortRef { module: vcf, port: 0 });
        p.connect(PortRef { module: vcf, port: 0 }, PortRef { module: out, port: 0 });
        p.set_param(vco, 0, 1.25);
        (p, [vco, vcf, out, lfo])
    }

    #[test]
    fn extract_keeps_only_internal_cables() {
        let (p, [vco, vcf, _out, _lfo]) = sample();
        // Select VCO + VCF: the VCO→VCF cable is internal; VCF→Output isn't.
        let sub = p.extract(&[vco, vcf]);
        assert_eq!(sub.modules.len(), 2);
        assert_eq!(sub.cables.len(), 1, "only the VCO→VCF cable should be captured");
        assert_eq!(sub.cables[0].from_port, "out");
        assert_eq!(sub.cables[0].to_port, "in");
        // Positions normalized: the leftmost module sits at column 0.
        assert_eq!(sub.modules.iter().map(|m| m.pos.1).min(), Some(0));
    }

    #[test]
    fn extract_preserves_params() {
        let (p, [vco, ..]) = sample();
        let sub = p.extract(&[vco]);
        assert_eq!(sub.modules[0].params.get("pitch"), Some(&1.25));
    }

    #[test]
    fn insert_recreates_modules_and_internal_cables() {
        let (p, [vco, vcf, out, _lfo]) = sample();
        let sub = p.extract(&[vco, vcf, out]);

        let mut dest = Patch::new();
        let new_ids = dest.insert(&sub, (5, 0));
        assert_eq!(new_ids.len(), 3);
        assert_eq!(dest.modules.len(), 3);
        // Both internal cables (VCO→VCF, VCF→Output) come back.
        assert_eq!(dest.cables.len(), 2);
        // Pasted at row 5: relative rows preserved.
        assert!(dest.modules.values().all(|m| m.pos.0 >= 5));
        // Param carried through.
        let vco_inst = dest.modules.values().find(|m| m.kind == ModuleKindId::Vco).unwrap();
        assert_eq!(vco_inst.params[0], 1.25);
    }

    #[test]
    fn paste_into_same_patch_duplicates_group() {
        let (mut p, [vco, vcf, out, _lfo]) = sample();
        let sub = p.extract(&[vco, vcf, out]);
        let before_mods = p.modules.len();
        let before_cables = p.cables.len();
        let new_ids = p.insert(&sub, (3, 0));
        assert_eq!(new_ids.len(), 3);
        assert_eq!(p.modules.len(), before_mods + 3);
        // The 2 internal cables are duplicated; the originals remain.
        assert_eq!(p.cables.len(), before_cables + 2);
    }

    #[test]
    fn json_roundtrip() {
        let (p, [vco, vcf, ..]) = sample();
        let sub = p.extract(&[vco, vcf]);
        let json = sub.to_json();
        let back = SubPatch::from_json(&json).expect("loads");
        assert_eq!(back.modules.len(), sub.modules.len());
        assert_eq!(back.cables.len(), sub.cables.len());
        assert_eq!(back.cables[0].from_port, "out");

        let mut dest = Patch::new();
        assert_eq!(dest.insert(&back, (0, 0)).len(), 2);
    }

    #[test]
    fn custom_library_roundtrips() {
        let (p, [vco, vcf, ..]) = sample();
        let lib = vec![
            CustomModule { name: "my voice".into(), sub: p.extract(&[vco, vcf]) },
            CustomModule { name: "just vco".into(), sub: p.extract(&[vco]) },
        ];
        let json = customs_to_json(&lib);
        let back = customs_from_json(&json);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "my voice");
        assert_eq!(back[0].sub.modules.len(), 2);
        assert_eq!(back[1].sub.modules.len(), 1);
        // Corrupt input degrades to an empty library, never panics.
        assert!(customs_from_json("not json").is_empty());
    }

    #[test]
    fn extract_captures_boundary_interface() {
        let (p, [vco, vcf, _out, _lfo]) = sample();
        let sub = p.extract(&[vco, vcf]);
        // VCO's v/oct input crosses in; VCF's out crosses out (to Output).
        // (sample wires NoteIn?? no — sample has no NoteIn; vco.in is its 0th
        // input. The boundary here is the VCF→Output cable on the output side.)
        assert!(sub.exposed_out.iter().any(|e| e.port == "out"));
    }

    #[test]
    fn insert_as_group_makes_a_collapsed_box_with_interface() {
        // Build a custom: VCO → VCF, expose VCO v/oct in and VCF out.
        let (p, [vco, vcf, ..]) = sample();
        let mut sub = p.extract(&[vco, vcf]);
        // Force a known interface regardless of boundary detection.
        sub.exposed_in = vec![SubExposed { module: 0, port: "v/oct".into() }];
        sub.exposed_out = vec![SubExposed { module: 1, port: "out".into() }];

        let mut dest = Patch::new();
        let gid = dest.insert_as_group(&sub, (0, 0), "my voice".into()).unwrap();
        let g = &dest.groups[&gid];
        assert!(g.collapsed);
        assert_eq!(g.members.len(), 2);
        assert_eq!(g.name, "my voice");
        // Interface resolved onto the new modules, even with no external wires.
        assert_eq!(g.exposed_in.len(), 1);
        assert_eq!(g.exposed_out.len(), 1);
        // The exposed input is a real input port on a member module.
        let p_in = g.exposed_in[0];
        assert!(g.members.contains(&p_in.module));
    }

    #[test]
    fn extract_group_uses_designed_interface() {
        let (mut p, [vco, vcf, _out, _lfo]) = sample();
        let gid = p.create_group(&[vco, vcf], "g".into(), (0, 0)).unwrap();
        // Hand-expose VCF's cutoff CV (input 1) which has no cable.
        p.toggle_exposed(PortRef { module: vcf, port: 1 }, true);
        let sub = p.extract_group(gid);
        // The designed interface (incl. the unconnected cutoff CV) is captured.
        assert!(sub.exposed_in.iter().any(|e| e.port == "cutoff cv"));
    }

    #[test]
    fn exposed_params_round_trip_through_a_custom() {
        let (mut p, [vco, vcf, _out, _lfo]) = sample();
        let gid = p.create_group(&[vco, vcf], "g".into(), (0, 0)).unwrap();
        // Surface the VCO's first knob on the box.
        let pname = ModuleKindId::Vco.desc().params[0].name;
        assert!(p.toggle_exposed_param(crate::ParamRef { module: vco, param: 0 }));
        // Extract as a custom, then stamp it back in as a fresh group.
        let sub = p.extract_group(gid);
        assert!(sub.exposed_params.iter().any(|e| e.param == pname));
        let mut dest = Patch::new();
        let new_gid = dest.insert_as_group(&sub, (0, 0), "copy".into()).unwrap();
        let g = &dest.groups[&new_gid];
        assert_eq!(g.exposed_params.len(), 1, "the exposed knob came along");
        assert!(g.members.contains(&g.exposed_params[0].module));
        assert_eq!(g.exposed_params[0].param, 0);
    }

    #[test]
    fn unknown_kinds_are_skipped_on_insert() {
        let json = r#"{"version":1,"modules":[
            {"type":"vco","pos":[0,0],"params":{}},
            {"type":"imaginary_module","pos":[0,5],"params":{}}
        ],"cables":[
            {"from_mod":0,"from_port":"out","to_mod":1,"to_port":"in"}
        ]}"#;
        let sub = SubPatch::from_json(json).unwrap();
        let mut p = Patch::new();
        let ids = p.insert(&sub, (0, 0));
        assert_eq!(ids.len(), 1, "only the VCO instantiates");
        assert_eq!(p.cables.len(), 0, "cable to the missing module is dropped");
    }
}
