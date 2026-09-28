//! Bounded RFC 7798 HEVC depacketization, without decoding slice syntax.
//! One reusable Annex-B buffer; packet fragments copy directly into that buffer.
//! The stream negotiates sprop-max-don-diff=0 (no decoding-order-number fields).
use std::fmt;

const START_CODE: &[u8; 4] = b"\0\0\0\x01";
const APPLE_TRAILER: &[u8; 14] = b"\x04\xf0\x0a\xc0\0\0\x03\0\0\x04\xec\x0a\xb0\x03";

/// Reference-compatible compound Receiver Report and SDES keepalive. Jitter
/// and sender-report timing fields are zero because they are not measured here.
pub fn receiver_report(local_ssrc: u32, remote_ssrc: u32, highest_sequence: u32) -> [u8; 44] {
    let mut packet = [0; 44];
    packet[..4].copy_from_slice(&[0x81, 201, 0, 7]);
    packet[4..8].copy_from_slice(&local_ssrc.to_be_bytes());
    packet[8..12].copy_from_slice(&remote_ssrc.to_be_bytes());
    packet[16..20].copy_from_slice(&highest_sequence.to_be_bytes());
    packet[32..36].copy_from_slice(&[0x81, 202, 0, 2]);
    packet[36..40].copy_from_slice(&local_ssrc.to_be_bytes());
    packet[40] = 1;
    packet
}

pub fn picture_loss_indication(local_ssrc: u32, remote_ssrc: u32) -> [u8; 12] {
    let mut packet = [0x81, 206, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0];
    packet[4..8].copy_from_slice(&local_ssrc.to_be_bytes());
    packet[8..12].copy_from_slice(&remote_ssrc.to_be_bytes());
    packet
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtpError {
    Header,
    Payload,
    UnsupportedNal(u8),
    Oversize,
}
impl fmt::Display for RtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header => f.write_str("malformed RTP header"),
            Self::Payload => f.write_str("malformed HEVC RTP payload"),
            Self::UnsupportedNal(n) => write!(f, "unsupported HEVC RTP NAL type {n}"),
            Self::Oversize => f.write_str("HEVC access unit exceeds configured size bound"),
        }
    }
}
impl std::error::Error for RtpError {}

#[derive(Clone, Copy, Debug)]
pub struct RtpPacket<'a> {
    pub payload: &'a [u8],
    pub marker: bool,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

/// RTCP packets on a multiplexed socket return None. Header extensions, CSRCs,
/// and padding are validated before calculating a payload slice.
pub fn parse_rtp(data: &[u8]) -> Result<Option<RtpPacket<'_>>, RtpError> {
    if data.len() < 2 || data[0] >> 6 != 2 {
        return Err(RtpError::Header);
    }
    if (192..=223).contains(&data[1]) {
        return Ok(None);
    }
    if data.len() < 12 {
        return Err(RtpError::Header);
    }
    let mut start = 12 + (data[0] as usize & 15) * 4;
    if start > data.len() {
        return Err(RtpError::Header);
    }
    if data[0] & 0x10 != 0 {
        if start + 4 > data.len() {
            return Err(RtpError::Header);
        }
        let words = u16::from_be_bytes([data[start + 2], data[start + 3]]) as usize;
        start += 4 + words * 4;
        if start > data.len() {
            return Err(RtpError::Header);
        }
    }
    let end = if data[0] & 0x20 != 0 {
        let padding = data[data.len() - 1] as usize;
        if padding == 0 || padding > data.len() - start {
            return Err(RtpError::Header);
        }
        data.len() - padding
    } else {
        data.len()
    };
    Ok(Some(RtpPacket {
        payload: &data[start..end],
        marker: data[1] & 0x80 != 0,
        sequence: u16::from_be_bytes([data[2], data[3]]),
        timestamp: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
        ssrc: u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
    }))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RtpStats {
    pub packets: u64,
    pub lost_packets: u64,
    pub late_packets: u64,
    pub access_units: u64,
    pub dropped_units: u64,
    pub highest_sequence: u32,
}

