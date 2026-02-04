use anyhow::{Context, Result};
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::parse::{parse_jpeg_header, parse_png_header, PageSize, PngInfo};

/// pre-processed image data ready for PDF insertion
enum PreparedImage {
    Jpeg {
        width: u32,
        height: u32,
        components: u8,
        data: Vec<u8>,
    },
    PngPassthrough {
        info: PngInfo,
    },
    /// decoded pixel data compressed with deflate
    Compressed {
        width: u32,
        height: u32,
        color_channels: u8,
        color_compressed: Vec<u8>,
        alpha_compressed: Option<Vec<u8>>,
    },
}

fn prepare_image(path: &Path) -> Result<PreparedImage> {
    let data = std::fs::read(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    anyhow::ensure!(data.len() >= 4, "File too small: {}", path.display());

    // JPEG: passthrough
    if data[0] == 0xFF && data[1] == 0xD8 {
        let (w, h, components) = parse_jpeg_header(&data)
            .with_context(|| format!("Failed to parse JPEG header: {}", path.display()))?;
        anyhow::ensure!(
            matches!(components, 1 | 3 | 4),
            "Unsupported JPEG component count {} in {}",
            components,
            path.display()
        );
        return Ok(PreparedImage::Jpeg {
            width: w,
            height: h,
            components,
            data,
        });
    }

    // PNG: passthrough for opaque, decode+split for alpha
    if data.len() >= 8 && data[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
        let info = parse_png_header(&data)
            .with_context(|| format!("Failed to parse PNG header: {}", path.display()))?;

        match info.color_type {
            0 | 2 | 3 => {
                if info.color_type == 3 {
                    anyhow::ensure!(
                        !info.plte_data.is_empty(),
                        "PNG palette image missing PLTE chunk: {}",
                        path.display()
                    );
                }
                return Ok(PreparedImage::PngPassthrough { info });
            }
            4 | 6 => {
                return decode_alpha_png(&data, &info, path);
            }
            _ => anyhow::bail!(
                "Unsupported PNG color type {} in {}",
                info.color_type,
                path.display()
            ),
        }
    }

    // generic image formats (TIFF, BMP, GIF, etc.) decode via image crate
    decode_generic_image(&data, path)
}

/// decode a PNG with alpha channel, split color+alpha, compress separately
fn decode_alpha_png(data: &[u8], info: &PngInfo, path: &Path) -> Result<PreparedImage> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("Failed to decode PNG: {}", path.display()))?;
    let buf_size = reader
        .output_buffer_size()
        .context("PNG output buffer size unknown")?;
    let mut buf = vec![0u8; buf_size];
    let output_info = reader
        .next_frame(&mut buf)
        .with_context(|| format!("Failed to read PNG frame: {}", path.display()))?;
    let pixels = &buf[..output_info.buffer_size()];

    let color_channels: usize = if info.color_type == 4 { 1 } else { 3 };
    let total_channels = color_channels + 1;
    let pixel_count = (info.width as usize) * (info.height as usize);

    // fused split + compress stream directly into zlib encoders
    let mut color_enc = ZlibEncoder::new(
        Vec::with_capacity(pixel_count * color_channels / 2),
        Compression::fast(),
    );
    let mut alpha_enc = ZlibEncoder::new(
        Vec::with_capacity(pixel_count / 2),
        Compression::fast(),
    );

    // process row-by-row for better cache locality
    let row_pixels = info.width as usize;
    let row_bytes = row_pixels * total_channels;
    for row in 0..info.height as usize {
        let row_start = row * row_bytes;
        let row_slice = &pixels[row_start..row_start + row_bytes];
        let mut color_row = Vec::with_capacity(row_pixels * color_channels);
        let mut alpha_row = Vec::with_capacity(row_pixels);
        for px in 0..row_pixels {
            let base = px * total_channels;
            color_row.extend_from_slice(&row_slice[base..base + color_channels]);
            alpha_row.push(row_slice[base + color_channels]);
        }
        color_enc.write_all(&color_row)?;
        alpha_enc.write_all(&alpha_row)?;
    }

    let color_compressed = color_enc.finish()?;
    let alpha_compressed = alpha_enc.finish()?;

    Ok(PreparedImage::Compressed {
        width: info.width,
        height: info.height,
        color_channels: color_channels as u8,
        color_compressed,
        alpha_compressed: Some(alpha_compressed),
    })
}

