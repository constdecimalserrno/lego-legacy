//! Zero-crate RFC 6455 WebSocket handshake/framing and XOR+PackBits-RLE delta
//! video codec for `/api/stream` (#310).
//!
//! Strictly pure-`std` and integer-only (ADR-0001): implements RFC 3174 SHA-1
//! (for `Sec-WebSocket-Accept`), RFC 6455 server/client framing, a 9-byte
//! binary frame envelope (`[1B flags][4B LE flip][2B LE w][2B LE h][payload]`),
//! and single-pass PackBits-RLE compression over BGRA XOR deltas.

use super::http::HttpRequest;
use super::json::base64_encode;
use std::io::{self, Read, Write};

/// RFC 6455 §1.3 WebSocket handshake magic GUID.
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// RFC 6455 opcodes.
pub const OPCODE_TEXT: u8 = 0x1;
pub const OPCODE_BINARY: u8 = 0x2;
pub const OPCODE_CLOSE: u8 = 0x8;
pub const OPCODE_PING: u8 = 0x9;
pub const OPCODE_PONG: u8 = 0xA;

/// Binary video frame header size: `1B flags + 4B LE flip + 2B LE width + 2B LE height`.
pub const FRAME_HEADER_LEN: usize = 9;
/// Binary frame flag: payload is a complete PNG image keyframe.
pub const FRAME_FLAG_PNG_KEYFRAME: u8 = 0;
/// Binary frame flag: payload is a PackBits-RLE encoded BGRA XOR delta.
pub const FRAME_FLAG_XOR_RLE_DELTA: u8 = 1;

/// Maximum client-to-server WebSocket frame payload accepted (64 KiB).
const MAX_CLIENT_FRAME_BYTES: usize = 64 * 1024;

/// Decoded client-to-server WebSocket control/data frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsClientFrame {
    Close(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Data { opcode: u8, payload: Vec<u8> },
}

