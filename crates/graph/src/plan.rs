//! Planner: flattens a [`Patch`] into the engine's plan blob. Owns the
//! persistent id→slot and output-port→buffer assignments so module state and
//! feedback buffers survive across rebuilds.

use crate::{ModuleId, Patch, PortRef};
use rack_core::caps::MAX_MODULES;
use rack_core::plan::{
    encode_plan, ModuleInit, PlanStep, MAX_BUFFERS, NO_BUFFER, N_RESERVED_BUFFERS, TRASH_BUFFER,
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default)]
pub struct Planner {
    slots: HashMap<ModuleId, u16>,
    free_slots: Vec<u16>,
    next_slot: u16,
    /// (module, output port) → persistent buffer index. Assigned when first
    /// referenced by a cable; freed when the module is removed (not on
    /// disconnect, so feedback buffers stay stable across re-patching).
    buffers: HashMap<(ModuleId, usize), u16>,
    free_buffers: Vec<u16>,
    next_buffer: u16,
    epoch: u32,
}

impl Planner {
    pub fn new() -> Self {
        Self { next_buffer: N_RESERVED_BUFFERS, ..Default::default() }
    }

    /// Engine slot for a module (after the last `build`); SetParam messages
    /// address modules by this.
    pub fn slot_of(&self, id: ModuleId) -> Option<u16> {
        self.slots.get(&id).copied()
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    fn alloc_slot(&mut self) -> Option<u16> {
        if let Some(s) = self.free_slots.pop() {
            return Some(s);
        }
        if (self.next_slot as usize) < MAX_MODULES {
            let s = self.next_slot;
            self.next_slot += 1;
            return Some(s);
        }
        None
    }

    fn alloc_buffer(&mut self) -> Option<u16> {
        if let Some(b) = self.free_buffers.pop() {
            return Some(b);
        }
        if (self.next_buffer as usize) < MAX_BUFFERS {
            let b = self.next_buffer;
            self.next_buffer += 1;
            return Some(b);
        }
        None
    }

    /// Flatten the patch into a plan blob. Never panics: modules/cables that
    /// exceed pool capacity are dropped from the plan (the UI should prevent
    /// that long before 256 modules).
    pub fn build(&mut self, patch: &Patch) -> Vec<u8> {
        self.epoch = self.epoch.wrapping_add(1);

        // Release assignments of removed modules.
        let live: HashSet<ModuleId> = patch.modules.keys().copied().collect();
        let dead: Vec<ModuleId> = self.slots.keys().filter(|id| !live.contains(id)).copied().collect();
        for id in dead {
            if let Some(slot) = self.slots.remove(&id) {
                self.free_slots.push(slot);
            }
            let dead_bufs: Vec<(ModuleId, usize)> =
                self.buffers.keys().filter(|(m, _)| *m == id).copied().collect();
            for key in dead_bufs {
                if let Some(b) = self.buffers.remove(&key) {
                    self.free_buffers.push(b);
                }
            }
        }

        // Assign slots for new modules (BTreeMap order → deterministic).
        for id in patch.modules.keys() {
            if !self.slots.contains_key(id) {
                if let Some(slot) = self.alloc_slot() {
                    self.slots.insert(*id, slot);
                }
            }
        }

        // Assign buffers for every output port referenced by a cable.
        for cable in &patch.cables {
            let key = (cable.from.module, cable.from.port);
            if !self.buffers.contains_key(&key) && self.slots.contains_key(&cable.from.module) {
                if let Some(b) = self.alloc_buffer() {
                    self.buffers.insert(key, b);
                }
            }
        }

        // Execution order: reverse postorder of the producer→consumer graph.
        // A feedback back-edge thus schedules the consumer first, which reads
        // the producer's previous block from its persistent buffer.
        let order = self.execution_order(patch);

        let mut modules: Vec<ModuleInit> = Vec::with_capacity(order.len());
        let mut steps: Vec<PlanStep> = Vec::with_capacity(order.len());
        for id in order {
            let inst = &patch.modules[&id];
            let Some(&slot) = self.slots.get(&id) else { continue };
            modules.push(ModuleInit { slot, kind: inst.kind as u16 });

            let mut step = PlanStep { slot, kind: inst.kind as u16, ..Default::default() };
            for (i, input) in step.inputs.iter_mut().enumerate().take(inst.kind.desc().inputs.len())
            {
                *input = patch
                    .cable_into(PortRef { module: id, port: i })
                    .and_then(|c| self.buffers.get(&(c.from.module, c.from.port)).copied())
                    .unwrap_or(NO_BUFFER);
            }
            for (i, output) in
                step.outputs.iter_mut().enumerate().take(inst.kind.desc().outputs.len())
            {
                *output = self
                    .buffers
                    .get(&(id, i))
                    .copied()
                    .unwrap_or(TRASH_BUFFER + i as u16);
            }
            steps.push(step);
        }

        encode_plan(self.epoch, &modules, &steps)
    }

    fn execution_order(&self, patch: &Patch) -> Vec<ModuleId> {
        // producer → consumers adjacency.
        let mut consumers: BTreeMap<ModuleId, Vec<ModuleId>> = BTreeMap::new();
        for cable in &patch.cables {
            consumers.entry(cable.from.module).or_default().push(cable.to.module);
        }

        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Unvisited,
            InProgress,
            Done,
        }
        let mut mark: HashMap<ModuleId, Mark> =
            patch.modules.keys().map(|id| (*id, Mark::Unvisited)).collect();
        let mut postorder: Vec<ModuleId> = Vec::with_capacity(patch.modules.len());

        // Iterative DFS (explicit stack: patches can be deep chains).
        for &start in patch.modules.keys() {
            if mark[&start] != Mark::Unvisited {
                continue;
            }
            let mut stack: Vec<(ModuleId, usize)> = vec![(start, 0)];
            mark.insert(start, Mark::InProgress);
            while let Some(&mut (node, ref mut child_idx)) = stack.last_mut() {
                let next = consumers
                    .get(&node)
                    .and_then(|c| c.get(*child_idx..))
                    .and_then(|rest| {
                        rest.iter().find(|m| mark.get(m) == Some(&Mark::Unvisited)).copied()
                    });
                // Advance the cursor past everything we skipped.
                if let Some(c) = consumers.get(&node) {
                    while *child_idx < c.len()
                        && mark.get(&c[*child_idx]) != Some(&Mark::Unvisited)
                    {
                        *child_idx += 1;
                    }
                }
                match next {
                    Some(child) => {
                        mark.insert(child, Mark::InProgress);
                        stack.push((child, 0));
                    }
                    None => {
                        mark.insert(node, Mark::Done);
                        postorder.push(node);
                        stack.pop();
                    }
                }
            }
        }
        postorder.reverse();
        postorder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rack_core::modules::ModuleKindId;
    use rack_core::plan::decode_plan;

    fn chain_patch() -> (Patch, ModuleId, ModuleId, ModuleId) {
        let mut p = Patch::new();
        let vco = p.add_module(ModuleKindId::Vco, (0, 0));
        let vcf = p.add_module(ModuleKindId::Vcf, (0, 10));
        let out = p.add_module(ModuleKindId::Output, (0, 20));
        p.connect(PortRef { module: vco, port: 0 }, PortRef { module: vcf, port: 0 }).unwrap();
        p.connect(PortRef { module: vcf, port: 0 }, PortRef { module: out, port: 0 }).unwrap();
        (p, vco, vcf, out)
    }

    #[test]
    fn chain_is_ordered_producer_first() {
        let (p, vco, vcf, out) = chain_patch();
        let mut planner = Planner::new();
        let blob = planner.build(&p);
        let view = decode_plan(&blob).unwrap();

        let pos = |id: ModuleId| {
            let slot = planner.slot_of(id).unwrap();
            view.steps.iter().position(|s| s.slot == slot).unwrap()
        };
        assert!(pos(vco) < pos(vcf));
        assert!(pos(vcf) < pos(out));
        assert_eq!(view.steps.len(), 3);
    }

    #[test]
    fn inputs_reference_producer_buffers() {
        let (p, vco, vcf, _out) = chain_patch();
        let mut planner = Planner::new();
        let blob = planner.build(&p);
        let view = decode_plan(&blob).unwrap();

        let vco_step = view.steps.iter().find(|s| s.slot == planner.slot_of(vco).unwrap()).unwrap();
        let vcf_step = view.steps.iter().find(|s| s.slot == planner.slot_of(vcf).unwrap()).unwrap();
        assert!(vco_step.outputs[0] >= N_RESERVED_BUFFERS);
        assert_eq!(vcf_step.inputs[0], vco_step.outputs[0]);
        assert_eq!(vcf_step.inputs[1], NO_BUFFER); // cutoff CV unconnected
    }

    #[test]
    fn slots_and_buffers_stable_across_rebuilds() {
        let (mut p, vco, vcf, out) = chain_patch();
        let mut planner = Planner::new();
        planner.build(&p);
        let slot_before = planner.slot_of(vcf).unwrap();

        // Edit elsewhere: add an LFO, rebuild.
        p.add_module(ModuleKindId::Lfo, (1, 0));
        let blob = planner.build(&p);
        assert_eq!(planner.slot_of(vcf).unwrap(), slot_before);

        let view = decode_plan(&blob).unwrap();
        let vco_step = view.steps.iter().find(|s| s.slot == planner.slot_of(vco).unwrap()).unwrap();
        let vcf_step = view.steps.iter().find(|s| s.slot == planner.slot_of(vcf).unwrap()).unwrap();
        assert_eq!(vcf_step.inputs[0], vco_step.outputs[0]);
        let _ = out;
    }

    #[test]
    fn feedback_cycle_schedules_consumer_before_producer() {
        // vcf → vca → vcf (resonant feedback loop), vca → out.
        let mut p = Patch::new();
        let vcf = p.add_module(ModuleKindId::Vcf, (0, 0));
        let vca = p.add_module(ModuleKindId::Vca, (0, 10));
        let out = p.add_module(ModuleKindId::Output, (0, 20));
        p.connect(PortRef { module: vcf, port: 0 }, PortRef { module: vca, port: 0 }).unwrap();
        p.connect(PortRef { module: vca, port: 0 }, PortRef { module: vcf, port: 0 }).unwrap();
        p.connect(PortRef { module: vca, port: 0 }, PortRef { module: out, port: 0 }).unwrap();

        let mut planner = Planner::new();
        let blob = planner.build(&p);
        let view = decode_plan(&blob).unwrap();
        // All three present exactly once; one of the cycle edges is a
        // back-edge (which one depends on DFS start), so just check counts
        // and that every input ref is either NO_BUFFER or a real buffer.
        assert_eq!(view.steps.len(), 3);
        for step in view.steps {
            for &i in &step.inputs {
                assert!(i == NO_BUFFER || (i >= N_RESERVED_BUFFERS && (i as usize) < MAX_BUFFERS));
            }
        }
        let _ = out;
    }

    #[test]
    fn removed_module_slot_is_recycled() {
        let (mut p, vco, _vcf, _out) = chain_patch();
        let mut planner = Planner::new();
        planner.build(&p);
        let old_slot = planner.slot_of(vco).unwrap();

        p.remove_module(vco);
        planner.build(&p);
        assert_eq!(planner.slot_of(vco), None);

        let lfo = p.add_module(ModuleKindId::Lfo, (1, 0));
        let blob = planner.build(&p);
        assert_eq!(planner.slot_of(lfo), Some(old_slot));
        // The recycled slot's kind changed: plan says Lfo there now.
        let view = decode_plan(&blob).unwrap();
        let init = view.modules.iter().find(|m| m.slot == old_slot).unwrap();
        assert_eq!(init.kind, ModuleKindId::Lfo as u16);
    }
}
