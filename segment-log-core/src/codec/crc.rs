const POLYNOMIAL: u32 = 0x82f63b78;

// A `static`, not a `const`: an unoptimized build copies a `const` array at every use, which
// here would be the whole 16 KiB table for each lookup.
static TABLES: [[u32; 256]; 16] = {
    let mut tables = [[0; 256]; 16];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (POLYNOMIAL & 0u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        tables[0][index] = crc;
        index += 1;
    }
    let mut table = 1;
    while table < 16 {
        let mut index = 0;
        while index < 256 {
            let previous = tables[table - 1][index];
            tables[table][index] = (previous >> 8) ^ tables[0][(previous & 0xff) as usize];
            index += 1;
        }
        table += 1;
    }
    tables
};

const STREAM_BYTES: usize = 1024;

/// The CRC register after `STREAM_BYTES` zero bytes, as a factor: `x^(8 * STREAM_BYTES)` modulo
/// the polynomial, in the reflected bit order of the register.
const STREAM_SHIFT: u32 = {
    let mut result = 1 << 31;
    let mut square = 1 << 23;
    let mut n = STREAM_BYTES;
    while n > 0 {
        if n & 1 == 1 {
            result = multiply(result, square);
        }
        square = multiply(square, square);
        n >>= 1;
    }
    result
};

/// Carry-less product of two reflected residues, modulo the polynomial.
const fn multiply(mut a: u32, mut b: u32) -> u32 {
    let mut product = 0;
    let mut bit = 0;
    while bit < 32 {
        product ^= b & 0u32.wrapping_sub(a >> 31);
        a <<= 1;
        b = (b >> 1) ^ (POLYNOMIAL & 0u32.wrapping_sub(b & 1));
        bit += 1;
    }
    product
}

fn step(crc: u32, chunk: &[u8; 16]) -> u32 {
    let (low, high) = chunk.split_at(8);
    let low = u64::from_le_bytes(low.try_into().unwrap()) ^ u64::from(crc);
    let high = u64::from_le_bytes(high.try_into().unwrap());
    let byte = |word: u64, i: u32| ((word >> (8 * i)) & 0xff) as usize;
    let mut crc = 0;
    for i in 0..8 {
        crc ^= TABLES[15 - i as usize][byte(low, i)] ^ TABLES[7 - i as usize][byte(high, i)];
    }
    crc
}

/// Four streams at a time, because each 16-byte step waits on the previous one's table lookups.
/// The streams start from zero and are joined by CRC linearity: the CRC of `a` then `b` is the
/// CRC of `a` shifted past `b`'s length, XOR the CRC of `b` from zero.
pub(super) fn update(mut crc: u32, bytes: &[u8]) -> u32 {
    let (rounds, rest) = bytes.as_chunks::<{ 4 * STREAM_BYTES }>();
    for round in rounds {
        let streams = round.as_chunks::<16>().0;
        let (a, rest) = streams.split_at(STREAM_BYTES / 16);
        let (b, rest) = rest.split_at(STREAM_BYTES / 16);
        let (c, d) = rest.split_at(STREAM_BYTES / 16);
        let mut sums = [crc, 0, 0, 0];
        for (((a, b), c), d) in a.iter().zip(b).zip(c).zip(d) {
            sums = [
                step(sums[0], a),
                step(sums[1], b),
                step(sums[2], c),
                step(sums[3], d),
            ];
        }
        crc = sums[1..]
            .iter()
            .fold(sums[0], |crc, &next| multiply(crc, STREAM_SHIFT) ^ next);
    }
    let (chunks, rest) = rest.as_chunks::<16>();
    for chunk in chunks {
        crc = step(crc, chunk);
    }
    for &byte in rest {
        crc = (crc >> 8) ^ TABLES[0][((crc ^ u32::from(byte)) & 0xff) as usize];
    }
    crc
}

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    !update(!0, bytes)
}
