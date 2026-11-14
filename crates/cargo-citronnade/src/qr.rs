//! QR codes (ISO/IEC 18004), for links the console scans: byte mode, error
//! correction level M, versions 1 to 10 (up to 213 bytes). The structure
//! follows Project Nayuki's reference implementation.

/// A QR code: `size` x `size` modules, true for dark.
pub struct QrCode {
    pub size: usize,
    modules: Vec<bool>,
    function: Vec<bool>,
}

/// Level M: error correction codewords per block and blocks, by version.
const ECC_PER_BLOCK: [usize; 11] = [0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26];
const BLOCKS: [usize; 11] = [0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5];
/// Level M's format bits.
const LEVEL_M: u32 = 0;

fn raw_modules(version: usize) -> usize {
    let mut n = (16 * version + 128) * version + 64;
    if version >= 2 {
        let align = version / 7 + 2;
        n -= (25 * align - 10) * align - 55;
        if version >= 7 {
            n -= 36;
        }
    }
    n
}

fn data_codewords(version: usize) -> usize {
    raw_modules(version) / 8 - ECC_PER_BLOCK[version] * BLOCKS[version]
}

fn gf_mul(mut x: u8, mut y: u8) -> u8 {
    let mut z = 0u8;
    while y != 0 {
        if y & 1 != 0 {
            z ^= x;
        }
        x = (x << 1) ^ if x & 0x80 != 0 { 0x1D } else { 0 };
        y >>= 1;
    }
    z
}

fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    result[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 2);
    }
    result
}

fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ result.remove(0);
        result.push(0);
        for (r, d) in result.iter_mut().zip(divisor) {
            *r ^= gf_mul(*d, factor);
        }
    }
    result
}

fn alignment_positions(version: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let count = version / 7 + 2;
    let size = version * 4 + 17;
    let step = (version * 8 + count * 3 + 5) / (count * 4 - 4) * 2;
    let mut positions: Vec<usize> = (0..count - 1).map(|i| size - 7 - i * step).collect();
    positions.push(6);
    positions.reverse();
    positions
}

