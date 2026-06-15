//! The audio engine: module implementations and the block executor.
//!
//! Realtime discipline: nothing in the process path allocates, locks, or
//! panics. All state is pre-allocated at construction.

pub mod buffer;
pub mod executor;
pub mod modules;

pub use buffer::{PortBuffer, BLOCK, MAX_CHANNELS};
pub use executor::Executor;

use rack_core::messages::{decode_batch, MsgTag};
use rack_core::modules::ModuleKindId;
use rack_core::plan::{encode_plan, ModuleInit, PlanStep, N_RESERVED_BUFFERS, PLAN_TAG};

/// Per-block processing context, fixed at engine construction.
#[derive(Clone, Copy, Debug)]
pub struct ProcessCtx {
    pub sample_rate: f32,
    pub inv_sample_rate: f32,
    /// One-pole coefficient for ~10 ms parameter smoothing.
    pub smooth_k: f32,
}

impl ProcessCtx {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            inv_sample_rate: 1.0 / sample_rate,
            smooth_k: rack_dsp::Smoothed::coeff(0.010, sample_rate),
        }
    }
}

/// The worklet-facing engine: an executor plus message decoding.
pub struct Engine {
    ctx: ProcessCtx,
    executor: Executor,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        let mut executor = Executor::new();
        // Boot with a minimal audible patch (VCO → Output) until the UI
        // sends a real plan; this keeps "power on, hear a saw" working with
        // zero round trips and exercises the executor in every smoke test.
        executor.apply_plan(&default_plan());
        Self { ctx: ProcessCtx::new(sample_rate), executor }
    }

    /// Render one Web Audio quantum. Quantum length is read from the slices
    /// (do not assume 128 — variable render quanta are coming to the spec).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let mut offset = 0;
        while offset < frames {
            let n = (frames - offset).min(BLOCK);
            self.executor.process_block(&self.ctx, n);
            left[offset..offset + n].copy_from_slice(&self.executor.master.l[..n]);
            right[offset..offset + n].copy_from_slice(&self.executor.master.r[..n]);
            offset += n;
        }
    }

    /// Apply one message payload from the UI: either a plan blob or a batch
    /// of 32-byte control records, distinguished by the leading u32 tag.
    /// Runs on the audio thread between quanta — must not allocate.
    pub fn on_message(&mut self, bytes: &[u8]) {
        let tag = bytes
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0);
        if tag == PLAN_TAG {
            self.executor.apply_plan(bytes);
            return;
        }
        for msg in decode_batch(bytes) {
            match MsgTag::from_u32(msg.tag) {
                Some(MsgTag::SetParam) => self.executor.set_param(msg.a, msg.b, msg.value),
                Some(MsgTag::NoteOn) => self.executor.note_on(msg.a as u8, msg.b as u8),
                Some(MsgTag::NoteOff) => self.executor.note_off(msg.a as u8),
                Some(MsgTag::AllNotesOff) => self.executor.all_notes_off(),
                None => {}
            }
        }
    }

    pub fn set_param(&mut self, slot: u32, param: u32, value: f32) {
        self.executor.set_param(slot, param, value);
    }

    /// Meter snapshot for the UI; resets peak accumulators. Call at UI rate.
    pub fn take_meters(&mut self) -> Vec<u8> {
        self.executor.take_meters()
    }
}

/// The boot patch: VCO in slot 0 feeding Output in slot 1.
fn default_plan() -> Vec<u8> {
    let buf = N_RESERVED_BUFFERS;
    let modules = [
        ModuleInit { slot: 0, kind: ModuleKindId::Vco as u16 },
        ModuleInit { slot: 1, kind: ModuleKindId::Output as u16 },
    ];
    let mut vco = PlanStep { slot: 0, kind: ModuleKindId::Vco as u16, ..Default::default() };
    vco.outputs[0] = buf;
    let mut out = PlanStep { slot: 1, kind: ModuleKindId::Output as u16, ..Default::default() };
    out.inputs[0] = buf;
    encode_plan(0, &modules, &[vco, out])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rack_core::messages::Msg;
    use rack_core::modules::params;

    #[test]
    fn renders_audible_bounded_audio() {
        let mut engine = Engine::new(48_000.0);
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        let mut peak = 0.0f32;
        let mut crossings = 0u32;
        let mut last = 0.0f32;
        for _ in 0..200 {
            engine.process(&mut l, &mut r);
            for &s in l.iter() {
                assert!(s.is_finite());
                assert!(s.abs() <= 1.0, "sample out of range: {s}");
                peak = peak.max(s.abs());
                if last < 0.0 && s >= 0.0 {
                    crossings += 1;
                }
                last = s;
            }
        }
        assert!(peak > 0.1, "output is near-silent: peak {peak}");
        // ≈0.533 s of 440 Hz → ~235 rising zero crossings.
        assert!((200..280).contains(&crossings), "crossings {crossings}");
        assert_eq!(l, r);
    }

    #[test]
    fn set_param_message_changes_pitch() {
        let mut engine = Engine::new(48_000.0);
        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];

        let count = |engine: &mut Engine, l: &mut [f32; 128], r: &mut [f32; 128]| {
            let mut crossings = 0u32;
            let mut last = 0.0f32;
            for _ in 0..100 {
                engine.process(l, r);
                for &s in l.iter() {
                    if last < 0.0 && s >= 0.0 {
                        crossings += 1;
                    }
                    last = s;
                }
            }
            crossings
        };

        let baseline = count(&mut engine, &mut l, &mut r);
        // One octave up, via the wire format.
        let batch = rack_core::messages::encode_batch(&[Msg::set_param(
            0,
            params::vco::PITCH,
            1.75,
        )]);
        engine.on_message(&batch);
        for _ in 0..50 {
            engine.process(&mut l, &mut r);
        }
        let doubled = count(&mut engine, &mut l, &mut r);
        let ratio = doubled as f32 / baseline as f32;
        assert!((ratio - 2.0).abs() < 0.1, "octave ratio {ratio}");
    }

    #[test]
    fn plan_message_replaces_patch() {
        let mut engine = Engine::new(48_000.0);
        // Empty plan → silence.
        engine.on_message(&encode_plan(9, &[], &[]));
        let mut l = [0.1f32; 128];
        let mut r = [0.1f32; 128];
        engine.process(&mut l, &mut r);
        assert!(l.iter().all(|&s| s == 0.0), "expected silence after empty plan");
    }
}