/// decode any image format via image crate and compress for PDF embedding
fn decode_generic_image(data: &[u8], path: &Path) -> Result<PreparedImage> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    use image::GenericImageView;
    let img = image::load_from_memory(data)
        .with_context(|| format!("Failed to decode image: {}", path.display()))?;
    let (width, height) = img.dimensions();

    let has_alpha = img.color().has_alpha();
    if has_alpha {
        let rgba = img.into_rgba8();
        let pixels = rgba.as_raw();
        let pixel_count = (width as usize) * (height as usize);

        let mut color_enc = ZlibEncoder::new(
            Vec::with_capacity(pixel_count * 3 / 2),
            Compression::fast(),
        );
        let mut alpha_enc = ZlibEncoder::new(
            Vec::with_capacity(pixel_count / 2),
            Compression::fast(),
        );

        for chunk in pixels.chunks_exact(4) {
            color_enc.write_all(&chunk[..3])?;
            alpha_enc.write_all(&chunk[3..4])?;
        }

        Ok(PreparedImage::Compressed {
            width,
            height,
            color_channels: 3,
            color_compressed: color_enc.finish()?,
            alpha_compressed: Some(alpha_enc.finish()?),
        })
    } else if img.color().channel_count() == 1 {
        let gray = img.into_luma8();
        let pixels = gray.as_raw();

        let mut enc = ZlibEncoder::new(
            Vec::with_capacity(pixels.len() / 2),
            Compression::fast(),
        );
        enc.write_all(pixels)?;

        Ok(PreparedImage::Compressed {
            width,
            height,
            color_channels: 1,
            color_compressed: enc.finish()?,
            alpha_compressed: None,
        })
    } else {
        let rgb = img.into_rgb8();
        let pixels = rgb.as_raw();

        let mut enc = ZlibEncoder::new(
            Vec::with_capacity(pixels.len() / 2),
            Compression::fast(),
        );
        enc.write_all(pixels)?;

        Ok(PreparedImage::Compressed {
            width,
            height,
            color_channels: 3,
            color_compressed: enc.finish()?,
            alpha_compressed: None,
        })
    }
}