/// Pure-`std` RFC 3174 / FIPS 180-4 SHA-1 digest (used exclusively for RFC 6455
/// `Sec-WebSocket-Accept` handshake derivation).
#[must_use]
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let pad_zeros = (56usize.wrapping_sub((data.len() + 1) % 64)) % 64;
    let total_len = data.len() + 1 + pad_zeros + 8;
    let mut padded = Vec::with_capacity(total_len);
    padded.extend_from_slice(data);
    padded.push(0x80);
    padded.resize(data.len() + 1 + pad_zeros, 0);
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in padded.as_chunks::<64>().0 {
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

/// Compute the RFC 6455 `Sec-WebSocket-Accept` header value for `client_key`.
#[must_use]
pub fn websocket_accept_key(client_key: &str) -> String {
    let trimmed = client_key.trim();
    let mut input = String::with_capacity(trimmed.len() + WS_GUID.len());
    input.push_str(trimmed);
    input.push_str(WS_GUID);
    base64_encode(&sha1(input.as_bytes()))
}

/// Return `true` when `req` is a valid RFC 6455 WebSocket upgrade request.
#[must_use]
pub fn is_websocket_upgrade(req: &HttpRequest) -> bool {
    let has_upgrade = req
        .headers
        .get("upgrade")
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let has_conn = req.headers.get("connection").is_some_and(|v| {
        v.split(',')
            .any(|tok| tok.trim().eq_ignore_ascii_case("upgrade"))
    });
    let has_key = req
        .headers
        .get("sec-websocket-key")
        .is_some_and(|k| !k.trim().is_empty());
    has_upgrade && has_conn && has_key
}

/// Write an RFC 6455 `101 Switching Protocols` handshake response.
///
/// # Errors
/// Returns any I/O error writing or flushing `stream`.
pub fn write_handshake_response(stream: &mut dyn Write, client_key: &str) -> io::Result<()> {
    let accept = websocket_accept_key(client_key);
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    stream.flush()
}

/// Encode a single unmasked server-to-client RFC 6455 WebSocket frame.
#[must_use]
pub fn encode_ws_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let hdr_len = if len <= 125 {
        2
    } else if len <= 65_535 {
        4
    } else {
        10
    };
    let mut out = Vec::with_capacity(hdr_len + len);
    out.push(0x80 | (opcode & 0x0f));
    if len <= 125 {
        out.push(len as u8);
    } else if len <= 65_535 {
        out.push(126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// Write a single unmasked server-to-client RFC 6455 frame and flush.
///
/// # Errors
/// Returns any I/O error writing or flushing `stream`.
pub fn write_ws_frame(stream: &mut dyn Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut header = [0u8; 10];
    header[0] = 0x80 | (opcode & 0x0f);
    let len = payload.len();
    let hdr_len = if len <= 125 {
        header[1] = len as u8;
        2
    } else if len <= 65_535 {
        header[1] = 126;
        header[2..4].copy_from_slice(&(len as u16).to_be_bytes());
        4
    } else {
        header[1] = 127;
        header[2..10].copy_from_slice(&(len as u64).to_be_bytes());
        10
    };
    stream.write_all(&header[..hdr_len])?;
    if !payload.is_empty() {
        stream.write_all(payload)?;
    }
    stream.flush()
}

fn read_exact_retrying(stream: &mut dyn Read, mut buf: &mut [u8]) -> io::Result<()> {
    let mut retries = 0u32;
    while !buf.is_empty() {
        match stream.read(buf) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "websocket peer closed mid-frame",
                ));
            }
            Ok(n) => {
                buf = &mut buf[n..];
                retries = 0;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                retries += 1;
                if retries > 40 {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Poll for one client-to-server RFC 6455 frame.
///
/// Returns `Ok(None)` when the underlying socket times out (`WouldBlock` /
/// `TimedOut`) before any frame header byte arrives, and `Err(UnexpectedEof)`
/// when the client closes the TCP connection.
///
/// # Errors
/// Returns an I/O error on peer disconnect, protocol violation, or oversized
/// client payload.
pub fn read_client_frame(stream: &mut dyn Read) -> io::Result<Option<WsClientFrame>> {
    let mut first = [0u8; 1];
    match stream.read(&mut first) {
        Ok(0) => {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "websocket peer closed",
            ));
        }
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(None);
        }
        Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(None),
        Err(e) => return Err(e),
    }

    let mut second = [0u8; 1];
    read_exact_retrying(stream, &mut second)?;

    let opcode = first[0] & 0x0f;
    let masked = (second[0] & 0x80) != 0;
    let len7 = second[0] & 0x7f;

    let payload_len: usize = match len7 {
        0..=125 => usize::from(len7),
        126 => {
            let mut ext = [0u8; 2];
            read_exact_retrying(stream, &mut ext)?;
            usize::from(u16::from_be_bytes(ext))
        }
        _ => {
            let mut ext = [0u8; 8];
            read_exact_retrying(stream, &mut ext)?;
            let v = u64::from_be_bytes(ext);
            usize::try_from(v).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "websocket length overflow")
            })?
        }
    };

    if payload_len > MAX_CLIENT_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "client websocket frame exceeds limit",
        ));
    }

    let mut mask_key = [0u8; 4];
    if masked {
        read_exact_retrying(stream, &mut mask_key)?;
    }

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        read_exact_retrying(stream, &mut payload)?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask_key[i & 3];
            }
        }
    }

    let frame = match opcode {
        OPCODE_CLOSE => WsClientFrame::Close(payload),
        OPCODE_PING => WsClientFrame::Ping(payload),
        OPCODE_PONG => WsClientFrame::Pong(payload),
        _ => WsClientFrame::Data { opcode, payload },
    };
    Ok(Some(frame))
}

