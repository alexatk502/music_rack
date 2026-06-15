//! Plan executor: fixed module slot pool, port-buffer pool, and double-
//! buffered execution plans. Everything is allocated at construction; plan
//! application and block processing are allocation- and panic-free.

use crate::buffer::{PortBuffer, BLOCK};
use crate::modules::{
    adsr::AdsrModule,
    arp::Arp,
    clock::Clock,
    clockdiv::ClockDiv,
    cvutil2::{MinMax, Offset, Phasor, TrackHold},
    delay::Delay,
    dynamics2::{Drive, Ducker, Gate, Transient},
    filters2::{Ladder, Lpg},
    filters3::{AutoWah, DualFilter, SvfMulti},
    funcgen::{ComplexLfo, EnvFollower, Maths},
    fx::{Chorus, Phaser, Reverb},
    fx2::{Flanger, Limiter, ParamEq, PingPong, Saturator, Stereo},
    lfo::Lfo,
    macro_osc::MacroOsc,
    mixer::Mixer,
    noise::Noise,
    note_in::NoteIn,
    osc2::{Additive, Drum, FmOp},
    output::OutputModule,
    quantizer::Quantizer,
    random::Random,
    route::{Compressor, Crossfade, CvMix, Octave, Pan, SeqSwitch, VcaBank},
    seq2::{Bernoulli, Euclid, Turing},
    seq3::{Beats, Ratchet},
    seq8::Seq8,
    sources2::{ChordOsc, Pluck, Resonator, SubOsc, Supersaw, Wavefold},
    timbre::{BitCrush, Comb, Formant, RingMod},
    timefx::{FreqShift, PitchShift, Shimmer, TapeDelay, Tremolo, Vibrato, Vocoder},
    util::{Attenuverter, Logic, Mult, SampleHold, Slew, Waveshaper},
    util2::{
        Burst, ClockMult, Comparator, GateDelay, MidiCc, Probe, Rectify, Tuner, TrigTool,
        Voltmeter,
    },
    vca::Vca,
    vcf::Vcf,
    vco::Vco,
    wtvco::WtVco,
};
use crate::ProcessCtx;
use rack_core::caps::MAX_MODULES;
use rack_core::meters::{encode_meters, MeterEntry, SCOPE_DECIM, SCOPE_LEN};
use rack_core::modules::ModuleKindId;
use rack_core::plan::{decode_plan, PlanStep, MAX_BUFFERS, MAX_PORTS_IN, NO_BUFFER};

/// Per-slot signal tap: decimated waveform ring plus running peak.
#[derive(Clone, Copy)]
struct ScopeTap {
    ring: [f32; SCOPE_LEN],
    pos: u16,
    /// Decimation phase carried across blocks.
    phase: u8,
    peak: f32,
}

impl ScopeTap {
    const fn new() -> Self {
        Self { ring: [0.0; SCOPE_LEN], pos: 0, phase: 0, peak: 0.0 }
    }

    #[inline]
    fn feed(&mut self, samples: &[f32]) {
        let mut i = self.phase as usize;
        while i < samples.len() {
            self.ring[self.pos as usize] = samples[i];
            self.pos = (self.pos + 1) % SCOPE_LEN as u16;
            i += SCOPE_DECIM;
        }
        self.phase = (i - samples.len()) as u8;
        for &s in samples {
            self.peak = self.peak.max(s.abs());
        }
    }

    /// Snapshot oldest-first and reset the peak accumulator.
    fn snapshot(&mut self, slot: u16) -> MeterEntry {
        let mut entry = MeterEntry { slot, peak: self.peak, ..Default::default() };
        for (i, dst) in entry.scope.iter_mut().enumerate() {
            *dst = self.ring[(self.pos as usize + i) % SCOPE_LEN];
        }
        self.peak = 0.0;
        entry
    }
}

/// One module instance in a slot. Enum (not Box<dyn>) so slots are fixed-size
/// pool entries and dispatch is a match, not call_indirect.
pub enum ModuleSlot {
    Vco(Vco),
    Vcf(Vcf),
    Vca(Vca),
    Adsr(AdsrModule),
    Lfo(Lfo),
    Mixer(Mixer),
    Output(OutputModule),
    NoteIn(NoteIn),
    Clock(Clock),
    Seq8(Seq8),
    Delay(Delay),
    Noise(Noise),
    SampleHold(SampleHold),
    Attenuverter(Attenuverter),
    Waveshaper(Waveshaper),
    WtVco(WtVco),
    Random(Random),
    Quantizer(Quantizer),
    ClockDiv(ClockDiv),
    Mult(Mult),
    Logic(Logic),
    Slew(Slew),
    Reverb(Reverb),
    Chorus(Chorus),
    Phaser(Phaser),
    FmOp(FmOp),
    Additive(Additive),
    Drum(Drum),
    RingMod(RingMod),
    BitCrush(BitCrush),
    Comb(Comb),
    Formant(Formant),
    Maths(Maths),
    ComplexLfo(ComplexLfo),
    EnvFollow(EnvFollower),
    CvMix(CvMix),
    Octave(Octave),
    Euclid(Euclid),
    Bernoulli(Bernoulli),
    Turing(Turing),
    SeqSwitch(SeqSwitch),
    Crossfade(Crossfade),
    VcaBank(VcaBank),
    Pan(Pan),
    Compressor(Compressor),
    MacroOsc(MacroOsc),
    Lpg(Lpg),
    Ladder(Ladder),
    Saturator(Saturator),
    Flanger(Flanger),
    PingPong(PingPong),
    ParamEq(ParamEq),
    Limiter(Limiter),
    Stereo(Stereo),
    Comparator(Comparator),
    Rectify(Rectify),
    Arp(Arp),
    ClockMult(ClockMult),
    GateDelay(GateDelay),
    Burst(Burst),
    TrigTool(TrigTool),
    Voltmeter(Voltmeter),
    Tuner(Tuner),
    MidiCc(MidiCc),
    SubOsc(SubOsc),
    Supersaw(Supersaw),
    Pluck(Pluck),
    Wavefold(Wavefold),
    ChordOsc(ChordOsc),
    Resonator(Resonator),
    SvfMulti(SvfMulti),
    AutoWah(AutoWah),
    DualFilter(DualFilter),
    Drive(Drive),
    Transient(Transient),
    Ducker(Ducker),
    Gate(Gate),
    Tremolo(Tremolo),
    Vibrato(Vibrato),
    TapeDelay(TapeDelay),
    PitchShift(PitchShift),
    FreqShift(FreqShift),
    Shimmer(Shimmer),
    Vocoder(Vocoder),
    Offset(Offset),
    TrackHold(TrackHold),
    Phasor(Phasor),
    MinMax(MinMax),
    Beats(Beats),
    Ratchet(Ratchet),
    Scope(Probe),
    Spectrum(Probe),
}

