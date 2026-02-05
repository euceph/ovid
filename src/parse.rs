use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ImageFormat {
    Png,
    Jpg,
}

/// PNG compression level
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum PngCompression {
    /// fastest encoding, larger files
    #[default]
    Fast,
    /// smaller files, slower encoding
    Small,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PageSize {
    A4,
    Letter,
    Legal,
    A3,
}

impl PageSize {
    pub fn dimensions_pt(self) -> (f32, f32) {
        match self {
            PageSize::A4 => (595.28, 841.89),
            PageSize::Letter => (612.0, 792.0),
            PageSize::Legal => (612.0, 1008.0),
            PageSize::A3 => (841.89, 1190.55),
        }
    }
}

/// parse page range string like "1,3-5,10" into 0-indexed page indices
pub fn parse_page_ranges(s: &str, num_pages: i32) -> Result<Vec<i32>> {
    let mut pages = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: i32 = start.trim().parse().context("Invalid page number in range")?;
            let end: i32 = end.trim().parse().context("Invalid page number in range")?;
            anyhow::ensure!(
                start >= 1 && end >= start && end <= num_pages,
                "Page range {}-{} out of bounds (document has {} pages)",
                start,
                end,
                num_pages
            );
            for p in start..=end {
                pages.push(p - 1);
            }
        } else {
            let p: i32 = part.parse().context("Invalid page number")?;
            anyhow::ensure!(
                p >= 1 && p <= num_pages,
                "Page {} out of bounds (document has {} pages)",
                p,
                num_pages
            );
            pages.push(p - 1);
        }
    }
    anyhow::ensure!(!pages.is_empty(), "No pages specified");
    pages.dedup();
    Ok(pages)
}

/// expand dirs in input list into sorted image files
pub fn expand_image_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tiff", "tif", "bmp", "gif"];
    let mut result = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
                .with_context(|| format!("Cannot read directory: {}", path.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                })
                .collect();
            entries.sort();
            anyhow::ensure!(
                !entries.is_empty(),
                "No image files found in {}",
                path.display()
            );
            result.extend(entries);
        } else {
            result.push(path.clone());
        }
    }
    Ok(result)
}

/// parse JPEG file's SOF marker to extract (width, height, num_components)
pub fn parse_jpeg_header(data: &[u8]) -> Result<(u32, u32, u8)> {
    anyhow::ensure!(
        data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8,
        "Not a valid JPEG file"
    );
    let mut pos = 2;
    while pos + 4 < data.len() {
        if data[pos] != 0xFF {
            anyhow::bail!("Invalid JPEG marker at offset {}", pos);
        }
        let marker = data[pos + 1];
        // skip padding 0xFF bytes
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        // skip RST markers and standalone markers (no length field)
        if marker == 0x00 || (0xD0..=0xD9).contains(&marker) {
            pos += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        // all SOF markers: SOF0-SOF3, SOF5-SOF7, SOF9-SOF11, SOF13-SOF15
        // (excludes 0xC4=DHT, 0xC8=JPG, 0xCC=DAC)
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            anyhow::ensure!(pos + 2 + len <= data.len() && len >= 8, "Truncated SOF");
            let height = u16::from_be_bytes([data[pos + 5], data[pos + 6]]) as u32;
            let width = u16::from_be_bytes([data[pos + 7], data[pos + 8]]) as u32;
            let components = data[pos + 9];
            return Ok((width, height, components));
        }
        pos += 2 + len;
    }
    anyhow::bail!("No SOF marker found in JPEG")
}

pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub idat_data: Vec<u8>,
    pub plte_data: Vec<u8>,
}