/// Pack a spectator binary video frame:
/// `[1B flags][4B LE flip][2B LE width][2B LE height][payload]`.
#[must_use]
pub fn encode_binary_frame(
    flags: u8,
    flip: u64,
    width: u32,
    height: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    out.push(flags);
    out.extend_from_slice(&(flip as u32).to_le_bytes());
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Unpack a spectator binary video frame header and payload slice.
#[must_use]
pub fn decode_binary_frame_header(data: &[u8]) -> Option<(u8, u32, u16, u16, &[u8])> {
    if data.len() < FRAME_HEADER_LEN {
        return None;
    }
    let flags = data[0];
    let flip = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    let width = u16::from_le_bytes([data[5], data[6]]);
    let height = u16::from_le_bytes([data[7], data[8]]);
    Some((flags, flip, width, height, &data[FRAME_HEADER_LEN..]))
}

/// Standard PackBits-RLE encoder operating on an arbitrary byte accessor
/// (allows zero-copy XOR-on-the-fly encoding without intermediate allocations).
fn packbits_encode_with<F>(len: usize, byte_at: F) -> Vec<u8>
where
    F: Fn(usize) -> u8,
{
    let mut out = Vec::with_capacity(len / 8 + 16);
    let mut i = 0usize;
    while i < len {
        let first = byte_at(i);
        let mut run = 1usize;
        while run < 128 && i + run < len && byte_at(i + run) == first {
            run += 1;
        }
        if run >= 3 || (run == 2 && i + 2 == len) {
            out.push((257 - run) as u8);
            out.push(first);
            i += run;
            continue;
        }

        // Collect a literal run of up to 128 bytes, breaking early when a
        // repeat run of >= 3 identical bytes starts.
        let lit_start = i;
        let mut lit_len = 0usize;
        while lit_len < 128 && i + lit_len < len {
            let pos = i + lit_len;
            if pos + 2 < len {
                let b0 = byte_at(pos);
                if byte_at(pos + 1) == b0 && byte_at(pos + 2) == b0 {
                    break;
                }
            }
            lit_len += 1;
        }
        // Guarantee forward progress even if a 3-byte run started right at `i`
        // (unreachable because `run >= 3` handled it above, kept defensive).
        if lit_len == 0 {
            lit_len = 1;
        }
        out.push((lit_len - 1) as u8);
        for k in 0..lit_len {
            out.push(byte_at(lit_start + k));
        }
        i += lit_len;
    }
    out
}

/// PackBits-RLE compress `input`.
#[must_use]
pub fn packbits_encode(input: &[u8]) -> Vec<u8> {
    packbits_encode_with(input.len(), |idx| input[idx])
}

/// PackBits-RLE decompress `encoded`, checking that it produces `expected_len` bytes.
///
/// # Errors
/// Returns a descriptive error if the stream is truncated or overflows `expected_len`.
pub fn packbits_decode(encoded: &[u8], expected_len: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected_len);
    let mut i = 0usize;
    while i < encoded.len() {
        let ctrl = encoded[i];
        i += 1;
        if ctrl == 128 {
            continue;
        }
        if ctrl <= 127 {
            let count = usize::from(ctrl) + 1;
            if i + count > encoded.len() {
                return Err("truncated PackBits literal run".to_owned());
            }
            if out.len() + count > expected_len {
                return Err("PackBits literal run exceeds expected length".to_owned());
            }
            out.extend_from_slice(&encoded[i..i + count]);
            i += count;
        } else {
            let count = 257usize - usize::from(ctrl);
            let Some(&val) = encoded.get(i) else {
                return Err("truncated PackBits repeat run".to_owned());
            };
            i += 1;
            if out.len() + count > expected_len {
                return Err("PackBits repeat run exceeds expected length".to_owned());
            }
            out.resize(out.len() + count, val);
        }
    }
    if out.len() != expected_len {
        return Err(format!(
            "PackBits decoded {} bytes, expected {expected_len}",
            out.len()
        ));
    }
    Ok(out)
}

/// Single-pass XOR + PackBits-RLE encoder over equal-length BGRA frame buffers.
///
/// # Errors
/// Returns an error if `prev.len() != curr.len()`.
pub fn encode_xor_rle(prev: &[u8], curr: &[u8]) -> Result<Vec<u8>, String> {
    if prev.len() != curr.len() {
        return Err(format!(
            "frame length mismatch: prev={} curr={}",
            prev.len(),
            curr.len()
        ));
    }
    Ok(packbits_encode_with(curr.len(), |idx| {
        prev[idx] ^ curr[idx]
    }))
}

