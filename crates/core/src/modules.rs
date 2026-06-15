//! Static module metadata shared by the UI (panels, plan building) and the
//! engine (port counts, param indices). Ports and params are addressed by
//! index at runtime; names exist for panels and patch serialization.

/// Module kind discriminant, stable across versions (serialized in patches
/// by name, sent in plans by value).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ModuleKindId {
    Vco = 0,
    Vcf = 1,
    Vca = 2,
    Adsr = 3,
    Lfo = 4,
    Mixer = 5,
    Output = 6,
    NoteIn = 7,
    Clock = 8,
    Seq8 = 9,
    Delay = 10,
    Noise = 11,
    SampleHold = 12,
    Attenuverter = 13,
    Waveshaper = 14,
    WtVco = 15,
    Random = 16,
    Quantizer = 17,
    ClockDiv = 18,
    Mult = 19,
    Logic = 20,
    Slew = 21,
    Reverb = 22,
    Chorus = 23,
    Phaser = 24,
    FmOp = 25,
    Additive = 26,
    Drum = 27,
    RingMod = 28,
    BitCrush = 29,
    Comb = 30,
    Formant = 31,
    Maths = 32,
    ComplexLfo = 33,
    EnvFollow = 34,
    CvMix = 35,
    Octave = 36,
    Euclid = 37,
    Bernoulli = 38,
    Turing = 39,
    SeqSwitch = 40,
    Crossfade = 41,
    VcaBank = 42,
    Pan = 43,
    Compressor = 44,
    MacroOsc = 45,
    Lpg = 46,
    Ladder = 47,
    Saturator = 48,
    Flanger = 49,
    PingPong = 50,
    ParamEq = 51,
    Limiter = 52,
    Stereo = 53,
    Comparator = 54,
    Rectify = 55,
    Arp = 56,
    ClockMult = 57,
    GateDelay = 58,
    Burst = 59,
    TrigTool = 60,
    Voltmeter = 61,
    Tuner = 62,
    MidiCc = 63,
    SubOsc = 64,
    Supersaw = 65,
    Pluck = 66,
    Wavefold = 67,
    ChordOsc = 68,
    Resonator = 69,
    SvfMulti = 70,
    AutoWah = 71,
    DualFilter = 72,
    Drive = 73,
    Transient = 74,
    Ducker = 75,
    Gate = 76,
    Tremolo = 77,
    Vibrato = 78,
    TapeDelay = 79,
    PitchShift = 80,
    FreqShift = 81,
    Shimmer = 82,
    Vocoder = 83,
    Offset = 84,
    TrackHold = 85,
    Phasor = 86,
    MinMax = 87,
    Beats = 88,
    Ratchet = 89,
    Scope = 90,
    Spectrum = 91,
}

impl ModuleKindId {
    pub const ALL: [ModuleKindId; 92] = [
        Self::Vco,
        Self::Vcf,
        Self::Vca,
        Self::Adsr,
        Self::Lfo,
        Self::Mixer,
        Self::Output,
        Self::NoteIn,
        Self::Clock,
        Self::Seq8,
        Self::Delay,
        Self::Noise,
        Self::SampleHold,
        Self::Attenuverter,
        Self::Waveshaper,
        Self::WtVco,
        Self::Random,
        Self::Quantizer,
        Self::ClockDiv,
        Self::Mult,
        Self::Logic,
        Self::Slew,
        Self::Reverb,
        Self::Chorus,
        Self::Phaser,
        Self::FmOp,
        Self::Additive,
        Self::Drum,
        Self::RingMod,
        Self::BitCrush,
        Self::Comb,
        Self::Formant,
        Self::Maths,
        Self::ComplexLfo,
        Self::EnvFollow,
        Self::CvMix,
        Self::Octave,
        Self::Euclid,
        Self::Bernoulli,
        Self::Turing,
        Self::SeqSwitch,
        Self::Crossfade,
        Self::VcaBank,
        Self::Pan,
        Self::Compressor,
        Self::MacroOsc,
        Self::Lpg,
        Self::Ladder,
        Self::Saturator,
        Self::Flanger,
        Self::PingPong,
        Self::ParamEq,
        Self::Limiter,
        Self::Stereo,
        Self::Comparator,
        Self::Rectify,
        Self::Arp,
        Self::ClockMult,
        Self::GateDelay,
        Self::Burst,
        Self::TrigTool,
        Self::Voltmeter,
        Self::Tuner,
        Self::MidiCc,
        Self::SubOsc,
        Self::Supersaw,
        Self::Pluck,
        Self::Wavefold,
        Self::ChordOsc,
        Self::Resonator,
        Self::SvfMulti,
        Self::AutoWah,
        Self::DualFilter,
        Self::Drive,
        Self::Transient,
        Self::Ducker,
        Self::Gate,
        Self::Tremolo,
        Self::Vibrato,
        Self::TapeDelay,
        Self::PitchShift,
        Self::FreqShift,
        Self::Shimmer,
        Self::Vocoder,
        Self::Offset,
        Self::TrackHold,
        Self::Phasor,
        Self::MinMax,
        Self::Beats,
        Self::Ratchet,
        Self::Scope,
        Self::Spectrum,
    ];

