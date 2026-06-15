//! Patch serialization. Stability rules: module types and ports serialize by
//! *name* (indices would break on reorder), params as a name→value map with
//! unknown names ignored and missing names defaulted (old patches survive
//! new params), `version` gates future migrations, `next_id` persists so IDs
//! are never reused across save/load.

use crate::{Cable, Group, ModuleInst, ParamRef, Patch, PortRef};
use rack_core::modules::ModuleKindId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PATCH_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct PatchDoc {
    version: u32,
    next_id: u64,
    modules: Vec<ModuleDoc>,
    cables: Vec<CableDoc>,
    /// Groups arrived after v1 patches; default-empty keeps old patches loading.
    #[serde(default)]
    groups: Vec<GroupDoc>,
}

#[derive(Serialize, Deserialize)]
struct GroupDoc {
    id: u64,
    name: String,
    members: Vec<u64>,
    collapsed: bool,
    pos: (i32, i32),
    #[serde(default)]
    exposed_in: Vec<PortDoc>,
    #[serde(default)]
    exposed_out: Vec<PortDoc>,
    #[serde(default)]
    exposed_params: Vec<ParamDoc>,
}

#[derive(Serialize, Deserialize)]
struct ParamDoc {
    module: u64,
    param: String,
}

#[derive(Serialize, Deserialize)]
struct ModuleDoc {
    id: u64,
    #[serde(rename = "type")]
    type_name: String,
    pos: (i32, i32),
    #[serde(default)]
    params: BTreeMap<String, f32>,
}

#[derive(Serialize, Deserialize)]
struct PortDoc {
    module: u64,
    port: String,
}

#[derive(Serialize, Deserialize)]
struct CableDoc {
    id: u64,
    from: PortDoc,
    to: PortDoc,
}

pub fn to_json(patch: &Patch) -> String {
    let modules = patch
        .modules
        .iter()
        .map(|(id, inst)| {
            let desc = inst.kind.desc();
            ModuleDoc {
                id: *id,
                type_name: desc.type_name.to_owned(),
                pos: inst.pos,
                params: desc
                    .params
                    .iter()
                    .zip(&inst.params)
                    .map(|(p, &v)| (p.name.to_owned(), v))
                    .collect(),
            }
        })
        .collect();
    let cables = patch
        .cables
        .iter()
        .filter_map(|c| {
            let from_desc = patch.modules.get(&c.from.module)?.kind.desc();
            let to_desc = patch.modules.get(&c.to.module)?.kind.desc();
            Some(CableDoc {
                id: c.id,
                from: PortDoc {
                    module: c.from.module,
                    port: from_desc.outputs.get(c.from.port)?.name.to_owned(),
                },
                to: PortDoc {
                    module: c.to.module,
                    port: to_desc.inputs.get(c.to.port)?.name.to_owned(),
                },
            })
        })
        .collect();
    let exposed_doc = |ports: &[PortRef], input: bool| -> Vec<PortDoc> {
        ports
            .iter()
            .filter_map(|p| {
                let desc = patch.modules.get(&p.module)?.kind.desc();
                let name = if input { desc.inputs.get(p.port) } else { desc.outputs.get(p.port) };
                Some(PortDoc { module: p.module, port: name?.name.to_owned() })
            })
            .collect()
    };
    let params_doc = |params: &[ParamRef]| -> Vec<ParamDoc> {
        params
            .iter()
            .filter_map(|p| {
                let desc = patch.modules.get(&p.module)?.kind.desc();
                Some(ParamDoc { module: p.module, param: desc.params.get(p.param)?.name.to_owned() })
            })
            .collect()
    };
    let groups = patch
        .groups
        .iter()
        .map(|(id, g)| GroupDoc {
            id: *id,
            name: g.name.clone(),
            members: g.members.clone(),
            collapsed: g.collapsed,
            pos: g.pos,
            exposed_in: exposed_doc(&g.exposed_in, true),
            exposed_out: exposed_doc(&g.exposed_out, false),
            exposed_params: params_doc(&g.exposed_params),
        })
        .collect();
    let doc =
        PatchDoc { version: PATCH_VERSION, next_id: patch.next_id(), modules, cables, groups };
    serde_json::to_string_pretty(&doc).expect("patch serialization is infallible")
}