/// Decode a PackBits-RLE XOR delta onto `prev`, reconstructing `curr` bit-for-bit.
///
/// # Errors
/// Returns an error if the PackBits stream is malformed or length-mismatched.
pub fn decode_xor_rle(prev: &[u8], encoded: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(prev.len());
    let mut i = 0usize;
    while i < encoded.len() {
        let ctrl = encoded[i];
        i += 1;
        if ctrl == 128 {
            continue;
        }
        if ctrl <= 127 {
            let count = usize::from(ctrl) + 1;
            if i + count > encoded.len() {
                return Err("truncated PackBits literal run".to_owned());
            }
            if out.len() + count > prev.len() {
                return Err("PackBits literal run exceeds frame size".to_owned());
            }
            let base = out.len();
            for k in 0..count {
                out.push(prev[base + k] ^ encoded[i + k]);
            }
            i += count;
        } else {
            let count = 257usize - usize::from(ctrl);
            let Some(&val) = encoded.get(i) else {
                return Err("truncated PackBits repeat run".to_owned());
            };
            i += 1;
            if out.len() + count > prev.len() {
                return Err("PackBits repeat run exceeds frame size".to_owned());
            }
            let base = out.len();
            for k in 0..count {
                out.push(prev[base + k] ^ val);
            }
        }
    }
    if out.len() != prev.len() {
        return Err(format!(
            "XOR+RLE decoded {} bytes, expected {}",
            out.len(),
            prev.len()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn hex_digest(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    #[test]
    fn sha1_matches_fips_180_4_and_rfc_6455_test_vectors() {
        assert_eq!(
            hex_digest(&sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hex_digest(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // 56-byte boundary (forces a second 64-byte padding block).
        assert_eq!(
            hex_digest(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // Official RFC 6455 §1.3 Sec-WebSocket-Accept vector.
        assert_eq!(
            websocket_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn websocket_upgrade_detection_and_framing_lengths_round_trip() {
        let mut headers = BTreeMap::new();
        headers.insert("upgrade".to_owned(), "WebSocket".to_owned());
        headers.insert("connection".to_owned(), "keep-alive, Upgrade".to_owned());
        headers.insert(
            "sec-websocket-key".to_owned(),
            "dGhlIHNhbXBsZSBub25jZQ==".to_owned(),
        );
        let req = HttpRequest {
            method: "GET".to_owned(),
            path: "/api/stream".to_owned(),
            query: BTreeMap::new(),
            headers,
            body: Vec::new(),
        };
        assert!(is_websocket_upgrade(&req));

        let mut handshake = Vec::new();
        write_handshake_response(&mut handshake, "dGhlIHNhbXBsZSBub25jZQ==").unwrap();
        let text = String::from_utf8(handshake).unwrap();
        assert!(text.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(text.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));

        // Short (<126), medium (126..=65535), and large (>65535) payloads.
        let small = encode_ws_frame(OPCODE_BINARY, b"hello");
        assert_eq!(&small[..2], &[0x82, 5]);
        assert_eq!(&small[2..], b"hello");

        let med_payload = vec![0xab; 300];
        let med = encode_ws_frame(OPCODE_BINARY, &med_payload);
        assert_eq!(&med[..4], &[0x82, 126, 0x01, 0x2c]);
        assert_eq!(&med[4..], med_payload.as_slice());

        let big_payload = vec![0x5a; 70_000];
        let big = encode_ws_frame(OPCODE_BINARY, &big_payload);
        assert_eq!(big[0], 0x82);
        assert_eq!(big[1], 127);
        assert_eq!(
            u64::from_be_bytes(big[2..10].try_into().unwrap()),
            70_000u64
        );
        assert_eq!(&big[10..], big_payload.as_slice());
    }

    #[test]
    fn client_masked_ping_and_close_frames_decode_cleanly() {
        let mask = [0x37u8, 0xfa, 0x21, 0x3d];
        let raw_msg = b"ping!";
        let mut wire = vec![0x80 | OPCODE_PING, 0x80 | (raw_msg.len() as u8)];
        wire.extend_from_slice(&mask);
        for (i, &b) in raw_msg.iter().enumerate() {
            wire.push(b ^ mask[i & 3]);
        }
        // Append a masked empty Close frame.
        wire.extend_from_slice(&[0x80 | OPCODE_CLOSE, 0x80, 1, 2, 3, 4]);

        let mut cursor = io::Cursor::new(wire);
        assert_eq!(
            read_client_frame(&mut cursor).unwrap(),
            Some(WsClientFrame::Ping(raw_msg.to_vec()))
        );
        assert_eq!(
            read_client_frame(&mut cursor).unwrap(),
            Some(WsClientFrame::Close(Vec::new()))
        );
        assert!(read_client_frame(&mut cursor).is_err());
    }

    #[test]
    fn binary_frame_header_and_xor_packbits_rle_are_bit_exact() {
        // 1. Header round-trip.
        let env = encode_binary_frame(FRAME_FLAG_XOR_RLE_DELTA, 42_001, 640, 480, b"xyz");
        assert_eq!(env.len(), FRAME_HEADER_LEN + 3);
        let (flags, flip, w, h, payload) = decode_binary_frame_header(&env).unwrap();
        assert_eq!(
            (flags, flip, w, h, payload),
            (1, 42_001, 640, 480, &b"xyz"[..])
        );

        // 2. Identical 640x480 BGRA frame compresses dramatically (>60x smaller).
        let mut prev = vec![0u8; 640 * 480 * 4];
        for (i, b) in prev.iter_mut().enumerate() {
            *b = ((i * 31) ^ (i >> 5)) as u8;
        }
        let same_delta = encode_xor_rle(&prev, &prev).unwrap();
        assert!(
            same_delta.len() < 24_000,
            "expected tight RLE on zero XOR, got {} bytes",
            same_delta.len()
        );
        assert_eq!(decode_xor_rle(&prev, &same_delta).unwrap(), prev);

        // 3. Partial sprite/cursor mutation + boundary run lengths (1, 2, 3, 127, 128, 129, 260).
        let mut curr = prev.clone();
        for y in 100..132 {
            for x in 200..232 {
                let idx = (y * 640 + x) * 4;
                curr[idx] = (x & 0xff) as u8;
                curr[idx + 1] = (y & 0xff) as u8;
                curr[idx + 2] = ((x + y) & 0xff) as u8;
                curr[idx + 3] = 0xff;
            }
        }
        let delta = encode_xor_rle(&prev, &curr).unwrap();
        assert!(
            delta.len() < 32_000,
            "small sprite delta should stay compact, got {} bytes",
            delta.len()
        );
        let decoded = decode_xor_rle(&prev, &delta).unwrap();
        assert_eq!(decoded, curr);

        // 4. Incompressible random-ish noise + edge run lengths.
        let mut noise_a = Vec::new();
        let mut noise_b = Vec::new();
        let mut state = 0x1234_5678u32;
        for _ in 0..1024 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            noise_a.push((state >> 24) as u8);
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            noise_b.push((state >> 24) as u8);
        }
        let noise_delta = encode_xor_rle(&noise_a, &noise_b).unwrap();
        assert_eq!(decode_xor_rle(&noise_a, &noise_delta).unwrap(), noise_b);

        // 5. PackBits edge sizes (0, 1, 2, 3, 128, 129, 256).
        for n in [0usize, 1, 2, 3, 4, 127, 128, 129, 255, 256, 300] {
            let ones = vec![0x77u8; n];
            assert_eq!(packbits_decode(&packbits_encode(&ones), n).unwrap(), ones);
            let ramp: Vec<u8> = (0..n).map(|k| (k % 251) as u8).collect();
            assert_eq!(packbits_decode(&packbits_encode(&ramp), n).unwrap(), ramp);
        }
    }
}