/// parse a PNG file to extract IHDR info and concatenated IDAT chunk data
pub fn parse_png_header(data: &[u8]) -> Result<PngInfo> {
    const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    anyhow::ensure!(
        data.len() >= 8 && data[..8] == SIGNATURE,
        "Not a valid PNG file"
    );

    let mut pos = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut idat_data = Vec::new();
    let mut plte_data = Vec::new();
    let mut got_ihdr = false;

    while pos + 8 <= data.len() {
        let chunk_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        let chunk_data_start = pos + 8;
        let chunk_end = chunk_data_start + chunk_len + 4; // +4 for CRC
        anyhow::ensure!(chunk_end <= data.len(), "Truncated PNG chunk");

        if chunk_type == b"IHDR" {
            anyhow::ensure!(chunk_len >= 13, "Truncated IHDR");
            let d = &data[chunk_data_start..];
            width = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
            height = u32::from_be_bytes([d[4], d[5], d[6], d[7]]);
            bit_depth = d[8];
            color_type = d[9];
            got_ihdr = true;
        } else if chunk_type == b"PLTE" {
            plte_data.extend_from_slice(&data[chunk_data_start..chunk_data_start + chunk_len]);
        } else if chunk_type == b"IDAT" {
            idat_data.extend_from_slice(&data[chunk_data_start..chunk_data_start + chunk_len]);
        } else if chunk_type == b"IEND" {
            break;
        }

        pos = chunk_end;
    }

    anyhow::ensure!(got_ihdr, "No IHDR chunk found in PNG");
    anyhow::ensure!(!idat_data.is_empty(), "No IDAT chunks found in PNG");

    Ok(PngInfo {
        width,
        height,
        bit_depth,
        color_type,
        idat_data,
        plte_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_pages_single() {
        assert_eq!(parse_page_ranges("1", 10).unwrap(), vec![0]);
        assert_eq!(parse_page_ranges("5", 10).unwrap(), vec![4]);
        assert_eq!(parse_page_ranges("10", 10).unwrap(), vec![9]);
    }

    #[test]
    fn parse_pages_multiple() {
        assert_eq!(parse_page_ranges("1,3,5", 10).unwrap(), vec![0, 2, 4]);
    }

    #[test]
    fn parse_pages_range() {
        assert_eq!(parse_page_ranges("2-5", 10).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn parse_pages_mixed() {
        assert_eq!(
            parse_page_ranges("1,3-5,10", 10).unwrap(),
            vec![0, 2, 3, 4, 9]
        );
    }

    #[test]
    fn parse_pages_single_page_range() {
        assert_eq!(parse_page_ranges("3-3", 5).unwrap(), vec![2]);
    }

    #[test]
    fn parse_pages_whitespace() {
        assert_eq!(
            parse_page_ranges(" 1 , 3 - 5 , 10 ", 10).unwrap(),
            vec![0, 2, 3, 4, 9]
        );
    }

    #[test]
    fn parse_pages_trailing_comma() {
        assert_eq!(parse_page_ranges("1,2,", 5).unwrap(), vec![0, 1]);
    }

    #[test]
    fn parse_pages_dedup_adjacent() {
        assert_eq!(parse_page_ranges("3,3", 5).unwrap(), vec![2]);
    }

    #[test]
    fn parse_pages_no_dedup_nonadjacent() {
        let result = parse_page_ranges("1,3,1", 5).unwrap();
        assert_eq!(result, vec![0, 2, 0]);
    }

    #[test]
    fn parse_pages_all_pages() {
        assert_eq!(parse_page_ranges("1-3", 3).unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn parse_pages_err_zero() {
        assert!(parse_page_ranges("0", 10).is_err());
    }

    #[test]
    fn parse_pages_err_exceeds_count() {
        assert!(parse_page_ranges("11", 10).is_err());
    }

    #[test]
    fn parse_pages_err_range_exceeds() {
        assert!(parse_page_ranges("5-11", 10).is_err());
    }

    #[test]
    fn parse_pages_err_reversed_range() {
        assert!(parse_page_ranges("5-3", 10).is_err());
    }

    #[test]
    fn parse_pages_err_empty() {
        assert!(parse_page_ranges("", 10).is_err());
    }

    #[test]
    fn parse_pages_err_garbage() {
        assert!(parse_page_ranges("abc", 10).is_err());
    }

    #[test]
    fn parse_pages_err_negative() {
        assert!(parse_page_ranges("-1", 10).is_err());
    }

    fn make_minimal_jpeg(width: u16, height: u16, components: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0xFF, 0xD8]);
        let sof_len: u16 = 8 + 3 * components as u16;
        buf.extend_from_slice(&[0xFF, 0xC0]);
        buf.extend_from_slice(&sof_len.to_be_bytes());
        buf.push(8);
        buf.extend_from_slice(&height.to_be_bytes());
        buf.extend_from_slice(&width.to_be_bytes());
        buf.push(components);
        for i in 0..components {
            buf.push(i + 1);
            buf.push(0x11);
            buf.push(0);
        }
        buf.extend_from_slice(&[0xFF, 0xD9]);
        buf
    }

    #[test]
    fn jpeg_header_rgb() {
        let data = make_minimal_jpeg(640, 480, 3);
        let (w, h, c) = parse_jpeg_header(&data).unwrap();
        assert_eq!((w, h, c), (640, 480, 3));
    }

    #[test]
    fn jpeg_header_grayscale() {
        let data = make_minimal_jpeg(100, 200, 1);
        let (w, h, c) = parse_jpeg_header(&data).unwrap();
        assert_eq!((w, h, c), (100, 200, 1));
    }

    #[test]
    fn jpeg_header_large_dimensions() {
        let data = make_minimal_jpeg(4096, 3000, 3);
        let (w, h, c) = parse_jpeg_header(&data).unwrap();
        assert_eq!((w, h, c), (4096, 3000, 3));
    }

    #[test]
    fn jpeg_header_with_app0_before_sof() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0xFF, 0xD8]);
        buf.extend_from_slice(&[0xFF, 0xE0]);
        let app0_payload = vec![0u8; 14];
        let app0_len = (app0_payload.len() + 2) as u16;
        buf.extend_from_slice(&app0_len.to_be_bytes());
        buf.extend_from_slice(&app0_payload);
        let sof_len: u16 = 8 + 3 * 3;
        buf.extend_from_slice(&[0xFF, 0xC0]);
        buf.extend_from_slice(&sof_len.to_be_bytes());
        buf.push(8);
        buf.extend_from_slice(&480u16.to_be_bytes());
        buf.extend_from_slice(&640u16.to_be_bytes());
        buf.push(3);
        for i in 0..3u8 {
            buf.push(i + 1);
            buf.push(0x11);
            buf.push(0);
        }
        buf.extend_from_slice(&[0xFF, 0xD9]);
        let (w, h, c) = parse_jpeg_header(&buf).unwrap();
        assert_eq!((w, h, c), (640, 480, 3));
    }

    #[test]
    fn jpeg_header_sof2_progressive() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0xFF, 0xD8]);
        let sof_len: u16 = 8 + 3 * 3;
        buf.extend_from_slice(&[0xFF, 0xC2]);
        buf.extend_from_slice(&sof_len.to_be_bytes());
        buf.push(8);
        buf.extend_from_slice(&768u16.to_be_bytes());
        buf.extend_from_slice(&1024u16.to_be_bytes());
        buf.push(3);
        for i in 0..3u8 {
            buf.push(i + 1);
            buf.push(0x11);
            buf.push(0);
        }
        buf.extend_from_slice(&[0xFF, 0xD9]);
        let (w, h, c) = parse_jpeg_header(&buf).unwrap();
        assert_eq!((w, h, c), (1024, 768, 3));
    }

    #[test]
    fn jpeg_header_err_not_jpeg() {
        assert!(parse_jpeg_header(&[0x89, 0x50]).is_err());
    }

    #[test]
    fn jpeg_header_err_too_short() {
        assert!(parse_jpeg_header(&[0xFF]).is_err());
    }

    #[test]
    fn jpeg_header_err_no_sof() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0xFF, 0xD8]);
        buf.extend_from_slice(&[0xFF, 0xD9]);
        assert!(parse_jpeg_header(&buf).is_err());
    }

    fn crc32_chunk(chunk_type: &[u8], data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in chunk_type.iter().chain(data.iter()) {
            let idx = ((crc ^ b as u32) & 0xFF) as usize;
            crc = CRC_TABLE[idx] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    const CRC_TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                if c & 1 != 0 {
                    c = 0xEDB8_8320 ^ (c >> 1);
                } else {
                    c >>= 1;
                }
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };

    fn make_minimal_png(width: u32, height: u32, color_type: u8, bit_depth: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

        fn write_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
            buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
            buf.extend_from_slice(chunk_type);
            buf.extend_from_slice(data);
            let crc = crc32_chunk(chunk_type, data);
            buf.extend_from_slice(&crc.to_be_bytes());
        }

        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.push(bit_depth);
        ihdr.push(color_type);
        ihdr.push(0);
        ihdr.push(0);
        ihdr.push(0);
        write_chunk(&mut buf, b"IHDR", &ihdr);

        if color_type == 3 {
            let palette = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
            write_chunk(&mut buf, b"PLTE", &palette);
        }

        let channels: usize = match color_type {
            0 => 1,
            2 => 3,
            3 => 1,
            4 => 2,
            6 => 4,
            _ => 1,
        };
        let row_bytes = width as usize * channels * (bit_depth as usize / 8);
        let mut raw = Vec::new();
        for _ in 0..height {
            raw.push(0);
            raw.extend(vec![128u8; row_bytes]);
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        write_chunk(&mut buf, b"IDAT", &compressed);

        write_chunk(&mut buf, b"IEND", &[]);
        buf
    }

    #[test]
    fn png_header_rgb() {
        let data = make_minimal_png(16, 8, 2, 8);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.width, 16);
        assert_eq!(info.height, 8);
        assert_eq!(info.color_type, 2);
        assert_eq!(info.bit_depth, 8);
        assert!(!info.idat_data.is_empty());
        assert!(info.plte_data.is_empty());
    }

    #[test]
    fn png_header_grayscale() {
        let data = make_minimal_png(32, 32, 0, 8);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.width, 32);
        assert_eq!(info.height, 32);
        assert_eq!(info.color_type, 0);
        assert_eq!(info.bit_depth, 8);
    }

    #[test]
    fn png_header_palette() {
        let data = make_minimal_png(4, 4, 3, 8);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.color_type, 3);
        assert_eq!(info.plte_data.len(), 12);
    }

    #[test]
    fn png_header_rgba() {
        let data = make_minimal_png(10, 10, 6, 8);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.color_type, 6);
        assert_eq!(info.bit_depth, 8);
    }

    #[test]
    fn png_header_gray_alpha() {
        let data = make_minimal_png(10, 10, 4, 8);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.color_type, 4);
    }

    #[test]
    fn png_header_16bit() {
        let data = make_minimal_png(8, 8, 2, 16);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.bit_depth, 16);
        assert_eq!(info.color_type, 2);
    }

    #[test]
    fn png_header_large_dimensions() {
        let data = make_minimal_png(4096, 3000, 2, 8);
        let info = parse_png_header(&data).unwrap();
        assert_eq!(info.width, 4096);
        assert_eq!(info.height, 3000);
    }

    #[test]
    fn png_header_err_not_png() {
        assert!(parse_png_header(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn png_header_err_too_short() {
        assert!(parse_png_header(&[137, 80, 78, 71]).is_err());
    }

    #[test]
    fn png_header_err_no_idat() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&4u32.to_be_bytes());
        ihdr.extend_from_slice(&4u32.to_be_bytes());
        ihdr.push(8);
        ihdr.push(2);
        ihdr.push(0);
        ihdr.push(0);
        ihdr.push(0);
        buf.extend_from_slice(&(ihdr.len() as u32).to_be_bytes());
        buf.extend_from_slice(b"IHDR");
        buf.extend_from_slice(&ihdr);
        let crc = crc32_chunk(b"IHDR", &ihdr);
        buf.extend_from_slice(&crc.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes());
        buf.extend_from_slice(b"IEND");
        let crc = crc32_chunk(b"IEND", &[]);
        buf.extend_from_slice(&crc.to_be_bytes());
        assert!(parse_png_header(&buf).is_err());
    }

    #[test]
    fn png_header_multiple_idat_chunks() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

        fn write_chunk_with(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
            buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
            buf.extend_from_slice(chunk_type);
            buf.extend_from_slice(data);
            let mut crc: u32 = 0xFFFF_FFFF;
            for &b in chunk_type.iter().chain(data.iter()) {
                let idx = ((crc ^ b as u32) & 0xFF) as usize;
                crc = super::tests::CRC_TABLE[idx] ^ (crc >> 8);
            }
            crc ^= 0xFFFF_FFFF;
            buf.extend_from_slice(&crc.to_be_bytes());
        }

        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&4u32.to_be_bytes());
        ihdr.extend_from_slice(&4u32.to_be_bytes());
        ihdr.push(8);
        ihdr.push(2);
        ihdr.push(0);
        ihdr.push(0);
        ihdr.push(0);
        write_chunk_with(&mut buf, b"IHDR", &ihdr);

        let mut raw = Vec::new();
        for _ in 0..4u32 {
            raw.push(0);
            raw.extend(vec![128u8; 12]);
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let mid = compressed.len() / 2;
        write_chunk_with(&mut buf, b"IDAT", &compressed[..mid]);
        write_chunk_with(&mut buf, b"IDAT", &compressed[mid..]);
        write_chunk_with(&mut buf, b"IEND", &[]);

        let info = parse_png_header(&buf).unwrap();
        assert_eq!(info.width, 4);
        assert_eq!(info.height, 4);
        assert_eq!(info.idat_data.len(), compressed.len());
    }

    #[test]
    fn expand_paths_files_only() {
        let dir = std::env::temp_dir().join("ovid_test_expand_files");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p1 = dir.join("a.png");
        let p2 = dir.join("b.jpg");
        std::fs::write(&p1, b"fake").unwrap();
        std::fs::write(&p2, b"fake").unwrap();
        let result = expand_image_paths(&[p1.clone(), p2.clone()]).unwrap();
        assert_eq!(result, vec![p1, p2]);
    }

    #[test]
    fn expand_paths_directory() {
        let dir = std::env::temp_dir().join("ovid_test_expand_dir");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.png"), b"fake").unwrap();
        std::fs::write(dir.join("a.jpg"), b"fake").unwrap();
        std::fs::write(dir.join("b.tiff"), b"fake").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not an image").unwrap();
        let result = expand_image_paths(&[dir.clone()]).unwrap();
        assert_eq!(result.len(), 3);
        assert!(
            result[0].file_name().unwrap().to_str().unwrap()
                < result[1].file_name().unwrap().to_str().unwrap()
        );
        assert!(result.iter().all(|p| p.extension().unwrap() != "txt"));
    }

    #[test]
    fn expand_paths_mixed() {
        let dir = std::env::temp_dir().join("ovid_test_expand_mixed");
        let _ = std::fs::remove_dir_all(&dir);
        let subdir = dir.join("sub");
        std::fs::create_dir_all(&subdir).unwrap();
        let explicit = dir.join("first.png");
        std::fs::write(&explicit, b"fake").unwrap();
        std::fs::write(subdir.join("a.jpg"), b"fake").unwrap();
        std::fs::write(subdir.join("b.png"), b"fake").unwrap();
        let result = expand_image_paths(&[explicit.clone(), subdir]).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], explicit);
    }

    #[test]
    fn expand_paths_empty_dir() {
        let dir = std::env::temp_dir().join("ovid_test_expand_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(expand_image_paths(&[dir]).is_err());
    }

    #[test]
    fn expand_paths_case_insensitive_ext() {
        let dir = std::env::temp_dir().join("ovid_test_expand_case");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("photo.JPG"), b"fake").unwrap();
        std::fs::write(dir.join("scan.Png"), b"fake").unwrap();
        std::fs::write(dir.join("doc.TIFF"), b"fake").unwrap();
        let result = expand_image_paths(&[dir]).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn expand_paths_all_extensions() {
        let dir = std::env::temp_dir().join("ovid_test_expand_allext");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for ext in &["png", "jpg", "jpeg", "tiff", "tif", "bmp", "gif"] {
            std::fs::write(dir.join(format!("file.{}", ext)), b"fake").unwrap();
        }
        let result = expand_image_paths(&[dir]).unwrap();
        assert_eq!(result.len(), 7);
    }

    #[test]
    fn page_size_dimensions() {
        let (w, h) = PageSize::A4.dimensions_pt();
        assert!((w - 595.28).abs() < 0.01);
        assert!((h - 841.89).abs() < 0.01);

        let (w, h) = PageSize::Letter.dimensions_pt();
        assert!((w - 612.0).abs() < 0.01);
        assert!((h - 792.0).abs() < 0.01);

        let (w, h) = PageSize::Legal.dimensions_pt();
        assert!((w - 612.0).abs() < 0.01);
        assert!((h - 1008.0).abs() < 0.01);

        let (w, h) = PageSize::A3.dimensions_pt();
        assert!((w - 841.89).abs() < 0.01);
        assert!((h - 1190.55).abs() < 0.01);
    }

    #[test]
    fn page_size_portrait_orientation() {
        for ps in [PageSize::A4, PageSize::Letter, PageSize::Legal, PageSize::A3] {
            let (w, h) = ps.dimensions_pt();
            assert!(h > w);
        }
    }
}
