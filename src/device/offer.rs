//! CoreDevice offer, matching the working pymobiledevice3 video settings.
//! Protocol field layout references pymobiledevice3 (GPL-3.0) and idevice (MIT).
use anyhow::Result;
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;

fn varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        out.push(byte | if value != 0 { 128 } else { 0 });
        if value == 0 {
            break;
        }
    }
}
fn uint(out: &mut Vec<u8>, field: u64, value: u64) {
    varint(out, field << 3);
    varint(out, value);
}
fn bytes(out: &mut Vec<u8>, field: u64, value: &[u8]) {
    varint(out, field << 3 | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn codec(payload: u64, pairs: usize, flags: u64) -> Vec<u8> {
    let mut out = Vec::new();
    uint(&mut out, 1, payload);
    for index in 0..pairs {
        let mut resolution = Vec::new();
        for (field, value) in [(1, 1), (2, 1 + (index % 2) as u64), (3, 50115), (4, 0)] {
            uint(&mut resolution, field, value);
        }
        bytes(&mut out, 2, &resolution);
    }
    bytes(&mut out, 3, b"FLS;SW:1;");
    uint(&mut out, 4, flags);
    out
}
pub(super) fn media_blob(ssrc: u32) -> Vec<u8> {
    let mut video = Vec::new();
    uint(&mut video, 1, ssrc as u64);
    uint(&mut video, 2, 0);
    bytes(&mut video, 3, &codec(123, 4, 1));
    bytes(&mut video, 3, &codec(100, 2, 14));
    for (field, value) in [(7, 0), (8, 63), (10, 1), (12, 1)] {
        uint(&mut video, field, value);
    }
    let mut out = Vec::new();
    uint(&mut out, 1, 1);
    uint(&mut out, 2, 1);
    bytes(&mut out, 5, &video);
    bytes(&mut out, 6, b"Viceroy 1.7.0");
    uint(&mut out, 8, 0);
    for (kind, rate, cap) in [
        (4074, 0, Some(16384)),
        (0, 75_000_000, Some(524288)),
        (0, 40_000_000, Some(12288)),
        (16, 4100, None),
        (0, 20_000_000, Some(98304)),
        (4, 6500, None),
        (0, 6_000_000, Some(131072)),
        (0, 100_000_000, Some(1048576)),
        (0, 60_000_000, Some(262144)),
        (1, 299, None),
    ] {
        let mut tier = Vec::new();
        uint(&mut tier, 1, kind);
        uint(&mut tier, 2, rate);
        if let Some(cap) = cap {
            uint(&mut tier, 3, cap);
        }
        bytes(&mut out, 9, &tier);
    }
    for (field, value) in [(13, 17137042128614416384), (14, 2), (16, 0), (18, 1)] {
        uint(&mut out, field, value);
    }
    out
}
pub fn build(ssrc: u32) -> Result<Vec<u8>> {
    let mut compression = ZlibEncoder::new(Vec::new(), Compression::best());
    compression.write_all(&media_blob(ssrc))?;
    let mut endpoint = Vec::new();
    uint(&mut endpoint, 1, 0);
    uint(&mut endpoint, 2, 1);
    for (field, value) in [(3, "Mac16,11"), (4, "2205.3.1"), (5, "25F80")] {
        bytes(&mut endpoint, field, value.as_bytes());
    }
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "avcMediaStreamOptionRemoteEndpointInfo".into(),
        plist::Value::Data(endpoint),
    );
    dict.insert("avcMediaStreamNegotiatorMode".into(), 5_i64.into());
    dict.insert(
        "avcMediaStreamNegotiatorMediaBlob".into(),
        plist::Value::Data(compression.finish()?),
    );
    dict.insert(
        "avcMediaStreamOptionCallID".into(),
        uuid::Uuid::new_v4().to_string().to_uppercase().into(),
    );
    let mut out = Vec::new();
    plist::to_writer_binary(&mut out, &dict)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn offer_matches_working_python_wire_format() {
        // Generated once from pymobiledevice3 11.13.1 defaults. Includes
        // LTRP=false, FEC=true, and the negotiated FLS codec feature strings.
        assert_eq!(
            super::media_blob(u32::MAX),
            include_bytes!("fixtures/video-offer.bin")
        );
    }
}
