//! A minimal, dependency-free PNG encoder (8-bit RGB, no filters).
//!
//! Emits valid PNG files using *stored* (uncompressed) DEFLATE
//! blocks: a zlib stream of `0x78 0x01` + stored blocks + Adler-32,
//! wrapped in the standard chunk envelope with CRC-32. This keeps
//! the M2 frame dumps lossless and readable by every image tool
//! without pulling a single crate into the tree (the cost gate).

use std::fs::File;
use std::io::Write;
use std::path::Path;

/// The CRC-32 table (IEEE 802.3 polynomial, public domain).
fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *entry = c;
    }
    table
}

fn crc32(data: &[u8]) -> u32 {
    let table = crc32_table();
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc = table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Adler-32 of the deflate payload.
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Write one PNG chunk (length + type + data + CRC).
fn write_chunk<W: Write>(out: &mut W, kind: &[u8; 4], data: &[u8]) -> std::io::Result<()> {
    out.write_all(&(data.len() as u32).to_be_bytes())?;
    out.write_all(kind)?;
    out.write_all(data)?;
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.write_all(&crc32(&crc_input).to_be_bytes())?;
    Ok(())
}

/// Write `rgb` (packed 3-byte pixels) as a `width x height` PNG.
pub fn write_png(path: &Path, rgb: &[u8], width: u32, height: u32) -> std::io::Result<()> {
    // Raw scanlines: filter byte 0 then the row bytes.
    let stride = (width as usize) * 3;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgb.chunks_exact(stride) {
        raw.push(0u8);
        raw.extend_from_slice(row);
    }

    // The zlib stream: header, stored deflate blocks, Adler-32.
    let mut zlib = Vec::with_capacity(raw.len() + raw.len() / 65535 * 5 + 16);
    zlib.push(0x78);
    zlib.push(0x01);
    let mut offset = 0usize;
    while offset < raw.len() {
        // Stored blocks carry at most 65,535 bytes.
        let chunk = (raw.len() - offset).min(65_535);
        let last = offset + chunk == raw.len();
        zlib.push(u8::from(last)); // BFINAL + BTYPE=00.
        zlib.extend_from_slice(&(chunk as u16).to_le_bytes());
        let nlen = !(chunk as u16);
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(&raw[offset..offset + chunk]);
        offset += chunk;
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = std::io::BufWriter::new(File::create(path)?);
    out.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])?;
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // depth 8, RGB, rest zero.
    write_chunk(&mut out, b"IHDR", &ihdr)?;
    write_chunk(&mut out, b"IDAT", &zlib)?;
    write_chunk(&mut out, b"IEND", &[])?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 image; the round trip is verified structurally (the file
    /// begins with the signature and contains the two mandatory
    /// chunks) plus the CRC/Adler helpers round their inputs.
    #[test]
    fn writes_a_valid_png_envelope() {
        let rgb = [
            0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let dir = std::env::temp_dir().join("se_cli_png_tests");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("t.png");
        write_png(&path, &rgb, 2, 2).expect("png");
        let data = std::fs::read(&path).expect("read back");
        assert_eq!(
            &data[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        // IHDR then IDAT then IEND in order.
        let find = |kind: &[u8; 4]| -> Option<usize> { data.windows(4).position(|w| w == kind) };
        let ihdr = find(b"IHDR").expect("IHDR");
        let idat = find(b"IDAT").expect("IDAT");
        let iend = find(b"IEND").expect("IEND");
        assert!(ihdr < idat && idat < iend);
        // The zlib stream starts right after the IDAT type field.
        let zlib_at = idat + 4;
        assert_eq!(data[zlib_at], 0x78);
        assert_eq!(data[zlib_at + 1], 0x01);
    }
    #[test]
    fn crc32_and_adler32_match_known_values() {
        // CRC-32("123456789") = 0xCBF43926 (IEEE test vector).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // Adler-32("Wikipedia") = 0x11E60398 (public test vector).
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
