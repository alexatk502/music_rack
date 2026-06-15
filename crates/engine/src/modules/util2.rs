//! Trigger/gate and CV utilities, plus the visual readout modules and
//! MIDI-CC source.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_core::modules::params::{
    burst as bp, comparator as cp, gate_delay as gp, midi_cc as mp, trig_tool as tp,
};
use rack_dsp::volts::{GATE_HIGH, GATE_THRESHOLD};
use rack_dsp::Smoothed;

// ---------------------------------------------------------------------------
// Comparator (Schmitt trigger with hysteresis)
// ---------------------------------------------------------------------------

pub struct Comparator {
    high: [bool; MAX_CHANNELS],
    threshold: Smoothed,
    hysteresis: Smoothed,
}

impl Comparator {
    pub fn new() -> Self {
        Self { high: [false; MAX_CHANNELS], threshold: Smoothed::new(0.0), hysteresis: Smoothed::new(0.1) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            cp::THRESHOLD => self.threshold.set_target(value.clamp(-10.0, 10.0)),
            cp::HYSTERESIS => self.hysteresis.set_target(value.clamp(0.0, 2.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        thresh_cv: Option<&PortBuffer>,
        gate: &mut PortBuffer,
        inv: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, thresh_cv]);
        gate.channels = channels;
        inv.channels = channels;
        let thresh = self.threshold.current();
        let hyst = self.hysteresis.current();
        for _ in 0..frames {
            self.threshold.tick(ctx.smooth_k);
            self.hysteresis.tick(ctx.smooth_k);
        }
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let cv = thresh_cv.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let x = in_data.map_or(0.0, |d| d[i]);
                let t = thresh + cv.map_or(0.0, |c| c[i]);
                // Hysteresis: rise above t+h to go high, fall below t-h to go low.
                if self.high[ch] {
                    if x < t - hyst {
                        self.high[ch] = false;
                    }
                } else if x > t + hyst {
                    self.high[ch] = true;
                }
                gate.data[ch][i] = if self.high[ch] { GATE_HIGH } else { 0.0 };
                inv.data[ch][i] = if self.high[ch] { 0.0 } else { GATE_HIGH };
            }
        }
    }
}

impl Default for Comparator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Rectify / min-max
// ---------------------------------------------------------------------------

pub struct Rectify;

impl Rectify {
    pub fn new() -> Self {
        Self
    }
    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    /// outputs: [|a|, max(a,b), min(a,b)]
    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        a: Option<&PortBuffer>,
        b: Option<&PortBuffer>,
        abs_out: &mut PortBuffer,
        max_out: &mut PortBuffer,
        min_out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[a, b]);
        abs_out.channels = channels;
        max_out.channels = channels;
        min_out.channels = channels;
        for ch in 0..channels as usize {
            let ad = a.map(|x| x.channel_or_broadcast(ch));
            let bd = b.map(|x| x.channel_or_broadcast(ch));
            for i in 0..frames {
                let av = ad.map_or(0.0, |d| d[i]);
                let bv = bd.map_or(0.0, |d| d[i]);
                abs_out.data[ch][i] = av.abs();
                max_out.data[ch][i] = av.max(bv);
                min_out.data[ch][i] = av.min(bv);
            }
        }
    }
}

