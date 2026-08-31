use flate2::read::ZlibDecoder;
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Read;
use surfacecheck_core::{
    Dimensions, MAX_DECODED_RGBA_BYTES, MAX_IMAGE_BYTES, MAX_IMAGE_DIMENSION, MAX_IMAGE_PIXELS,
};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, Copy)]
pub struct PngLimits {
    pub max_file_bytes: usize,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
}

impl Default for PngLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_IMAGE_BYTES as usize,
            max_width: MAX_IMAGE_DIMENSION,
            max_height: MAX_IMAGE_DIMENSION,
            max_pixels: MAX_IMAGE_PIXELS,
            max_decoded_bytes: MAX_DECODED_RGBA_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPng {
    pub dimensions: Dimensions,
    pub pixels: Vec<u8>,
    pub has_alpha: bool,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngError(pub String);

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for PngError {}

pub fn decode_png(input: &[u8], limits: PngLimits) -> Result<DecodedPng, PngError> {
    if input.len() > limits.max_file_bytes {
        return Err(PngError("image exceeds the configured byte bound".into()));
    }
    if input.len() < PNG_SIGNATURE.len() || &input[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(PngError("invalid PNG signature".into()));
    }

    let mut offset = PNG_SIGNATURE.len();
    let mut ihdr: Option<(u32, u32, u8, u8, u8)> = None;
    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut transparency: Option<Vec<u8>> = None;
    let mut compressed = Vec::new();
    let mut saw_idat = false;
    let mut saw_iend = false;

    while offset < input.len() {
        if input.len() - offset < 12 {
            return Err(PngError("truncated PNG chunk".into()));
        }
        let length =
            u32::from_be_bytes(input[offset..offset + 4].try_into().expect("four bytes")) as usize;
        let chunk_end = offset
            .checked_add(12)
            .and_then(|end| end.checked_add(length))
            .ok_or_else(|| PngError("PNG chunk length overflows".into()))?;
        if chunk_end > input.len() {
            return Err(PngError("PNG chunk extends beyond input".into()));
        }
        let kind = &input[offset + 4..offset + 8];
        let data = &input[offset + 8..offset + 8 + length];
        let expected_crc = u32::from_be_bytes(
            input[offset + 8 + length..chunk_end]
                .try_into()
                .expect("four bytes"),
        );
        let actual_crc = crc32(&input[offset + 4..offset + 8 + length]);
        if expected_crc != actual_crc {
            return Err(PngError("PNG chunk CRC mismatch".into()));
        }

        match kind {
            b"IHDR" => {
                if offset != PNG_SIGNATURE.len() || ihdr.is_some() || data.len() != 13 {
                    return Err(PngError("invalid IHDR placement or length".into()));
                }
                let width = u32::from_be_bytes(data[0..4].try_into().expect("four bytes"));
                let height = u32::from_be_bytes(data[4..8].try_into().expect("four bytes"));
                let bit_depth = data[8];
                let color_type = data[9];
                let compression = data[10];
                let filter = data[11];
                let interlace = data[12];
                if width == 0
                    || height == 0
                    || width > limits.max_width
                    || height > limits.max_height
                {
                    return Err(PngError(
                        "PNG dimensions exceed the configured bound".into(),
                    ));
                }
                let pixels = u64::from(width) * u64::from(height);
                if pixels > limits.max_pixels
                    || pixels
                        .checked_mul(4)
                        .ok_or_else(|| PngError("decoded size overflows".into()))?
                        > limits.max_decoded_bytes
                {
                    return Err(PngError("PNG pixel or decoded-byte bound exceeded".into()));
                }
                if compression != 0
                    || filter != 0
                    || interlace != 0
                    || bit_depth != 8
                    || !matches!(color_type, 0 | 2 | 3 | 4 | 6)
                {
                    return Err(PngError(
                        "unsupported PNG color, depth, compression, filter, or interlace mode"
                            .into(),
                    ));
                }
                ihdr = Some((width, height, bit_depth, color_type, interlace));
            }
            b"PLTE" => {
                if data.is_empty()
                    || !data.len().is_multiple_of(3)
                    || data.len() > 256 * 3
                    || saw_idat
                {
                    return Err(PngError("invalid PNG palette".into()));
                }
                let (entries, remainder) = data.as_chunks::<3>();
                debug_assert!(remainder.is_empty());
                palette = Some(entries.to_vec());
            }
            b"tRNS" => {
                if saw_idat || data.len() > 256 * 2 {
                    return Err(PngError("invalid PNG transparency data".into()));
                }
                transparency = Some(data.to_vec());
            }
            b"IDAT" => {
                if ihdr.is_none() || saw_iend {
                    return Err(PngError("IDAT appears before IHDR or after IEND".into()));
                }
                saw_idat = true;
                compressed.extend_from_slice(data);
                if compressed.len() > limits.max_file_bytes {
                    return Err(PngError(
                        "compressed PNG data exceeds the configured bound".into(),
                    ));
                }
            }
            b"IEND" => {
                if data.is_empty() && saw_idat {
                    saw_iend = true;
                } else {
                    return Err(PngError("invalid IEND chunk".into()));
                }
            }
            _ => {
                // Ancillary metadata is deliberately discarded. Unknown
                // critical chunks are unsafe to interpret as a valid image.
                if kind[0].is_ascii_uppercase() {
                    return Err(PngError("unknown critical PNG chunk".into()));
                }
            }
        }
        offset = chunk_end;
        if saw_iend {
            if offset != input.len() {
                return Err(PngError("bytes follow IEND".into()));
            }
            break;
        }
    }

    let (width, height, _bit_depth, color_type, _interlace) =
        ihdr.ok_or_else(|| PngError("missing IHDR".into()))?;
    if !saw_iend || compressed.is_empty() {
        return Err(PngError("missing IDAT or IEND".into()));
    }
    if color_type == 3 && palette.is_none() {
        return Err(PngError("indexed PNG is missing PLTE".into()));
    }

    let channels = match color_type {
        0 | 3 => 1usize,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => unreachable!(),
    };
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(channels))
        .ok_or_else(|| PngError("scanline length overflows".into()))?;
    let expected = usize::try_from(height)
        .ok()
        .and_then(|value| value.checked_mul(row_bytes.checked_add(1)?))
        .ok_or_else(|| PngError("scanline allocation overflows".into()))?;
    if u64::try_from(expected).map_err(|_| PngError("scanline length overflows".into()))?
        > limits.max_decoded_bytes
    {
        return Err(PngError(
            "scanline data exceeds the configured bound".into(),
        ));
    }
    let mut decoder = ZlibDecoder::new(compressed.as_slice());
    let mut filtered = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = decoder
            .read(&mut buffer)
            .map_err(|_| PngError("PNG zlib stream is invalid".into()))?;
        if read == 0 {
            break;
        }
        if filtered
            .len()
            .checked_add(read)
            .ok_or_else(|| PngError("decompressed PNG size overflows".into()))?
            > expected
        {
            return Err(PngError(
                "PNG decompressed data exceeds expected scanlines".into(),
            ));
        }
        filtered.extend_from_slice(&buffer[..read]);
    }
    if filtered.len() != expected {
        return Err(PngError("PNG scanlines are truncated".into()));
    }
    if decoder
        .read(&mut [0u8; 1])
        .map_err(|_| PngError("PNG zlib stream is invalid".into()))?
        != 0
    {
        return Err(PngError("PNG has trailing decompressed data".into()));
    }

    let mut rows = vec![
        0u8;
        row_bytes
            .checked_mul(usize::try_from(height).expect("height bounded"))
            .ok_or_else(|| PngError("pixel allocation overflows".into()))?
    ];
    for row in 0..usize::try_from(height).expect("height bounded") {
        let source_start = row * (row_bytes + 1);
        let destination_start = row * row_bytes;
        let filter = filtered[source_start];
        let source = &filtered[source_start + 1..source_start + 1 + row_bytes];
        let (before, current) = rows.split_at_mut(destination_start);
        let current = &mut current[..row_bytes];
        let previous = if row == 0 {
            None
        } else {
            Some(&before[destination_start - row_bytes..destination_start])
        };
        unfilter_row(filter, source, current, previous, channels)?;
    }

    let pixel_count = usize::try_from(u64::from(width) * u64::from(height)).expect("pixel bound");
    let mut pixels = Vec::with_capacity(
        pixel_count
            .checked_mul(4)
            .ok_or_else(|| PngError("RGBA allocation overflows".into()))?,
    );
    let mut has_alpha = matches!(color_type, 4 | 6) || transparency.is_some();
    for index in 0..pixel_count {
        let source = &rows[index * channels..(index + 1) * channels];
        let (red, green, blue, alpha) = match color_type {
            0 => {
                let value = source[0];
                let alpha = transparency.as_ref().map_or(255, |t| {
                    if t.len() >= 2 && u16::from_be_bytes([t[0], t[1]]) == u16::from(value) {
                        0
                    } else {
                        255
                    }
                });
                (value, value, value, alpha)
            }
            2 => {
                let alpha = transparency.as_ref().map_or(255, |t| {
                    if t.len() >= 6
                        && t[0] == 0
                        && t[1] == source[0]
                        && t[2] == 0
                        && t[3] == source[1]
                        && t[4] == 0
                        && t[5] == source[2]
                    {
                        0
                    } else {
                        255
                    }
                });
                (source[0], source[1], source[2], alpha)
            }
            3 => {
                let entry = palette
                    .as_ref()
                    .expect("indexed palette checked")
                    .get(usize::from(source[0]))
                    .ok_or_else(|| PngError("palette index is out of bounds".into()))?;
                let alpha = transparency
                    .as_ref()
                    .and_then(|t| t.get(usize::from(source[0])))
                    .copied()
                    .unwrap_or(255);
                (entry[0], entry[1], entry[2], alpha)
            }
            4 => (source[0], source[0], source[0], source[1]),
            6 => (source[0], source[1], source[2], source[3]),
            _ => unreachable!(),
        };
        if alpha != 255 {
            has_alpha = true;
        }
        pixels.extend_from_slice(&[red, green, blue, alpha]);
    }

    let mut digest = Sha256::new();
    digest.update(input);
    let sha256 = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(DecodedPng {
        dimensions: Dimensions { width, height },
        pixels,
        has_alpha,
        sha256,
    })
}

