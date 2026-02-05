#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod merge;
mod parse;
mod split;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use std::path::{Path, PathBuf};

use parse::{ImageFormat, PageSize, PngCompression};

#[derive(Parser)]
#[command(name = "ovid", version, about = "Lightning-fast PDF / Image converter")]
struct Cli {
    /// num parallel threads (default number of CPUs)
    #[arg(short = 'j', long, global = true)]
    threads: Option<usize>,

    /// suppress progress output
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// convert PDF pages to images (PNG or JPG)
    Split {
        /// input PDF file
        input: PathBuf,

        /// output dir (default next to input file), or "-" for stdout (single page only)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// image format
        #[arg(short, long, default_value = "png")]
        format: ImageFormat,

        /// rendering DPI (72-2400)
        #[arg(short, long, default_value_t = 300, value_parser = clap::value_parser!(u32).range(72..=2400))]
        dpi: u32,

        /// PNG compression: fast (speed) or small (filesize)
        #[arg(short, long, default_value = "fast")]
        compress: PngCompression,

        /// render in grayscale
        #[arg(long)]
        gray: bool,

        /// page selection (e.g. "1", "1,3-5,10")
        #[arg(short, long)]
        pages: Option<String>,

        /// JPEG quality (1-100)
        #[arg(long, default_value_t = 75, value_parser = clap::value_parser!(u8).range(1..=100))]
        quality: u8,
    },
    /// combine images into a single PDF
    Merge {
        /// input image files or dirs (png, jpg, tiff, bmp, gif)
        images: Vec<PathBuf>,

        /// output PDF path, "-" for stdout
        #[arg(short, long, default_value = "output.pdf")]
        output: PathBuf,

        /// DPI of input images, used for page sizing (72-2400)
        #[arg(short, long, default_value_t = 300, value_parser = clap::value_parser!(u32).range(72..=2400))]
        dpi: u32,

        /// PDF title metadata
        #[arg(long)]
        title: Option<String>,

        /// PDF author metadata
        #[arg(long)]
        author: Option<String>,

        /// page size (overrides DPI-based sizing, scales image to fit)
        #[arg(long)]
        pagesize: Option<PageSize>,
    },
    /// generate shell completions
    Completions {
        /// shell to generate completions for
        shell: clap_complete::Shell,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(threads) = cli.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .context("Failed to configure thread pool")?;
    }

    let quiet = cli.quiet;

    match cli.command {
        Commands::Split {
            input,
            output,
            format,
            dpi,
            compress,
            gray,
            pages,
            quality,
        } => {
            let output_dir = output.unwrap_or_else(|| {
                input
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            });
            split::split_pdf(
                &input,
                &output_dir,
                format,
                dpi,
                compress,
                gray,
                pages.as_deref(),
                quality,
                quiet,
            )?;
        }
        Commands::Merge {
            images,
            output,
            dpi,
            title,
            author,
            pagesize,
        } => {
            let images = parse::expand_image_paths(&images)?;
            anyhow::ensure!(!images.is_empty(), "No input images provided");
            merge::merge_images(
                &images,
                &output,
                dpi,
                quiet,
                title.as_deref(),
                author.as_deref(),
                pagesize,
            )?;
        }
        Commands::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "ovid",
                &mut std::io::stdout(),
            );
        }
    }

    Ok(())
}