pub fn merge_images(
    images: &[PathBuf],
    output: &Path,
    dpi: u32,
    quiet: bool,
    title: Option<&str>,
    author: Option<&str>,
    pagesize: Option<PageSize>,
) -> Result<()> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};

    if !quiet {
        eprintln!("Merging {} image(s) -> {}", images.len(), output.display());
    }
    let start = std::time::Instant::now();

    // phase 1 - parallel image processing (file I/O + decode + compress)
    let prepared: Vec<Result<PreparedImage>> = images
        .par_iter()
        .map(|path| prepare_image(path))
        .collect();

    // phase 2 - sequential PDF assembly
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut page_ids: Vec<Object> = Vec::with_capacity(images.len());

    for (i, result) in prepared.into_iter().enumerate() {
        let img = result?;
        let path = &images[i];

        let (img_width, img_height, image_id) = match img {
            PreparedImage::Jpeg {
                width,
                height,
                components,
                data,
            } => {
                let (color_space, decode) = match components {
                    1 => (Object::Name(b"DeviceGray".to_vec()), None),
                    3 => (Object::Name(b"DeviceRGB".to_vec()), None),
                    4 => (
                        Object::Name(b"DeviceCMYK".to_vec()),
                        Some(Object::Array(vec![
                            1.into(),
                            0.into(),
                            1.into(),
                            0.into(),
                            1.into(),
                            0.into(),
                            1.into(),
                            0.into(),
                        ])),
                    ),
                    _ => unreachable!(),
                };
                let mut dict = dictionary! {
                    "Type" => Object::Name(b"XObject".to_vec()),
                    "Subtype" => Object::Name(b"Image".to_vec()),
                    "Width" => width as i64,
                    "Height" => height as i64,
                    "ColorSpace" => color_space,
                    "BitsPerComponent" => 8,
                    "Filter" => Object::Name(b"DCTDecode".to_vec()),
                    "Length" => data.len() as i64,
                };
                if let Some(d) = decode {
                    dict.set("Decode", d);
                }
                (width, height, doc.add_object(Stream::new(dict, data)))
            }
            PreparedImage::PngPassthrough { info } => match info.color_type {
                0 | 2 => {
                    let channels: u8 = if info.color_type == 0 { 1 } else { 3 };
                    let color_space = if info.color_type == 0 {
                        Object::Name(b"DeviceGray".to_vec())
                    } else {
                        Object::Name(b"DeviceRGB".to_vec())
                    };
                    let decode_parms = dictionary! {
                        "Predictor" => 15,
                        "Colors" => channels as i64,
                        "BitsPerComponent" => info.bit_depth as i64,
                        "Columns" => info.width as i64,
                    };
                    let stream = Stream::new(
                        dictionary! {
                            "Type" => Object::Name(b"XObject".to_vec()),
                            "Subtype" => Object::Name(b"Image".to_vec()),
                            "Width" => info.width as i64,
                            "Height" => info.height as i64,
                            "ColorSpace" => color_space,
                            "BitsPerComponent" => info.bit_depth as i64,
                            "Filter" => Object::Name(b"FlateDecode".to_vec()),
                            "DecodeParms" => Object::Dictionary(decode_parms),
                            "Length" => info.idat_data.len() as i64,
                        },
                        info.idat_data,
                    );
                    (info.width, info.height, doc.add_object(stream))
                }
                3 => {
                    let num_entries = info.plte_data.len() / 3;
                    let color_space = Object::Array(vec![
                        Object::Name(b"Indexed".to_vec()),
                        Object::Name(b"DeviceRGB".to_vec()),
                        Object::Integer((num_entries - 1) as i64),
                        Object::String(info.plte_data, lopdf::StringFormat::Hexadecimal),
                    ]);
                    let decode_parms = dictionary! {
                        "Predictor" => 15,
                        "Colors" => 1_i64,
                        "BitsPerComponent" => info.bit_depth as i64,
                        "Columns" => info.width as i64,
                    };
                    let stream = Stream::new(
                        dictionary! {
                            "Type" => Object::Name(b"XObject".to_vec()),
                            "Subtype" => Object::Name(b"Image".to_vec()),
                            "Width" => info.width as i64,
                            "Height" => info.height as i64,
                            "ColorSpace" => color_space,
                            "BitsPerComponent" => info.bit_depth as i64,
                            "Filter" => Object::Name(b"FlateDecode".to_vec()),
                            "DecodeParms" => Object::Dictionary(decode_parms),
                            "Length" => info.idat_data.len() as i64,
                        },
                        info.idat_data,
                    );
                    (info.width, info.height, doc.add_object(stream))
                }
                _ => unreachable!(),
            },
            PreparedImage::Compressed {
                width,
                height,
                color_channels,
                color_compressed,
                alpha_compressed,
            } => {
                let color_space = if color_channels == 1 {
                    Object::Name(b"DeviceGray".to_vec())
                } else {
                    Object::Name(b"DeviceRGB".to_vec())
                };
                let image_stream = if let Some(alpha_data) = alpha_compressed {
                    let smask_stream = Stream::new(
                        dictionary! {
                            "Type" => Object::Name(b"XObject".to_vec()),
                            "Subtype" => Object::Name(b"Image".to_vec()),
                            "Width" => width as i64,
                            "Height" => height as i64,
                            "ColorSpace" => Object::Name(b"DeviceGray".to_vec()),
                            "BitsPerComponent" => 8,
                            "Filter" => Object::Name(b"FlateDecode".to_vec()),
                            "Length" => alpha_data.len() as i64,
                        },
                        alpha_data,
                    );
                    let smask_id = doc.add_object(smask_stream);
                    Stream::new(
                        dictionary! {
                            "Type" => Object::Name(b"XObject".to_vec()),
                            "Subtype" => Object::Name(b"Image".to_vec()),
                            "Width" => width as i64,
                            "Height" => height as i64,
                            "ColorSpace" => color_space,
                            "BitsPerComponent" => 8,
                            "Filter" => Object::Name(b"FlateDecode".to_vec()),
                            "SMask" => smask_id,
                            "Length" => color_compressed.len() as i64,
                        },
                        color_compressed,
                    )
                } else {
                    Stream::new(
                        dictionary! {
                            "Type" => Object::Name(b"XObject".to_vec()),
                            "Subtype" => Object::Name(b"Image".to_vec()),
                            "Width" => width as i64,
                            "Height" => height as i64,
                            "ColorSpace" => color_space,
                            "BitsPerComponent" => 8,
                            "Filter" => Object::Name(b"FlateDecode".to_vec()),
                            "Length" => color_compressed.len() as i64,
                        },
                        color_compressed,
                    )
                };
                (width, height, doc.add_object(image_stream))
            }
        };

        // page dimensions
        let (page_w_pts, page_h_pts, img_w_pts, img_h_pts, x_off, y_off) =
            if let Some(ps) = pagesize {
                // Scale image to fit within the page, maintaining aspect ratio
                let (pw, ph) = ps.dimensions_pt();
                let img_w = img_width as f32 * 72.0 / dpi as f32;
                let img_h = img_height as f32 * 72.0 / dpi as f32;
                let scale = (pw / img_w).min(ph / img_h);
                let w = img_w * scale;
                let h = img_h * scale;
                (pw, ph, w, h, (pw - w) / 2.0, (ph - h) / 2.0)
            } else {
                // Page sized to image at given DPI
                let w = img_width as f32 * 72.0 / dpi as f32;
                let h = img_height as f32 * 72.0 / dpi as f32;
                (w, h, w, h, 0.0, 0.0)
            };

        // content stream, scale image to page size and draw
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        Object::Real(img_w_pts),
                        Object::Integer(0),
                        Object::Integer(0),
                        Object::Real(img_h_pts),
                        Object::Real(x_off),
                        Object::Real(y_off),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content
                .encode()
                .context("Failed to encode content stream")?,
        ));

        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! {
                "Im0" => image_id,
            },
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => Object::Name(b"Page".to_vec()),
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(page_w_pts), Object::Real(page_h_pts)],
            "Contents" => content_id,
            "Resources" => resources_id,
        });
        page_ids.push(page_id.into());

        if !quiet {
            eprintln!("  [{}/{}] {}", i + 1, images.len(), path.display());
        }
    }

    // build pages tree
    let count = page_ids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => Object::Name(b"Pages".to_vec()),
            "Kids" => page_ids,
            "Count" => count,
        }),
    );

    // catalog
    let catalog_id = doc.add_object(dictionary! {
        "Type" => Object::Name(b"Catalog".to_vec()),
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    // PDF metadata
    if title.is_some() || author.is_some() {
        let mut info_dict = lopdf::Dictionary::new();
        if let Some(t) = title {
            info_dict.set(
                "Title",
                Object::String(t.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            );
        }
        if let Some(a) = author {
            info_dict.set(
                "Author",
                Object::String(a.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            );
        }
        let info_id = doc.add_object(Object::Dictionary(info_dict));
        doc.trailer.set("Info", info_id);
    }

    // write output
    let to_stdout = output == Path::new("-");
    if to_stdout {
        let stdout = std::io::stdout();
        let mut out = std::io::BufWriter::new(stdout.lock());
        doc.save_to(&mut out)
            .context("Failed to write PDF to stdout")?;
    } else {
        doc.save(output)
            .with_context(|| format!("Failed to save {}", output.display()))?;
    }

    if !quiet {
        let elapsed = start.elapsed();
        eprintln!("Done. PDF saved in {:.2}s", elapsed.as_secs_f64());
    }
    Ok(())
}
