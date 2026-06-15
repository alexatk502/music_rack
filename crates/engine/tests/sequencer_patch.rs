//! End-to-end: a self-playing generative patch — CLOCK → SEQ-8 → VCO, clock
//! gating the VCA, through the DELAY to the output — built entirely through
//! the wire formats.

use rack_core::messages::{encode_batch, Msg};
use rack_core::modules::{params, ModuleKindId};
use rack_core::plan::{encode_plan, ModuleInit, PlanStep};
use rack_engine::Engine;

const CLOCK: u16 = 0;
const SEQ: u16 = 1;
const VCO: u16 = 2;
const VCA: u16 = 3;
const DELAY: u16 = 4;
const OUT: u16 = 5;

fn patch_plan() -> Vec<u8> {
    let k = |k: ModuleKindId| k as u16;
    let modules = [
        ModuleInit { slot: CLOCK, kind: k(ModuleKindId::Clock) },
        ModuleInit { slot: SEQ, kind: k(ModuleKindId::Seq8) },
        ModuleInit { slot: VCO, kind: k(ModuleKindId::Vco) },
        ModuleInit { slot: VCA, kind: k(ModuleKindId::Vca) },
        ModuleInit { slot: DELAY, kind: k(ModuleKindId::Delay) },
        ModuleInit { slot: OUT, kind: k(ModuleKindId::Output) },
    ];
    // Buffers: 8 clock, 9 /2, 10 /4, 11 seq voct, 12 seq gate, 13 vco,
    // 14 vca, 15 delay.
    let mut steps = Vec::new();
    let mut clock = PlanStep { slot: CLOCK, kind: k(ModuleKindId::Clock), ..Default::default() };
    clock.outputs[..3].copy_from_slice(&[8, 9, 10]);
    steps.push(clock);
    let mut seq = PlanStep { slot: SEQ, kind: k(ModuleKindId::Seq8), ..Default::default() };
    seq.inputs[0] = 8;
    seq.outputs[..2].copy_from_slice(&[11, 12]);
    steps.push(seq);
    let mut vco = PlanStep { slot: VCO, kind: k(ModuleKindId::Vco), ..Default::default() };
    vco.inputs[0] = 11;
    vco.outputs[0] = 13;
    steps.push(vco);
    let mut vca = PlanStep { slot: VCA, kind: k(ModuleKindId::Vca), ..Default::default() };
    vca.inputs[0] = 13;
    vca.inputs[1] = 12; // gated by the sequencer's gate
    vca.outputs[0] = 14;
    steps.push(vca);
    let mut delay = PlanStep { slot: DELAY, kind: k(ModuleKindId::Delay), ..Default::default() };
    delay.inputs[0] = 14;
    delay.outputs[0] = 15;
    steps.push(delay);
    let mut out = PlanStep { slot: OUT, kind: k(ModuleKindId::Output), ..Default::default() };
    out.inputs[0] = 15;
    steps.push(out);
    encode_plan(1, &modules, &steps)
}

#[test]
fn generative_patch_pulses_and_changes_pitch() {
    let mut engine = Engine::new(48_000.0);
    engine.on_message(&patch_plan());
    engine.on_message(&encode_batch(&[
        Msg::set_param(CLOCK as u32, params::clock::BPM, 240.0), // 4 Hz
        Msg::set_param(CLOCK as u32, params::clock::WIDTH, 0.5),
        Msg::set_param(SEQ as u32, params::seq8::STEPS, 2.0),
        Msg::set_param(SEQ as u32, params::seq8::PITCH_BASE, 0.0), // C4
        Msg::set_param(SEQ as u32, params::seq8::PITCH_BASE + 1, 1.0), // C5
        Msg::set_param(SEQ as u32, params::seq8::PITCH_BASE + 8, 0.0), // out of range: ignored
        Msg::set_param(DELAY as u32, params::delay::MIX, 0.0), // dry for analysis
    ]));

    let mut l = [0.0f32; 128];
    let mut r = [0.0f32; 128];
    // Settle one clock period so the sequencer has started.
    for _ in 0..200 {
        engine.process(&mut l, &mut r);
    }

    // Measure RMS per 10 ms window over 2 s: the 4 Hz half-width gate must
    // produce loud and silent windows.
    let mut window_rms = Vec::new();
    let mut sum = 0.0f64;
    let mut n = 0u32;
    let mut crossings_per_window = Vec::new();
    let mut crossings = 0u32;
    let mut last = 0.0f32;
    for q in 0..750 {
        engine.process(&mut l, &mut r);
        for &s in l.iter() {
            assert!(s.is_finite());
            sum += (s as f64) * (s as f64);
            if last < 0.0 && s >= 0.0 {
                crossings += 1;
            }
            last = s;
            n += 1;
        }
        if q % 4 == 3 {
            window_rms.push((sum / n as f64).sqrt() as f32);
            crossings_per_window.push(crossings);
            sum = 0.0;
            n = 0;
            crossings = 0;
        }
    }
    let loud = window_rms.iter().filter(|&&w| w > 0.05).count();
    let quiet = window_rms.iter().filter(|&&w| w < 0.01).count();
    assert!(loud > 30, "no audible pulses: {loud} loud windows");
    assert!(quiet > 30, "gate never closes: {quiet} quiet windows");

    // Pitch alternates between C4 (~262 Hz) and C5 (~523 Hz): loud windows
    // should show two distinct zero-crossing rates.
    let loud_crossings: Vec<u32> = window_rms
        .iter()
        .zip(&crossings_per_window)
        .filter(|(w, _)| **w > 0.05)
        .map(|(_, c)| *c)
        .collect();
    let lows = loud_crossings.iter().filter(|&&c| c < 4).count();
    let highs = loud_crossings.iter().filter(|&&c| c >= 4).count();
    assert!(
        lows > 5 && highs > 5,
        "sequencer pitch not alternating: lows {lows}, highs {highs} ({loud_crossings:?})"
    );
}
