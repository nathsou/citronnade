//! zlib streams: writing with stored (uncompressed) blocks, which is all
//! 3dslink needs, and a complete inflater for reading.

use alloc::vec::Vec;
use core::fmt;

/// Adler-32 checksum.
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// Wraps `data` in a zlib stream of stored blocks.
pub fn compress_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = data.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(last as u8);
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InflateError {
    BadHeader,
    Truncated,
    BadBlockType,
    BadStoredLength,
    BadCode,
    BadDistance,
    ChecksumMismatch,
}

impl fmt::Display for InflateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InflateError::BadHeader => "bad zlib header",
            InflateError::Truncated => "truncated deflate data",
            InflateError::BadBlockType => "bad deflate block type",
            InflateError::BadStoredLength => "bad stored block length",
            InflateError::BadCode => "bad Huffman code",
            InflateError::BadDistance => "distance too far back",
            InflateError::ChecksumMismatch => "zlib checksum mismatch",
        })
    }
}

/// Decompresses a zlib stream.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, InflateError> {
    if data.len() < 2 || (data[0] & 0x0F) != 8 || ((data[0] as u16) << 8 | data[1] as u16) % 31 != 0 {
        return Err(InflateError::BadHeader);
    }
    if data[1] & 0x20 != 0 {
        return Err(InflateError::BadHeader); // preset dictionary
    }
    let (out, used) = inflate_raw(&data[2..])?;
    let tail = &data[2 + used..];
    if tail.len() >= 4 {
        let expected = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]);
        if expected != adler32(&out) {
            return Err(InflateError::ChecksumMismatch);
        }
    }
    Ok(out)
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u32,
    nbits: u32,
}

impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<(), InflateError> {
        while self.nbits < n {
            let byte = *self.data.get(self.pos).ok_or(InflateError::Truncated)?;
            self.pos += 1;
            self.bit |= (byte as u32) << self.nbits;
            self.nbits += 8;
        }
        Ok(())
    }

    fn take(&mut self, n: u32) -> Result<u32, InflateError> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = self.bit & ((1u32 << n) - 1);
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }

    fn align(&mut self) {
        self.bit = 0;
        self.nbits = 0;
    }
}

/// A canonical Huffman decoder (counts per length, then symbols by code).
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, InflateError> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offsets = [0u16; 16];
        for i in 1..16 {
            offsets[i] = offsets[i - 1] + counts[i - 1];
        }
        let mut symbols = alloc::vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = sym as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Huffman { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, InflateError> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(InflateError::BadCode)
    }
}

const LENGTH_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Inflates raw deflate data. Returns the output and the bytes consumed.
pub fn inflate_raw(data: &[u8]) -> Result<(Vec<u8>, usize), InflateError> {
    let mut bits = Bits { data, pos: 0, bit: 0, nbits: 0 };
    let mut out = Vec::new();
    loop {
        let last = bits.take(1)?;
        match bits.take(2)? {
            0 => {
                bits.align();
                let p = bits.pos;
                let hdr = data.get(p..p + 4).ok_or(InflateError::Truncated)?;
                let len = u16::from_le_bytes([hdr[0], hdr[1]]);
                let nlen = u16::from_le_bytes([hdr[2], hdr[3]]);
                if len != !nlen {
                    return Err(InflateError::BadStoredLength);
                }
                let body = data.get(p + 4..p + 4 + len as usize).ok_or(InflateError::Truncated)?;
                out.extend_from_slice(body);
                bits.pos = p + 4 + len as usize;
            }
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                let lit = Huffman::new(&lengths)?;
                let dist = Huffman::new(&[5u8; 30])?;
                codes(&mut bits, &mut out, &lit, &dist)?;
            }
            2 => {
                let hlit = bits.take(5)? as usize + 257;
                let hdist = bits.take(5)? as usize + 1;
                let hclen = bits.take(4)? as usize + 4;
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let mut cl = [0u8; 19];
                for &i in ORDER.iter().take(hclen) {
                    cl[i] = bits.take(3)? as u8;
                }
                let clh = Huffman::new(&cl)?;
                let mut lengths = alloc::vec![0u8; hlit + hdist];
                let mut i = 0;
                while i < hlit + hdist {
                    let sym = clh.decode(&mut bits)?;
                    match sym {
                        0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            let prev = *lengths.get(i.wrapping_sub(1)).ok_or(InflateError::BadCode)?;
                            for _ in 0..3 + bits.take(2)? {
                                *lengths.get_mut(i).ok_or(InflateError::BadCode)? = prev;
                                i += 1;
                            }
                        }
                        17 | 18 => {
                            let n = if sym == 17 { 3 + bits.take(3)? } else { 11 + bits.take(7)? };
                            for _ in 0..n {
                                *lengths.get_mut(i).ok_or(InflateError::BadCode)? = 0;
                                i += 1;
                            }
                        }
                        _ => return Err(InflateError::BadCode),
                    }
                }
                let lit = Huffman::new(&lengths[..hlit])?;
                let dist = Huffman::new(&lengths[hlit..])?;
                codes(&mut bits, &mut out, &lit, &dist)?;
            }
            _ => return Err(InflateError::BadBlockType),
        }
        if last == 1 {
            break;
        }
    }
    // Give back whole bytes still buffered in the bit reader.
    let consumed = bits.pos - (bits.nbits / 8) as usize;
    Ok((out, consumed))
}

