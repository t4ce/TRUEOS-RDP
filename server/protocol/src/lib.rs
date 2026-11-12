//! The existing TRUEOS TME1 video transport, plus view-only stream metadata.
#![no_std]

extern crate alloc;

use alloc::vec::Vec;

pub const DEFAULT_PORT: u16 = 43402;
pub const SUBSCRIBE_TOKEN: &[u8; 8] = b"TME1GET1";
pub const HEADER_LEN: usize = 32;
pub const MAX_DATAGRAM_LEN: usize = 1200;
pub const MAX_PAYLOAD_LEN: usize = MAX_DATAGRAM_LEN - HEADER_LEN;
pub const MAX_FRAGMENT_COUNT: u16 = 4096;
pub const MAX_ACCESS_UNIT_BYTES: usize = 4 * 1024 * 1024;
pub const FLAG_START: u8 = 1;
pub const FLAG_END: u8 = 2;
pub const FLAG_KEYFRAME: u8 = 4;
pub const FLAG_SESSION_END: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub session_id: u32,
    pub width: u16,
    pub height: u16,
    pub fps: u16,
}

impl StreamInfo {
    pub fn encode(self) -> [u8; 20] {
        let mut bytes = [0; 20];
        bytes[..8].copy_from_slice(b"TME1INFO");
        bytes[8..12].copy_from_slice(&self.session_id.to_be_bytes());
        bytes[12..14].copy_from_slice(&self.width.to_be_bytes());
        bytes[14..16].copy_from_slice(&self.height.to_be_bytes());
        bytes[16..18].copy_from_slice(&self.fps.to_be_bytes());
        bytes
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() != 20 || &bytes[..8] != b"TME1INFO" || bytes[18..20] != [0, 0] {
            return Err("invalid stream metadata");
        }
        let info = Self {
            session_id: read_u32(bytes, 8),
            width: read_u16(bytes, 12),
            height: read_u16(bytes, 14),
            fps: read_u16(bytes, 16),
        };
        if info.session_id == 0
            || !(64..=1920).contains(&info.width)
            || !info.width.is_multiple_of(2)
            || !(64..=1080).contains(&info.height)
            || !info.height.is_multiple_of(2)
            || !(1..=60).contains(&info.fps)
        {
            return Err("unsupported stream metadata value");
        }
        Ok(info)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Packet<'a> {
    pub flags: u8,
    pub session_id: u32,
    pub datagram_seq: u32,
    pub access_unit_seq: u32,
    pub fragment_index: u16,
    pub fragment_count: u16,
    pub payload: &'a [u8],
}

impl<'a> Packet<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if !(HEADER_LEN..=MAX_DATAGRAM_LEN).contains(&bytes.len()) {
            return Err("invalid datagram size");
        }
        if &bytes[..4] != b"TME1" || bytes[4] != 1 || read_u16(bytes, 6) as usize != HEADER_LEN {
            return Err("invalid transport header");
        }
        let flags = bytes[5];
        let fragment_index = read_u16(bytes, 20);
        let fragment_count = read_u16(bytes, 22);
        if flags & !15 != 0
            || fragment_count == 0
            || fragment_count > MAX_FRAGMENT_COUNT
            || fragment_index >= fragment_count
            || (flags & FLAG_START != 0) != (fragment_index == 0)
            || (flags & FLAG_END != 0) != (fragment_index + 1 == fragment_count)
            || (flags & FLAG_SESSION_END != 0 && fragment_index + 1 != fragment_count)
        {
            return Err("invalid fragment flags or count");
        }
        let payload = &bytes[HEADER_LEN..];
        if payload.is_empty()
            || payload.len() != read_u16(bytes, 24) as usize
            || read_u16(bytes, 26) != 0
            || crc32fast::hash(payload) != read_u32(bytes, 28)
        {
            return Err("invalid payload size, reserved field or CRC");
        }
        Ok(Self {
            flags,
            session_id: read_u32(bytes, 8),
            datagram_seq: read_u32(bytes, 12),
            access_unit_seq: read_u32(bytes, 16),
            fragment_index,
            fragment_count,
            payload,
        })
    }
}

