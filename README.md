<h1 align="center">ovid</h1>

<p align="center"><i>"And our bodies themselves are always, restlessly, changing: we shall not be, tomorrow, what we were, or what we are." — Metamorphoses</i></p>

Fast, bidirectional PDF to image converter and image to PDF merger. A replacement for most pdf and image CLI libraries.

## Install

**Pre-built binaries (macOS & Linux):**
```bash
curl -fsSL https://raw.githubusercontent.com/euceph/ovid/main/install.sh | sh
```

**Cargo:**
```bash
cargo install ovid
```

<details>
<summary>Build from source</summary>

**macOS:**
```bash
brew install nasm jpeg-turbo
cargo install --path .
```

**Linux (Debian/Ubuntu):**
```bash
apt install cmake nasm libclang-dev libfontconfig1-dev libjpeg-turbo8-dev pkg-config
cargo install --path .
```
</details>

## Usage

### Split - PDF to images

```bash
# All pages to PNG at 300 DPI (default)
ovid split document.pdf

# Specific pages, JPG format, custom DPI
ovid split document.pdf -f jpg -d 150 --pages 1,3-5

# Grayscale output
ovid split document.pdf --gray

# Single page to stdout (pipe to other tools)
ovid split document.pdf -o - --pages 3 > page3.png

# Smaller PNG files (~2.5x smaller, ~20% slower)
ovid split document.pdf -c small

# JPEG quality control
ovid split document.pdf -f jpg --quality 90
```

### Merge - images to PDF

```bash
# Merge images into a PDF
ovid merge page1.png page2.png -o output.pdf

# Merge a whole directory of images
ovid merge ./scanned_pages/ -o combined.pdf

# Set page size (scales images to fit, centered)
ovid merge photos/*.jpg -o album.pdf --pagesize a4

# Add PDF metadata
ovid merge slides/*.png -o presentation.pdf --title "My Slides" --author "Jane Doe"

# Supports PNG, JPEG, TIFF, BMP, and GIF
ovid merge scan.tiff photo.bmp diagram.gif -o mixed.pdf

# Write PDF to stdout
ovid merge *.png -o - > output.pdf
```

### Options

```
-j, --threads <N>    Number of parallel threads (default: all CPUs)
-q, --quiet          Suppress progress output
-d, --dpi <DPI>      Rendering/sizing DPI, 72-2400 (default: 300)
```

### Shell completions

```bash
# Bash
ovid completions bash > /etc/bash_completion.d/ovid

# Zsh
ovid completions zsh > ~/.zfunc/_ovid

# Fish
ovid completions fish > ~/.config/fish/completions/ovid.fish
```

## Performance

### Split (PDF to Images)

#### macOS — Apple M3 Pro

**150-page PDF, 300 DPI:**

| Tool | PNG | JPG |
|------|-----|-----|
| **ovid** | **0.44s** | **0.55s** |
| mutool | 12.5s | — |
| Ghostscript | 19.6s | 2.78s |
| pdftoppm | 74.7s | 3.44s |

**Speedups (150pg, 300 DPI):**

| Format | vs mutool | vs Ghostscript | vs pdftoppm |
|--------|-----------|----------------|-------------|
| PNG | **28x** | **44x** | **169x** |
| JPG | — | **5.1x** | **6.3x** |

**Scaling across document sizes (ovid only):**

| Config | 15-page | 50-page | 150-page |
|--------|---------|---------|----------|
| PNG 150dpi | 0.13s | 0.24s | 0.36s |
| PNG 300dpi | 0.19s | 0.29s | 0.46s |
| JPG 150dpi | 0.13s | 0.23s | 0.37s |
| JPG 300dpi | 0.19s | 0.29s | 0.43s |

#### Linux — 2-core x86_64 VPS

**150-page PDF, 300 DPI:**

| Tool | PNG | JPG |
|------|-----|-----|
| **ovid** | **1.44s** | **1.89s** |
| mutool | 15.7s | — |
| Ghostscript | 25.2s | 2.89s |
| pdftoppm | 85.8s | 4.12s |

**Speedups (150pg, 300 DPI):**

| Format | vs mutool | vs Ghostscript | vs pdftoppm |
|--------|-----------|----------------|-------------|
| PNG | **11x** | **18x** | **61x** |
| JPG | — | **1.6x** | **2.2x** |

### Merge (Images to PDF)

**50 images, 300 DPI:**

| Config | ovid | img2pdf | ImageMagick |
|--------|------|---------|-------------|
| PNG | **38ms** | 3.03s | 11.15s |
| JPG | **14ms** | 115ms | 4.65s |

## Contributions

Contributions are, of course, always welcome and encouraged. Please make sure they're grounded in reality.

## License

MIT
