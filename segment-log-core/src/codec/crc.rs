// A `static`, not a `const`: an unoptimized build copies a `const` array at every use, which
// here was the whole 8 KiB table for each lookup.
static TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0x82f63b78 & 0u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        tables[0][index] = crc;
        index += 1;
    }
    let mut table = 1;
    while table < 8 {
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

pub(super) fn update(mut crc: u32, bytes: &[u8]) -> u32 {
    let (chunks, rest) = bytes.as_chunks::<8>();
    for chunk in chunks {
        let word = u64::from_le_bytes(*chunk) ^ u64::from(crc);
        crc = TABLES[7][(word & 0xff) as usize]
            ^ TABLES[6][((word >> 8) & 0xff) as usize]
            ^ TABLES[5][((word >> 16) & 0xff) as usize]
            ^ TABLES[4][((word >> 24) & 0xff) as usize]
            ^ TABLES[3][((word >> 32) & 0xff) as usize]
            ^ TABLES[2][((word >> 40) & 0xff) as usize]
            ^ TABLES[1][((word >> 48) & 0xff) as usize]
            ^ TABLES[0][(word >> 56) as usize];
    }
    for &byte in rest {
        crc = (crc >> 8) ^ TABLES[0][((crc ^ u32::from(byte)) & 0xff) as usize];
    }
    crc
}

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    !update(!0, bytes)
}