impl Default for Rectify {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Gate delay (delays the whole gate/CV signal by a fixed time) — mono
// ---------------------------------------------------------------------------

const GATE_BUF: usize = 96_004; // ~2 s at 48 kHz

pub struct GateDelay {
    buf: Box<[f32; GATE_BUF]>,
    pos: usize,
    delay: Smoothed,
}

impl GateDelay {
    pub fn new() -> Self {
        Self { buf: Box::new([0.0; GATE_BUF]), pos: 0, delay: Smoothed::new(0.1) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == gp::DELAY {
            self.delay.set_target(value.clamp(0.001, 2.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let in_data = input.map(|b| b.channel_or_broadcast(0));
        let time_k = Smoothed::coeff(0.05, ctx.sample_rate);
        for i in 0..frames {
            let delay = (self.delay.tick(time_k) * ctx.sample_rate).clamp(1.0, (GATE_BUF - 1) as f32);
            self.buf[self.pos] = in_data.map_or(0.0, |d| d[i]);
            let read = self.pos as f32 - delay;
            let read = if read < 0.0 { read + GATE_BUF as f32 } else { read };
            out.data[0][i] = self.buf[read as usize % GATE_BUF];
            self.pos = (self.pos + 1) % GATE_BUF;
        }
    }
}

impl Default for GateDelay {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Burst / ratchet generator — mono
// ---------------------------------------------------------------------------

pub struct Burst {
    count: u32,
    rate: Smoothed,
    remaining: u32,
    timer: f32,
    trig_high: bool,
}

impl Burst {
    pub fn new() -> Self {
        Self { count: 4, rate: Smoothed::new(10.0), remaining: 0, timer: 0.0, trig_high: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            bp::COUNT => self.count = (value as u32 + 1).clamp(1, 16),
            bp::RATE => self.rate.set_target(value.clamp(1.0, 50.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        trig: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let trig_data = trig.map(|b| b.channel_or_broadcast(0));
        for i in 0..frames {
            let rate = self.rate.tick(ctx.smooth_k);
            let period = ctx.sample_rate / rate;
            if let Some(t) = trig_data {
                let high = t[i] >= GATE_THRESHOLD;
                if high && !self.trig_high {
                    self.remaining = self.count;
                    self.timer = 0.0;
                }
                self.trig_high = high;
            }
            // Emit a 1 ms pulse at the start of each sub-interval.
            let mut level = 0.0;
            if self.remaining > 0 {
                if self.timer < 0.001 * ctx.sample_rate {
                    level = GATE_HIGH;
                }
                self.timer += 1.0;
                if self.timer >= period {
                    self.timer -= period;
                    self.remaining -= 1;
                }
            }
            out.data[0][i] = level;
        }
    }
}

impl Default for Burst {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Trigger tool: stretch a trigger to a gate, and emit a clean edge trigger
// ---------------------------------------------------------------------------

pub struct TrigTool {
    length: Smoothed,
    gate_timer: f32,
    trig_timer: f32,
    high: bool,
}

impl TrigTool {
    pub fn new() -> Self {
        Self { length: Smoothed::new(0.01), gate_timer: 0.0, trig_timer: 0.0, high: false }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == tp::LENGTH {
            self.length.set_target(value.clamp(0.001, 1.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        gate: &mut PortBuffer,
        trig: &mut PortBuffer,
        frames: usize,
    ) {
        gate.channels = 1;
        trig.channels = 1;
        let in_data = input.map(|b| b.channel_or_broadcast(0));
        for i in 0..frames {
            let len = self.length.tick(ctx.smooth_k) * ctx.sample_rate;
            let high = in_data.map_or(false, |d| d[i] >= GATE_THRESHOLD);
            if high && !self.high {
                self.gate_timer = len;
                self.trig_timer = 0.001 * ctx.sample_rate; // 1 ms edge pulse
            }
            self.high = high;
            gate.data[0][i] = if self.gate_timer > 0.0 { GATE_HIGH } else { 0.0 };
            trig.data[0][i] = if self.trig_timer > 0.0 { GATE_HIGH } else { 0.0 };
            self.gate_timer = (self.gate_timer - 1.0).max(0.0);
            self.trig_timer = (self.trig_timer - 1.0).max(0.0);
        }
    }
}

impl Default for TrigTool {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Clock multiplier (measures input period, emits ×2/×3/×4)
// ---------------------------------------------------------------------------

pub struct ClockMult {
    period: f32,
    since_edge: f32,
    clock_high: bool,
    timers: [f32; 3],
    pulse: [f32; 3],
}

const MULTS: [f32; 3] = [2.0, 3.0, 4.0];

impl ClockMult {
    pub fn new() -> Self {
        Self {
            period: 24_000.0, // 0.5 s default until measured
            since_edge: 0.0,
            clock_high: false,
            timers: [0.0; 3],
            pulse: [0.0; 3],
        }
    }

    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        outs: [&mut PortBuffer; 3],
        frames: usize,
    ) {
        let [o2, o3, o4] = outs;
        o2.channels = 1;
        o3.channels = 1;
        o4.channels = 1;
        let clock_data = clock.map(|b| b.channel_or_broadcast(0));
        let pulse_len = 0.001 * ctx.sample_rate;

        for i in 0..frames {
            self.since_edge += 1.0;
            if let Some(c) = clock_data {
                let high = c[i] >= GATE_THRESHOLD;
                if high && !self.clock_high {
                    // Measure the period and re-align all sub-clocks.
                    if self.since_edge > 1.0 {
                        self.period = self.since_edge;
                    }
                    self.since_edge = 0.0;
                    for j in 0..3 {
                        self.timers[j] = 0.0;
                        self.pulse[j] = pulse_len;
                    }
                }
                self.clock_high = high;
            }
            // Each output fires every period/mult samples.
            for j in 0..3 {
                let sub = (self.period / MULTS[j]).max(2.0);
                self.timers[j] += 1.0;
                if self.timers[j] >= sub {
                    self.timers[j] -= sub;
                    self.pulse[j] = pulse_len;
                }
                self.pulse[j] = (self.pulse[j] - 1.0).max(0.0);
            }
            o2.data[0][i] = if self.pulse[0] > 0.0 { GATE_HIGH } else { 0.0 };
            o3.data[0][i] = if self.pulse[1] > 0.0 { GATE_HIGH } else { 0.0 };
            o4.data[0][i] = if self.pulse[2] > 0.0 { GATE_HIGH } else { 0.0 };
        }
    }
}

impl Default for ClockMult {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Voltmeter (pass-through; the UI reads its meter and shows the value)
// ---------------------------------------------------------------------------

pub struct Voltmeter;

impl Voltmeter {
    pub fn new() -> Self {
        Self
    }
    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                out.data[ch][i] = in_data.map_or(0.0, |d| d[i]);
            }
        }
    }
}

impl Default for Voltmeter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Probe: a transparent pass-through used by the SCOPE and SPECTRUM display
// modules. The UI reads the standard per-module waveform tap (the signal on
// its `thru` output) and draws it large / as a spectrum.
// ---------------------------------------------------------------------------

pub struct Probe;

impl Probe {
    pub fn new() -> Self {
        Self
    }
    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input]);
        out.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                out.data[ch][i] = in_data.map_or(0.0, |d| d[i]);
            }
        }
    }
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tuner (autocorrelation pitch detection → v/oct out; UI shows the note)
// ---------------------------------------------------------------------------

const TUNE_WIN: usize = 2048;

pub struct Tuner {
    buf: Box<[f32; TUNE_WIN]>,
    pos: usize,
    detected_voct: f32,
}

impl Tuner {
    pub fn new() -> Self {
        Self { buf: Box::new([0.0; TUNE_WIN]), pos: 0, detected_voct: 0.0 }
    }

    pub fn set_param(&mut self, _p: u32, _v: f32) {}

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        out.channels = 1;
        let in_data = input.map(|b| b.channel_or_broadcast(0));
        for i in 0..frames {
            self.buf[self.pos] = in_data.map_or(0.0, |d| d[i]);
            self.pos += 1;
            if self.pos >= TUNE_WIN {
                self.pos = 0;
                self.detect(ctx.sample_rate);
            }
            out.data[0][i] = self.detected_voct;
        }
    }

    /// Normalized autocorrelation: find the lag of the first strong peak.
    fn detect(&mut self, sr: f32) {
        let energy: f32 = self.buf.iter().map(|&s| s * s).sum();
        if energy < 1e-3 {
            return; // too quiet to estimate
        }
        let min_lag = (sr / 2000.0) as usize; // up to ~2 kHz
        let max_lag = (sr / 50.0) as usize; // down to ~50 Hz
        let max_lag = max_lag.min(TUNE_WIN - 1);
        let mut best_lag = 0usize;
        let mut best = 0.0f32;
        for lag in min_lag..=max_lag {
            let mut sum = 0.0f32;
            for i in 0..(TUNE_WIN - lag) {
                sum += self.buf[i] * self.buf[i + lag];
            }
            let norm = sum / (TUNE_WIN - lag) as f32;
            if norm > best {
                best = norm;
                best_lag = lag;
            }
        }
        if best_lag > 0 && best > energy / TUNE_WIN as f32 * 0.3 {
            let hz = sr / best_lag as f32;
            // v/oct relative to C4.
            self.detected_voct = (hz / rack_dsp::volts::C4_HZ).log2();
        }
    }
}

impl Default for Tuner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// MIDI-CC source (value pushed by the app via the VALUE param)
// ---------------------------------------------------------------------------

pub struct MidiCc {
    value: Smoothed,
    cc: u8,
}

impl MidiCc {
    pub fn new() -> Self {
        Self { value: Smoothed::new(0.0), cc: 1 }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            mp::CC => self.cc = (value as u8).min(127),
            mp::VALUE => self.value.set_target(value.clamp(0.0, GATE_HIGH)),
            _ => {}
        }
    }

    /// Which CC number this module listens to (the app reads this to route).
    pub fn cc(&self) -> u8 {
        self.cc
    }

    pub fn process(&mut self, ctx: &ProcessCtx, out: &mut PortBuffer, frames: usize) {
        out.channels = 1;
        for i in 0..frames {
            out.data[0][i] = self.value.tick(ctx.smooth_k);
        }
    }
}

impl Default for MidiCc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BLOCK;
    use rack_dsp::volts::AUDIO_PEAK;

    #[test]
    fn comparator_has_hysteresis() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut c = Comparator::new();
        c.threshold.set_immediate(0.0);
        c.hysteresis.set_immediate(1.0);
        let mut gate = PortBuffer::silent();
        let mut inv = PortBuffer::silent();
        let run = |c: &mut Comparator, v: f32, gate: &mut PortBuffer, inv: &mut PortBuffer| {
            let mut input = PortBuffer::silent();
            input.data[0] = [v; BLOCK];
            c.process(&ctx, Some(&input), None, gate, inv, BLOCK);
            gate.data[0][BLOCK - 1] > 1.0
        };
        // Below +hyst stays low; above +1 goes high; between stays high.
        assert!(!run(&mut c, 0.5, &mut gate, &mut inv));
        assert!(run(&mut c, 1.5, &mut gate, &mut inv));
        assert!(run(&mut c, 0.5, &mut gate, &mut inv)); // hysteresis holds
        assert!(!run(&mut c, -1.5, &mut gate, &mut inv));
        // inv is the complement.
        assert!(inv.data[0][BLOCK - 1] > 1.0);
    }

    #[test]
    fn rectify_outputs() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut r = Rectify::new();
        let mut a = PortBuffer::silent();
        let mut b = PortBuffer::silent();
        a.data[0] = [-3.0; BLOCK];
        b.data[0] = [1.0; BLOCK];
        let (mut abs, mut mx, mut mn) = (PortBuffer::silent(), PortBuffer::silent(), PortBuffer::silent());
        r.process(&ctx, Some(&a), Some(&b), &mut abs, &mut mx, &mut mn, BLOCK);
        assert_eq!(abs.data[0][0], 3.0);
        assert_eq!(mx.data[0][0], 1.0);
        assert_eq!(mn.data[0][0], -3.0);
    }

    #[test]
    fn gate_delay_shifts_in_time() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut gd = GateDelay::new();
        gd.delay.set_immediate(0.1); // 4800 samples
        let mut out = PortBuffer::silent();
        let mut input = PortBuffer::silent();
        input.data[0][0] = 10.0; // pulse in the first sample
        let silent = PortBuffer::silent();
        let mut arrival = None;
        for block in 0..400 {
            let inp = if block == 0 { &input } else { &silent };
            gd.process(&ctx, Some(inp), &mut out, BLOCK);
            if arrival.is_none() {
                if let Some(i) = out.data[0][..BLOCK].iter().position(|&s| s > 1.0) {
                    arrival = Some(block * BLOCK + i);
                }
            }
        }
        let at = arrival.expect("delayed pulse never arrived");
        assert!((at as i64 - 4800).abs() <= 2, "arrived at {at}, expected ~4800");
    }

