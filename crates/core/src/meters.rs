//! Engine → UI meter snapshots: per-module peak plus a short waveform tap.
//! The worklet posts one snapshot every ~30 ms; the UI draws scope strips and
//! peak LEDs from the latest one. Layout: `MeterHeader | MeterEntry × n`.

use bytemuck::{Pod, Zeroable};

/// First u32 of a meter blob (distinct from MsgTag values and PLAN_TAG).
pub const METER_TAG: u32 = 200;

/// Scope samples per module. Collected at 1/4 sample rate, so 128 samples
/// span ~10.7 ms at 48 kHz — a few cycles of anything musical.
pub const SCOPE_LEN: usize = 128;
/// Decimation factor for scope collection.
pub const SCOPE_DECIM: usize = 4;

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct MeterHeader {
    pub tag: u32,
    pub n_entries: u32,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct MeterEntry {
    pub slot: u16,
    pub _pad: u16,
    /// Peak |voltage| on the module's first output since the last snapshot.
    pub peak: f32,
    /// Most recent waveform, oldest sample first, in volts.
    pub scope: [f32; SCOPE_LEN],
}

impl Default for MeterEntry {
    fn default() -> Self {
        Self { slot: 0, _pad: 0, peak: 0.0, scope: [0.0; SCOPE_LEN] }
    }
}

pub fn encode_meters(entries: &[MeterEntry]) -> Vec<u8> {
    let header = MeterHeader { tag: METER_TAG, n_entries: entries.len() as u32 };
    let mut out =
        Vec::with_capacity(core::mem::size_of::<MeterHeader>() + core::mem::size_of_val(entries));
    out.extend_from_slice(bytemuck::bytes_of(&header));
    out.extend_from_slice(bytemuck::cast_slice(entries));
    out
}

pub fn decode_meters(bytes: &[u8]) -> Option<&[MeterEntry]> {
    let header_len = core::mem::size_of::<MeterHeader>();
    let header: MeterHeader = *bytemuck::try_from_bytes(bytes.get(..header_len)?).ok()?;
    if header.tag != METER_TAG {
        return None;
    }
    let entries_len = header.n_entries as usize * core::mem::size_of::<MeterEntry>();
    bytemuck::try_cast_slice(bytes.get(header_len..header_len + entries_len)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut e = MeterEntry { slot: 7, peak: 4.2, ..Default::default() };
        e.scope[0] = -1.5;
        e.scope[SCOPE_LEN - 1] = 2.5;
        let bytes = encode_meters(&[e, MeterEntry::default()]);
        let decoded = decode_meters(&bytes).expect("decodes");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].slot, 7);
        assert_eq!(decoded[0].peak, 4.2);
        assert_eq!(decoded[0].scope[SCOPE_LEN - 1], 2.5);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_meters(&[]).is_none());
        let mut bytes = encode_meters(&[MeterEntry::default()]);
        bytes[0] = 9; // wrong tag
        assert!(decode_meters(&bytes).is_none());
        let bytes = encode_meters(&[MeterEntry::default()]);
        assert!(decode_meters(&bytes[..bytes.len() - 1]).is_none());
    }
}