#[derive(Debug)]
pub struct AccessUnit<'a> {
    pub data: &'a [u8],
    pub timestamp: u32,
    pub keyframe: bool,
    /// At least one preceding access unit was damaged. Request an IDR and
    /// flush decoder references before resuming at a clean random-access unit.
    pub discontinuity: bool,
}

#[derive(Debug)]
pub struct HevcDepacketizer {
    au: Vec<u8>,
    max_bytes: usize,
    clear_next: bool,
    timestamp: Option<u32>,
    ssrc: Option<u32>,
    sequence: Option<u16>,
    fu_header: Option<[u8; 2]>,
    fu_start: usize,
    damaged: bool,
    discontinuity: bool,
    keyframe: bool,
    parameters: [Vec<u8>; 3],
    unit_parameters: u8,
    stats: RtpStats,
}

impl Default for HevcDepacketizer {
    fn default() -> Self {
        Self::new(16 * 1024 * 1024)
    }
}

impl HevcDepacketizer {
    pub fn new(max_unit_bytes: usize) -> Self {
        Self {
            au: Vec::with_capacity(max_unit_bytes.min(256 * 1024)),
            max_bytes: max_unit_bytes,
            clear_next: false,
            timestamp: None,
            ssrc: None,
            sequence: None,
            fu_header: None,
            fu_start: 0,
            damaged: false,
            discontinuity: false,
            keyframe: false,
            parameters: std::array::from_fn(|_| Vec::new()),
            unit_parameters: 0,
            stats: RtpStats::default(),
        }
    }
    pub fn stats(&self) -> RtpStats {
        self.stats
    }