fn codes(bits: &mut Bits<'_>, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman) -> Result<(), InflateError> {
    loop {
        let sym = lit.decode(bits)? as usize;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Ok(()),
            257..=285 => {
                let i = sym - 257;
                let len = LENGTH_BASE[i] as usize + bits.take(LENGTH_EXTRA[i] as u32)? as usize;
                let d = dist.decode(bits)? as usize;
                if d >= 30 {
                    return Err(InflateError::BadCode);
                }
                let distance = DIST_BASE[d] as usize + bits.take(DIST_EXTRA[d] as u32)? as usize;
                if distance > out.len() {
                    return Err(InflateError::BadDistance);
                }
                let start = out.len() - distance;
                for k in 0..len {
                    let b = out[start + k];
                    out.push(b);
                }
            }
            _ => return Err(InflateError::BadCode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn adler32_known_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn stored_round_trip() {
        for len in [0usize, 1, 100, 65535, 65536, 200_000] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            let z = compress_stored(&data);
            assert_eq!(decompress(&z).unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn inflates_real_zlib_output() {
        // zlib.compress(b"hello hello hello hello, citronnade!", 9) (fixed Huffman, back-references)
        let z = [0x78, 0xda, 0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0x75, 0x14, 0x92, 0x33, 0x4b, 0x8a, 0xf2, 0xf3, 0xf2, 0x12, 0x53, 0x52, 0x15, 0x01, 0xf8, 0x19, 0x0d, 0x45];
        assert_eq!(decompress(&z).unwrap(), b"hello hello hello hello, citronnade!".to_vec());
    }

    #[test]
    fn inflates_dynamic_huffman() {
        // zlib.compress(data, 9) of 4000 skewed pseudo-random bytes (a dynamic block)
        let alphabet = b"aaaaaaabbbbcccdde fghijklmnop\n";
        let mut x: u32 = 1;
        let mut expected = Vec::new();
        for _ in 0..4000 {
            x = (x.wrapping_mul(1103515245).wrapping_add(12345)) & 0x7fff_ffff;
            expected.push(alphabet[((x >> 16) as usize) % alphabet.len()]);
        }
        let z = include_bytes!("../tests/dynamic.zlib");
        assert_eq!(decompress(z).unwrap(), expected);
    }

    #[test]
    fn rejects_corruption() {
        let mut z = compress_stored(b"some data");
        let n = z.len();
        z[n - 1] ^= 1;
        assert_eq!(decompress(&z), Err(InflateError::ChecksumMismatch));
        assert_eq!(decompress(&[0, 0]), Err(InflateError::BadHeader));
        let _ = vec![0u8];
    }
}
