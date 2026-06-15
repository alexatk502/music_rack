//! UI → engine control messages. Fixed-size 32-byte POD records so a batch
//! is one flat `ArrayBuffer` through `postMessage` (transferable, one small
//! allocation per UI frame) and a later move to SharedArrayBuffer rings is a
//! transport swap only.

use bytemuck::{Pod, Zeroable};

pub const MSG_SIZE: usize = core::mem::size_of::<Msg>();

/// Message discriminant. Stored as u32 in [`Msg::tag`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum MsgTag {
    /// `a` = module slot, `b` = param index, `value` = new value.
    SetParam = 1,
    /// `a` = note number, `b` = velocity (0-127), `c` = frame offset in quantum.
    NoteOn = 2,
    /// `a` = note number, `c` = frame offset in quantum.
    NoteOff = 3,
    /// Release everything (focus loss, panic button).
    AllNotesOff = 4,
}

impl MsgTag {
    pub fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            1 => Self::SetParam,
            2 => Self::NoteOn,
            3 => Self::NoteOff,
            4 => Self::AllNotesOff,
            _ => return None,
        })
    }
}

/// One 32-byte control record. Field meaning depends on `tag` (see
/// [`MsgTag`]); unused fields are zero.
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Msg {
    pub tag: u32,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub value: f32,
    pub _pad: [u32; 3],
}

impl Msg {
    pub fn set_param(slot: u32, param: u32, value: f32) -> Self {
        Self { tag: MsgTag::SetParam as u32, a: slot, b: param, value, ..Default::default() }
    }

    pub fn note_on(note: u8, velocity: u8, frame_offset: u8) -> Self {
        Self {
            tag: MsgTag::NoteOn as u32,
            a: note as u32,
            b: velocity as u32,
            c: frame_offset as u32,
            ..Default::default()
        }
    }

    pub fn note_off(note: u8, frame_offset: u8) -> Self {
        Self {
            tag: MsgTag::NoteOff as u32,
            a: note as u32,
            c: frame_offset as u32,
            ..Default::default()
        }
    }

    pub fn all_notes_off() -> Self {
        Self { tag: MsgTag::AllNotesOff as u32, ..Default::default() }
    }
}

/// Iterate the records of a received batch. Trailing partial records (which
/// would indicate a sender bug) are ignored rather than panicking — this runs
/// on the audio thread.
pub fn decode_batch(bytes: &[u8]) -> impl Iterator<Item = Msg> + '_ {
    bytes.chunks_exact(MSG_SIZE).map(|chunk| *bytemuck::from_bytes::<Msg>(chunk))
}

/// Serialize a batch of messages to bytes for `postMessage`.
pub fn encode_batch(msgs: &[Msg]) -> Vec<u8> {
    bytemuck::cast_slice(msgs).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msg_is_32_bytes() {
        assert_eq!(MSG_SIZE, 32);
    }

    #[test]
    fn roundtrip() {
        let msgs = vec![
            Msg::set_param(3, 1, 0.75),
            Msg::note_on(60, 100, 17),
            Msg::note_off(60, 99),
            Msg::all_notes_off(),
        ];
        let bytes = encode_batch(&msgs);
        let decoded: Vec<Msg> = decode_batch(&bytes).collect();
        assert_eq!(decoded.len(), 4);
        assert_eq!(decoded[0].tag, MsgTag::SetParam as u32);
        assert_eq!(decoded[0].value, 0.75);
        assert_eq!(decoded[1].a, 60);
        assert_eq!(decoded[1].b, 100);
        assert_eq!(MsgTag::from_u32(decoded[3].tag), Some(MsgTag::AllNotesOff));
    }

    #[test]
    fn partial_trailing_record_is_ignored() {
        let bytes = encode_batch(&[Msg::all_notes_off(), Msg::set_param(0, 0, 1.0)]);
        let truncated = &bytes[..40];
        assert_eq!(decode_batch(truncated).count(), 1);
    }
}
