//! CV utilities: a DC offset/scale, a track-and-hold, a clock-synced phasor,
//! and a min/max/mean combiner.

use crate::buffer::{propagate_channels, PortBuffer, MAX_CHANNELS};
use crate::ProcessCtx;
use rack_dsp::volts::{GATE_HIGH, GATE_THRESHOLD};
use rack_dsp::Smoothed;

// ---------------------------------------------------------------------------
// DC Offset / scale. With nothing patched it is a constant voltage source.
// params: 0 offset (V), 1 scale
// ---------------------------------------------------------------------------

pub struct Offset {
    offset: Smoothed,
    scale: Smoothed,
}

impl Offset {
    pub fn new() -> Self {
        Self { offset: Smoothed::new(0.0), scale: Smoothed::new(1.0) }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        match param {
            0 => self.offset.set_target(value.clamp(-10.0, 10.0)),
            1 => self.scale.set_target(value.clamp(-2.0, 2.0)),
            _ => {}
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = input.map(|b| b.channels.max(1)).unwrap_or(1);
        out.channels = channels;
        let mut off = [0.0f32; crate::buffer::BLOCK];
        let mut scl = [1.0f32; crate::buffer::BLOCK];
        for i in 0..frames {
            off[i] = self.offset.tick(ctx.smooth_k);
            scl[i] = self.scale.tick(ctx.smooth_k);
        }
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                out.data[ch][i] = in_data.map_or(0.0, |d| d[i]) * scl[i] + off[i];
            }
        }
    }
}

impl Default for Offset {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Track & Hold: passes the input while the gate is high, holds on gate-low.
// ---------------------------------------------------------------------------

pub struct TrackHold {
    held: [f32; MAX_CHANNELS],
}

impl TrackHold {
    pub fn new() -> Self {
        Self { held: [0.0; MAX_CHANNELS] }
    }

    pub fn set_param(&mut self, _param: u32, _value: f32) {}

    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        input: Option<&PortBuffer>,
        gate: Option<&PortBuffer>,
        out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[input, gate]).max(1);
        out.channels = channels;
        for ch in 0..channels as usize {
            let in_data = input.map(|b| b.channel_or_broadcast(ch));
            let g = gate.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                // No gate patched → track continuously.
                let track = g.map_or(true, |gg| gg[i] >= GATE_THRESHOLD);
                if track {
                    self.held[ch] = in_data.map_or(0.0, |d| d[i]);
                }
                out.data[ch][i] = self.held[ch];
            }
        }
    }
}

impl Default for TrackHold {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Clock-synced phasor: measures the clock period and emits a ramp (and an
// end-of-cycle pulse) locked to it, scaled by a ratio.
// params: 0 ratio
// ---------------------------------------------------------------------------

pub struct Phasor {
    phase: [f32; MAX_CHANNELS],
    period: [f32; MAX_CHANNELS],
    since: [f32; MAX_CHANNELS],
    prev_clock: [bool; MAX_CHANNELS],
    prev_reset: [bool; MAX_CHANNELS],
    ratio: Smoothed,
}

impl Phasor {
    pub fn new() -> Self {
        Self {
            phase: [0.0; MAX_CHANNELS],
            period: [24000.0; MAX_CHANNELS],
            since: [0.0; MAX_CHANNELS],
            prev_clock: [false; MAX_CHANNELS],
            prev_reset: [false; MAX_CHANNELS],
            ratio: Smoothed::new(1.0),
        }
    }

    pub fn set_param(&mut self, param: u32, value: f32) {
        if param == 0 {
            self.ratio.set_target(value.clamp(0.25, 4.0));
        }
    }

    pub fn process(
        &mut self,
        ctx: &ProcessCtx,
        clock: Option<&PortBuffer>,
        reset: Option<&PortBuffer>,
        ramp: &mut PortBuffer,
        pulse: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[clock, reset]).max(1);
        ramp.channels = channels;
        pulse.channels = channels;
        for ch in 0..channels as usize {
            let c = clock.map(|b| b.channel_or_broadcast(ch));
            let r = reset.map(|b| b.channel_or_broadcast(ch));
            for i in 0..frames {
                let ratio = self.ratio.tick(ctx.smooth_k);
                self.since[ch] += 1.0;

                let rs = r.map_or(false, |rr| rr[i] >= GATE_THRESHOLD);
                if rs && !self.prev_reset[ch] {
                    self.phase[ch] = 0.0;
                }
                self.prev_reset[ch] = rs;

                let ck = c.map_or(false, |cc| cc[i] >= GATE_THRESHOLD);
                if ck && !self.prev_clock[ch] {
                    // New clock edge: lock the measured period and reset phase.
                    self.period[ch] = self.since[ch].clamp(2.0, ctx.sample_rate);
                    self.since[ch] = 0.0;
                    self.phase[ch] = 0.0;
                }
                self.prev_clock[ch] = ck;

                let inc = ratio / self.period[ch];
                let prev = self.phase[ch];
                let mut ph = prev + inc;
                let wrapped = ph >= 1.0;
                if wrapped {
                    ph -= ph.floor();
                }
                self.phase[ch] = ph;
                ramp.data[ch][i] = ph * GATE_HIGH;
                pulse.data[ch][i] = if wrapped { GATE_HIGH } else { 0.0 };
            }
        }
    }
}

impl Default for Phasor {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Min / Max / Mean of up to three CV inputs (only the connected ones count).
// ---------------------------------------------------------------------------

pub struct MinMax;

impl MinMax {
    pub fn new() -> Self {
        Self
    }

    pub fn set_param(&mut self, _param: u32, _value: f32) {}

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        _ctx: &ProcessCtx,
        a: Option<&PortBuffer>,
        b: Option<&PortBuffer>,
        c: Option<&PortBuffer>,
        min_out: &mut PortBuffer,
        max_out: &mut PortBuffer,
        mean_out: &mut PortBuffer,
        frames: usize,
    ) {
        let channels = propagate_channels(&[a, b, c]).max(1);
        min_out.channels = channels;
        max_out.channels = channels;
        mean_out.channels = channels;
        for ch in 0..channels as usize {
            let ad = a.map(|x| x.channel_or_broadcast(ch));
            let bd = b.map(|x| x.channel_or_broadcast(ch));
            let cd = c.map(|x| x.channel_or_broadcast(ch));
            for i in 0..frames {
                let mut lo = f32::INFINITY;
                let mut hi = f32::NEG_INFINITY;
                let mut sum = 0.0;
                let mut n = 0.0;
                for v in [ad.map(|d| d[i]), bd.map(|d| d[i]), cd.map(|d| d[i])].into_iter().flatten()
                {
                    lo = lo.min(v);
                    hi = hi.max(v);
                    sum += v;
                    n += 1.0;
                }
                if n == 0.0 {
                    lo = 0.0;
                    hi = 0.0;
                }
                min_out.data[ch][i] = lo;
                max_out.data[ch][i] = hi;
                mean_out.data[ch][i] = if n > 0.0 { sum / n } else { 0.0 };
            }
        }
    }
}

impl Default for MinMax {
    fn default() -> Self {
        Self::new()
    }
}