fn unfilter_row(
    filter: u8,
    source: &[u8],
    destination: &mut [u8],
    previous: Option<&[u8]>,
    bytes_per_pixel: usize,
) -> Result<(), PngError> {
    if source.len() != destination.len() {
        return Err(PngError("PNG row length mismatch".into()));
    }
    for (index, &value) in source.iter().enumerate() {
        let left = if index >= bytes_per_pixel {
            destination[index - bytes_per_pixel]
        } else {
            0
        };
        let above = previous.map_or(0, |row| row[index]);
        let upper_left = if index >= bytes_per_pixel {
            previous.map_or(0, |row| row[index - bytes_per_pixel])
        } else {
            0
        };
        destination[index] = match filter {
            0 => value,
            1 => value.wrapping_add(left),
            2 => value.wrapping_add(above),
            3 => value.wrapping_add(((u16::from(left) + u16::from(above)) / 2) as u8),
            4 => value.wrapping_add(paeth(left, above, upper_left)),
            _ => return Err(PngError("unsupported PNG row filter".into())),
        };
    }
    Ok(())
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = i32::from(a);
    let b = i32::from(b);
    let c = i32::from(c);
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(
            &(u32::try_from(data.len()).expect("test data length")).to_be_bytes(),
        );
        output.extend_from_slice(kind);
        output.extend_from_slice(data);
        output.extend_from_slice(&crc32(&output[4..]).to_be_bytes());
        output
    }

    fn png(width: u32, height: u32, color_type: u8, scanlines: &[u8]) -> Vec<u8> {
        let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
        compressed.write_all(scanlines).expect("zlib write");
        let compressed = compressed.finish().expect("zlib finish");
        let mut output = PNG_SIGNATURE.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
        output.extend_from_slice(&chunk(b"IHDR", &ihdr));
        if color_type == 3 {
            output.extend_from_slice(&chunk(b"PLTE", &[10, 20, 30, 40, 50, 60]));
        }
        output.extend_from_slice(&chunk(b"IDAT", &compressed));
        output.extend_from_slice(&chunk(b"IEND", &[]));
        output
    }

    #[test]
    fn decodes_rgba_and_records_input_checksum() {
        let input = png(2, 1, 6, &[0, 255, 0, 0, 128, 0, 0, 255, 255]);
        let decoded = decode_png(&input, PngLimits::default()).expect("valid PNG");
        assert_eq!(
            decoded.dimensions,
            (Dimensions {
                width: 2,
                height: 1
            })
        );
        assert_eq!(decoded.pixels, vec![255, 0, 0, 128, 0, 0, 255, 255]);
        assert!(decoded.has_alpha);
        assert_eq!(decoded.sha256.len(), 64);
    }

    #[test]
    fn all_scanline_filters_are_supported() {
        // Two grayscale rows. The encoded bytes are produced for each filter
        // from the same expected pixels: [10, 20, 30] / [40, 50, 60].
        let rows = [
            (0, vec![10, 20, 30]),
            (1, vec![10, 10, 10]),
            (2, vec![10, 20, 30]),
            (3, vec![10, 15, 20]),
            (4, vec![10, 10, 10]),
        ];
        for (filter, first_row) in rows {
            let mut scanlines = vec![filter];
            scanlines.extend(first_row);
            scanlines.extend_from_slice(&[0, 40, 50, 60]);
            let decoded =
                decode_png(&png(3, 2, 0, &scanlines), PngLimits::default()).expect("valid filter");
            assert_eq!(
                decoded.pixels,
                vec![
                    10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255, 40, 40, 40, 255, 50, 50, 50,
                    255, 60, 60, 60, 255
                ]
            );
        }
    }

    #[test]
    fn decodes_indexed_palette() {
        let decoded =
            decode_png(&png(2, 1, 3, &[0, 0, 1]), PngLimits::default()).expect("valid indexed PNG");
        assert_eq!(decoded.pixels, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn rejects_crc_corruption_and_truncation() {
        let mut input = png(1, 1, 6, &[0, 1, 2, 3, 4]);
        input[29] ^= 0xff;
        assert!(decode_png(&input, PngLimits::default()).is_err());
        let mut truncated = png(1, 1, 6, &[0, 1, 2, 3, 4]);
        truncated.truncate(truncated.len() - 3);
        assert!(decode_png(&truncated, PngLimits::default()).is_err());
    }

    #[test]
    fn rejects_extreme_dimensions_before_idat_processing() {
        let input = png(MAX_IMAGE_DIMENSION + 1, 1, 6, &[]);
        assert!(decode_png(&input, PngLimits::default()).is_err());
    }

    #[test]
    fn rejects_decompression_output_beyond_expected_scanlines() {
        let input = png(1, 1, 6, &[0, 1, 2, 3, 4, 5]);
        assert!(decode_png(&input, PngLimits::default()).is_err());
    }
}