impl ModuleSlot {
    fn new(kind: ModuleKindId) -> Option<Self> {
        Some(match kind {
            ModuleKindId::Vco => Self::Vco(Vco::new()),
            ModuleKindId::Vcf => Self::Vcf(Vcf::new()),
            ModuleKindId::Vca => Self::Vca(Vca::new()),
            ModuleKindId::Adsr => Self::Adsr(AdsrModule::new()),
            ModuleKindId::Lfo => Self::Lfo(Lfo::new()),
            ModuleKindId::Mixer => Self::Mixer(Mixer::new()),
            ModuleKindId::Output => Self::Output(OutputModule::new()),
            ModuleKindId::NoteIn => Self::NoteIn(NoteIn::new()),
            ModuleKindId::Clock => Self::Clock(Clock::new()),
            ModuleKindId::Seq8 => Self::Seq8(Seq8::new()),
            ModuleKindId::Delay => Self::Delay(Delay::new()),
            ModuleKindId::Noise => Self::Noise(Noise::new()),
            ModuleKindId::SampleHold => Self::SampleHold(SampleHold::new()),
            ModuleKindId::Attenuverter => Self::Attenuverter(Attenuverter::new()),
            ModuleKindId::Waveshaper => Self::Waveshaper(Waveshaper::new()),
            ModuleKindId::WtVco => Self::WtVco(WtVco::new()),
            ModuleKindId::Random => Self::Random(Random::new()),
            ModuleKindId::Quantizer => Self::Quantizer(Quantizer::new()),
            ModuleKindId::ClockDiv => Self::ClockDiv(ClockDiv::new()),
            ModuleKindId::Mult => Self::Mult(Mult::new()),
            ModuleKindId::Logic => Self::Logic(Logic::new()),
            ModuleKindId::Slew => Self::Slew(Slew::new()),
            ModuleKindId::Reverb => Self::Reverb(Reverb::new()),
            ModuleKindId::Chorus => Self::Chorus(Chorus::new()),
            ModuleKindId::Phaser => Self::Phaser(Phaser::new()),
            ModuleKindId::FmOp => Self::FmOp(FmOp::new()),
            ModuleKindId::Additive => Self::Additive(Additive::new()),
            ModuleKindId::Drum => Self::Drum(Drum::new()),
            ModuleKindId::RingMod => Self::RingMod(RingMod::new()),
            ModuleKindId::BitCrush => Self::BitCrush(BitCrush::new()),
            ModuleKindId::Comb => Self::Comb(Comb::new()),
            ModuleKindId::Formant => Self::Formant(Formant::new()),
            ModuleKindId::Maths => Self::Maths(Maths::new()),
            ModuleKindId::ComplexLfo => Self::ComplexLfo(ComplexLfo::new()),
            ModuleKindId::EnvFollow => Self::EnvFollow(EnvFollower::new()),
            ModuleKindId::CvMix => Self::CvMix(CvMix::new()),
            ModuleKindId::Octave => Self::Octave(Octave::new()),
            ModuleKindId::Euclid => Self::Euclid(Euclid::new()),
            ModuleKindId::Bernoulli => Self::Bernoulli(Bernoulli::new()),
            ModuleKindId::Turing => Self::Turing(Turing::new()),
            ModuleKindId::SeqSwitch => Self::SeqSwitch(SeqSwitch::new()),
            ModuleKindId::Crossfade => Self::Crossfade(Crossfade::new()),
            ModuleKindId::VcaBank => Self::VcaBank(VcaBank::new()),
            ModuleKindId::Pan => Self::Pan(Pan::new()),
            ModuleKindId::Compressor => Self::Compressor(Compressor::new()),
            ModuleKindId::MacroOsc => Self::MacroOsc(MacroOsc::new()),
            ModuleKindId::Lpg => Self::Lpg(Lpg::new()),
            ModuleKindId::Ladder => Self::Ladder(Ladder::new()),
            ModuleKindId::Saturator => Self::Saturator(Saturator::new()),
            ModuleKindId::Flanger => Self::Flanger(Flanger::new()),
            ModuleKindId::PingPong => Self::PingPong(PingPong::new()),
            ModuleKindId::ParamEq => Self::ParamEq(ParamEq::new()),
            ModuleKindId::Limiter => Self::Limiter(Limiter::new()),
            ModuleKindId::Stereo => Self::Stereo(Stereo::new()),
            ModuleKindId::Comparator => Self::Comparator(Comparator::new()),
            ModuleKindId::Rectify => Self::Rectify(Rectify::new()),
            ModuleKindId::Arp => Self::Arp(Arp::new()),
            ModuleKindId::ClockMult => Self::ClockMult(ClockMult::new()),
            ModuleKindId::GateDelay => Self::GateDelay(GateDelay::new()),
            ModuleKindId::Burst => Self::Burst(Burst::new()),
            ModuleKindId::TrigTool => Self::TrigTool(TrigTool::new()),
            ModuleKindId::Voltmeter => Self::Voltmeter(Voltmeter::new()),
            ModuleKindId::Tuner => Self::Tuner(Tuner::new()),
            ModuleKindId::MidiCc => Self::MidiCc(MidiCc::new()),
            ModuleKindId::SubOsc => Self::SubOsc(SubOsc::new()),
            ModuleKindId::Supersaw => Self::Supersaw(Supersaw::new()),
            ModuleKindId::Pluck => Self::Pluck(Pluck::new()),
            ModuleKindId::Wavefold => Self::Wavefold(Wavefold::new()),
            ModuleKindId::ChordOsc => Self::ChordOsc(ChordOsc::new()),
            ModuleKindId::Resonator => Self::Resonator(Resonator::new()),
            ModuleKindId::SvfMulti => Self::SvfMulti(SvfMulti::new()),
            ModuleKindId::AutoWah => Self::AutoWah(AutoWah::new()),
            ModuleKindId::DualFilter => Self::DualFilter(DualFilter::new()),
            ModuleKindId::Drive => Self::Drive(Drive::new()),
            ModuleKindId::Transient => Self::Transient(Transient::new()),
            ModuleKindId::Ducker => Self::Ducker(Ducker::new()),
            ModuleKindId::Gate => Self::Gate(Gate::new()),
            ModuleKindId::Tremolo => Self::Tremolo(Tremolo::new()),
            ModuleKindId::Vibrato => Self::Vibrato(Vibrato::new()),
            ModuleKindId::TapeDelay => Self::TapeDelay(TapeDelay::new()),
            ModuleKindId::PitchShift => Self::PitchShift(PitchShift::new()),
            ModuleKindId::FreqShift => Self::FreqShift(FreqShift::new()),
            ModuleKindId::Shimmer => Self::Shimmer(Shimmer::new()),
            ModuleKindId::Vocoder => Self::Vocoder(Vocoder::new()),
            ModuleKindId::Offset => Self::Offset(Offset::new()),
            ModuleKindId::TrackHold => Self::TrackHold(TrackHold::new()),
            ModuleKindId::Phasor => Self::Phasor(Phasor::new()),
            ModuleKindId::MinMax => Self::MinMax(MinMax::new()),
            ModuleKindId::Beats => Self::Beats(Beats::new()),
            ModuleKindId::Ratchet => Self::Ratchet(Ratchet::new()),
            ModuleKindId::Scope => Self::Scope(Probe::new()),
            ModuleKindId::Spectrum => Self::Spectrum(Probe::new()),
        })
    }

