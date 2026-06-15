//! Polyphonic port buffers: channel-major so per-channel inner loops over a
//! contiguous 32-frame run autovectorize, and inactive channels are skipped.

pub use rack_core::caps::{BLOCK, MAX_CHANNELS};

#[derive(Clone, Copy)]
#[repr(C)]
pub struct PortBuffer {
    pub data: [[f32; BLOCK]; MAX_CHANNELS],
    /// Active channel count, 1..=MAX_CHANNELS.
    pub channels: u8,
}

impl PortBuffer {
    pub const fn silent() -> Self {
        Self { data: [[0.0; BLOCK]; MAX_CHANNELS], channels: 1 }
    }

    /// VCV broadcast rule: a mono buffer feeds every poly lane; mismatched
    /// poly counts clamp to the last active channel.
    #[inline]
    pub fn channel_or_broadcast(&self, ch: usize) -> &[f32; BLOCK] {
        let c = self.channels.max(1) as usize;
        let idx = if c == 1 { 0 } else { ch.min(c - 1) };
        &self.data[idx]
    }

    /// Sum of all active channels at `frame`. Mono modules (effects fed a
    /// polyphonic signal) use this so every voice is heard, rather than only
    /// channel 0 — otherwise a new note allocated to another poly channel is
    /// silent until the first voice releases and frees channel 0.
    #[inline]
    pub fn mono(&self, frame: usize) -> f32 {
        let c = self.channels.max(1) as usize;
        let mut sum = 0.0;
        for ch in 0..c {
            sum += self.data[ch][frame];
        }
        sum
    }

    pub fn clear(&mut self) {
        self.data = [[0.0; BLOCK]; MAX_CHANNELS];
        self.channels = 1;
    }
}

impl Default for PortBuffer {
    fn default() -> Self {
        Self::silent()
    }
}

/// Channel count an output should adopt given its connected inputs.
#[inline]
pub fn propagate_channels(inputs: &[Option<&PortBuffer>]) -> u8 {
    inputs
        .iter()
        .flatten()
        .map(|b| b.channels.max(1))
        .max()
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_broadcasts_to_all_lanes() {
        let mut b = PortBuffer::silent();
        b.channels = 1;
        b.data[0][0] = 3.5;
        for ch in 0..MAX_CHANNELS {
            assert_eq!(b.channel_or_broadcast(ch)[0], 3.5);
        }
    }

    #[test]
    fn poly_clamps_to_last_channel() {
        let mut b = PortBuffer::silent();
        b.channels = 4;
        for ch in 0..4 {
            b.data[ch][0] = ch as f32;
        }
        assert_eq!(b.channel_or_broadcast(2)[0], 2.0);
        assert_eq!(b.channel_or_broadcast(9)[0], 3.0); // clamped to ch 3
    }

    #[test]
    fn mono_sums_active_channels() {
        let mut b = PortBuffer::silent();
        b.channels = 3;
        b.data[0][0] = 1.0;
        b.data[1][0] = 2.0;
        b.data[2][0] = 3.0;
        b.data[3][0] = 9.0; // inactive channel, must be ignored
        assert_eq!(b.mono(0), 6.0);
        // A mono buffer sums to just channel 0.
        let mut m = PortBuffer::silent();
        m.data[0][0] = 4.0;
        assert_eq!(m.mono(0), 4.0);
    }

    #[test]
    fn channel_propagation_takes_max() {
        let mut a = PortBuffer::silent();
        a.channels = 4;
        let mut b = PortBuffer::silent();
        b.channels = 8;
        assert_eq!(propagate_channels(&[Some(&a), None, Some(&b)]), 8);
        assert_eq!(propagate_channels(&[None, None]), 1);
        assert_eq!(propagate_channels(&[]), 1);
    }
}
