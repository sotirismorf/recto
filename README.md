# Recto

**The powerful book scanning post-processor**

Recto is a desktop application for post-processing scanned book pages. It provides an intuitive graphical workflow to import, crop, color-correct, and export scanned images to various formats.

## Features

- **Import** — Load scanned pages via drag & drop or file picker. Supports JPEG, PNG, TIFF, WebP, and BMP. EXIF orientation is automatically detected and corrected.
- **Crop** — Interactive crop tool with auto-detected presets based on image dimensions. Color-coded presets make it easy to handle different page sizes and orientations.
- **Color adjustment** — Adjust brightness and contrast with live preview rendering.
- **Export** — Output to PNG, JPEG, TIFF, or PDF. PDF export includes configurable metadata (title, author, keywords) and DPI settings.
- **Projects** — Save and restore your work with `.pcut` project files.

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

- Rust toolchain (stable, 1.75+)
- GTK4 development libraries (`libgtk-4-dev`)
- libadwaita development libraries

#### Build

```bash
git clone https://github.com/sotirismorf/pagecutter2.git
cd pagecutter2
cargo build --release
```

The binary will be at `target/release/recto`.

#### Optional: OpenCV for auto-crop detection

Auto-crop detection is powered by OpenCV and is a standard feature of the app.

## Usage

Launch the application:

```bash
recto
```

Or open a project directly:

```bash
recto --project path/to/project.pcut
```

### Workflow

1. **Start** — Drop images onto the landing page or use the toolbar to add them.
2. **Import** — Review and rotate imported pages as needed.
3. **Crop** — Define crop presets and adjust crop rectangles per page.
4. **Colors** — Fine-tune brightness and contrast with live preview.
5. **Export** — Choose format, output directory, and export your processed pages.

## Project structure

```
crates/
├── core/          # recto-core library (pipeline, project persistence, geometry, EXIF)
└── ui/            # recto-ui binary (GTK4 + libadwaita application)
```

## License

[GPL-3.0-or-later](LICENSE)