/// Load a patch. Unknown module types, ports, and params are dropped with a
/// best-effort policy rather than failing the whole load.
pub fn from_json(json: &str) -> Result<Patch, String> {
    let doc: PatchDoc = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if doc.version > PATCH_VERSION {
        return Err(format!("patch version {} is newer than supported {PATCH_VERSION}", doc.version));
    }

    let mut patch = Patch::new();
    let mut max_id = 0u64;
    for m in &doc.modules {
        let Some(kind) = ModuleKindId::from_type_name(&m.type_name) else { continue };
        let desc = kind.desc();
        let params = desc
            .params
            .iter()
            .map(|p| m.params.get(p.name).copied().unwrap_or(p.default))
            .collect();
        patch
            .modules
            .insert(m.id, ModuleInst { kind, pos: m.pos, params });
        max_id = max_id.max(m.id);
    }
    for c in &doc.cables {
        let (Some(from_inst), Some(to_inst)) =
            (patch.modules.get(&c.from.module), patch.modules.get(&c.to.module))
        else {
            continue;
        };
        let from_port =
            from_inst.kind.desc().outputs.iter().position(|p| p.name == c.from.port);
        let to_port = to_inst.kind.desc().inputs.iter().position(|p| p.name == c.to.port);
        let (Some(from_port), Some(to_port)) = (from_port, to_port) else { continue };
        // One cable per input.
        let to = PortRef { module: c.to.module, port: to_port };
        patch.cables.retain(|existing| existing.to != to);
        patch.cables.push(Cable {
            id: c.id,
            from: PortRef { module: c.from.module, port: from_port },
            to,
        });
        max_id = max_id.max(c.id);
    }
    for g in &doc.groups {
        // Keep only members that still exist; drop a group that ends up empty.
        let members: Vec<u64> =
            g.members.iter().copied().filter(|m| patch.modules.contains_key(m)).collect();
        if members.is_empty() {
            continue;
        }
        let resolve = |specs: &[PortDoc], input: bool| -> Vec<PortRef> {
            specs
                .iter()
                .filter_map(|pd| {
                    let inst = patch.modules.get(&pd.module)?;
                    let pi = if input {
                        inst.kind.desc().inputs.iter().position(|p| p.name == pd.port)
                    } else {
                        inst.kind.desc().outputs.iter().position(|p| p.name == pd.port)
                    }?;
                    Some(PortRef { module: pd.module, port: pi })
                })
                .collect()
        };
        let exposed_in = resolve(&g.exposed_in, true);
        let exposed_out = resolve(&g.exposed_out, false);
        let exposed_params: Vec<ParamRef> = g
            .exposed_params
            .iter()
            .filter_map(|pd| {
                let inst = patch.modules.get(&pd.module)?;
                let pi = inst.kind.desc().params.iter().position(|p| p.name == pd.param)?;
                Some(ParamRef { module: pd.module, param: pi })
            })
            .collect();
        patch.groups.insert(
            g.id,
            Group {
                name: g.name.clone(),
                members,
                collapsed: g.collapsed,
                pos: g.pos,
                exposed_in,
                exposed_out,
                exposed_params,
            },
        );
        max_id = max_id.max(g.id);
    }
    patch.set_next_id(doc.next_id.max(max_id + 1));
    Ok(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_patch() -> Patch {
        let mut p = Patch::new();
        let notes = p.add_module(ModuleKindId::NoteIn, (0, 2));
        let vco = p.add_module(ModuleKindId::Vco, (0, 16));
        let out = p.add_module(ModuleKindId::Output, (1, 30));
        p.connect(PortRef { module: notes, port: 0 }, PortRef { module: vco, port: 0 });
        p.connect(PortRef { module: vco, port: 0 }, PortRef { module: out, port: 0 });
        p.set_param(vco, 0, 1.25);
        p.set_param(notes, 0, 8.0);
        p
    }

    #[test]
    fn roundtrip_preserves_everything() {
        let patch = voice_patch();
        let json = to_json(&patch);
        let loaded = from_json(&json).expect("loads");

        assert_eq!(loaded.modules, patch.modules);
        assert_eq!(loaded.cables, patch.cables);
        assert_eq!(loaded.next_id(), patch.next_id());

        // IDs allocated after a load don't collide with existing ones.
        let mut loaded = loaded;
        let new = loaded.add_module(ModuleKindId::Lfo, (0, 0));
        assert!(!patch.modules.contains_key(&new));
    }

    #[test]
    fn unknown_params_and_modules_are_tolerated() {
        let json = r#"{
            "version": 1,
            "next_id": 50,
            "modules": [
                {"id": 1, "type": "vco", "pos": [0, 0], "params": {"pitch": 0.5, "flux": 9.9}},
                {"id": 2, "type": "hypothetical_reverb", "pos": [0, 9], "params": {}},
                {"id": 3, "type": "output", "pos": [0, 20]}
            ],
            "cables": [
                {"id": 10, "from": {"module": 1, "port": "out"}, "to": {"module": 3, "port": "left"}},
                {"id": 11, "from": {"module": 2, "port": "out"}, "to": {"module": 3, "port": "right"}}
            ]
        }"#;
        let patch = from_json(json).expect("loads");
        assert_eq!(patch.modules.len(), 2); // unknown module dropped
        assert_eq!(patch.cables.len(), 1); // its cable too
        let vco = &patch.modules[&1];
        assert_eq!(vco.params[0], 0.5); // known param applied
        // Missing "wave"/"pw" got defaults.
        assert_eq!(vco.params[1], ModuleKindId::Vco.desc().params[1].default);
        assert_eq!(patch.next_id(), 50);
    }

    #[test]
    fn groups_survive_roundtrip() {
        let mut p = voice_patch();
        let ids: Vec<_> = p.modules.keys().copied().collect();
        let g = p.create_group(&ids[..2], "my group".into(), (2, 5)).unwrap();
        let json = to_json(&p);
        let loaded = from_json(&json).expect("loads");
        assert_eq!(loaded.groups.len(), 1);
        let lg = &loaded.groups[&g];
        assert_eq!(lg.name, "my group");
        assert_eq!(lg.pos, (2, 5));
        assert!(lg.collapsed);
        assert_eq!(lg.members.len(), 2);
    }

    #[test]
    fn exposed_params_survive_roundtrip() {
        let mut p = voice_patch();
        let ids: Vec<_> = p.modules.keys().copied().collect();
        let g = p.create_group(&ids[..2], "g".into(), (0, 0)).unwrap();
        // Expose the VCO's pitch knob (param 0) on the group.
        let vco = ids[1]; // voice_patch: notes, vco, out — index 1 is the VCO
        assert!(p.toggle_exposed_param(crate::ParamRef { module: vco, param: 0 }));
        let json = to_json(&p);
        let loaded = from_json(&json).expect("loads");
        let lg = &loaded.groups[&g];
        assert_eq!(lg.exposed_params, vec![crate::ParamRef { module: vco, param: 0 }]);
    }

    #[test]
    fn newer_version_is_rejected() {
        let json = r#"{"version": 99, "next_id": 1, "modules": [], "cables": []}"#;
        assert!(from_json(json).is_err());
    }
}
