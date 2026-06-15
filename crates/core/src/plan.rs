//! Execution plan wire format. The UI thread flattens the patch graph into a
//! plan blob; the audio thread memcpys it into a pre-allocated arena and
//! swaps. Layout (little-endian, all POD):
//!
//! ```text
//! PlanHeader | ModuleInit × n_modules | PlanStep × n_steps
//! ```
//!
//! Steps are in execution order (reverse postorder of the producer→consumer
//! DAG, so feedback back-edges naturally read the previous block).

use bytemuck::{Pod, Zeroable};

/// First u32 of a plan blob; distinguishes it from a control-message batch
/// (whose first u32 is a MsgTag < 100).
pub const PLAN_TAG: u32 = 100;

/// Max input/output ports per module (validated against descriptors by test).
pub const MAX_PORTS_IN: usize = 8;
pub const MAX_PORTS_OUT: usize = 8;

/// Port-buffer pool size. Indices 0..N_RESERVED are reserved:
/// trash buffers for unconnected output ports (one per port position so
/// `get_disjoint_mut` indices stay distinct).
pub const MAX_BUFFERS: usize = 1024;
pub const TRASH_BUFFER: u16 = 0;
pub const N_RESERVED_BUFFERS: u16 = MAX_PORTS_OUT as u16;
/// "Not connected" marker for input port refs.
pub const NO_BUFFER: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct PlanHeader {
    pub tag: u32,
    pub epoch: u32,
    pub n_modules: u32,
    pub n_steps: u32,
}

/// One live module: the engine constructs/keeps slot contents to match.
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct ModuleInit {
    pub slot: u16,
    pub kind: u16,
}

/// One execution step. `inputs[i]` is a buffer index or NO_BUFFER;
/// `outputs[i]` is a buffer index (TRASH_BUFFER + i for unconnected ports).
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct PlanStep {
    pub slot: u16,
    pub kind: u16,
    pub inputs: [u16; MAX_PORTS_IN],
    pub outputs: [u16; MAX_PORTS_OUT],
}

impl Default for PlanStep {
    fn default() -> Self {
        let mut outputs = [0u16; MAX_PORTS_OUT];
        for (i, o) in outputs.iter_mut().enumerate() {
            *o = TRASH_BUFFER + i as u16;
        }
        Self { slot: 0, kind: 0, inputs: [NO_BUFFER; MAX_PORTS_IN], outputs }
    }
}

/// Serialize a plan to one blob for postMessage.
pub fn encode_plan(epoch: u32, modules: &[ModuleInit], steps: &[PlanStep]) -> Vec<u8> {
    let header = PlanHeader {
        tag: PLAN_TAG,
        epoch,
        n_modules: modules.len() as u32,
        n_steps: steps.len() as u32,
    };
    let mut out = Vec::with_capacity(
        core::mem::size_of::<PlanHeader>()
            + core::mem::size_of_val(modules)
            + core::mem::size_of_val(steps),
    );
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(bytemuck::cast_slice(modules));
    out.extend_from_slice(bytemuck::cast_slice(steps));
    out
}

/// Borrowed view of a decoded plan blob. Returns None on any inconsistency
/// (runs on the audio thread: reject, never panic).
pub struct PlanView<'a> {
    pub header: PlanHeader,
    pub modules: &'a [ModuleInit],
    pub steps: &'a [PlanStep],
}

pub fn decode_plan(bytes: &[u8]) -> Option<PlanView<'_>> {
    let header_len = core::mem::size_of::<PlanHeader>();
    let header: PlanHeader = *bytemuck::try_from_bytes(bytes.get(..header_len)?).ok()?;
    if header.tag != PLAN_TAG {
        return None;
    }
    let modules_len = header.n_modules as usize * core::mem::size_of::<ModuleInit>();
    let steps_len = header.n_steps as usize * core::mem::size_of::<PlanStep>();
    let modules_bytes = bytes.get(header_len..header_len + modules_len)?;
    let steps_bytes = bytes.get(header_len + modules_len..header_len + modules_len + steps_len)?;
    let modules = bytemuck::try_cast_slice(modules_bytes).ok()?;
    let steps = bytemuck::try_cast_slice(steps_bytes).ok()?;
    Some(PlanView { header, modules, steps })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let modules = [ModuleInit { slot: 0, kind: 0 }, ModuleInit { slot: 3, kind: 6 }];
        let mut step = PlanStep::default();
        step.slot = 3;
        step.kind = 6;
        step.inputs[0] = 42;
        let blob = encode_plan(7, &modules, &[step]);

        let view = decode_plan(&blob).expect("decodes");
        assert_eq!(view.header.epoch, 7);
        assert_eq!(view.modules.len(), 2);
        assert_eq!(view.modules[1].slot, 3);
        assert_eq!(view.steps.len(), 1);
        assert_eq!(view.steps[0].inputs[0], 42);
        assert_eq!(view.steps[0].inputs[1], NO_BUFFER);
        assert_eq!(view.steps[0].outputs[1], TRASH_BUFFER + 1);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_plan(&[]).is_none());
        assert!(decode_plan(&[1, 2, 3]).is_none());
        // Wrong tag.
        let blob = encode_plan(0, &[], &[]);
        let mut bad = blob.clone();
        bad[0] = 5;
        assert!(decode_plan(&bad).is_none());
        // Truncated payload.
        let blob = encode_plan(0, &[ModuleInit { slot: 0, kind: 0 }], &[]);
        assert!(decode_plan(&blob[..blob.len() - 1]).is_none());
    }
}
