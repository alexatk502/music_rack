//! End-to-end: a full polyphonic subtractive voice built and played entirely
//! through the wire formats (plan blob + message batches), exactly as the
//! browser app drives the engine.

use rack_core::messages::{encode_batch, Msg};
use rack_core::modules::{params, ModuleKindId};
use rack_core::plan::{encode_plan, ModuleInit, PlanStep};
use rack_engine::Engine;

const NOTE_IN: u16 = 0;
const VCO: u16 = 1;
const VCF: u16 = 2;
const ADSR: u16 = 3;
const VCA: u16 = 4;
const OUT: u16 = 5;

/// NoteIn(voct,gate,_,retrig) → VCO → VCF → VCA(cv=ADSR) → Output.
fn voice_plan() -> Vec<u8> {
    let k = |k: ModuleKindId| k as u16;
    let modules = [
        ModuleInit { slot: NOTE_IN, kind: k(ModuleKindId::NoteIn) },
        ModuleInit { slot: VCO, kind: k(ModuleKindId::Vco) },
        ModuleInit { slot: VCF, kind: k(ModuleKindId::Vcf) },
        ModuleInit { slot: ADSR, kind: k(ModuleKindId::Adsr) },
        ModuleInit { slot: VCA, kind: k(ModuleKindId::Vca) },
        ModuleInit { slot: OUT, kind: k(ModuleKindId::Output) },
    ];
    // Buffers: 8 voct, 9 gate, 10 vel, 11 retrig, 12 vco, 13 vcf, 14 env, 15 vca.
    let mut steps = Vec::new();
    let mut note = PlanStep { slot: NOTE_IN, kind: k(ModuleKindId::NoteIn), ..Default::default() };
    note.outputs[..4].copy_from_slice(&[8, 9, 10, 11]);
    steps.push(note);
    let mut vco = PlanStep { slot: VCO, kind: k(ModuleKindId::Vco), ..Default::default() };
    vco.inputs[0] = 8;
    vco.outputs[0] = 12;
    steps.push(vco);
    let mut vcf = PlanStep { slot: VCF, kind: k(ModuleKindId::Vcf), ..Default::default() };
    vcf.inputs[0] = 12;
    vcf.outputs[0] = 13;
    steps.push(vcf);
    let mut adsr = PlanStep { slot: ADSR, kind: k(ModuleKindId::Adsr), ..Default::default() };
    adsr.inputs[0] = 9;
    adsr.inputs[1] = 11;
    adsr.outputs[0] = 14;
    steps.push(adsr);
    let mut vca = PlanStep { slot: VCA, kind: k(ModuleKindId::Vca), ..Default::default() };
    vca.inputs[0] = 13;
    vca.inputs[1] = 14;
    vca.outputs[0] = 15;
    steps.push(vca);
    let mut out = PlanStep { slot: OUT, kind: k(ModuleKindId::Output), ..Default::default() };
    out.inputs[0] = 15;
    steps.push(out);
    encode_plan(1, &modules, &steps)
}

fn rms(engine: &mut Engine, quanta: usize) -> f32 {
    let mut l = [0.0f32; 128];
    let mut r = [0.0f32; 128];
    let mut sum = 0.0f64;
    let mut n = 0u32;
    for _ in 0..quanta {
        engine.process(&mut l, &mut r);
        for &s in l.iter() {
            assert!(s.is_finite());
            sum += (s as f64) * (s as f64);
            n += 1;
        }
    }
    ((sum / n as f64).sqrt()) as f32
}

#[test]
fn poly_voice_plays_and_releases() {
    let mut engine = Engine::new(48_000.0);
    engine.on_message(&voice_plan());
    engine.on_message(&encode_batch(&[
        Msg::set_param(NOTE_IN as u32, params::note_in::POLYPHONY, 8.0),
        Msg::set_param(ADSR as u32, params::adsr::RELEASE, 0.05),
        Msg::set_param(ADSR as u32, params::adsr::ATTACK, 0.002),
    ]));

    // Silent before any note (VCA closed by zero envelope).
    let silent = rms(&mut engine, 50);
    assert!(silent < 1e-4, "voice leaking while idle: rms {silent}");

    // C major chord.
    engine.on_message(&encode_batch(&[
        Msg::note_on(60, 100, 0),
        Msg::note_on(64, 100, 0),
        Msg::note_on(67, 100, 0),
    ]));
    let playing = rms(&mut engine, 100);
    assert!(playing > 0.02, "chord inaudible: rms {playing}");

    // Release all. The envelope's idle snap needs ~9 release time constants
    // (ln(1e4) ≈ 9.2), so give the 50 ms release a generous 0.8 s tail.
    engine.on_message(&encode_batch(&[Msg::all_notes_off()]));
    let _tail = rms(&mut engine, 300);
    let after = rms(&mut engine, 50);
    assert!(after < 1e-4, "voice still sounding after release: rms {after}");
}

#[test]
fn more_voices_more_signal() {
    let mut engine = Engine::new(48_000.0);
    engine.on_message(&voice_plan());
    engine.on_message(&encode_batch(&[
        Msg::set_param(NOTE_IN as u32, params::note_in::POLYPHONY, 8.0),
        Msg::set_param(ADSR as u32, params::adsr::ATTACK, 0.002),
        Msg::set_param(ADSR as u32, params::adsr::SUSTAIN, 1.0),
        // Keep the master quiet enough that the output soft-clipper stays
        // linear, otherwise voice summing is compressed away.
        Msg::set_param(OUT as u32, params::output::LEVEL, 0.15),
    ]));

    engine.on_message(&encode_batch(&[Msg::note_on(60, 100, 0)]));
    let one = rms(&mut engine, 150);
    engine.on_message(&encode_batch(&[
        Msg::note_on(64, 100, 0),
        Msg::note_on(67, 100, 0),
        Msg::note_on(72, 100, 0),
    ]));
    let four = rms(&mut engine, 150);
    assert!(four > one * 1.5, "polyphony not summing: 1 voice {one}, 4 voices {four}");
}

#[test]
fn mono_legato_steals() {
    let mut engine = Engine::new(48_000.0);
    engine.on_message(&voice_plan());
    engine.on_message(&encode_batch(&[
        Msg::set_param(ADSR as u32, params::adsr::ATTACK, 0.002),
        Msg::set_param(ADSR as u32, params::adsr::SUSTAIN, 1.0),
        Msg::set_param(ADSR as u32, params::adsr::RELEASE, 0.05),
    ]));

    // Hold two notes at polyphony 1: the second steals; releasing it
    // silences (first note's lane was stolen).
    engine.on_message(&encode_batch(&[Msg::note_on(60, 100, 0), Msg::note_on(72, 100, 0)]));
    let playing = rms(&mut engine, 100);
    assert!(playing > 0.02);
    engine.on_message(&encode_batch(&[Msg::note_off(72, 0)]));
    let _tail = rms(&mut engine, 300);
    let after = rms(&mut engine, 50);
    assert!(after < 1e-3, "stolen lane resurrected: {after}");
}