    pub fn from_u16(v: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|k| *k as u16 == v)
    }

    pub fn desc(self) -> &'static ModuleDesc {
        &DESCS[self as usize]
    }

    /// Stable string for patch serialization.
    pub fn type_name(self) -> &'static str {
        self.desc().type_name
    }

    pub fn from_type_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.type_name() == name)
    }
}

pub struct PortDesc {
    pub name: &'static str,
}

pub struct ParamDesc {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Discrete params (waveform/mode switches) snap instead of smoothing
    /// and render as selectors rather than knobs.
    pub steps: Option<u32>,
}

const fn knob(name: &'static str, min: f32, max: f32, default: f32) -> ParamDesc {
    ParamDesc { name, min, max, default, steps: None }
}

const fn switch(name: &'static str, steps: u32, default: f32) -> ParamDesc {
    ParamDesc { name, min: 0.0, max: (steps - 1) as f32, default, steps: Some(steps) }
}

const fn port(name: &'static str) -> PortDesc {
    PortDesc { name }
}

pub struct ModuleDesc {
    pub kind: ModuleKindId,
    /// Display name.
    pub name: &'static str,
    /// Stable serde key.
    pub type_name: &'static str,
    pub inputs: &'static [PortDesc],
    pub outputs: &'static [PortDesc],
    pub params: &'static [ParamDesc],
}