/// Encode one AU using MTU-safe datagrams. Sequence numbers wrap on the wire.
pub fn packetize(
    bytes: &[u8],
    session_id: u32,
    datagram_seq: &mut u32,
    access_unit_seq: u32,
    keyframe: bool,
) -> Result<Vec<Vec<u8>>, &'static str> {
    if bytes.is_empty() || bytes.len() > MAX_ACCESS_UNIT_BYTES {
        return Err("access unit exceeds transport limit");
    }
    let fragment_count = bytes.len().div_ceil(MAX_PAYLOAD_LEN) as u16;
    let mut packets = Vec::with_capacity(fragment_count as usize);
    for (index, payload) in bytes.chunks(MAX_PAYLOAD_LEN).enumerate() {
        let mut datagram = alloc::vec![0; HEADER_LEN + payload.len()];
        datagram[..4].copy_from_slice(b"TME1");
        datagram[4] = 1;
        datagram[5] = if index == 0 { FLAG_START } else { 0 }
            | if index + 1 == fragment_count as usize {
                FLAG_END
            } else {
                0
            }
            | if keyframe { FLAG_KEYFRAME } else { 0 };
        datagram[6..8].copy_from_slice(&(HEADER_LEN as u16).to_be_bytes());
        datagram[8..12].copy_from_slice(&session_id.to_be_bytes());
        datagram[12..16].copy_from_slice(&datagram_seq.to_be_bytes());
        datagram[16..20].copy_from_slice(&access_unit_seq.to_be_bytes());
        datagram[20..22].copy_from_slice(&(index as u16).to_be_bytes());
        datagram[22..24].copy_from_slice(&fragment_count.to_be_bytes());
        datagram[24..26].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        datagram[28..32].copy_from_slice(&crc32fast::hash(payload).to_be_bytes());
        datagram[HEADER_LEN..].copy_from_slice(payload);
        *datagram_seq = datagram_seq.wrapping_add(1);
        packets.push(datagram);
    }
    Ok(packets)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
}
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Find the next Annex-B NAL, returning (start-code offset, payload offset).
pub fn start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut cursor = from;
    while cursor + 3 <= bytes.len() {
        if bytes[cursor..cursor + 3] == [0, 0, 1] {
            return Some((cursor, cursor + 3));
        }
        if cursor + 4 <= bytes.len() && bytes[cursor..cursor + 4] == [0, 0, 0, 1] {
            return Some((cursor, cursor + 4));
        }
        cursor += 1;
    }
    None
}

pub fn contains_nal(bytes: &[u8], kind: u8) -> bool {
    let mut cursor = 0;
    while let Some((_, payload)) = start_code(bytes, cursor) {
        if bytes.get(payload).is_some_and(|byte| byte & 31 == kind) {
            return true;
        }
        cursor = payload;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_access_unit_round_trip_and_crc() {
        let source = alloc::vec![0x65; MAX_PAYLOAD_LEN * 2 + 7];
        let mut seq = u32::MAX;
        let mut packets = packetize(&source, 17, &mut seq, 28, true).unwrap();
        let mut restored = Vec::new();
        for (index, bytes) in packets.iter().enumerate() {
            let packet = Packet::parse(bytes).unwrap();
            assert_eq!(packet.fragment_index as usize, index);
            assert_eq!(packet.fragment_count, 3);
            assert_eq!(packet.access_unit_seq, 28);
            assert_eq!(packet.session_id, 17);
            assert_ne!(packet.flags & FLAG_KEYFRAME, 0);
            restored.extend_from_slice(packet.payload);
        }
        assert_eq!(seq, 2);
        assert_eq!(restored, source);
        packets[0][HEADER_LEN] ^= 1;
        assert!(Packet::parse(&packets[0]).is_err());
    }

    #[test]
    fn rejects_malformed_metadata_and_fragment_counts() {
        let info = StreamInfo {
            session_id: 17,
            width: 1280,
            height: 720,
            fps: 30,
        };
        assert_eq!(StreamInfo::parse(&info.encode()), Ok(info));
        let mut bytes = info.encode();
        bytes[18] = 1;
        assert!(StreamInfo::parse(&bytes).is_err());
        let mut packet = packetize(&[0, 0, 1, 0x65], 17, &mut 0, 0, true)
            .unwrap()
            .remove(0);
        packet[22..24].copy_from_slice(&0u16.to_be_bytes());
        assert!(Packet::parse(&packet).is_err());
        for len in 0..HEADER_LEN {
            assert!(Packet::parse(&packet[..len]).is_err());
        }
    }
}