    fn reset_unit(&mut self) {
        self.au.clear();
        self.fu_header = None;
        self.keyframe = false;
        self.damaged = false;
        self.clear_next = false;
        self.unit_parameters = 0;
    }
    fn corrupt(&mut self) {
        self.au.clear();
        self.fu_header = None;
        self.damaged = true;
        self.discontinuity = true;
    }
    fn room(&self, additional: usize) -> Result<(), RtpError> {
        if additional > self.max_bytes.saturating_sub(self.au.len()) {
            Err(RtpError::Oversize)
        } else {
            Ok(())
        }
    }
    fn cache_parameter(&mut self, start: usize) -> Result<(), RtpError> {
        let nal = &self.au[start..];
        let kind = (nal[0] >> 1) & 63;
        if (32..=34).contains(&kind) {
            if nal.len() > 65536 {
                return Err(RtpError::Oversize);
            }
            let index = (kind - 32) as usize;
            if self.parameters[index] != nal {
                self.parameters[index].clear();
                self.parameters[index].extend_from_slice(nal);
            }
            self.unit_parameters |= 1 << index;
        }
        Ok(())
    }
    fn prepend_parameters(&mut self) -> Result<(), RtpError> {
        if self.unit_parameters == 7 {
            return Ok(());
        }
        // Prefix a complete cached bundle in dependency order when any set is
        // absent. Prefixing only SPS/PPS before an in-band VPS would reorder it.
        let additional: usize = self
            .parameters
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| p.len() + 4)
            .sum();
        self.room(additional)?;
        if additional == 0 {
            return Ok(());
        }
        let old_len = self.au.len();
        self.au.resize(old_len + additional, 0);
        self.au.copy_within(0..old_len, additional);
        let mut offset = 0;
        for p in &self.parameters {
            if !p.is_empty() {
                self.au[offset..offset + 4].copy_from_slice(START_CODE);
                offset += 4;
                self.au[offset..offset + p.len()].copy_from_slice(p);
                offset += p.len();
            }
        }
        Ok(())
    }
    fn append_nal(&mut self, nal: &[u8]) -> Result<(), RtpError> {
        if nal.len() < 2 || nal[0] & 0x80 != 0 || nal[1] & 7 == 0 {
            return Err(RtpError::Payload);
        }
        let kind = (nal[0] >> 1) & 63;
        if kind >= 48 {
            return Err(RtpError::UnsupportedNal(kind));
        }
        let nal = nal.strip_suffix(APPLE_TRAILER).unwrap_or(nal);
        self.room(4 + nal.len())?;
        self.keyframe |= (16..=23).contains(&kind);
        self.au.extend_from_slice(START_CODE);
        let start = self.au.len();
        self.au.extend_from_slice(nal);
        self.cache_parameter(start)
    }
    fn payload(&mut self, p: &[u8]) -> Result<(), RtpError> {
        if p.len() < 2 || p[0] & 0x80 != 0 || p[1] & 7 == 0 {
            return Err(RtpError::Payload);
        }
        match (p[0] >> 1) & 63 {
            48 => {
                if self.fu_header.is_some() {
                    return Err(RtpError::Payload);
                }
                let mut position = 2;
                let mut count = 0;
                while position < p.len() {
                    if position + 2 > p.len() {
                        return Err(RtpError::Payload);
                    }
                    let len = u16::from_be_bytes([p[position], p[position + 1]]) as usize;
                    position += 2;
                    if len < 2 || len > p.len() - position {
                        return Err(RtpError::Payload);
                    }
                    self.append_nal(&p[position..position + len])?;
                    position += len;
                    count += 1;
                }
                if count == 0 {
                    return Err(RtpError::Payload);
                }
            }
            49 => {
                if p.len() < 4 {
                    return Err(RtpError::Payload);
                }
                let (start, end, kind) = (p[2] & 0x80 != 0, p[2] & 0x40 != 0, p[2] & 63);
                if start && end || kind >= 48 {
                    return Err(RtpError::Payload);
                }
                let header = [(p[0] & 0x81) | kind << 1, p[1]];
                if start {
                    if self.fu_header.is_some() {
                        return Err(RtpError::Payload);
                    }
                    self.room(6 + p.len() - 3)?;
                    self.fu_start = self.au.len();
                    self.au.extend_from_slice(START_CODE);
                    self.au.extend_from_slice(&header);
                    self.fu_header = Some(header);
                    self.keyframe |= (16..=23).contains(&kind);
                } else if self.fu_header != Some(header) {
                    return Err(RtpError::Payload);
                }
                self.room(p.len() - 3)?;
                self.au.extend_from_slice(&p[3..]);
                if end {
                    if self.au.len() - self.fu_start >= 6 + APPLE_TRAILER.len()
                        && self.au.ends_with(APPLE_TRAILER)
                    {
                        self.au.truncate(self.au.len() - APPLE_TRAILER.len());
                    }
                    self.fu_header = None;
                    self.cache_parameter(self.fu_start + 4)?;
                }
            }
            kind if kind < 48 => {
                if self.fu_header.is_some() {
                    return Err(RtpError::Payload);
                }
                self.append_nal(p)?;
            }
            kind => return Err(RtpError::UnsupportedNal(kind)),
        }
        Ok(())
    }

    /// The returned slice remains valid until the next push. Consume or copy it
    /// into the decoder before receiving another packet. The assembly buffer is
    /// reused and grows only when its existing capacity is insufficient.
    pub fn push(&mut self, data: &[u8]) -> Result<Option<AccessUnit<'_>>, RtpError> {
        if self.clear_next {
            self.reset_unit();
            self.timestamp = None;
        }
        let packet = match parse_rtp(data) {
            Ok(Some(p)) => p,
            Ok(None) => return Ok(None),
            Err(e) => {
                self.corrupt();
                return Err(e);
            }
        };
        self.stats.packets += 1;
        if self.ssrc.is_some_and(|s| s != packet.ssrc) {
            self.reset_unit();
            self.timestamp = None;
            self.sequence = None;
            self.discontinuity = true;
            for parameter in &mut self.parameters {
                parameter.clear();
            }
        }
        self.ssrc = Some(packet.ssrc);
        let mut lost = false;
        if let Some(last) = self.sequence {
            let distance = packet.sequence.wrapping_sub(last);
            if distance == 0 || distance >= 0x8000 {
                self.stats.late_packets += 1;
                return Ok(None);
            }
            if distance != 1 {
                self.stats.lost_packets += u64::from(distance - 1);
                lost = true;
            }
            self.stats.highest_sequence = self
                .stats
                .highest_sequence
                .wrapping_add(u32::from(distance));
        } else {
            self.stats.highest_sequence = u32::from(packet.sequence);
        }
        self.sequence = Some(packet.sequence);
        if self.timestamp.is_some_and(|t| t != packet.timestamp) {
            if !self.au.is_empty() || self.damaged {
                self.stats.dropped_units += 1;
                self.discontinuity = true;
            }
            self.reset_unit();
        }
        self.timestamp = Some(packet.timestamp);
        if lost {
            self.corrupt();
        }
        if !self.damaged
            && let Err(error) = self.payload(packet.payload)
        {
            self.corrupt();
            if packet.marker {
                self.stats.dropped_units += 1;
                self.clear_next = true;
            }
            return Err(error);
        }
        if !packet.marker {
            return Ok(None);
        }
        self.clear_next = true;
        if self.damaged || self.fu_header.is_some() || self.au.is_empty() {
            self.corrupt();
            self.stats.dropped_units += 1;
            return Ok(None);
        }
        if self.keyframe
            && let Err(error) = self.prepend_parameters()
        {
            self.corrupt();
            self.stats.dropped_units += 1;
            return Err(error);
        }
        self.stats.access_units += 1;
        let discontinuity = std::mem::take(&mut self.discontinuity);
        Ok(Some(AccessUnit {
            data: &self.au,
            timestamp: packet.timestamp,
            keyframe: self.keyframe,
            discontinuity,
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    fn packet(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80, 96 | if marker { 128 } else { 0 }];
        p.extend(seq.to_be_bytes());
        p.extend(ts.to_be_bytes());
        p.extend(7u32.to_be_bytes());
        p.extend(payload);
        p
    }
    #[test]
    fn single_nal_and_ap_preserve_bytes() {
        let mut d = HevcDepacketizer::default();
        let au = d.push(&packet(1, 10, true, &[38, 1, 42])).unwrap().unwrap();
        assert_eq!(au.data, &[0, 0, 0, 1, 38, 1, 42]);
        assert!(au.keyframe);
        let au = d
            .push(&packet(
                2,
                11,
                true,
                &[96, 1, 0, 3, 64, 1, 9, 0, 3, 2, 1, 8],
            ))
            .unwrap()
            .unwrap();
        assert_eq!(au.data, &[0, 0, 0, 1, 64, 1, 9, 0, 0, 0, 1, 2, 1, 8]);
        assert!(!au.keyframe);
        assert!(!au.discontinuity);
    }
    #[test]
    fn fragmented_nal_wrap_and_trailer() {
        let mut d = HevcDepacketizer::default();
        assert!(
            d.push(&packet(65535, 10, false, &[98, 1, 0x80 | 19, 42]))
                .unwrap()
                .is_none()
        );
        let mut last = vec![98, 1, 0x40 | 19, 43];
        last.extend(APPLE_TRAILER);
        let au = d.push(&packet(0, 10, true, &last)).unwrap().unwrap();
        assert_eq!(au.data, &[0, 0, 0, 1, 38, 1, 42, 43]);
        assert_eq!(d.stats().highest_sequence, 65536);
        assert_eq!(d.stats().lost_packets, 0);
    }
    #[test]
    fn loss_drops_whole_unit_and_late_packet_does_not_poison_next() {
        let mut d = HevcDepacketizer::default();
        d.push(&packet(1, 10, false, &[98, 1, 0x81, 42])).unwrap();
        assert!(
            d.push(&packet(3, 10, true, &[98, 1, 0x41, 43]))
                .unwrap()
                .is_none()
        );
        assert!(
            d.push(&packet(2, 10, false, &[98, 1, 1, 1]))
                .unwrap()
                .is_none()
        );
        let au = d.push(&packet(4, 11, true, &[38, 1, 9])).unwrap().unwrap();
        assert!(au.discontinuity);
        assert!(au.keyframe);
        assert_eq!(d.stats().lost_packets, 1);
        assert_eq!(d.stats().late_packets, 1);
    }
    #[test]
    fn malformed_fragments_and_missing_marker_are_rejected() {
        let mut d = HevcDepacketizer::default();
        assert_eq!(
            d.push(&packet(1, 10, true, &[98, 1, 0x41, 1])).unwrap_err(),
            RtpError::Payload
        );
        assert_eq!(
            d.push(&packet(2, 11, true, &[96, 1, 0, 10, 2, 1]))
                .unwrap_err(),
            RtpError::Payload
        );
        d.push(&packet(3, 12, false, &[2, 1, 7])).unwrap();
        let au = d.push(&packet(4, 13, true, &[38, 1, 8])).unwrap().unwrap();
        assert_eq!(au.data, &[0, 0, 0, 1, 38, 1, 8]);
        assert!(au.discontinuity);
    }
    #[test]
    fn maximum_unit_size_enforced() {
        let mut d = HevcDepacketizer::new(6);
        assert_eq!(
            d.push(&packet(1, 1, true, &[2, 1, 7])).unwrap_err(),
            RtpError::Oversize
        );
        assert!(d.au.capacity() <= 6);
    }
    #[test]
    fn extensions_csrc_padding_and_rtcp() {
        let mut p = packet(1, 5, true, &[]);
        p[0] = 0xb1; // padding, extension, one CSRC
        p.extend([0; 4]);
        p.extend([0xbe, 0xde, 0, 1]);
        p.extend([0; 4]);
        p.extend([38, 1, 8, 0, 0, 3]);
        assert_eq!(parse_rtp(&p).unwrap().unwrap().payload, &[38, 1, 8]);
        assert!(parse_rtp(&[0x80, 200, 0, 0]).unwrap().is_none());
        p[18] = 0xff;
        assert_eq!(parse_rtp(&p).unwrap_err(), RtpError::Header);
    }
    #[test]
    fn arbitrary_short_packets_never_panic() {
        for len in 0..64 {
            for byte in 0..=255 {
                let bytes = vec![byte; len];
                let mut d = HevcDepacketizer::new(1024);
                let _ = d.push(&bytes);
            }
        }
    }
    #[test]
    fn parameter_only_units_are_cached_for_recovery() {
        let mut d = HevcDepacketizer::default();
        d.push(&packet(1, 1, true, &[64, 1, 9])).unwrap();
        d.push(&packet(2, 2, true, &[66, 1, 8])).unwrap();
        d.push(&packet(3, 3, true, &[68, 1, 7])).unwrap();
        assert!(d.push(&packet(5, 4, true, &[2, 1, 6])).unwrap().is_none());
        let au = d.push(&packet(6, 5, true, &[38, 1, 5])).unwrap().unwrap();
        assert!(au.discontinuity);
        assert_eq!(
            au.data,
            &[
                0, 0, 0, 1, 64, 1, 9, 0, 0, 0, 1, 66, 1, 8, 0, 0, 0, 1, 68, 1, 7, 0, 0, 0, 1, 38,
                1, 5
            ]
        );
    }
    #[test]
    fn rtcp_matches_reference_layout() {
        let rr = receiver_report(0x11223344, 0x55667788, 0x00010005);
        assert_eq!(
            &rr[..20],
            &[
                0x81, 201, 0, 7, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0, 0, 0, 0, 0, 1,
                0, 5
            ]
        );
        assert_eq!(
            &rr[32..],
            &[0x81, 202, 0, 2, 0x11, 0x22, 0x33, 0x44, 1, 0, 0, 0]
        );
        assert_eq!(
            picture_loss_indication(1, 2),
            [0x81, 206, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2]
        );
    }
    #[test]
    fn all_fragment_boundaries_reassemble_exactly() {
        for length in 2..128 {
            let body: Vec<u8> = (0..length).map(|n| n as u8).collect();
            for chunk_size in 1..length {
                let mut d = HevcDepacketizer::default();
                let chunks = body.chunks(chunk_size);
                let count = chunks.len();
                for (index, chunk) in chunks.enumerate() {
                    let end = index + 1 == count;
                    let flags = if index == 0 {
                        0x80
                    } else if end {
                        0x40
                    } else {
                        0
                    };
                    let mut payload = vec![98, 1, flags | 19];
                    payload.extend(chunk);
                    let seq = 65500u16.wrapping_add(index as u16);
                    let result = d.push(&packet(seq, 123, end, &payload)).unwrap();
                    if end {
                        let unit = result.unwrap();
                        assert_eq!(&unit.data[..6], &[0, 0, 0, 1, 38, 1]);
                        assert_eq!(&unit.data[6..], &body);
                    } else {
                        assert!(result.is_none());
                    }
                }
            }
        }
    }
}