/// Param indices. Match positions in the desc arrays below.
pub mod params {
    pub mod vco {
        pub const PITCH: u32 = 0;
        pub const WAVE: u32 = 1;
        pub const PW: u32 = 2;
    }
    pub mod vcf {
        pub const CUTOFF: u32 = 0;
        pub const RES: u32 = 1;
        pub const MODE: u32 = 2;
        pub const DRIVE: u32 = 3;
    }
    pub mod vca {
        pub const GAIN: u32 = 0;
        pub const RESPONSE: u32 = 1;
    }
    pub mod adsr {
        pub const ATTACK: u32 = 0;
        pub const DECAY: u32 = 1;
        pub const SUSTAIN: u32 = 2;
        pub const RELEASE: u32 = 3;
    }
    pub mod lfo {
        pub const RATE: u32 = 0;
        pub const WAVE: u32 = 1;
        pub const BIPOLAR: u32 = 2;
    }
    pub mod mixer {
        pub const LEVEL1: u32 = 0;
        pub const LEVEL2: u32 = 1;
        pub const LEVEL3: u32 = 2;
        pub const LEVEL4: u32 = 3;
        pub const MASTER: u32 = 4;
    }
    pub mod output {
        pub const LEVEL: u32 = 0;
    }
    pub mod note_in {
        pub const POLYPHONY: u32 = 0;
    }
    pub mod clock {
        pub const BPM: u32 = 0;
        pub const WIDTH: u32 = 1;
    }
    pub mod seq8 {
        pub const STEPS: u32 = 0;
        /// Pitch knobs occupy params 1..=8.
        pub const PITCH_BASE: u32 = 1;
    }
    pub mod delay {
        pub const TIME: u32 = 0;
        pub const FEEDBACK: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod noise {
        pub const KIND: u32 = 0;
        pub const LEVEL: u32 = 1;
    }
    pub mod attenuverter {
        pub const GAIN: u32 = 0;
        pub const OFFSET: u32 = 1;
    }
    pub mod waveshaper {
        pub const DRIVE: u32 = 0;
        pub const MODE: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod wtvco {
        pub const PITCH: u32 = 0;
        pub const POSITION: u32 = 1;
    }
    pub mod random {
        pub const RATE: u32 = 0;
        pub const SLEW: u32 = 1;
    }
    pub mod quantizer {
        pub const SCALE: u32 = 0;
        pub const ROOT: u32 = 1;
    }
    pub mod slew {
        pub const RISE: u32 = 0;
        pub const FALL: u32 = 1;
    }
    pub mod reverb {
        pub const DECAY: u32 = 0;
        pub const MIX: u32 = 1;
    }
    pub mod chorus {
        pub const RATE: u32 = 0;
        pub const DEPTH: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod phaser {
        pub const RATE: u32 = 0;
        pub const DEPTH: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod fmop {
        pub const PITCH: u32 = 0;
        pub const RATIO: u32 = 1;
        pub const INDEX: u32 = 2;
        pub const FEEDBACK: u32 = 3;
    }
    pub mod additive {
        pub const PITCH: u32 = 0;
        pub const PARTIALS: u32 = 1;
        pub const ROLLOFF: u32 = 2;
        pub const ODD_EVEN: u32 = 3;
    }
    pub mod drum {
        pub const KIND: u32 = 0;
        pub const TUNE: u32 = 1;
        pub const DECAY: u32 = 2;
    }
    pub mod bitcrush {
        pub const BITS: u32 = 0;
        pub const DOWNSAMPLE: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod comb {
        pub const PITCH: u32 = 0;
        pub const DECAY: u32 = 1;
        pub const DAMP: u32 = 2;
    }
    pub mod formant {
        pub const VOWEL: u32 = 0;
        pub const RES: u32 = 1;
    }
    pub mod maths {
        pub const RISE: u32 = 0;
        pub const FALL: u32 = 1;
        pub const CYCLE: u32 = 2;
    }
    pub mod complexlfo {
        pub const RATE: u32 = 0;
    }
    pub mod envfollow {
        pub const ATTACK: u32 = 0;
        pub const RELEASE: u32 = 1;
    }
    pub mod octave {
        pub const OCTAVES: u32 = 0;
        pub const SEMIS: u32 = 1;
    }
    pub mod euclid {
        pub const LENGTH: u32 = 0;
        pub const FILL: u32 = 1;
        pub const ROTATE: u32 = 2;
    }
    pub mod bernoulli {
        pub const PROB: u32 = 0;
    }
    pub mod turing {
        pub const LENGTH: u32 = 0;
        pub const PROB: u32 = 1;
    }
    pub mod seqswitch {
        pub const STEPS: u32 = 0;
    }
    pub mod crossfade {
        pub const MIX: u32 = 0;
    }
    pub mod vcabank {
        pub const LEVEL1: u32 = 0;
        pub const LEVEL2: u32 = 1;
        pub const LEVEL3: u32 = 2;
        pub const LEVEL4: u32 = 3;
    }
    pub mod pan {
        pub const PAN: u32 = 0;
    }
    pub mod compressor {
        pub const THRESHOLD: u32 = 0;
        pub const RATIO: u32 = 1;
        pub const ATTACK: u32 = 2;
        pub const RELEASE: u32 = 3;
        pub const MAKEUP: u32 = 4;
    }
    pub mod macro_osc {
        pub const MODEL: u32 = 0;
        pub const PITCH: u32 = 1;
        pub const HARMONICS: u32 = 2;
        pub const TIMBRE: u32 = 3;
        pub const MORPH: u32 = 4;
    }
    pub mod lpg {
        pub const FREQ: u32 = 0;
        pub const DECAY: u32 = 1;
        pub const RESPONSE: u32 = 2;
    }
    pub mod ladder {
        pub const CUTOFF: u32 = 0;
        pub const RES: u32 = 1;
        pub const DRIVE: u32 = 2;
    }
    pub mod saturator {
        pub const DRIVE: u32 = 0;
        pub const TONE: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod flanger {
        pub const RATE: u32 = 0;
        pub const DEPTH: u32 = 1;
        pub const FEEDBACK: u32 = 2;
        pub const MIX: u32 = 3;
    }
    pub mod pingpong {
        pub const TIME: u32 = 0;
        pub const FEEDBACK: u32 = 1;
        pub const MIX: u32 = 2;
    }
    pub mod param_eq {
        pub const LOW: u32 = 0;
        pub const MID_FREQ: u32 = 1;
        pub const MID: u32 = 2;
        pub const HIGH: u32 = 3;
    }
    pub mod limiter {
        pub const THRESHOLD: u32 = 0;
        pub const RELEASE: u32 = 1;
    }
    pub mod stereo {
        pub const WIDTH: u32 = 0;
    }
    pub mod comparator {
        pub const THRESHOLD: u32 = 0;
        pub const HYSTERESIS: u32 = 1;
    }
    pub mod arp {
        pub const MODE: u32 = 0;
        pub const OCTAVES: u32 = 1;
        pub const GATE_LEN: u32 = 2;
    }
    pub mod gate_delay {
        pub const DELAY: u32 = 0;
    }
    pub mod burst {
        pub const COUNT: u32 = 0;
        pub const RATE: u32 = 1;
    }
    pub mod trig_tool {
        pub const LENGTH: u32 = 0;
    }
    pub mod midi_cc {
        pub const CC: u32 = 0;
        /// Set by the app from incoming MIDI CC; not a user knob.
        pub const VALUE: u32 = 1;
    }
}

static DESCS: [ModuleDesc; 92] = [
    ModuleDesc {
        kind: ModuleKindId::Vco,
        name: "VCO",
        type_name: "vco",
        inputs: &[port("v/oct"), port("pwm")],
        outputs: &[port("out")],
        params: &[knob("pitch", -3.0, 3.0, 0.75), switch("wave", 4, 2.0), knob("pw", 0.05, 0.95, 0.5)],
    },
    ModuleDesc {
        kind: ModuleKindId::Vcf,
        name: "VCF",
        type_name: "vcf",
        inputs: &[port("in"), port("cutoff cv")],
        outputs: &[port("out")],
        params: &[
            // Cutoff stored as volts relative to C4 so CV adds (V/oct).
            knob("cutoff", -4.0, 6.0, 3.0),
            knob("res", 0.5, 10.0, 0.7071),
            switch("mode", 3, 0.0),
            knob("drive", 0.2, 4.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Vca,
        name: "VCA",
        type_name: "vca",
        inputs: &[port("in"), port("cv")],
        outputs: &[port("out")],
        params: &[knob("gain", 0.0, 1.0, 1.0), switch("response", 2, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Adsr,
        name: "ADSR",
        type_name: "adsr",
        inputs: &[port("gate"), port("retrig")],
        outputs: &[port("env")],
        params: &[
            knob("attack", 0.001, 4.0, 0.01),
            knob("decay", 0.001, 4.0, 0.2),
            knob("sustain", 0.0, 1.0, 0.7),
            knob("release", 0.001, 8.0, 0.3),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Lfo,
        name: "LFO",
        type_name: "lfo",
        inputs: &[port("reset")],
        outputs: &[port("out")],
        params: &[knob("rate", 0.02, 20.0, 2.0), switch("wave", 4, 0.0), switch("bipolar", 2, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Mixer,
        name: "MIXER",
        type_name: "mixer",
        inputs: &[port("in 1"), port("in 2"), port("in 3"), port("in 4")],
        outputs: &[port("out")],
        params: &[
            knob("level 1", 0.0, 1.0, 0.8),
            knob("level 2", 0.0, 1.0, 0.8),
            knob("level 3", 0.0, 1.0, 0.8),
            knob("level 4", 0.0, 1.0, 0.8),
            knob("master", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Output,
        name: "OUTPUT",
        type_name: "output",
        inputs: &[port("left"), port("right")],
        outputs: &[],
        params: &[knob("level", 0.0, 1.0, 0.8)],
    },
    ModuleDesc {
        kind: ModuleKindId::NoteIn,
        name: "NOTES",
        type_name: "note_in",
        inputs: &[],
        outputs: &[port("v/oct"), port("gate"), port("velocity"), port("retrig")],
        params: &[knob("polyphony", 1.0, 16.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Clock,
        name: "CLOCK",
        type_name: "clock",
        inputs: &[],
        outputs: &[port("out"), port("/2"), port("/4")],
        params: &[knob("bpm", 30.0, 300.0, 120.0), knob("width", 0.05, 0.95, 0.5)],
    },
    ModuleDesc {
        kind: ModuleKindId::Seq8,
        name: "SEQ-8",
        type_name: "seq8",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("v/oct"), port("gate")],
        params: &[
            knob("steps", 1.0, 8.0, 8.0),
            knob("1", -2.0, 2.0, 0.0),
            knob("2", -2.0, 2.0, 0.0),
            knob("3", -2.0, 2.0, 0.0),
            knob("4", -2.0, 2.0, 0.0),
            knob("5", -2.0, 2.0, 0.0),
            knob("6", -2.0, 2.0, 0.0),
            knob("7", -2.0, 2.0, 0.0),
            knob("8", -2.0, 2.0, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Delay,
        name: "DELAY",
        type_name: "delay",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("time", 0.02, 2.0, 0.4),
            knob("feedback", 0.0, 0.95, 0.4),
            knob("mix", 0.0, 1.0, 0.4),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Noise,
        name: "NOISE",
        type_name: "noise",
        inputs: &[],
        outputs: &[port("out")],
        params: &[switch("kind", 2, 0.0), knob("level", 0.0, 1.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::SampleHold,
        name: "S&H",
        type_name: "snh",
        inputs: &[port("in"), port("trig")],
        outputs: &[port("out")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Attenuverter,
        name: "ATTN",
        type_name: "attn",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("gain", -2.0, 2.0, 1.0), knob("offset", -10.0, 10.0, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Waveshaper,
        name: "SHAPE",
        type_name: "shape",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("drive", 1.0, 20.0, 2.0),
            switch("mode", 2, 0.0),
            knob("mix", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::WtVco,
        name: "WT-OSC",
        type_name: "wtvco",
        inputs: &[port("v/oct"), port("pos cv")],
        outputs: &[port("out")],
        // position scans sine→triangle→saw→square.
        params: &[knob("pitch", -3.0, 3.0, 0.75), knob("position", 0.0, 1.0, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Random,
        name: "RANDOM",
        type_name: "random",
        inputs: &[port("trig")],
        outputs: &[port("stepped"), port("smooth")],
        // rate drives an internal clock when nothing is patched to trig.
        params: &[knob("rate", 0.1, 30.0, 4.0), knob("slew", 0.0, 1.0, 0.2)],
    },
    ModuleDesc {
        kind: ModuleKindId::Quantizer,
        name: "QUANT",
        type_name: "quantizer",
        inputs: &[port("in")],
        outputs: &[port("out")],
        // scale: chromatic / major / minor / pentatonic.
        params: &[switch("scale", 4, 1.0), switch("root", 12, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::ClockDiv,
        name: "CLK÷",
        type_name: "clockdiv",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("÷2"), port("÷4"), port("÷8"), port("÷16")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Mult,
        name: "MULT",
        type_name: "mult",
        inputs: &[port("in")],
        outputs: &[port("1"), port("2"), port("3"), port("4")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Logic,
        name: "LOGIC",
        type_name: "logic",
        inputs: &[port("a"), port("b")],
        outputs: &[port("and"), port("or"), port("xor")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Slew,
        name: "SLEW",
        type_name: "slew",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("rise", 0.0, 2.0, 0.1), knob("fall", 0.0, 2.0, 0.1)],
    },
    ModuleDesc {
        kind: ModuleKindId::Reverb,
        name: "REVERB",
        type_name: "reverb",
        inputs: &[port("in")],
        outputs: &[port("left"), port("right")],
        params: &[knob("decay", 0.0, 0.97, 0.75), knob("mix", 0.0, 1.0, 0.3)],
    },
    ModuleDesc {
        kind: ModuleKindId::Chorus,
        name: "CHORUS",
        type_name: "chorus",
        inputs: &[port("in")],
        outputs: &[port("left"), port("right")],
        params: &[
            knob("rate", 0.05, 5.0, 0.8),
            knob("depth", 0.0, 1.0, 0.5),
            knob("mix", 0.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Phaser,
        name: "PHASER",
        type_name: "phaser",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("rate", 0.05, 5.0, 0.5),
            knob("depth", 0.0, 1.0, 0.7),
            knob("mix", 0.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::FmOp,
        name: "FM-OSC",
        type_name: "fmop",
        inputs: &[port("v/oct"), port("fm")],
        outputs: &[port("out")],
        params: &[
            knob("pitch", -3.0, 3.0, 0.75),
            knob("ratio", 0.5, 12.0, 2.0),
            knob("index", 0.0, 10.0, 2.0),
            knob("feedback", 0.0, 1.0, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Additive,
        name: "ADDITIVE",
        type_name: "additive",
        inputs: &[port("v/oct")],
        outputs: &[port("out")],
        params: &[
            knob("pitch", -3.0, 3.0, 0.75),
            knob("partials", 1.0, 16.0, 8.0),
            knob("rolloff", 0.2, 3.0, 1.0),
            knob("odd/even", 0.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Drum,
        name: "DRUM",
        type_name: "drum",
        inputs: &[port("trig"), port("accent")],
        outputs: &[port("out")],
        // kind: kick / snare / hi-hat.
        params: &[switch("kind", 3, 0.0), knob("tune", -2.0, 2.0, 0.0), knob("decay", 0.02, 2.0, 0.3)],
    },
    ModuleDesc {
        kind: ModuleKindId::RingMod,
        name: "RINGMOD",
        type_name: "ringmod",
        inputs: &[port("a"), port("b")],
        outputs: &[port("out")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::BitCrush,
        name: "CRUSH",
        type_name: "bitcrush",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("bits", 1.0, 16.0, 8.0), knob("downsample", 1.0, 64.0, 1.0), knob("mix", 0.0, 1.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Comb,
        name: "KARPLUS",
        type_name: "comb",
        inputs: &[port("in"), port("v/oct")],
        outputs: &[port("out")],
        params: &[knob("pitch", -3.0, 3.0, 0.0), knob("decay", 0.8, 0.999, 0.98), knob("damp", 0.0, 1.0, 0.3)],
    },
    ModuleDesc {
        kind: ModuleKindId::Formant,
        name: "FORMANT",
        type_name: "formant",
        inputs: &[port("in"), port("vowel cv")],
        outputs: &[port("out")],
        // vowel: A E I O U.
        params: &[switch("vowel", 5, 0.0), knob("res", 2.0, 20.0, 8.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Maths,
        name: "MATHS",
        type_name: "maths",
        inputs: &[port("trig")],
        outputs: &[port("out"), port("eoc")],
        params: &[knob("rise", 0.001, 4.0, 0.2), knob("fall", 0.001, 4.0, 0.4), switch("cycle", 2, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::ComplexLfo,
        name: "CLX-LFO",
        type_name: "complexlfo",
        inputs: &[port("rate cv")],
        outputs: &[port("0°"), port("90°"), port("180°"), port("270°")],
        params: &[knob("rate", 0.02, 20.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::EnvFollow,
        name: "ENV-FOL",
        type_name: "envfollow",
        inputs: &[port("in")],
        outputs: &[port("env")],
        params: &[knob("attack", 0.001, 0.5, 0.01), knob("release", 0.001, 2.0, 0.1)],
    },
    ModuleDesc {
        kind: ModuleKindId::CvMix,
        name: "CV-MIX",
        type_name: "cvmix",
        inputs: &[port("a"), port("b"), port("c"), port("d")],
        outputs: &[port("sum")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Octave,
        name: "OCTAVE",
        type_name: "octave",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[switch("octaves", 9, 4.0), switch("semis", 25, 12.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Euclid,
        name: "EUCLID",
        type_name: "euclid",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("gate")],
        params: &[switch("length", 16, 15.0), switch("fill", 17, 4.0), switch("rotate", 16, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Bernoulli,
        name: "COIN",
        type_name: "bernoulli",
        inputs: &[port("trig"), port("prob cv")],
        outputs: &[port("a"), port("b")],
        params: &[knob("prob", 0.0, 1.0, 0.5)],
    },
    ModuleDesc {
        kind: ModuleKindId::Turing,
        name: "TURING",
        type_name: "turing",
        inputs: &[port("clock")],
        outputs: &[port("gate"), port("cv")],
        params: &[switch("length", 16, 8.0), knob("prob", 0.0, 1.0, 0.5)],
    },
    ModuleDesc {
        kind: ModuleKindId::SeqSwitch,
        name: "SWITCH",
        type_name: "seqswitch",
        inputs: &[port("clock"), port("reset"), port("1"), port("2"), port("3"), port("4")],
        outputs: &[port("out")],
        params: &[switch("steps", 4, 3.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Crossfade,
        name: "XFADE",
        type_name: "crossfade",
        inputs: &[port("a"), port("b"), port("cv")],
        outputs: &[port("out")],
        params: &[knob("mix", 0.0, 1.0, 0.5)],
    },
    ModuleDesc {
        kind: ModuleKindId::VcaBank,
        name: "VCA×4",
        type_name: "vcabank",
        inputs: &[
            port("in 1"), port("in 2"), port("in 3"), port("in 4"),
            port("cv 1"), port("cv 2"), port("cv 3"), port("cv 4"),
        ],
        outputs: &[port("1"), port("2"), port("3"), port("4")],
        params: &[
            knob("lvl 1", 0.0, 1.0, 1.0),
            knob("lvl 2", 0.0, 1.0, 1.0),
            knob("lvl 3", 0.0, 1.0, 1.0),
            knob("lvl 4", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Pan,
        name: "PAN",
        type_name: "pan",
        inputs: &[port("in"), port("pan cv")],
        outputs: &[port("left"), port("right")],
        params: &[knob("pan", -1.0, 1.0, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Compressor,
        name: "COMP",
        type_name: "compressor",
        inputs: &[port("in"), port("sidechain")],
        outputs: &[port("out")],
        params: &[
            knob("threshold", -40.0, 0.0, -18.0),
            knob("ratio", 1.0, 20.0, 4.0),
            knob("attack", 0.001, 0.2, 0.01),
            knob("release", 0.01, 1.0, 0.15),
            knob("makeup", 0.0, 24.0, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::MacroOsc,
        name: "MACRO",
        type_name: "macro_osc",
        inputs: &[port("v/oct"), port("timbre cv"), port("morph cv")],
        outputs: &[port("main"), port("aux")],
        // model: VA / fold / FM / additive / chord / particle.
        // harmonics/timbre/morph re-purpose per model, Plaits-style.
        params: &[
            switch("model", 6, 0.0),
            knob("pitch", -3.0, 3.0, 0.75),
            knob("harmonics", 0.0, 1.0, 0.5),
            knob("timbre", 0.0, 1.0, 0.5),
            knob("morph", 0.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Lpg,
        name: "LPG",
        type_name: "lpg",
        inputs: &[port("in"), port("trig"), port("cv")],
        outputs: &[port("out")],
        // response: lowpass / VCA / both.
        params: &[knob("freq", -2.0, 6.0, 2.0), knob("decay", 0.005, 2.0, 0.2), switch("response", 3, 2.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Ladder,
        name: "LADDER",
        type_name: "ladder",
        inputs: &[port("in"), port("cutoff cv")],
        outputs: &[port("out")],
        params: &[knob("cutoff", -2.0, 6.0, 3.0), knob("res", 0.0, 1.0, 0.3), knob("drive", 1.0, 8.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Saturator,
        name: "SATURATE",
        type_name: "saturate",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("drive", 1.0, 20.0, 2.0), knob("tone", 0.0, 1.0, 0.5), knob("mix", 0.0, 1.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Flanger,
        name: "FLANGER",
        type_name: "flanger",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("rate", 0.05, 5.0, 0.3),
            knob("depth", 0.0, 1.0, 0.7),
            knob("feedback", -0.95, 0.95, 0.5),
            knob("mix", 0.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::PingPong,
        name: "PINGPONG",
        type_name: "pingpong",
        inputs: &[port("left"), port("right")],
        outputs: &[port("left"), port("right")],
        params: &[knob("time", 0.02, 1.5, 0.3), knob("feedback", 0.0, 0.95, 0.5), knob("mix", 0.0, 1.0, 0.4)],
    },
    ModuleDesc {
        kind: ModuleKindId::ParamEq,
        name: "EQ",
        type_name: "eq",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("low dB", -15.0, 15.0, 0.0),
            knob("mid Hz", 200.0, 5000.0, 1000.0),
            knob("mid dB", -15.0, 15.0, 0.0),
            knob("high dB", -15.0, 15.0, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Limiter,
        name: "LIMITER",
        type_name: "limiter",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("threshold", -24.0, 0.0, -3.0), knob("release", 0.01, 0.5, 0.05)],
    },
    ModuleDesc {
        kind: ModuleKindId::Stereo,
        name: "STEREO",
        type_name: "stereo",
        inputs: &[port("left"), port("right")],
        outputs: &[port("left"), port("right")],
        params: &[knob("width", 0.0, 2.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Comparator,
        name: "COMPARE",
        type_name: "comparator",
        inputs: &[port("in"), port("thresh cv")],
        outputs: &[port("gate"), port("inv")],
        params: &[knob("threshold", -10.0, 10.0, 0.0), knob("hysteresis", 0.0, 2.0, 0.1)],
    },
    ModuleDesc {
        kind: ModuleKindId::Rectify,
        name: "RECTIFY",
        type_name: "rectify",
        inputs: &[port("a"), port("b")],
        outputs: &[port("|a|"), port("max"), port("min")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Arp,
        name: "ARP",
        type_name: "arp",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("v/oct"), port("gate")],
        // mode: up / down / up-down / random.
        params: &[switch("mode", 4, 0.0), switch("octaves", 4, 0.0), knob("gate len", 0.05, 0.95, 0.5)],
    },
    ModuleDesc {
        kind: ModuleKindId::ClockMult,
        name: "CLK×",
        type_name: "clockmult",
        inputs: &[port("clock")],
        outputs: &[port("×2"), port("×3"), port("×4")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::GateDelay,
        name: "GATE-DLY",
        type_name: "gatedelay",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("delay", 0.001, 2.0, 0.1)],
    },
    ModuleDesc {
        kind: ModuleKindId::Burst,
        name: "BURST",
        type_name: "burst",
        inputs: &[port("trig")],
        outputs: &[port("out")],
        params: &[switch("count", 16, 3.0), knob("rate", 1.0, 50.0, 10.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::TrigTool,
        name: "TRIG",
        type_name: "trigtool",
        inputs: &[port("in")],
        outputs: &[port("gate"), port("trig")],
        params: &[knob("length", 0.001, 1.0, 0.01)],
    },
    ModuleDesc {
        kind: ModuleKindId::Voltmeter,
        name: "VOLTS",
        type_name: "voltmeter",
        inputs: &[port("in")],
        outputs: &[port("thru")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Tuner,
        name: "TUNER",
        type_name: "tuner",
        inputs: &[port("in")],
        outputs: &[port("v/oct")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::MidiCc,
        name: "MIDI-CC",
        type_name: "midicc",
        inputs: &[],
        outputs: &[port("cv")],
        params: &[switch("cc", 128, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::SubOsc,
        name: "SUB OSC",
        type_name: "subosc",
        inputs: &[port("v/oct")],
        outputs: &[port("out")],
        params: &[
            knob("pitch", -4.0, 4.0, 0.0),
            knob("fund", 0.0, 1.0, 0.3),
            knob("-1 oct", 0.0, 1.0, 0.7),
            knob("-2 oct", 0.0, 1.0, 0.4),
            switch("wave", 2, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Supersaw,
        name: "SUPERSAW",
        type_name: "supersaw",
        inputs: &[port("v/oct")],
        outputs: &[port("out")],
        params: &[
            knob("pitch", -4.0, 4.0, 0.0),
            knob("detune", 0.0, 1.0, 0.2),
            knob("mix", 0.0, 1.0, 0.7),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Pluck,
        name: "PLUCK",
        type_name: "pluck",
        inputs: &[port("v/oct"), port("trig")],
        outputs: &[port("out")],
        params: &[
            knob("pitch", -4.0, 4.0, 0.0),
            knob("decay", 0.0, 1.0, 0.6),
            knob("tone", 0.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Wavefold,
        name: "WAVEFOLD",
        type_name: "wavefold",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("fold", 1.0, 8.0, 1.0),
            knob("symmetry", -1.0, 1.0, 0.0),
            knob("mix", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::ChordOsc,
        name: "CHORD",
        type_name: "chordosc",
        inputs: &[port("v/oct")],
        outputs: &[port("out")],
        // chord: maj / min / dom7 / maj7 / min7 / sus4.
        params: &[
            knob("pitch", -4.0, 4.0, 0.0),
            switch("chord", 6, 0.0),
            switch("wave", 2, 0.0),
            knob("detune", 0.0, 0.2, 0.02),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Resonator,
        name: "RESONATOR",
        type_name: "resonator",
        inputs: &[port("in"), port("v/oct")],
        outputs: &[port("out")],
        params: &[
            knob("pitch", -4.0, 4.0, 0.0),
            knob("structure", 0.0, 1.0, 0.5),
            knob("bright", 0.0, 1.0, 0.5),
            knob("decay", 0.0, 1.0, 0.7),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::SvfMulti,
        name: "SVF",
        type_name: "svf",
        inputs: &[port("in"), port("cutoff cv")],
        outputs: &[port("lp"), port("bp"), port("hp"), port("notch")],
        params: &[
            knob("cutoff", 20.0, 16000.0, 1000.0),
            knob("res", 0.5, 20.0, 0.7),
            knob("cv amt", -1.0, 1.0, 0.5),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::AutoWah,
        name: "AUTO-WAH",
        type_name: "autowah",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("sens", 0.0, 1.0, 0.5),
            knob("range", 0.0, 1.0, 0.6),
            knob("res", 0.5, 15.0, 4.0),
            knob("base", 20.0, 2000.0, 300.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::DualFilter,
        name: "DUAL FILTER",
        type_name: "dualfilter",
        inputs: &[port("left"), port("right")],
        outputs: &[port("left"), port("right")],
        // mode: LP / BP / HP.
        params: &[
            knob("cutoff", 20.0, 16000.0, 1000.0),
            knob("res", 0.5, 15.0, 1.0),
            switch("mode", 3, 0.0),
            knob("spread", -1.0, 1.0, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Drive,
        name: "DRIVE",
        type_name: "drive",
        inputs: &[port("in")],
        outputs: &[port("out")],
        // type: tube / diode / fuzz / fold.
        params: &[
            switch("type", 4, 0.0),
            knob("drive", 1.0, 30.0, 4.0),
            knob("tone", 0.0, 1.0, 0.5),
            knob("mix", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Transient,
        name: "TRANSIENT",
        type_name: "transient",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("attack", -1.0, 1.0, 0.0), knob("sustain", -1.0, 1.0, 0.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Ducker,
        name: "DUCKER",
        type_name: "ducker",
        inputs: &[port("in"), port("key")],
        outputs: &[port("out")],
        params: &[
            knob("amount", 0.0, 1.0, 0.8),
            knob("attack", 0.001, 0.1, 0.01),
            knob("release", 0.02, 1.0, 0.2),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Gate,
        name: "GATE",
        type_name: "gate",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("thresh dB", -60.0, 0.0, -40.0),
            knob("attack", 0.0005, 0.05, 0.002),
            knob("release", 0.01, 0.5, 0.1),
            knob("hold", 0.0, 0.5, 0.05),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Tremolo,
        name: "TREMOLO",
        type_name: "tremolo",
        inputs: &[port("in")],
        outputs: &[port("out")],
        // shape: sine / square.
        params: &[
            knob("rate", 0.05, 20.0, 4.0),
            knob("depth", 0.0, 1.0, 0.7),
            switch("shape", 2, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Vibrato,
        name: "VIBRATO",
        type_name: "vibrato",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("rate", 0.1, 12.0, 5.0), knob("depth", 0.0, 1.0, 0.3)],
    },
    ModuleDesc {
        kind: ModuleKindId::TapeDelay,
        name: "TAPE DELAY",
        type_name: "tapedelay",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("time", 0.02, 1.5, 0.3),
            knob("feedback", 0.0, 0.95, 0.4),
            knob("tone", 0.0, 1.0, 0.5),
            knob("wow", 0.0, 1.0, 0.2),
            knob("mix", 0.0, 1.0, 0.4),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::PitchShift,
        name: "PITCH SHIFT",
        type_name: "pitchshift",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("semis", -24.0, 24.0, 0.0),
            knob("fine", -100.0, 100.0, 0.0),
            knob("mix", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::FreqShift,
        name: "FREQ SHIFT",
        type_name: "freqshift",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("shift Hz", -1000.0, 1000.0, 0.0), knob("mix", 0.0, 1.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::Shimmer,
        name: "SHIMMER",
        type_name: "shimmer",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[
            knob("size", 0.0, 1.0, 0.7),
            knob("shimmer", 0.0, 1.0, 0.4),
            knob("tone", 0.0, 1.0, 0.5),
            knob("mix", 0.0, 1.0, 0.4),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Vocoder,
        name: "VOCODER",
        type_name: "vocoder",
        inputs: &[port("carrier"), port("mod")],
        outputs: &[port("out")],
        params: &[
            knob("formant", -12.0, 12.0, 0.0),
            knob("res", 1.0, 10.0, 5.0),
            knob("mix", 0.0, 1.0, 1.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Offset,
        name: "OFFSET",
        type_name: "offset",
        inputs: &[port("in")],
        outputs: &[port("out")],
        params: &[knob("offset", -10.0, 10.0, 0.0), knob("scale", -2.0, 2.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::TrackHold,
        name: "TRACK-HOLD",
        type_name: "trackhold",
        inputs: &[port("in"), port("gate")],
        outputs: &[port("out")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Phasor,
        name: "PHASOR",
        type_name: "phasor",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("ramp"), port("pulse")],
        params: &[knob("ratio", 0.25, 4.0, 1.0)],
    },
    ModuleDesc {
        kind: ModuleKindId::MinMax,
        name: "MIN/MAX",
        type_name: "minmax",
        inputs: &[port("a"), port("b"), port("c")],
        outputs: &[port("min"), port("max"), port("mean")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Beats,
        name: "BEATS",
        type_name: "beats",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("t1"), port("t2"), port("t3"), port("t4")],
        // 16-step on/off pattern per track, packed as a 16-bit mask. Edited via
        // the clickable grid on the panel, not as knobs.
        params: &[
            knob("track 1", 0.0, 65535.0, 4369.0),
            knob("track 2", 0.0, 65535.0, 4112.0),
            knob("track 3", 0.0, 65535.0, 21845.0),
            knob("track 4", 0.0, 65535.0, 0.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Ratchet,
        name: "RATCHET",
        type_name: "ratchet",
        inputs: &[port("clock"), port("reset")],
        outputs: &[port("gate")],
        params: &[
            switch("step 1", 8, 0.0),
            switch("step 2", 8, 1.0),
            switch("step 3", 8, 0.0),
            switch("step 4", 8, 3.0),
        ],
    },
    ModuleDesc {
        kind: ModuleKindId::Scope,
        name: "SCOPE",
        type_name: "scope",
        inputs: &[port("in")],
        outputs: &[port("thru")],
        params: &[],
    },
    ModuleDesc {
        kind: ModuleKindId::Spectrum,
        name: "SPECTRUM",
        type_name: "spectrum",
        inputs: &[port("in")],
        outputs: &[port("thru")],
        params: &[],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrips() {
        for kind in ModuleKindId::ALL {
            assert_eq!(ModuleKindId::from_u16(kind as u16), Some(kind));
            assert_eq!(ModuleKindId::from_type_name(kind.type_name()), Some(kind));
            assert_eq!(kind.desc().kind, kind);
        }
    }

    #[test]
    fn port_counts_fit_plan_limits() {
        for kind in ModuleKindId::ALL {
            let d = kind.desc();
            assert!(d.inputs.len() <= crate::plan::MAX_PORTS_IN, "{}", d.name);
            assert!(d.outputs.len() <= crate::plan::MAX_PORTS_OUT, "{}", d.name);
        }
    }
}
