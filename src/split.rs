use anyhow::{Context, Result};
use rayon::prelude::*;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::parse::{parse_page_ranges, ImageFormat, PngCompression};

fn encode_png(
    data: &[u8],
    width: u32,
    height: u32,
    gray: bool,
    compress: PngCompression,
    writer: impl Write,
) -> Result<()> {
    let writer = std::io::BufWriter::new(writer);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(if gray {
        png::ColorType::Grayscale
    } else {
        png::ColorType::Rgb
    });
    encoder.set_depth(png::BitDepth::Eight);

    // set compression and filter based on level:
    // - fast: fastest encoding, larger files (fdeflate + Paeth)
    // - small: smaller files, slower encoding (zlib + NoFilter)
    match compress {
        PngCompression::Fast => {
            encoder.set_compression(png::Compression::Fast);
            encoder.set_filter(png::Filter::Paeth);
        }
        PngCompression::Small => {
            encoder.set_compression(png::Compression::Balanced);
            encoder.set_filter(png::Filter::NoFilter);
        }
    }

    let mut writer = encoder
        .write_header()
        .context("Failed to write PNG header")?;
    writer
        .write_image_data(data)
        .context("Failed to encode PNG data")?;
    Ok(())
}

fn encode_jpg(
    data: &[u8],
    width: u32,
    height: u32,
    gray: bool,
    quality: u8,
    mut writer: impl Write,
) -> Result<()> {
    let pixel_format = if gray {
        turbojpeg::PixelFormat::GRAY
    } else {
        turbojpeg::PixelFormat::RGB
    };
    let image = turbojpeg::Image {
        pixels: data,
        width: width as usize,
        height: height as usize,
        pitch: width as usize * if gray { 1 } else { 3 },
        format: pixel_format,
    };
    let mut compressor = turbojpeg::Compressor::new()?;
    compressor.set_quality(quality as i32)?;
    compressor.set_subsamp(if gray {
        turbojpeg::Subsamp::Gray
    } else {
        turbojpeg::Subsamp::Sub2x2
    })?;
    let jpeg_data = compressor.compress_to_vec(image)?;
    writer.write_all(&jpeg_data)?;
    Ok(())
}

pub fn split_pdf(
    input: &Path,
    output_dir: &Path,
    format: ImageFormat,
    dpi: u32,
    compress: PngCompression,
    gray: bool,
    pages: Option<&str>,
    quality: u8,
    quiet: bool,
) -> Result<()> {
    let input_str = input.to_str().context("Invalid path")?.to_string();
    let num_pages = {
        let doc = mupdf::Document::open(&input_str)?;
        doc.page_count()?
    };

    let page_indices: Vec<i32> = match pages {
        Some(s) => parse_page_ranges(s, num_pages)?,
        None => (0..num_pages).collect(),
    };
    let total = page_indices.len();

    let to_stdout = output_dir == Path::new("-");

    // render single page and write to stdout
    if to_stdout {
        anyhow::ensure!(
            total == 1,
            "Stdout output requires exactly one page (got {}). Use --pages to select one.",
            total
        );
        let page_idx = page_indices[0];
        let doc = mupdf::Document::open(&input_str)?;
        let page = doc.load_page(page_idx)?;
        let scale = dpi as f32 / 72.0;
        let matrix = mupdf::Matrix::new_scale(scale, scale);
        let colorspace = if gray {
            mupdf::Colorspace::device_gray()
        } else {
            mupdf::Colorspace::device_rgb()
        };
        let pixmap = page.to_pixmap(&matrix, &colorspace, false, true)?;
        let width = pixmap.width();
        let height = pixmap.height();
        let stdout = std::io::stdout();
        let out = stdout.lock();
        match format {
            ImageFormat::Png => {
                encode_png(pixmap.samples(), width, height, gray, compress, out)?;
            }
            ImageFormat::Jpg => {
                encode_jpg(pixmap.samples(), width, height, gray, quality, out)?;
            }
        }
        return Ok(());
    }

    // dir output
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Cannot create output dir: {}", output_dir.display()))?;

    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("page")
        .to_string();

    let ext = match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpg => "jpg",
    };

    if !quiet {
        if pages.is_some() {
            eprintln!(
                "Splitting {} ({} of {} page{}) at {} DPI -> {}",
                input.display(),
                total,
                num_pages,
                if num_pages == 1 { "" } else { "s" },
                dpi,
                output_dir.display()
            );
        } else {
            eprintln!(
                "Splitting {} ({} page{}) at {} DPI -> {}",
                input.display(),
                num_pages,
                if num_pages == 1 { "" } else { "s" },
                dpi,
                output_dir.display()
            );
        }
    }

    let start = std::time::Instant::now();
    let done_count = AtomicUsize::new(0);

    // divide pages into N chunks; each chunk is one rayon task that opens
    // MuPDF Document once and processes its pages sequentially
    // chunk count bounds concurrency (and thus peak memory)
    let num_workers = rayon::current_num_threads();
    let chunk_size = (page_indices.len() + num_workers - 1) / num_workers;

    let errors: Vec<_> = page_indices
        .chunks(chunk_size)
        .par_bridge()
        .flat_map(|chunk| {
            let doc = mupdf::Document::open(&input_str)
                .unwrap_or_else(|e| panic!("Failed to open {}: {}", input_str, e));
            chunk
                .iter()
                .filter_map(|&i| {
                    let result: Result<()> = (|| {
                        let page = doc.load_page(i)?;

                        let scale = dpi as f32 / 72.0;
                        let matrix = mupdf::Matrix::new_scale(scale, scale);
                        let colorspace = if gray {
                            mupdf::Colorspace::device_gray()
                        } else {
                            mupdf::Colorspace::device_rgb()
                        };
                        let pixmap = page.to_pixmap(&matrix, &colorspace, false, true)?;

                        let width = pixmap.width();
                        let height = pixmap.height();
                        let filename = format!("{}_{:04}.{}", stem, i + 1, ext);
                        let out_path = output_dir.join(&filename);

                        match format {
                            ImageFormat::Png => {
                                let file = std::fs::File::create(&out_path).with_context(
                                    || format!("Failed to create {}", out_path.display()),
                                )?;
                                encode_png(
                                    pixmap.samples(),
                                    width,
                                    height,
                                    gray,
                                    compress,
                                    file,
                                )?;
                            }
                            ImageFormat::Jpg => {
                                let file = std::fs::File::create(&out_path).with_context(
                                    || format!("Failed to create {}", out_path.display()),
                                )?;
                                let out = std::io::BufWriter::new(file);
                                encode_jpg(
                                    pixmap.samples(),
                                    width,
                                    height,
                                    gray,
                                    quality,
                                    out,
                                )?;
                            }
                        }

                        if !quiet {
                            let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
                            eprintln!("  [{}/{}] {}", done, total, filename);
                        }
                        Ok(())
                    })();

                    result.err().map(|e| (i, e))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if !errors.is_empty() {
        let count = errors.len();
        for &(page, ref err) in &errors {
            eprintln!("  error: page {}: {}", page + 1, err);
        }
        let (page, err) = errors.into_iter().next().unwrap();
        return Err(err.context(format!(
            "Failed on page {} ({} total error{})",
            page + 1,
            count,
            if count == 1 { "" } else { "s" }
        )));
    }

    if !quiet {
        let elapsed = start.elapsed();
        eprintln!(
            "Done. {} images in {:.2}s",
            total,
            elapsed.as_secs_f64()
        );
    }
    Ok(())
}
