//! LLUDP packet framing: header, message number, zero-coding, appended acks.
//!
//! Layout: flags (1), sequence number (4, big-endian), extra-header length (1),
//! extra header, message number (1, 2 or 4 bytes by frequency), body, and
//! optionally appended acks (4 bytes big-endian each, then a count byte).

use std::fmt;

pub const FLAG_ZEROCODED: u8 = 0x80;
pub const FLAG_RELIABLE: u8 = 0x40;
pub const FLAG_RESENT: u8 = 0x20;
pub const FLAG_ACKS: u8 = 0x10;

const HEADER_LEN: usize = 6;

/// Message numbers in one namespace: High `n`, Medium `0xFF00 | n`,
/// Low `0xFFFF_0000 | n`, Fixed `0xFFFF_FF00 | n`.
pub type MessageId = u32;

pub const fn high(n: u8) -> MessageId {
    n as u32
}

pub const fn medium(n: u8) -> MessageId {
    0xFF00 | n as u32
}

pub const fn low(n: u16) -> MessageId {
    0xFFFF_0000 | n as u32
}

#[derive(Debug, PartialEq)]
pub struct Packet {
    pub flags: u8,
    pub sequence: u32,
    pub acks: Vec<u32>,
    pub id: MessageId,
    pub body: Vec<u8>,
}

impl Packet {
    pub fn is_reliable(&self) -> bool {
        self.flags & FLAG_RELIABLE != 0
    }
}

#[derive(Debug, PartialEq)]
pub struct PacketError(pub &'static str);

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Replaces each run of zero bytes with `00 <count>`.
pub fn zero_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] != 0 {
            out.push(data[i]);
            i += 1;
            continue;
        }
        let run = data[i..].iter().take(255).take_while(|b| **b == 0).count();
        out.extend([0, run as u8]);
        i += run;
    }
    out
}

pub fn zero_decode(data: &[u8]) -> Result<Vec<u8>, PacketError> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut bytes = data.iter();
    while let Some(&byte) = bytes.next() {
        if byte != 0 {
            out.push(byte);
            continue;
        }
        let count = *bytes.next().ok_or(PacketError("zero-coded data ends inside a run"))?;
        out.resize(out.len() + usize::from(count), 0);
    }
    Ok(out)
}

fn id_bytes(id: MessageId) -> Vec<u8> {
    match id {
        0..=0xFE => vec![id as u8],
        0xFF00..=0xFFFE => vec![0xFF, id as u8],
        _ => id.to_be_bytes().to_vec(),
    }
}

/// Builds a datagram. Zero-coding is dropped when it would not make the
/// packet smaller.
pub fn build(flags: u8, sequence: u32, id: MessageId, body: &[u8], zerocode: bool) -> Vec<u8> {
    let mut payload = id_bytes(id);
    payload.extend_from_slice(body);
    let mut flags = flags & !FLAG_ZEROCODED;
    if zerocode {
        let encoded = zero_encode(&payload);
        if encoded.len() < payload.len() {
            payload = encoded;
            flags |= FLAG_ZEROCODED;
        }
    }
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(flags);
    out.extend(sequence.to_be_bytes());
    out.push(0);
    out.extend(payload);
    out
}

