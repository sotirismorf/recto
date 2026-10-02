# Recto

**The powerful book scanning post-processor**

Recto is a desktop application for post-processing scanned book pages. It provides an intuitive graphical workflow to import, crop, color-correct, and export scanned images to various formats.

## Features

- **Import** — Load scanned pages via drag & drop or file picker. Supports JPEG, PNG, TIFF, WebP, and BMP. EXIF orientation is automatically detected and corrected.
- **Crop** — Interactive crop tool with auto-detected presets based on image dimensions. Color-coded presets make it easy to handle different page sizes and orientations.
- **Color correction** — Exposure, black level, white balance (temperature/tint) and saturation, applied in linear light with a live preview.
- **Export** — Output to PNG, JPEG, TIFF, or PDF. PDF export embeds pages as JPEG or lossless, with configurable metadata, DPI and physical page height (optionally estimated from camera EXIF).
- **Projects** — Save and restore your work with `.recto` project files.

## Screenshots

<!-- TODO: add screenshots -->

## Installation

### Flatpak (recommended)

Flatpak is the recommended installation method. The package is built from the GNOME runtime.

```bash
flatpak-builder --user --install --force-clean build-dir flatpak/io.github.sotirismorf.Recto.json
```

### Build from source

#### Requirements

- Rust toolchain (stable, 1.88+)
- GTK 4.14+ and libadwaita 1.5+ development libraries
- OpenCV 5 development libraries (page auto-detection), plus clang/libclang
  for the `opencv` crate's bindings

#### Build

```bash
git clone https://github.com/sotirismorf/recto.git
cd recto
cargo build --release
```

The binary will be at `target/release/recto`.

## Usage

Launch the application:

```bash
recto
```

Or open a project, or start a new one from images:

```bash
recto path/to/project.recto
recto scans/*.jpg
```

### Workflow

1. **Start** — Drop images onto the landing page or click it to choose files.
2. **Arrange** — Reorder, rotate and remove pages.
3. **Crop** — Auto-detect pages or define crop presets and adjust each page's crop.
4. **Color** — Correct exposure, black level and white balance with a live preview.
5. **Export** — Choose format, output directory, and export your processed pages.

## Project structure

```
crates/
├── core/          # recto-core library (pipeline, project persistence, geometry, EXIF)
└── ui/            # recto-ui binary (GTK4 + libadwaita application)
```

## License

[GPL-3.0-or-later](LICENSE)