    fn kind(&self) -> ModuleKindId {
        match self {
            Self::Vco(_) => ModuleKindId::Vco,
            Self::Vcf(_) => ModuleKindId::Vcf,
            Self::Vca(_) => ModuleKindId::Vca,
            Self::Adsr(_) => ModuleKindId::Adsr,
            Self::Lfo(_) => ModuleKindId::Lfo,
            Self::Mixer(_) => ModuleKindId::Mixer,
            Self::Output(_) => ModuleKindId::Output,
            Self::NoteIn(_) => ModuleKindId::NoteIn,
            Self::Clock(_) => ModuleKindId::Clock,
            Self::Seq8(_) => ModuleKindId::Seq8,
            Self::Delay(_) => ModuleKindId::Delay,
            Self::Noise(_) => ModuleKindId::Noise,
            Self::SampleHold(_) => ModuleKindId::SampleHold,
            Self::Attenuverter(_) => ModuleKindId::Attenuverter,
            Self::Waveshaper(_) => ModuleKindId::Waveshaper,
            Self::WtVco(_) => ModuleKindId::WtVco,
            Self::Random(_) => ModuleKindId::Random,
            Self::Quantizer(_) => ModuleKindId::Quantizer,
            Self::ClockDiv(_) => ModuleKindId::ClockDiv,
            Self::Mult(_) => ModuleKindId::Mult,
            Self::Logic(_) => ModuleKindId::Logic,
            Self::Slew(_) => ModuleKindId::Slew,
            Self::Reverb(_) => ModuleKindId::Reverb,
            Self::Chorus(_) => ModuleKindId::Chorus,
            Self::Phaser(_) => ModuleKindId::Phaser,
            Self::FmOp(_) => ModuleKindId::FmOp,
            Self::Additive(_) => ModuleKindId::Additive,
            Self::Drum(_) => ModuleKindId::Drum,
            Self::RingMod(_) => ModuleKindId::RingMod,
            Self::BitCrush(_) => ModuleKindId::BitCrush,
            Self::Comb(_) => ModuleKindId::Comb,
            Self::Formant(_) => ModuleKindId::Formant,
            Self::Maths(_) => ModuleKindId::Maths,
            Self::ComplexLfo(_) => ModuleKindId::ComplexLfo,
            Self::EnvFollow(_) => ModuleKindId::EnvFollow,
            Self::CvMix(_) => ModuleKindId::CvMix,
            Self::Octave(_) => ModuleKindId::Octave,
            Self::Euclid(_) => ModuleKindId::Euclid,
            Self::Bernoulli(_) => ModuleKindId::Bernoulli,
            Self::Turing(_) => ModuleKindId::Turing,
            Self::SeqSwitch(_) => ModuleKindId::SeqSwitch,
            Self::Crossfade(_) => ModuleKindId::Crossfade,
            Self::VcaBank(_) => ModuleKindId::VcaBank,
            Self::Pan(_) => ModuleKindId::Pan,
            Self::Compressor(_) => ModuleKindId::Compressor,
            Self::MacroOsc(_) => ModuleKindId::MacroOsc,
            Self::Lpg(_) => ModuleKindId::Lpg,
            Self::Ladder(_) => ModuleKindId::Ladder,
            Self::Saturator(_) => ModuleKindId::Saturator,
            Self::Flanger(_) => ModuleKindId::Flanger,
            Self::PingPong(_) => ModuleKindId::PingPong,
            Self::ParamEq(_) => ModuleKindId::ParamEq,
            Self::Limiter(_) => ModuleKindId::Limiter,
            Self::Stereo(_) => ModuleKindId::Stereo,
            Self::Comparator(_) => ModuleKindId::Comparator,
            Self::Rectify(_) => ModuleKindId::Rectify,
            Self::Arp(_) => ModuleKindId::Arp,
            Self::ClockMult(_) => ModuleKindId::ClockMult,
            Self::GateDelay(_) => ModuleKindId::GateDelay,
            Self::Burst(_) => ModuleKindId::Burst,
            Self::TrigTool(_) => ModuleKindId::TrigTool,
            Self::Voltmeter(_) => ModuleKindId::Voltmeter,
            Self::Tuner(_) => ModuleKindId::Tuner,
            Self::MidiCc(_) => ModuleKindId::MidiCc,
            Self::SubOsc(_) => ModuleKindId::SubOsc,
            Self::Supersaw(_) => ModuleKindId::Supersaw,
            Self::Pluck(_) => ModuleKindId::Pluck,
            Self::Wavefold(_) => ModuleKindId::Wavefold,
            Self::ChordOsc(_) => ModuleKindId::ChordOsc,
            Self::Resonator(_) => ModuleKindId::Resonator,
            Self::SvfMulti(_) => ModuleKindId::SvfMulti,
            Self::AutoWah(_) => ModuleKindId::AutoWah,
            Self::DualFilter(_) => ModuleKindId::DualFilter,
            Self::Drive(_) => ModuleKindId::Drive,
            Self::Transient(_) => ModuleKindId::Transient,
            Self::Ducker(_) => ModuleKindId::Ducker,
            Self::Gate(_) => ModuleKindId::Gate,
            Self::Tremolo(_) => ModuleKindId::Tremolo,
            Self::Vibrato(_) => ModuleKindId::Vibrato,
            Self::TapeDelay(_) => ModuleKindId::TapeDelay,
            Self::PitchShift(_) => ModuleKindId::PitchShift,
            Self::FreqShift(_) => ModuleKindId::FreqShift,
            Self::Shimmer(_) => ModuleKindId::Shimmer,
            Self::Vocoder(_) => ModuleKindId::Vocoder,
            Self::Offset(_) => ModuleKindId::Offset,
            Self::TrackHold(_) => ModuleKindId::TrackHold,
            Self::Phasor(_) => ModuleKindId::Phasor,
            Self::MinMax(_) => ModuleKindId::MinMax,
            Self::Beats(_) => ModuleKindId::Beats,
            Self::Ratchet(_) => ModuleKindId::Ratchet,
            Self::Scope(_) => ModuleKindId::Scope,
            Self::Spectrum(_) => ModuleKindId::Spectrum,
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match self {
            Self::Vco(m) => m.set_param(param, value),
            Self::Vcf(m) => m.set_param(param, value),
            Self::Vca(m) => m.set_param(param, value),
            Self::Adsr(m) => m.set_param(param, value),
            Self::Lfo(m) => m.set_param(param, value),
            Self::Mixer(m) => m.set_param(param, value),
            Self::Output(m) => m.set_param(param, value),
            Self::NoteIn(m) => m.set_param(param, value),
            Self::Clock(m) => m.set_param(param, value),
            Self::Seq8(m) => m.set_param(param, value),
            Self::Delay(m) => m.set_param(param, value),
            Self::Noise(m) => m.set_param(param, value),
            Self::SampleHold(m) => m.set_param(param, value),
            Self::Attenuverter(m) => m.set_param(param, value),
            Self::Waveshaper(m) => m.set_param(param, value),
            Self::WtVco(m) => m.set_param(param, value),
            Self::Random(m) => m.set_param(param, value),
            Self::Quantizer(m) => m.set_param(param, value),
            Self::ClockDiv(m) => m.set_param(param, value),
            Self::Mult(m) => m.set_param(param, value),
            Self::Logic(m) => m.set_param(param, value),
            Self::Slew(m) => m.set_param(param, value),
            Self::Reverb(m) => m.set_param(param, value),
            Self::Chorus(m) => m.set_param(param, value),
            Self::Phaser(m) => m.set_param(param, value),
            Self::FmOp(m) => m.set_param(param, value),
            Self::Additive(m) => m.set_param(param, value),
            Self::Drum(m) => m.set_param(param, value),
            Self::RingMod(m) => m.set_param(param, value),
            Self::BitCrush(m) => m.set_param(param, value),
            Self::Comb(m) => m.set_param(param, value),
            Self::Formant(m) => m.set_param(param, value),
            Self::Maths(m) => m.set_param(param, value),
            Self::ComplexLfo(m) => m.set_param(param, value),
            Self::EnvFollow(m) => m.set_param(param, value),
            Self::CvMix(m) => m.set_param(param, value),
            Self::Octave(m) => m.set_param(param, value),
            Self::Euclid(m) => m.set_param(param, value),
            Self::Bernoulli(m) => m.set_param(param, value),
            Self::Turing(m) => m.set_param(param, value),
            Self::SeqSwitch(m) => m.set_param(param, value),
            Self::Crossfade(m) => m.set_param(param, value),
            Self::VcaBank(m) => m.set_param(param, value),
            Self::Pan(m) => m.set_param(param, value),
            Self::Compressor(m) => m.set_param(param, value),
            Self::MacroOsc(m) => m.set_param(param, value),
            Self::Lpg(m) => m.set_param(param, value),
            Self::Ladder(m) => m.set_param(param, value),
            Self::Saturator(m) => m.set_param(param, value),
            Self::Flanger(m) => m.set_param(param, value),
            Self::PingPong(m) => m.set_param(param, value),
            Self::ParamEq(m) => m.set_param(param, value),
            Self::Limiter(m) => m.set_param(param, value),
            Self::Stereo(m) => m.set_param(param, value),
            Self::Comparator(m) => m.set_param(param, value),
            Self::Rectify(m) => m.set_param(param, value),
            Self::Arp(m) => m.set_param(param, value),
            Self::ClockMult(m) => m.set_param(param, value),
            Self::GateDelay(m) => m.set_param(param, value),
            Self::Burst(m) => m.set_param(param, value),
            Self::TrigTool(m) => m.set_param(param, value),
            Self::Voltmeter(m) => m.set_param(param, value),
            Self::Tuner(m) => m.set_param(param, value),
            Self::MidiCc(m) => m.set_param(param, value),
            Self::SubOsc(m) => m.set_param(param, value),
            Self::Supersaw(m) => m.set_param(param, value),
            Self::Pluck(m) => m.set_param(param, value),
            Self::Wavefold(m) => m.set_param(param, value),
            Self::ChordOsc(m) => m.set_param(param, value),
            Self::Resonator(m) => m.set_param(param, value),
            Self::SvfMulti(m) => m.set_param(param, value),
            Self::AutoWah(m) => m.set_param(param, value),
            Self::DualFilter(m) => m.set_param(param, value),
            Self::Drive(m) => m.set_param(param, value),
            Self::Transient(m) => m.set_param(param, value),
            Self::Ducker(m) => m.set_param(param, value),
            Self::Gate(m) => m.set_param(param, value),
            Self::Tremolo(m) => m.set_param(param, value),
            Self::Vibrato(m) => m.set_param(param, value),
            Self::TapeDelay(m) => m.set_param(param, value),
            Self::PitchShift(m) => m.set_param(param, value),
            Self::FreqShift(m) => m.set_param(param, value),
            Self::Shimmer(m) => m.set_param(param, value),
            Self::Vocoder(m) => m.set_param(param, value),
            Self::Offset(m) => m.set_param(param, value),
            Self::TrackHold(m) => m.set_param(param, value),
            Self::Phasor(m) => m.set_param(param, value),
            Self::MinMax(m) => m.set_param(param, value),
            Self::Beats(m) => m.set_param(param, value),
            Self::Ratchet(m) => m.set_param(param, value),
            Self::Scope(m) => m.set_param(param, value),
            Self::Spectrum(m) => m.set_param(param, value),
        }
    }
}

/// Master stereo bus written by Output modules.
pub struct MasterOut {
    pub l: [f32; BLOCK],
    pub r: [f32; BLOCK],
}

pub struct Executor {
    slots: Vec<Option<ModuleSlot>>,
    buffers: Vec<PortBuffer>,
    /// Double-buffered plans; `active` indexes the one `process` runs.
    plans: [Vec<PlanStep>; 2],
    active: usize,
    epoch: u32,
    /// Scratch copies of input buffers, so a module reading its own output
    /// (self-feedback) sees the previous block rather than aliasing.
    scratch: Box<[PortBuffer; MAX_PORTS_IN]>,
    /// Signal taps for UI meters, indexed by slot.
    taps: Box<[ScopeTap; MAX_MODULES]>,
    pub master: MasterOut,
}

impl Executor {
    pub fn new() -> Self {
        let mut plans_a = Vec::with_capacity(MAX_MODULES);
        let plans_b = Vec::with_capacity(MAX_MODULES);
        plans_a.clear();
        Self {
            slots: (0..MAX_MODULES).map(|_| None).collect(),
            buffers: vec![PortBuffer::silent(); MAX_BUFFERS],
            plans: [plans_a, plans_b],
            active: 0,
            epoch: 0,
            scratch: Box::new([PortBuffer::silent(); MAX_PORTS_IN]),
            taps: Box::new([ScopeTap::new(); MAX_MODULES]),
            master: MasterOut { l: [0.0; BLOCK], r: [0.0; BLOCK] },
        }
    }