impl QrCode {
    /// Encodes `text`; `None` when it is longer than 213 bytes.
    pub fn encode(text: &[u8]) -> Option<QrCode> {
        let version = (1..=10).find(|&v| {
            let count_bits = if v < 10 { 8 } else { 16 };
            4 + count_bits + text.len() * 8 <= data_codewords(v) * 8
        })?;
        // the bit stream: byte mode, the length, the bytes, a terminator,
        // then padding to the capacity
        let capacity = data_codewords(version) * 8;
        let mut bits: Vec<bool> = Vec::new();
        let mut push = |value: usize, n: usize| (0..n).rev().for_each(|i| bits.push((value >> i) & 1 != 0));
        push(0b0100, 4);
        push(text.len(), if version < 10 { 8 } else { 16 });
        for &b in text {
            push(b as usize, 8);
        }
        let terminator = (capacity - bits.len()).min(4);
        bits.extend(std::iter::repeat_n(false, terminator));
        while bits.len() % 8 != 0 {
            bits.push(false);
        }
        let mut data: Vec<u8> = bits.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| a << 1 | b as u8)).collect();
        for pad in [0xEC, 0x11].iter().cycle() {
            if data.len() * 8 >= capacity {
                break;
            }
            data.push(*pad);
        }

        // error correction, in blocks, interleaved
        let (blocks_n, ecc_len) = (BLOCKS[version], ECC_PER_BLOCK[version]);
        let raw = raw_modules(version) / 8;
        let short_blocks = blocks_n - raw % blocks_n;
        let short_len = raw / blocks_n;
        let divisor = rs_divisor(ecc_len);
        let mut blocks = Vec::new();
        let mut k = 0;
        for i in 0..blocks_n {
            let len = short_len - ecc_len + usize::from(i >= short_blocks);
            let dat = &data[k..k + len];
            k += len;
            let mut block = dat.to_vec();
            if i < short_blocks {
                block.push(0);
            }
            block.extend(rs_remainder(dat, &divisor));
            blocks.push(block);
        }
        let mut codewords = Vec::new();
        for i in 0..blocks[0].len() {
            for (j, block) in blocks.iter().enumerate() {
                if i != short_len - ecc_len || j >= short_blocks {
                    codewords.push(block[i]);
                }
            }
        }

        let size = version * 4 + 17;
        let mut qr = QrCode { size, modules: vec![false; size * size], function: vec![false; size * size] };
        qr.draw_function_patterns(version);
        qr.draw_codewords(&codewords);
        // the mask with the lowest penalty
        let mut best = (u32::MAX, 0);
        for mask in 0..8 {
            qr.apply_mask(mask);
            qr.draw_format_bits(mask);
            let penalty = qr.penalty();
            if penalty < best.0 {
                best = (penalty, mask);
            }
            qr.apply_mask(mask);
        }
        qr.apply_mask(best.1);
        qr.draw_format_bits(best.1);
        Some(qr)
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }

    fn set_function(&mut self, x: usize, y: usize, dark: bool) {
        self.modules[y * self.size + x] = dark;
        self.function[y * self.size + x] = true;
    }

    fn draw_function_patterns(&mut self, version: usize) {
        let size = self.size;
        for i in 0..size {
            self.set_function(6, i, i % 2 == 0);
            self.set_function(i, 6, i % 2 == 0);
        }
        for (cx, cy) in [(3, 3), (size - 4, 3), (3, size - 4)] {
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                    if (0..size as i32).contains(&x) && (0..size as i32).contains(&y) {
                        let dist = dx.abs().max(dy.abs());
                        self.set_function(x as usize, y as usize, dist != 2 && dist != 4);
                    }
                }
            }
        }
        let positions = alignment_positions(version);
        let last = positions.len().saturating_sub(1);
        for (i, &ay) in positions.iter().enumerate() {
            for (j, &ax) in positions.iter().enumerate() {
                if (i == 0 && j == 0) || (i == 0 && j == last) || (i == last && j == 0) {
                    continue;
                }
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        self.set_function((ax as i32 + dx) as usize, (ay as i32 + dy) as usize, dx.abs().max(dy.abs()) != 1);
                    }
                }
            }
        }
        self.draw_format_bits(0); // reserved now, written once the mask is chosen
        if version >= 7 {
            let mut rem = version as u32;
            for _ in 0..12 {
                rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
            }
            let bits = (version as u32) << 12 | rem;
            for i in 0..18 {
                let bit = (bits >> i) & 1 != 0;
                let (a, b) = (size - 11 + i % 3, i / 3);
                self.set_function(a, b, bit);
                self.set_function(b, a, bit);
            }
        }
    }

    fn draw_format_bits(&mut self, mask: u32) {
        let data = LEVEL_M << 3 | mask;
        let mut rem = data;
        for _ in 0..10 {
            rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        }
        let bits = (data << 10 | rem) ^ 0x5412;
        let bit = |i: u32| (bits >> i) & 1 != 0;
        let size = self.size;
        for i in 0..=5 {
            self.set_function(8, i as usize, bit(i));
        }
        self.set_function(8, 7, bit(6));
        self.set_function(8, 8, bit(7));
        self.set_function(7, 8, bit(8));
        for i in 9..15 {
            self.set_function(14 - i as usize, 8, bit(i));
        }
        for i in 0..8 {
            self.set_function(size - 1 - i as usize, 8, bit(i));
        }
        for i in 8..15 {
            self.set_function(8, size - 15 + i as usize, bit(i));
        }
        self.set_function(8, size - 8, true);
    }

    fn draw_codewords(&mut self, data: &[u8]) {
        let size = self.size;
        let mut i = 0;
        let mut right = size - 1;
        loop {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = right - j;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert };
                    if !self.function[y * size + x] && i < data.len() * 8 {
                        self.modules[y * size + x] = (data[i >> 3] >> (7 - (i & 7))) & 1 != 0;
                        i += 1;
                    }
                }
            }
            if right < 2 {
                break;
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u32) {
        for y in 0..self.size {
            for x in 0..self.size {
                let invert = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                if invert && !self.function[y * self.size + x] {
                    self.modules[y * self.size + x] ^= true;
                }
            }
        }
    }

    /// The standard's penalty, without its finder-like pattern rule: runs
    /// of five or more, 2x2 blocks and the dark/light balance.
    fn penalty(&self) -> u32 {
        let n = self.size;
        let mut score = 0u32;
        for line in 0..n {
            for horizontal in [true, false] {
                let mut run = 1;
                for i in 1..n {
                    let (a, b) = if horizontal { (self.get(i, line), self.get(i - 1, line)) } else { (self.get(line, i), self.get(line, i - 1)) };
                    if a == b {
                        run += 1;
                        if run == 5 {
                            score += 3;
                        } else if run > 5 {
                            score += 1;
                        }
                    } else {
                        run = 1;
                    }
                }
            }
        }
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = self.get(x, y);
                if c == self.get(x + 1, y) && c == self.get(x, y + 1) && c == self.get(x + 1, y + 1) {
                    score += 3;
                }
            }
        }
        let dark = self.modules.iter().filter(|&&m| m).count();
        let total = n * n;
        let k = ((dark * 20).abs_diff(total * 10)).div_ceil(total).saturating_sub(1);
        score + k as u32 * 10
    }

    /// The code for a terminal: two rows per line in half blocks, dark on a
    /// forced light background, with a four-module quiet zone.
    pub fn to_terminal(&self) -> String {
        let quiet = 4i32;
        let n = self.size as i32;
        let dark = |x: i32, y: i32| x >= 0 && y >= 0 && x < n && y < n && self.get(x as usize, y as usize);
        let mut out = String::new();
        let mut y = -quiet;
        while y < n + quiet {
            out.push_str("\x1b[30;107m");
            for x in -quiet..n + quiet {
                out.push(match (dark(x, y), dark(x, y + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                });
            }
            out.push_str("\x1b[0m\n");
            y += 2;
        }
        out
    }

    /// The code as an SVG image (with its quiet zone).
    pub fn to_svg(&self) -> String {
        let total = self.size + 8;
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {total} {total}\" width=\"{}\" height=\"{}\" shape-rendering=\"crispEdges\">\n<rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>\n<path fill=\"#000\" d=\"",
            total * 8,
            total * 8
        );
        for y in 0..self.size {
            for x in 0..self.size {
                if self.get(x, y) {
                    out.push_str(&format!("M{} {}h1v1h-1z", x + 4, y + 4));
                }
            }
        }
        out.push_str("\"/>\n</svg>\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities() {
        // level M's data codewords for versions 1 to 10
        let expected = [16, 28, 44, 64, 86, 108, 124, 154, 182, 216];
        for v in 1..=10 {
            assert_eq!(data_codewords(v), expected[v - 1], "version {v}");
        }
        assert_eq!(alignment_positions(7), vec![6, 22, 38]);
        assert_eq!(alignment_positions(10), vec![6, 28, 50]);
    }

    #[test]
    fn sizes() {
        assert_eq!(QrCode::encode(b"http://192.168.1.20:8000/a.cia").unwrap().size, 29); // version 3
        assert_eq!(QrCode::encode(&[b'x'; 213]).unwrap().size, 57); // version 10
        assert!(QrCode::encode(&[b'x'; 214]).is_none());
    }
}