pub fn parse(datagram: &[u8]) -> Result<Packet, PacketError> {
    if datagram.len() < HEADER_LEN + 1 {
        return Err(PacketError("datagram shorter than a packet header"));
    }
    let flags = datagram[0];
    let sequence = u32::from_be_bytes([datagram[1], datagram[2], datagram[3], datagram[4]]);
    let start = HEADER_LEN + usize::from(datagram[5]);
    let mut end = datagram.len();

    let mut acks = Vec::new();
    if flags & FLAG_ACKS != 0 {
        let count = usize::from(datagram[end - 1]);
        let acks_start = (end - 1)
            .checked_sub(count * 4)
            .filter(|s| *s >= start)
            .ok_or(PacketError("appended acks overrun the packet"))?;
        let (chunks, _) = datagram[acks_start..end - 1].as_chunks::<4>();
        acks = chunks.iter().map(|chunk| u32::from_be_bytes(*chunk)).collect();
        end = acks_start;
    }
    if start >= end {
        return Err(PacketError("packet has no message number"));
    }

    let payload = if flags & FLAG_ZEROCODED != 0 {
        zero_decode(&datagram[start..end])?
    } else {
        datagram[start..end].to_vec()
    };
    let (id, id_len) = match payload[..] {
        [0xFF, 0xFF, a, b, ..] => (u32::from_be_bytes([0xFF, 0xFF, a, b]), 4),
        [0xFF, 0xFF, ..] => return Err(PacketError("truncated message number")),
        [0xFF, n, ..] => (0xFF00 | u32::from(n), 2),
        [0xFF] => return Err(PacketError("truncated message number")),
        [n, ..] => (u32::from(n), 1),
        [] => return Err(PacketError("packet has no message number")),
    };
    Ok(Packet { flags, sequence, acks, id, body: payload[id_len..].to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_coding_round_trips() {
        let cases: [&[u8]; 6] = [
            &[],
            &[1, 2, 3],
            &[0],
            &[1, 0, 0, 0, 2, 0],
            &[0, 0, 0, 0],
            &[0xFF, 0xFF, 0, 3, 0, 0, 0, 0, 9],
        ];
        for case in cases {
            assert_eq!(zero_decode(&zero_encode(case)).unwrap(), case);
        }
        assert_eq!(zero_encode(&[1, 0, 0, 0, 2, 0]), [1, 0, 3, 2, 0, 1]);
    }

    #[test]
    fn zero_coding_splits_long_runs() {
        let mut data = vec![7];
        data.extend(vec![0; 600]);
        data.push(8);
        let encoded = zero_encode(&data);
        assert_eq!(encoded, [7, 0, 255, 0, 255, 0, 90, 8]);
        assert_eq!(zero_decode(&encoded).unwrap(), data);
    }

    #[test]
    fn zero_decode_rejects_dangling_run() {
        assert!(zero_decode(&[1, 0]).is_err());
    }

    #[test]
    fn builds_and_parses_each_frequency() {
        for id in [high(4), 0xFF05, low(148), 0xFFFF_FFFB] {
            let datagram = build(FLAG_RELIABLE, 0x0102_0304, id, &[9, 8, 7], false);
            assert_eq!(datagram[..6], [FLAG_RELIABLE, 1, 2, 3, 4, 0]);
            let packet = parse(&datagram).unwrap();
            assert_eq!((packet.id, packet.sequence), (id, 0x0102_0304));
            assert_eq!(packet.body, [9, 8, 7]);
            assert!(packet.is_reliable() && packet.acks.is_empty());
        }
        assert_eq!(build(0, 1, high(4), &[], false)[6..], [4]);
        assert_eq!(build(0, 1, 0xFF05, &[], false)[6..], [0xFF, 5]);
        assert_eq!(build(0, 1, low(148), &[], false)[6..], [0xFF, 0xFF, 0, 148]);
    }

    #[test]
    fn zerocoded_packet_round_trips_and_sets_flag() {
        let body = [1, 0, 0, 0, 0, 0, 0, 2];
        let datagram = build(0, 5, low(149), &body, true);
        assert_eq!(datagram[0], FLAG_ZEROCODED);
        assert!(datagram.len() < 6 + 4 + body.len());
        let packet = parse(&datagram).unwrap();
        assert_eq!((packet.id, packet.body.as_slice()), (low(149), &body[..]));
    }

    #[test]
    fn zerocoding_is_skipped_when_it_does_not_help() {
        let datagram = build(0, 5, high(2), &[1, 0, 2], true);
        assert_eq!(datagram[0], 0);
        assert_eq!(parse(&datagram).unwrap().body, [1, 0, 2]);
    }

    #[test]
    fn appended_acks_are_stripped_before_zero_decoding() {
        // Zero-coded body followed by two raw acks whose bytes contain zeros.
        let mut datagram = build(FLAG_ACKS, 9, high(1), &[5, 0, 0, 0, 0], true);
        assert_eq!(datagram[0], FLAG_ACKS | FLAG_ZEROCODED);
        datagram.extend([0, 0, 0, 17, 0, 0, 1, 0, 2]);
        let packet = parse(&datagram).unwrap();
        assert_eq!(packet.acks, [17, 256]);
        assert_eq!(packet.body, [5, 0, 0, 0, 0]);
    }

    #[test]
    fn extra_header_is_skipped() {
        let mut datagram = vec![0, 0, 0, 0, 1, 2, 0xAA, 0xBB];
        datagram.extend([2, 42]);
        let packet = parse(&datagram).unwrap();
        assert_eq!((packet.id, packet.body.as_slice()), (high(2), &[42u8][..]));
    }

    #[test]
    fn rejects_malformed_datagrams() {
        assert!(parse(&[0, 0, 0, 0, 1, 0]).is_err());
        assert!(parse(&[FLAG_ACKS, 0, 0, 0, 1, 0, 1, 9]).is_err());
        assert!(parse(&[0, 0, 0, 0, 1, 0, 0xFF]).is_err());
        assert!(parse(&[0, 0, 0, 0, 1, 0, 0xFF, 0xFF, 1]).is_err());
        assert!(parse(&[0, 0, 0, 0, 1, 5, 1]).is_err());
    }
}