    /// Meter snapshot for every module in the active plan. Allocates one Vec
    /// — called at UI rate (~30 Hz) from the message path, not per quantum.
    pub fn take_meters(&mut self) -> Vec<u8> {
        let plan = &self.plans[self.active];
        let mut entries = Vec::with_capacity(plan.len());
        for step in plan {
            entries.push(self.taps[step.slot as usize].snapshot(step.slot));
        }
        encode_meters(&entries)
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn set_param(&mut self, slot: u32, param: u32, value: f32) {
        if let Some(Some(module)) = self.slots.get_mut(slot as usize) {
            module.set_param(param, value);
        }
    }

    /// Note events broadcast to every note-consuming module (NoteIn and Arp).
    pub fn note_on(&mut self, note: u8, velocity: u8) {
        for slot in self.slots.iter_mut().flatten() {
            match slot {
                ModuleSlot::NoteIn(m) => m.note_on(note, velocity),
                ModuleSlot::Arp(m) => m.note_on(note),
                _ => {}
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        for slot in self.slots.iter_mut().flatten() {
            match slot {
                ModuleSlot::NoteIn(m) => m.note_off(note),
                ModuleSlot::Arp(m) => m.note_off(note),
                _ => {}
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for slot in self.slots.iter_mut().flatten() {
            match slot {
                ModuleSlot::NoteIn(m) => m.all_notes_off(),
                ModuleSlot::Arp(m) => m.all_notes_off(),
                _ => {}
            }
        }
    }

    /// Apply a plan blob. Runs between quanta on the audio thread: validates
    /// and rejects rather than panicking, fills pre-allocated storage only.
    /// Returns true if the plan was accepted.
    pub fn apply_plan(&mut self, bytes: &[u8]) -> bool {
        let Some(view) = decode_plan(bytes) else { return false };
        if view.steps.len() > MAX_MODULES {
            return false;
        }
        // Validate all indices before touching state.
        for step in view.steps {
            if step.slot as usize >= MAX_MODULES {
                return false;
            }
            if step.inputs.iter().any(|&i| i != NO_BUFFER && i as usize >= MAX_BUFFERS) {
                return false;
            }
            if step.outputs.iter().any(|&o| o as usize >= MAX_BUFFERS) {
                return false;
            }
        }
        for init in view.modules {
            if init.slot as usize >= MAX_MODULES {
                return false;
            }
        }

        // Sync the slot pool: construct new/changed modules, drop dead ones.
        let mut seen = [false; MAX_MODULES];
        for init in view.modules {
            let slot = init.slot as usize;
            seen[slot] = true;
            let Some(kind) = ModuleKindId::from_u16(init.kind) else { continue };
            let needs_build = !matches!(&self.slots[slot], Some(m) if m.kind() == kind);
            if needs_build {
                self.slots[slot] = ModuleSlot::new(kind);
            }
        }
        for (slot, module) in self.slots.iter_mut().enumerate() {
            if !seen[slot] {
                *module = None;
            }
        }

        // Copy steps into the inactive plan and swap.
        let inactive = 1 - self.active;
        self.plans[inactive].clear();
        self.plans[inactive].extend_from_slice(view.steps);
        self.active = inactive;
        self.epoch = view.header.epoch;
        true
    }

    /// Render one sub-block (frames ≤ BLOCK) into `self.master`.
    pub fn process_block(&mut self, ctx: &ProcessCtx, frames: usize) {
        self.master.l[..frames].fill(0.0);
        self.master.r[..frames].fill(0.0);

        let plan = &self.plans[self.active];
        for step in plan {
            let Some(Some(module)) = self.slots.get_mut(step.slot as usize) else { continue };

            // Copy connected inputs to scratch (handles self-feedback
            // aliasing; a feedback consumer reads the previous block).
            let mut connected = [false; MAX_PORTS_IN];
            for (i, &buf_idx) in step.inputs.iter().enumerate() {
                if buf_idx != NO_BUFFER {
                    self.scratch[i] = self.buffers[buf_idx as usize];
                    connected[i] = true;
                }
            }
            let input = |i: usize| -> Option<&PortBuffer> {
                connected[i].then(|| &self.scratch[i])
            };

            match module {
                ModuleSlot::Vco(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Vcf(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Vca(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Adsr(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Lfo(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Mixer(m) => {
                    let inputs = [input(0), input(1), input(2), input(3)];
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, inputs, out, frames);
                }
                ModuleSlot::Output(m) => {
                    m.process(
                        ctx,
                        input(0),
                        input(1),
                        &mut self.master.l[..frames],
                        &mut self.master.r[..frames],
                    );
                }
                ModuleSlot::NoteIn(m) => {
                    // Four distinct output buffers (planner guarantees
                    // distinct indices; reject the step if not).
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, outs, frames);
                    }
                }
                ModuleSlot::Clock(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, outs, frames);
                    }
                }
                ModuleSlot::Seq8(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([voct, gate]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), voct, gate, frames);
                    }
                }
                ModuleSlot::Delay(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Noise(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, out, frames);
                }
                ModuleSlot::SampleHold(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Attenuverter(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Waveshaper(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::WtVco(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Random(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([stepped, smooth]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), stepped, smooth, frames);
                    }
                }
                ModuleSlot::Quantizer(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::ClockDiv(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), outs, frames);
                    }
                }
                ModuleSlot::Mult(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), outs, frames);
                    }
                }
                ModuleSlot::Logic(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                    ];
                    if let Ok([a_out, o_out, x_out]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), a_out, o_out, x_out, frames);
                    }
                }
                ModuleSlot::Slew(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Reverb(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([l, r]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), l, r, frames);
                    }
                }
                ModuleSlot::Chorus(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([l, r]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), l, r, frames);
                    }
                }
                ModuleSlot::Phaser(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::FmOp(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Additive(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Drum(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::RingMod(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::BitCrush(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Comb(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Formant(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Maths(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([out, eoc]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), out, eoc, frames);
                    }
                }
                ModuleSlot::ComplexLfo(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), outs, frames);
                    }
                }
                ModuleSlot::EnvFollow(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::CvMix(m) => {
                    let ins = [input(0), input(1), input(2), input(3)];
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, ins, out, frames);
                }
                ModuleSlot::Octave(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Euclid(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Bernoulli(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([a, b]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), a, b, frames);
                    }
                }
                ModuleSlot::Turing(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([gate, cv]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), gate, cv, frames);
                    }
                }
                ModuleSlot::SeqSwitch(m) => {
                    let ins = [input(2), input(3), input(4), input(5)];
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), ins, out, frames);
                }
                ModuleSlot::Crossfade(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), input(2), out, frames);
                }
                ModuleSlot::VcaBank(m) => {
                    let ins = [input(0), input(1), input(2), input(3)];
                    let cvs = [input(4), input(5), input(6), input(7)];
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, ins, cvs, outs, frames);
                    }
                }
                ModuleSlot::Pan(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([l, r]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), l, r, frames);
                    }
                }
                ModuleSlot::Compressor(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::MacroOsc(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([main, aux]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), input(2), main, aux, frames);
                    }
                }
                ModuleSlot::Lpg(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), input(2), out, frames);
                }
                ModuleSlot::Ladder(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Saturator(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Flanger(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::PingPong(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([l, r]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), l, r, frames);
                    }
                }
                ModuleSlot::ParamEq(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Limiter(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Stereo(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([l, r]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), l, r, frames);
                    }
                }
                ModuleSlot::Comparator(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([gate, inv]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), gate, inv, frames);
                    }
                }
                ModuleSlot::Rectify(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                    ];
                    if let Ok([a, mx, mn]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), a, mx, mn, frames);
                    }
                }
                ModuleSlot::Arp(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([voct, gate]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), voct, gate, frames);
                    }
                }
                ModuleSlot::ClockMult(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                    ];
                    if let Ok(outs) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), outs, frames);
                    }
                }
                ModuleSlot::GateDelay(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Burst(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::TrigTool(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([gate, trig]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), gate, trig, frames);
                    }
                }
                ModuleSlot::Voltmeter(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Tuner(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::MidiCc(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, out, frames);
                }
                ModuleSlot::SubOsc(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Supersaw(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Pluck(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Wavefold(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::ChordOsc(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Resonator(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::SvfMulti(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok([lp, bp, hp, notch]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), lp, bp, hp, notch, frames);
                    }
                }
                ModuleSlot::AutoWah(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::DualFilter(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([l, r]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), l, r, frames);
                    }
                }
                ModuleSlot::Drive(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Transient(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Ducker(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Gate(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Tremolo(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Vibrato(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::TapeDelay(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::PitchShift(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::FreqShift(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Shimmer(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::Vocoder(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Offset(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
                ModuleSlot::TrackHold(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Phasor(m) => {
                    let idx = [step.outputs[0] as usize, step.outputs[1] as usize];
                    if let Ok([ramp, pulse]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), ramp, pulse, frames);
                    }
                }
                ModuleSlot::MinMax(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                    ];
                    if let Ok([mn, mx, mean]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), input(2), mn, mx, mean, frames);
                    }
                }
                ModuleSlot::Beats(m) => {
                    let idx = [
                        step.outputs[0] as usize,
                        step.outputs[1] as usize,
                        step.outputs[2] as usize,
                        step.outputs[3] as usize,
                    ];
                    if let Ok([a, b, c, d]) = self.buffers.get_disjoint_mut(idx) {
                        m.process(ctx, input(0), input(1), &mut [a, b, c, d], frames);
                    }
                }
                ModuleSlot::Ratchet(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), input(1), out, frames);
                }
                ModuleSlot::Scope(m) | ModuleSlot::Spectrum(m) => {
                    let out = &mut self.buffers[step.outputs[0] as usize];
                    m.process(ctx, input(0), out, frames);
                }
            }

            // Tap the produced signal for UI meters (Output taps the master
            // bus since it has no output buffer).
            let tap = &mut self.taps[step.slot as usize];
            if step.kind == ModuleKindId::Output as u16 {
                tap.feed(&self.master.l[..frames]);
            } else {
                tap.feed(&self.buffers[step.outputs[0] as usize].data[0][..frames]);
            }
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rack_core::modules::params;
    use rack_core::plan::{encode_plan, ModuleInit, TRASH_BUFFER};

    fn demo_plan() -> Vec<u8> {
        // VCO (slot 0, buffer 8) → VCF (slot 1, buffer 9) → VCA (slot 2,
        // buffer 10) → Output (slot 3).
        let modules = [
            ModuleInit { slot: 0, kind: ModuleKindId::Vco as u16 },
            ModuleInit { slot: 1, kind: ModuleKindId::Vcf as u16 },
            ModuleInit { slot: 2, kind: ModuleKindId::Vca as u16 },
            ModuleInit { slot: 3, kind: ModuleKindId::Output as u16 },
        ];
        let mut steps = [PlanStep::default(); 4];
        steps[0] = PlanStep { slot: 0, kind: ModuleKindId::Vco as u16, ..Default::default() };
        steps[0].outputs[0] = 8;
        steps[1] = PlanStep { slot: 1, kind: ModuleKindId::Vcf as u16, ..Default::default() };
        steps[1].inputs[0] = 8;
        steps[1].outputs[0] = 9;
        steps[2] = PlanStep { slot: 2, kind: ModuleKindId::Vca as u16, ..Default::default() };
        steps[2].inputs[0] = 9;
        steps[2].outputs[0] = 10;
        steps[3] = PlanStep { slot: 3, kind: ModuleKindId::Output as u16, ..Default::default() };
        steps[3].inputs[0] = 10;
        encode_plan(1, &modules, &steps)
    }

    #[test]
    fn scope_passes_audio_through() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        // VCO (slot 0, buf 8) → Scope (slot 1, thru buf 9) → Output (slot 2).
        let modules = [
            ModuleInit { slot: 0, kind: ModuleKindId::Vco as u16 },
            ModuleInit { slot: 1, kind: ModuleKindId::Scope as u16 },
            ModuleInit { slot: 2, kind: ModuleKindId::Output as u16 },
        ];
        let mut steps = [PlanStep::default(); 3];
        steps[0] = PlanStep { slot: 0, kind: ModuleKindId::Vco as u16, ..Default::default() };
        steps[0].outputs[0] = 8;
        steps[1] = PlanStep { slot: 1, kind: ModuleKindId::Scope as u16, ..Default::default() };
        steps[1].inputs[0] = 8;
        steps[1].outputs[0] = 9;
        steps[2] = PlanStep { slot: 2, kind: ModuleKindId::Output as u16, ..Default::default() };
        steps[2].inputs[0] = 9;
        assert!(ex.apply_plan(&encode_plan(1, &modules, &steps)));
        let mut peak = 0.0f32;
        for _ in 0..200 {
            ex.process_block(&ctx, BLOCK);
            for &s in &ex.master.l[..BLOCK] {
                peak = peak.max(s.abs());
            }
        }
        assert!(peak > 0.05, "scope did not pass audio: peak {peak}");
    }

    #[test]
    fn beats_drives_output_on_a_clock() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        // Clock (slot 0, out → buf 8) → Beats (slot 1, t1 → buf 9) → Output.
        let modules = [
            ModuleInit { slot: 0, kind: ModuleKindId::Clock as u16 },
            ModuleInit { slot: 1, kind: ModuleKindId::Beats as u16 },
            ModuleInit { slot: 2, kind: ModuleKindId::Output as u16 },
        ];
        let mut steps = [PlanStep::default(); 3];
        steps[0] = PlanStep { slot: 0, kind: ModuleKindId::Clock as u16, ..Default::default() };
        steps[0].outputs[0] = 8;
        steps[1] = PlanStep { slot: 1, kind: ModuleKindId::Beats as u16, ..Default::default() };
        steps[1].inputs[0] = 8; // clock
        steps[1].outputs[0] = 9; // t1 connected
        steps[1].outputs[1] = TRASH_BUFFER + 1; // t2..t4 unconnected (as the planner does)
        steps[1].outputs[2] = TRASH_BUFFER + 2;
        steps[1].outputs[3] = TRASH_BUFFER + 3;
        steps[2] = PlanStep { slot: 2, kind: ModuleKindId::Output as u16, ..Default::default() };
        steps[2].inputs[0] = 9;
        assert!(ex.apply_plan(&encode_plan(1, &modules, &steps)));
        ex.set_param(0, 0, 300.0); // fast clock so edges come quickly
        ex.set_param(1, 0, 0xFFFF as f32); // track 1: every step on

        let mut peak = 0.0f32;
        for _ in 0..4000 {
            ex.process_block(&ctx, BLOCK);
            for &s in &ex.master.l[..BLOCK] {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
        }
        assert!(peak > 0.05, "beats produced no output: peak {peak}");
    }

    #[test]
    fn full_chain_makes_sound() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        assert!(ex.apply_plan(&demo_plan()));

        let mut peak = 0.0f32;
        for _ in 0..200 {
            ex.process_block(&ctx, BLOCK);
            for &s in &ex.master.l[..BLOCK] {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
        }
        assert!(peak > 0.05, "chain is silent: peak {peak}");
    }

    #[test]
    fn closing_the_filter_silences_the_chain() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        ex.apply_plan(&demo_plan());
        // Slam the cutoff to the knob minimum (-4 V ≈ 16 Hz).
        ex.set_param(1, params::vcf::CUTOFF, -4.0);
        // Let smoothing settle, then measure.
        for _ in 0..2000 {
            ex.process_block(&ctx, BLOCK);
        }
        let mut peak = 0.0f32;
        for _ in 0..200 {
            ex.process_block(&ctx, BLOCK);
            for &s in &ex.master.l[..BLOCK] {
                peak = peak.max(s.abs());
            }
        }
        assert!(peak < 0.02, "filter at 16 Hz still passing audio: {peak}");
    }

    #[test]
    fn live_plan_swap_keeps_module_state_and_audio() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        ex.apply_plan(&demo_plan());
        for _ in 0..100 {
            ex.process_block(&ctx, BLOCK);
        }

        // New plan: bypass the VCF (VCO → VCA → Output), same slots.
        let modules = [
            ModuleInit { slot: 0, kind: ModuleKindId::Vco as u16 },
            ModuleInit { slot: 2, kind: ModuleKindId::Vca as u16 },
            ModuleInit { slot: 3, kind: ModuleKindId::Output as u16 },
        ];
        let mut steps = [PlanStep::default(); 3];
        steps[0] = PlanStep { slot: 0, kind: ModuleKindId::Vco as u16, ..Default::default() };
        steps[0].outputs[0] = 8;
        steps[1] = PlanStep { slot: 2, kind: ModuleKindId::Vca as u16, ..Default::default() };
        steps[1].inputs[0] = 8;
        steps[1].outputs[0] = 10;
        steps[2] = PlanStep { slot: 3, kind: ModuleKindId::Output as u16, ..Default::default() };
        steps[2].inputs[0] = 10;
        assert!(ex.apply_plan(&encode_plan(2, &modules, &steps)));
        assert_eq!(ex.epoch(), 2);

        let mut peak = 0.0f32;
        for _ in 0..200 {
            ex.process_block(&ctx, BLOCK);
            for &s in &ex.master.l[..BLOCK] {
                assert!(s.is_finite());
                peak = peak.max(s.abs());
            }
        }
        assert!(peak > 0.05, "audio died after live edit: {peak}");
        // VCF slot was dropped.
        assert!(ex.slots[1].is_none());
        // VCO survived (same slot, same kind → state kept).
        assert!(matches!(ex.slots[0], Some(ModuleSlot::Vco(_))));
    }

    #[test]
    fn meters_report_signal_and_reset_peaks() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        ex.apply_plan(&demo_plan());
        for _ in 0..50 {
            ex.process_block(&ctx, BLOCK);
        }
        let blob = ex.take_meters();
        let entries = rack_core::meters::decode_meters(&blob).expect("decodes");
        assert_eq!(entries.len(), 4);
        // The VCO (slot 0) swings ±5 V; its scope must show real waveform.
        let vco = entries.iter().find(|e| e.slot == 0).unwrap();
        assert!(vco.peak > 4.0, "vco peak {}", vco.peak);
        assert!(vco.scope.iter().any(|&s| s > 1.0));
        assert!(vco.scope.iter().any(|&s| s < -1.0));
        // Output module taps the master bus (±1-ish after scaling).
        let out = entries.iter().find(|e| e.slot == 3).unwrap();
        assert!(out.peak > 0.05 && out.peak <= 1.0, "master peak {}", out.peak);

        // Peaks reset between snapshots: silence the patch, expect ~0 next.
        ex.set_param(2, rack_core::modules::params::vca::GAIN, 0.0);
        for _ in 0..2000 {
            ex.process_block(&ctx, BLOCK);
        }
        let _ = ex.take_meters();
        for _ in 0..50 {
            ex.process_block(&ctx, BLOCK);
        }
        let blob = ex.take_meters();
        let entries = rack_core::meters::decode_meters(&blob).unwrap();
        let out = entries.iter().find(|e| e.slot == 3).unwrap();
        assert!(out.peak < 0.01, "peak did not reset: {}", out.peak);
    }

    #[test]
    fn garbage_plans_are_rejected() {
        let mut ex = Executor::new();
        assert!(!ex.apply_plan(&[1, 2, 3]));
        // Out-of-range buffer index.
        let modules = [ModuleInit { slot: 0, kind: ModuleKindId::Vco as u16 }];
        let mut step = PlanStep { slot: 0, kind: ModuleKindId::Vco as u16, ..Default::default() };
        step.outputs[0] = (MAX_BUFFERS + 1) as u16;
        assert!(!ex.apply_plan(&encode_plan(1, &modules, &[step])));
        // Valid plans still apply afterwards.
        assert!(ex.apply_plan(&demo_plan()));
    }

    #[test]
    fn self_feedback_does_not_explode() {
        // VCA feeding its own CV input (degenerate but legal patch).
        let ctx = ProcessCtx::new(48_000.0);
        let mut ex = Executor::new();
        let modules = [
            ModuleInit { slot: 0, kind: ModuleKindId::Vco as u16 },
            ModuleInit { slot: 1, kind: ModuleKindId::Vca as u16 },
            ModuleInit { slot: 2, kind: ModuleKindId::Output as u16 },
        ];
        let mut steps = [PlanStep::default(); 3];
        steps[0] = PlanStep { slot: 0, kind: ModuleKindId::Vco as u16, ..Default::default() };
        steps[0].outputs[0] = 8;
        steps[1] = PlanStep { slot: 1, kind: ModuleKindId::Vca as u16, ..Default::default() };
        steps[1].inputs[0] = 8;
        steps[1].inputs[1] = 9; // its own output
        steps[1].outputs[0] = 9;
        steps[2] = PlanStep { slot: 2, kind: ModuleKindId::Output as u16, ..Default::default() };
        steps[2].inputs[0] = 9;
        assert!(ex.apply_plan(&encode_plan(1, &modules, &steps)));
        for _ in 0..500 {
            ex.process_block(&ctx, BLOCK);
            for &s in &ex.master.l[..BLOCK] {
                assert!(s.is_finite());
                assert!(s.abs() <= 1.0);
            }
        }
    }
}