    #[test]
    fn burst_emits_count_pulses() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut burst = Burst::new();
        burst.set_param(bp::COUNT, 4.0); // count = 5
        burst.rate.set_immediate(50.0);
        let mut out = PortBuffer::silent();
        let mut trig = PortBuffer::silent();
        trig.data[0][0] = 10.0;
        let mut rises = 0u32;
        let mut last = 0.0f32;
        let silent = PortBuffer::silent();
        for block in 0..200 {
            let inp = if block == 0 { &trig } else { &silent };
            burst.process(&ctx, Some(inp), &mut out, BLOCK);
            for &s in &out.data[0][..BLOCK] {
                if last < 1.0 && s >= 1.0 {
                    rises += 1;
                }
                last = s;
            }
        }
        assert_eq!(rises, 5, "expected 5 burst pulses, got {rises}");
    }

    #[test]
    fn trig_tool_stretches_to_gate() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut tt = TrigTool::new();
        tt.length.set_immediate(0.05); // 2400-sample gate
        let mut gate = PortBuffer::silent();
        let mut trig = PortBuffer::silent();
        let mut input = PortBuffer::silent();
        input.data[0][0] = 10.0; // one-sample trigger
        tt.process(&ctx, Some(&input), &mut gate, &mut trig, BLOCK);
        // Gate stays high across the whole first block (32 << 2400 samples).
        assert!(gate.data[0][BLOCK - 1] > 1.0, "gate didn't stretch");
        // Keep running; gate should still be high well before 2400 samples.
        let silent = PortBuffer::silent();
        tt.process(&ctx, Some(&silent), &mut gate, &mut trig, BLOCK);
        assert!(gate.data[0][BLOCK - 1] > 1.0);
    }

    #[test]
    fn clock_mult_doubles() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut cm = ClockMult::new();
        let mut o2 = PortBuffer::silent();
        let mut o3 = PortBuffer::silent();
        let mut o4 = PortBuffer::silent();
        // Feed a steady clock at a known period and count ×2 output edges.
        let period_blocks = 30; // 30*32 = 960 samples between input edges
        let mut in_rises = 0u32;
        let mut x2_rises = 0u32;
        let mut last2 = 0.0f32;
        for block in 0..600 {
            let mut clk = PortBuffer::silent();
            // Rising edge at the start of each period.
            if block % period_blocks == 0 {
                clk.data[0] = [10.0; BLOCK];
                in_rises += 1;
            }
            cm.process(&ctx, Some(&clk), [&mut o2, &mut o3, &mut o4], BLOCK);
            for &s in &o2.data[0][..BLOCK] {
                if last2 < 1.0 && s >= 1.0 {
                    x2_rises += 1;
                }
                last2 = s;
            }
        }
        // ×2 should fire roughly twice per input period (allow warm-up slack).
        assert!(in_rises > 5);
        assert!(x2_rises as f32 > in_rises as f32 * 1.5, "×2 not multiplying: in {in_rises}, x2 {x2_rises}");
    }

    #[test]
    fn tuner_detects_a440() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut tuner = Tuner::new();
        let mut out = PortBuffer::silent();
        let mut input = PortBuffer::silent();
        let mut phase = 0.0f32;
        // Feed 440 Hz; after enough samples the detected v/oct ≈ 0.75 (A4).
        for _ in 0..400 {
            for i in 0..BLOCK {
                input.data[0][i] = (core::f32::consts::TAU * phase).sin() * AUDIO_PEAK;
                phase += 440.0 / 48_000.0;
                if phase >= 1.0 { phase -= 1.0; }
            }
            tuner.process(&ctx, Some(&input), &mut out, BLOCK);
        }
        let voct = out.data[0][BLOCK - 1];
        // A4 = 0.75 V/oct above C4. Allow a semitone of slack.
        assert!((voct - 0.75).abs() < 0.08, "tuner detected {voct} V/oct (want ~0.75)");
    }

    #[test]
    fn midi_cc_outputs_set_value() {
        let ctx = ProcessCtx::new(48_000.0);
        let mut cc = MidiCc::new();
        cc.set_param(mp::CC, 74.0);
        assert_eq!(cc.cc(), 74);
        cc.value.set_immediate(7.0);
        let mut out = PortBuffer::silent();
        cc.process(&ctx, &mut out, BLOCK);
        assert!((out.data[0][BLOCK - 1] - 7.0).abs() < 0.01);
    }
}
