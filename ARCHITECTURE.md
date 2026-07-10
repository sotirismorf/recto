# Architecture

> A guideline, not a rulebook. Prefer clarity over dogma.

## Design Philosophy

- **Pure transforms, impure edges.** Transforms are plain functions that take
  ownership of an image and return a new one. File I/O, GTK, and OpenCV live at
  the boundary and never leak inward.
- **Commands, not setters.** Every state mutation flows through an explicit
  `Command`. No `project.brightness = 0.5` called from a random signal handler.
- **Types over comments.** Domain invariants live in the type system. A
  `Rotation` newtype enforces `0|90|180|270` at construction — no scattered
  `debug_assert!` calls.
- **Plain functions over trait objects.** A trait with one implementation is an
  indirection tax. Rust's module system provides the namespacing; plain
  functions provide the dispatch (zero-cost, inlinable, readable).
- **Snapshot undo.** For a domain where the aggregate root (`Project`) is ~KB of
  paths and settings, clone-on-mutation is correct and simple. The actual image
  data is never stored in the project.
- **One worker, one channel shape.** All CPU-bound work is dispatched through a
  single `Worker` abstraction that owns one `std::thread`. Tasks use `rayon`
  internally for data parallelism.
- **Views compose widgets; widgets are passive.** Widgets receive data through
  setter methods and emit signals. They never import `AppState`. Views wire
  widget signals to `Command` dispatches and `AppEvent` subscriptions.

---

## Crate Layout

```
crates/
├── core/                       # recto-core — pure domain + transforms + I/O adapters
│   ├── lib.rs                  #   public API re-exports
│   ├── error.rs                #   Error enum, Result alias
│   │
│   ├── domain/                 #   pure data types — zero I/O deps
│   │   ├── mod.rs
│   │   ├── project.rs          #     Project (aggregate root), Page (entity)
│   │   ├── crop.rs             #     CropBox, CropPreset
│   │   ├── export.rs           #     ExportSettings, OutputSize
│   │   ├── geometry.rs         #     Rect, aspect-ratio math, outlier detection
│   │   └── values.rs           #     newtype wrappers: Rotation, Brightness, Contrast, JpegQuality, Dpi
│   │
│   ├── command/                #   CQRS-lite: every mutation is a Command
│   │   ├── mod.rs              #     Command enum (~14 variants)
│   │   ├── state.rs            #     AppState (owns Project, undo/redo stacks, event tx)
│   │   └── event.rs            #     AppEvent enum
│   │
│   ├── transform/              #   pure image transforms — one plain fn per file
│   │   ├── mod.rs              #     transform_page() orchestrator
│   │   ├── context.rs          #     PipelineContext (immutable snapshot)
│   │   ├── load.rs             #     load image from disk
│   │   ├── exif.rs             #     EXIF orientation correction (the image transform)
│   │   ├── rotate.rs           #     apply_rotation
│   │   ├── crop.rs             #     apply_crop
│   │   ├── color.rs            #     apply_colors (rayon par_iter over pixels)
│   │   └── resize.rs           #     apply_resize (Lanczos3)
│   │
│   ├── export/                 #   thin orchestrators over transforms
│   │   ├── mod.rs
│   │   ├── batch.rs            #     run_batch (PNG/JPEG/TIFF per-page export)
│   │   └── pdf.rs              #     export_to_pdf (single-file PDF with JPEG streams)
│   │
│   └── io/                     #   filesystem adapters — the only module that touches disks
│       ├── mod.rs
│       ├── atomic.rs           #     write_atomic (tempfile + persist)
│       ├── image.rs            #     load, save_png, save_jpeg, save_tiff
│       ├── exif.rs             #     EXIF reading from file (kamadak-exif) — I/O concern
│       ├── project.rs          #     save_project, load_project, schema migration
│       └── config.rs           #     PdfMeta persistence (~/.config/recto/)
│
└── ui/                         # recto-ui — GTK4 + libadwaita
    ├── main.rs                 #   entry point, CLI (clap), adw::Application setup
    ├── window.rs               #   main window builder: header bar, actions, save/load
    ├── types.rs                #   PageId, RequestId, AppError — shared across all UI
    ├── worker.rs               #   Worker — single std::thread + async_channel
    │
    ├── session/                #   application-level session wiring
    │   ├── mod.rs              #     State alias (Rc<AppState>), MarkDirty, new_state(), new_store()
    │   └── session.rs          #     Session GObject (path, dirty flag, display name)
    │
    ├── views/                  #   full-page views (Gtk::Stack children)
    │   ├── mod.rs              #     re-exports, PanedSync
    │   ├── start.rs            #     landing page: brand, logo, drop area, "Open" button
    │   ├── workspace.rs        #     main editing area (sidebar + preview + thumbnail grid)
    │   ├── workspace/          #     workspace sub-modules
    │   │   ├── mod.rs
    │   │   ├── css.rs          #       sidebar + preset chip CSS providers
    │   │   └── helpers.rs      #       make_mode_button, make_section, rotate_selected
    │   └── export.rs           #     export dialog: format toggles, quality, PDF metadata
    │
    └── widgets/                #   reusable GTK components — passive, receive data, emit signals
        ├── mod.rs
        ├── page_item.rs        #     PageItem GObject (thumbnail, filename, rotation)
        ├── thumbnail_grid.rs   #     grid factory: overlay_factory, build_grid_scroll
        ├── thumbnail_loader.rs #     ThumbData, LoadMsg, loading helpers
        ├── preview_canvas.rs   #     custom gtk::Widget: GPU texture rendering, zoom/pan
        ├── zoom_pan.rs         #     ZoomPanController: scroll zoom, drag pan, fit
        ├── crop_picker.rs      #     interactive crop selection: Handle enum, drag-to-crop
        ├── crop_overlay.rs     #     draw_crop_overlay on thumbnails
        ├── preset_chips.rs     #     preset chip flowbox (data in via setter, signals out)
        └── color_preview.rs    #     ColorReq/ColorResult types, channel wiring
```

---

## Layer Architecture

```
 ┌──────────────────────────────────────────────────────┐
 │                    UI (recto-ui)                      │
 │  Views ──► Widgets      Worker      Session           │
 │    │          │             │            │             │
 │    │    emit signals    spawn(f)    path + dirty      │
 │    ▼          ▼             ▼            ▼             │
 │  dispatch(Command)    composables    async_channel    │
 ├──────────────────────────┬───────────────────────────┤
 │           CORE           │                            │
 │  ┌────────────┐  ┌───────┴──────┐  ┌───────────┐    │
 │  │   domain   │  │  transform   │  │    io      │    │
 │  │            │  │              │  │            │    │
 │  │ Project    │  │ context.rs   │  │ atomic.rs  │    │
 │  │ Page       │  │ rotate.rs    │  │ image.rs   │    │
 │  │ CropBox    │  │ crop.rs      │  │ project.rs │    │
 │  │ values.rs  │  │ color.rs     │  │ exif.rs    │    │
 │  │ geometry   │  │ resize.rs    │  │ config.rs  │    │
 │  └────────────┘  └──────────────┘  └───────────┘    │
 │                                            │          │
 │  ┌────────────┐  ┌──────────────┐         │          │
 │  │  command   │  │   export     │◄────────┘          │
 │  │            │  │              │                     │
 │  │ Command    │  │ batch.rs     │    Dependency       │
 │  │ AppState   │  │ pdf.rs       │    direction:       │
 │  │ AppEvent   │  └──────────────┘    io ──► transform │
 │  └────────────┘                      io ──► domain    │
 │                                      transform ──► domain
 │  Dependency direction:               command ──► domain
 │  io ──► export ──► transform ──► domain               │
 │  command ──► domain                  Never reversed.  │
 └──────────────────────────────────────────────────────┘
```

- **`domain`** depends on nothing external. Pure data.
- **`transform`** depends on `domain` and `io` (only `transform/load.rs` touches `io`).
- **`export`** depends on `transform`, `domain`, and `io`.
- **`command`** depends on `domain` (validates and applies mutations to it).
- **`io`** depends on `domain` for types, on crates (`image`, `pdf-writer`, `kamadak-exif`, `tempfile`) for implementations.
- **`ui`** depends on `core` for everything. Core never imports GTK.

---

## Domain Layer (`core::domain`)

Zero dependencies on I/O, GTK, or any external system. Pure data and invariants
enforced by **newtypes** — validity is established once, at construction.

### Newtype Wrappers (`values.rs`)

Every scalar domain value that carries a constraint gets its own type:

```rust
/// A valid rotation angle. Only 0, 90, 180, and 270 are representable.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rotation(u16);

impl Rotation {
    pub const ZERO:  Self = Self(0);
    pub const DEG90:  Self = Self(90);
    pub const DEG180: Self = Self(180);
    pub const DEG270: Self = Self(270);

    /// Normalize any angle to the nearest valid step.
    pub fn new(degrees: u16) -> Self {
        match degrees % 360 {
            0   => Self(0),
            90  => Self(90),
            180 => Self(180),
            _   => Self(270),
        }
    }

    pub fn as_degrees(self) -> u16 { self.0 }
}

/// Brightness in [-1.0, 1.0]. Clamped at construction.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brightness(f32);

impl Brightness {
    pub const ZERO: Self = Self(0.0);
    pub const MIN: Self = Self(-1.0);
    pub const MAX: Self = Self(1.0);

    pub fn new(val: f32) -> Self {
        Self(val.clamp(-1.0, 1.0))
    }

    pub fn as_f32(self) -> f32 { self.0 }
}

/// Contrast in [-1.0, 1.0]. Clamped at construction.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contrast(f32);

impl Contrast {
    pub const ZERO: Self = Self(0.0);

    pub fn new(val: f32) -> Self {
        Self(val.clamp(-1.0, 1.0))
    }

    pub fn as_f32(self) -> f32 { self.0 }
}

/// JPEG/PDF quality in [1, 100].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JpegQuality(u8);

impl JpegQuality {
    pub const DEFAULT: Self = Self(90);

    pub fn new(val: u8) -> Self {
        Self(val.clamp(1, 100))
    }

    pub fn as_u8(self) -> u8 { self.0 }
}

/// Dots per inch for PDF rendering. Clamped to [1, 2400].
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dpi(f64);

impl Dpi {
    pub const DEFAULT: Self = Self(300.0);

    pub fn new(val: f64) -> Self {
        Self(val.clamp(1.0, 2400.0))
    }

    pub fn as_f64(self) -> f64 { self.0 }
}
```

**Key property:** Once a `Rotation`, `Brightness`, etc. is constructed, it is
always valid. The rest of the codebase never checks bounds again.

### Aggregate Root (`project.rs`)

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub pages: Vec<Page>,
    pub crop_presets: Vec<CropPreset>,
    pub export: ExportSettings,
    pub output_dir: PathBuf,
    pub prefix: String,
    pub brightness: Brightness,
    pub contrast: Contrast,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub path: PathBuf,
    pub rotation: Rotation,
    pub crop: Option<CropBox>,
    pub crop_preset: Option<usize>,
    pub output: Option<OutputSize>,
}

impl Page {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            rotation: Rotation::ZERO,
            crop: None,
            crop_preset: None,
            output: None,
        }
    }
}
```

No `set_rotation(&mut self, u16)` with a `debug_assert!` — the type already
guarantees validity. `Page.rotation` is now a `Rotation`, not a raw `u16`.

### Other Domain Types (`crop.rs`, `export.rs`, `geometry.rs`)

```rust
// crop.rs
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropBox { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropPreset { pub name: String, pub w: u32, pub h: u32, pub locked: bool }

// export.rs
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputSize { pub w: u32, pub h: u32 }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
#[non_exhaustive]
pub enum ExportSettings {
    Png,
    Jpeg { quality: JpegQuality },
    Tiff,
    Pdf { quality: JpegQuality },
}

// geometry.rs
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
// plus output_dimensions, median_aspect_ratio, outlier_indices
```

### Design notes

- **`CropBox` stays as raw `u32` fields.** It's validated against image
  dimensions at pipeline time (the source image defines the bounds, not the
  domain). Not worth a newtype here.
- **`#[non_exhaustive]` on `ExportSettings`** prevents downstream crates from
  exhaustive matching without a wildcard arm — future-proof for adding formats.
- **Serde derive on newtypes** uses the inner type's serialization (e.g.
  `Brightness(0.5)` serializes as `0.5`). Add `#[serde(transparent)]` to
  newtypes that need it, though untagged `f32`/`u8`/`u16` serializes directly.

---

## Command & Event System (`core::command`)

Every mutation to the project is a `Command`. Commands are applied through
`AppState`, which:
1. Snapshots the current `Project` onto the undo stack.
2. Applies the command, mutating the project in-place.
3. Clears the redo stack (a new command invalidates the redo chain).
4. Emits `AppEvent` variants to subscribers via `async_channel`.

### Command enum

```rust
#[derive(Clone, Debug)]
pub enum Command {
    // Page mutations
    AddPages(Vec<PathBuf>),
    RemovePages(Vec<usize>),
    ReorderPages { from: usize, to: usize },
    SetRotation { index: usize, rotation: Rotation },
    SetCrop { index: usize, crop: Option<CropBox> },
    SetCropPreset { index: usize, preset: Option<usize> },
    SetOutputSize { index: usize, size: Option<OutputSize> },

    // Global mutations
    SetBrightness(Brightness),
    SetContrast(Contrast),
    SetExportSettings(ExportSettings),
    SetOutputDir(PathBuf),
    SetPrefix(String),

    // Preset mutations
    AddCropPreset(CropPreset),
    RemoveCropPreset(usize),
}
```

Note: `SetRotation` and `SetBrightness`/`SetContrast` now take newtype wrappers
instead of raw `u16`/`f32`. The caller (usually a GTK slider handler) constructs
the newtype at the boundary.

### AppEvent enum

```rust
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    PagesAdded(Vec<usize>),
    PagesRemoved(Vec<usize>),
    PageChanged(usize),
    GlobalSettingsChanged,
    PresetsChanged,
    ProjectLoaded,
    ProjectCleared,
    ProjectChanged,        // emitted on undo/redo (full state replacement)
}
```

### AppState

```rust
pub struct AppState {
    project: Project,
    undo_stack: Vec<Project>,    // snapshot-based, capped at MAX_UNDO (50)
    redo_stack: Vec<Project>,
    event_tx: async_channel::Sender<AppEvent>,
}

impl AppState {
    /// Executes a command, pushing the current state onto the undo stack.
    pub fn dispatch(&mut self, cmd: Command);

    /// Apply without recording undo (for internal sync).
    pub fn dispatch_without_undo(&mut self, cmd: Command);

    /// Replace the entire project (load / reset). Old project is pushed to undo.
    pub fn load_project(&mut self, project: Project);

    /// Reset to a default empty project.
    pub fn clear(&mut self);

    pub fn undo(&mut self) -> bool;
    pub fn redo(&mut self) -> bool;
    pub fn can_undo(&self) -> bool;
    pub fn can_redo(&self) -> bool;
}
```

### Why snapshot undo instead of command-inversion

The `Project` struct is ~KB of data (paths + small settings). Cloning it for
every mutation is negligible. Command-inversion (storing inverse `Command`s)
would require `InverseCommand` variants and per-command undo logic that is more
error-prone than cloning. Snapshot undo is:

- **Simple** — `push`, `pop`, `clone`, done.
- **Correct** — no chance of misapplied inverse operations.
- **Fast enough** — capped at 50 snapshots, each a few KB.

### How the UI dispatches commands

```rust
// In a view signal handler
brightness_slider.connect_value_changed(glib::clone!(@weak state => move |scale| {
    let val = Brightness::new(scale.value() as f32);
    state.dispatch(Command::SetBrightness(val));
}));
```

### How the UI subscribes to events

```rust
// In workspace view initialization
let event_rx = state.event_sender();   // clones the Sender
glib::spawn_future_local(async move {
    while let Ok(event) = event_rx.recv().await {
        match event {
            AppEvent::PageChanged(i) => {
                thumbnail_grid.update_row(i);
                request_arrange_preview();
            }
            AppEvent::GlobalSettingsChanged => {
                request_color_preview();
            }
            AppEvent::ProjectLoaded => {
                thumbnail_grid.rebuild_all(state.project());
            }
            _ => {}
        }
    }
});
```

---

## Transform Layer (`core::transform`)

Image processing is a six-step linear chain. Each step is a **plain function**
that takes ownership of a `DynamicImage` and returns a new one. No trait
objects, no dynamic dispatch, no allocations.

### PipelineContext (`context.rs`)

```rust
/// Immutable snapshot of all parameters for one page's transform.
/// Constructed before any work begins. Once created, project mutations
/// cannot affect an in-flight transform.
#[derive(Clone)]
pub struct PipelineContext<'a> {
    pub source_path: &'a Path,
    pub rotation: Rotation,
    pub crop: Option<CropBox>,
    pub output: Option<OutputSize>,
    pub brightness: Brightness,
    pub contrast: Contrast,
}

impl<'a> PipelineContext<'a> {
    pub fn from_page(project: &'a Project, page: &'a Page) -> Self {
        Self {
            source_path: &page.path,
            rotation: page.rotation,
            crop: page.crop,
            output: page.output,
            brightness: project.brightness,
            contrast: project.contrast,
        }
    }
}
```

### Orchestrator (`mod.rs`)

```rust
/// Full export pipeline: Load → EXIF → Rotate → Crop → Color → Resize.
pub fn transform_page(ctx: &PipelineContext) -> Result<DynamicImage> {
    let img = load::load_image(ctx.source_path)?;
    let img = exif::correct_orientation(img, ctx.source_path);
    let img = rotate::apply(img, ctx.rotation);
    let img = crop::apply(img, ctx.crop);
    let img = color::apply(img, ctx.brightness, ctx.contrast);
    let img = resize::apply(img, ctx.output);
    Ok(img)
}

/// Lightweight preview pipeline: Load → EXIF → Rotate → Crop → Color.
/// Skips resize (preview canvas handles scaling) and limits dimensions.
pub fn transform_page_preview(ctx: &PipelineContext) -> Result<DynamicImage> {
    let img = load::load_image(ctx.source_path)?;
    let img = exif::correct_orientation(img, ctx.source_path);
    let img = rotate::apply(img, ctx.rotation);
    let img = crop::apply(img, ctx.crop);
    let img = color::apply(img, ctx.brightness, ctx.contrast);
    Ok(img)
}
```

### Individual Transforms

Each transform is a public, `#[must_use]` function in its own file. This makes
every step independently testable, documentable, and separable.

```rust
// rotate.rs
/// Apply a user-requested rotation. Zero degrees is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, rotation: Rotation) -> DynamicImage {
    match rotation {
        Rotation::ZERO  => img,
        Rotation::DEG90  => img.rotate90(),
        Rotation::DEG180 => img.rotate180(),
        Rotation::DEG270 => img.rotate270(),
    }
}

// crop.rs
/// Crop to the given box, clamped to image bounds. None is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, crop: Option<CropBox>) -> DynamicImage {
    let Some(c) = crop else { return img };
    let (iw, ih) = (img.width(), img.height());
    let x = c.x.min(iw.saturating_sub(1));
    let y = c.y.min(ih.saturating_sub(1));
    let w = c.w.min(iw - x);
    let h = c.h.min(ih - y);
    img.crop_imm(x, y, w, h)
}

// color.rs
/// Adjust brightness and contrast. Both at zero is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, brightness: Brightness, contrast: Contrast) -> DynamicImage {
    if brightness == Brightness::ZERO && contrast == Contrast::ZERO {
        return img;
    }
    let c = (contrast.as_f32() as f64 + 1.0).max(0.0);
    let b = (brightness.as_f32() as f64 * 128.0).round() as i32;
    let mut rgb = img.into_rgb8();
    rgb.par_iter_mut().for_each(|ch| {
        let v = ((f64::from(*ch) - 128.0) * c + 128.0).round() as i32 + b;
        *ch = v.clamp(0, 255) as u8;
    });
    DynamicImage::ImageRgb8(rgb)
}

// resize.rs
/// Resize to output dimensions. None or already-correct dimensions is a no-op.
#[must_use]
pub fn apply(img: DynamicImage, output: Option<OutputSize>) -> DynamicImage {
    let Some(o) = output else { return img };
    if o.w == img.width() && o.h == img.height() {
        return img;
    }
    img.resize_exact(o.w, o.h, FilterType::Lanczos3)
}

// load.rs
/// Load an image from disk (handles format auto-detection).
pub fn load_image(path: &Path) -> Result<DynamicImage> {
    crate::io::image::load(path)
}

// exif.rs
/// Read EXIF orientation and apply rotation/flip. Non-EXIF sources are a no-op.
#[must_use]
pub fn correct_orientation(img: DynamicImage, path: &Path) -> DynamicImage {
    let orientation = crate::io::exif::read_orientation(path);
    orientation.apply(img)
}
```

### Design choice: plain functions over a trait

| Approach | Benefits | Costs |
|---|---|---|
| `trait Transform` + `Box<dyn Transform>` | Runtime-configurable pipeline | vtable overhead, heap alloc per step, `name()` method that exists only for logging |
| `enum TransformStep` with match dispatch | Zero-cost, still an enum registry | Match arm per step in orchestrator, new step = new variant |
| **Plain functions (chosen)** | Zero-cost, inlinable, no allocation, one line per step in orchestrator | Cannot swap steps at runtime (not a real requirement) |

The pipeline order is always: load → EXIF → rotate → crop → color → resize.
There is no plausible scenario where a user wants to run crop before rotate, or
skip EXIF for a JPEG. A hardcoded function chain is the simplest thing that
works. Tests verify each function in isolation.

Adding a `watermark` transform means:
1. Create `transform/watermark.rs` with `fn apply(img, params) -> DynamicImage`.
2. Insert one line into `transform_page()`.
3. Done. Three lines of orchestration glue.

---

## Export Layer (`core::export`)

Thin orchestrators that run transforms over the project and save results.

### `batch.rs`

```rust
/// Process every page in parallel, saving individual image files.
/// `on_progress(done, total)` is called after each page completes.
///
/// Returns one `Result<PathBuf>` per page, in order.
/// Returns `Err(PdfNotSupportedInBatch)` for PDF format — use `export_to_pdf`.
pub fn run_batch<F>(
    project: &Project,
    on_progress: F,
) -> Vec<Result<PathBuf>>
where F: Fn(usize, usize) + Sync + Send;
```

Internally uses `rayon::par_iter()` over `project.pages`. Each page:
1. Builds `PipelineContext::from_page(project, page)`.
2. Runs `transform_page(&ctx)`.
3. Saves via `io::image::save_*`.
4. Increments `AtomicUsize` progress counter.

### `pdf.rs`

```rust
/// Export all pages as a single PDF with JPEG-encoded image streams.
/// Metadata from PdfMeta is embedded in the document info dictionary.
pub fn export_to_pdf<F>(
    project: &Project,
    out_path: &Path,
    meta: &PdfMeta,
    quality: JpegQuality,
    on_progress: F,
) -> Result<()>
where F: Fn(usize, usize) + Sync + Send;
```

Internally: `par_iter` over pages → `transform_page` → JPEG encode in memory
→ build PDF document with `pdf-writer` → `write_atomic`.

### Why export is separate from transform

Transforms know how to **process** one image. Export knows how to **iterate**
over pages, manage parallelism, format output filenames, and wire progress
callbacks. This separation means:
- `batch.rs` and `pdf.rs` can evolve independently.
- Transforms don't know about filenames, directories, or parallelism.
- Adding a new export format only touches `io::image` and the export layer.

---

## I/O Adapters (`core::io`)

Shell around the filesystem. All save operations go through `write_atomic`.
Defined as **plain functions**, not traits.

### When to use a trait here

| Situation | Use |
|---|---|
| One real implementation, tests use temp dirs | **Plain function** (e.g. `save_png`, `load_project`) |
| Need to swap backend (e.g. `image` crate vs custom decoder) | **Trait** behind a feature flag |
| OpenCV auto-crop | `autodetect.rs` function, isolated in `crates/core` |

The current codebase has exactly one real implementation for every I/O
operation. Adding traits now would be premature abstraction. If a second
image decoder or a cloud storage backend is needed in the future, extract a
trait at that point.

### File listing

```rust
// atomic.rs — the single output path everything goes through
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()>;

// image.rs — image crate wrappers
pub fn load(path: &Path) -> Result<DynamicImage>;
pub fn save_png(img: &DynamicImage, path: &Path) -> Result<()>;
pub fn save_jpeg(img: &DynamicImage, path: &Path, quality: JpegQuality) -> Result<()>;
pub fn save_tiff(img: &DynamicImage, path: &Path) -> Result<()>;

// exif.rs — kamadak-exif integration (I/O concern, not image transform)
pub fn read_orientation(path: &Path) -> Orientation;

// project.rs — .recto file serialization
pub fn save_project(project: &Project, path: &Path) -> Result<()>;
pub fn load_project(path: &Path) -> Result<Project>;

// config.rs — user preferences persistence
pub fn load_pdf_meta() -> PdfMeta;
pub fn save_pdf_meta(meta: &PdfMeta);
```

---

## UI Architecture (`recto-ui`)

### View / Widget separation

| Layer | Purpose | Can import | Can do |
|---|---|---|---|
| **Views** | Full-page layouts (Gtk::Stack children) | `core::*`, widgets, `AppState` | Dispatch commands, subscribe to events, compose widgets |
| **Widgets** | Reusable GTK components | GTK crates, core domain types (read-only) | Receive data via setters, emit signals, NEVER dispatch commands |

Views are the "imperative shell" of the functional core. They:
1. Wire GTK signal handlers to `Command` dispatches.
2. Subscribe to `AppEvent` channels and update widgets accordingly.
3. Manage `Gtk::Stack` page transitions.

Widgets are passive UI primitives:
1. Expose setter methods (`set_pages`, `set_image`, `set_rotation`).
2. Emit signals (`page-selected`, `crop-changed`, `preset-activated`).
3. Never hold an `Rc<AppState>`. Never call `state.dispatch()`.

### Widget contract

```rust
impl ThumbnailGrid {
    /// Rebuild the grid from a slice of domain data.
    pub fn set_pages(&self, pages: &[Page]);

    /// Update a single row after a PageChanged event.
    pub fn update_page(&self, index: usize, page: &Page);

    /// Emitted when the user selects a row.
    pub fn connect_selected(&self, cb: impl Fn(Option<usize>) + 'static) -> SignalHandlerId;
}
```

The view connects widget signals to commands:

```rust
// In workspace.rs
grid.connect_selected(clone!(@weak state => move |index| {
    let index = match index {
        Some(i) => i,
        None => return,
    };
    update_arrange_preview(&state, index);
}));

preset_chips.connect_activated(clone!(@weak state, @weak grid => move |preset| {
    if let Some(selected) = grid.selected_index() {
        state.dispatch(Command::SetCropPreset { index: selected, preset: Some(preset) });
    }
}));
```

### Session (`session/session.rs`)

A `glib::Object` subclass that owns the app's lifecycle state:

```rust
pub struct Session {
    // Properties (GLib-derivable, notify on change):
    dirty: Cell<bool>,              // unsaved changes
    display_name: RefCell<String>,  // "MyProject" or "MyProject •"

    // Non-property fields:
    path: RefCell<Option<PathBuf>>, // current save path (None = untitled)
    state: RefCell<Option<State>>,  // Rc<AppState>
}
```

The window title bar binds to `display_name`. Save/load/export actions read
`path` and `state` from the session.

### Worker (`worker.rs`)

A single `std::thread` that consumes closures via `async_channel`. Tasks use
`rayon::par_iter` internally for data parallelism.

```rust
pub struct Worker {
    task_tx: Option<async_channel::Sender<Box<dyn FnOnce() + Send + 'static>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn new() -> Self;
    pub fn spawn<F>(&self, f: F) where F: FnOnce() + Send + 'static;
}

impl Drop for Worker {
    // Drop sender → close channel → thread exits → join
}
```

Every `std::thread::spawn` in the codebase goes through `Worker::spawn`. Views
never spawn threads directly.

### Deduplication pattern

Preview requests carry a monotonically increasing `RequestId`:

```rust
/// In a view that manages previews:
let request_id = Rc::new(Cell::new(0u64));

fn request_preview(&self) {
    let id = self.request_id.get() + 1;
    self.request_id.set(id);

    // Snapshot project state for the worker closure.
    let project = self.state.project().clone();
    let page = project.pages[self.selected_index].clone();
    let ctx = PipelineContext::from_page(&project, &page);

    self.worker.spawn(glib::clone!(@weak self as view => move || {
        let pixbuf = render_preview(&ctx);
        let _ = view.preview_result_tx.try_send((id, pixbuf));
    }));
}

fn handle_preview_result(&self, id: u64, pixbuf: Pixbuf) {
    if id == self.request_id.get() {
        self.canvas.set_image(&pixbuf);
    }
    // else: stale result from a superseded request — discard
}
```

### Composables (no file yet — pattern to adopt)

Reusable signal-wiring helpers extracted from repetitive view code:

```rust
/// Bind a Gtk::Scale widget to a Brightness newtype and dispatch on change.
pub fn bind_brightness(
    scale: &gtk::Scale,
    state: State,
) -> SignalHandlerId {
    scale.connect_value_changed(move |s| {
        let val = Brightness::new(s.value() as f32);
        state.dispatch(Command::SetBrightness(val));
    })
}

/// Sync a Gtk::Label with the page count from AppState.
pub fn bind_page_count(
    label: &gtk::Label,
    event_rx: async_channel::Receiver<AppEvent>,
    state: State,
) {
    glib::spawn_future_local(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                AppEvent::PagesAdded(_) | AppEvent::PagesRemoved(_) | AppEvent::ProjectLoaded => {
                    label.set_text(&format!("{} images", state.project().pages.len()));
                }
                _ => {}
            }
        }
    });
}
```

These composables keep view files focused on layout rather than boilerplate
signal wiring.

---

## Data Flow: End-to-End Examples

### User rotates a page

```
1. User clicks "Rotate 90°" button in workspace view
2. Button signal → workspace.rs helper:
   rotate_selected(&selection, &page_store, &state, Rotation::DEG90)
3. For each selected page:
   a. page_item.apply_rotation_delta(Rotation::DEG90) — updates the widget pixbuf
   b. state.dispatch(Command::SetRotation { index, rotation: Rotation::DEG90 })
4. AppState::dispatch:
   a. Push current Project snapshot onto undo_stack
   b. project.pages[index].rotation = Rotation::DEG90
   c. Emit AppEvent::PageChanged(index)
   d. Emit AppEvent::ProjectChanged (for undo/redo subscribers)
5. Event subscribers in workspace view:
   a. Thumbnail grid: update rotation indicator on the card
   b. Preview manager: increment request_id, spawn preview render on worker
   c. Session: mark_dirty() → title bar shows "MyProject •"
6. Worker thread:
   a. Pixbuf::from_file → rotate → scale → send (pixbuf, request_id) to main thread
7. Main thread: check request_id matches latest → set on PreviewCanvas → refit zoom
```

### User exports to PDF

```
1. User opens export dialog (views/export.rs), picks PDF, sets quality, clicks Export
2. export.rs:
   a. Reads PdfMeta from config
   b. Clones Project from state.project() (snapshot — prevents mid-export mutations)
   c. Builds PipelineContext for each page
3. worker.spawn(move || {
     export_to_pdf(&project, &out_path, &meta, JpegQuality::new(quality), |done, total| {
         let _ = progress_tx.try_send(ExportMsg::Progress(done, total));
     })
   })
4. Worker thread:
   a. par_iter over pages
   b. Each page: transform_page(&ctx) → JPEG encode to Vec<u8>
   c. After all pages: build PDF with pdf-writer
   d. write_atomic(out_path, &pdf_bytes)
   e. Progress callback at each page: send ExportMsg::Progress(done, total)
5. Main thread: ExportMsg::Progress → progress bar; ExportMsg::Done → show results
```

### User saves project

```
1. Ctrl+S → window.rs action handler
2. session.state().dispatch_without_undo(Command::SetOutputDir(...))  // if needed
3. save_project(&session.state().project(), &session.path())
4. session.clear_dirty() → title bar removes "•"
```

---

## Testing Strategy

### Unit tests (in `core`)

Each transform function is tested independently with synthetic images:

```rust
// domain/values.rs — verify newtype clamping
#[test]
fn brightness_clamps_to_range() {
    assert_eq!(Brightness::new(2.0), Brightness::MAX);
    assert_eq!(Brightness::new(-5.0), Brightness::MIN);
}

// domain/geometry.rs — pure math
#[test]
fn rect_clamp_to_smaller() {
    let r = Rect::new(10.0, 10.0, 100.0, 100.0);
    let clamped = r.clamp_to(50, 50);
    assert_eq!(clamped, Rect::new(10.0, 10.0, 50.0, 50.0));
}

// transform/color.rs — no-op is identity
#[test]
fn apply_colors_noop() {
    let img = test_image_rgb(64, 64, |x, y| Rgb([x as u8, y as u8, 128]));
    let result = apply_colors(img.clone(), Brightness::ZERO, Contrast::ZERO);
    assert_image_eq(&img, &result);
}

// transform/rotate.rs — composability
#[test]
fn double_90_is_180() {
    let img = test_image_rgb(64, 48, |x, y| Rgb([x as u8, 0, 0]));
    let once = apply_rotation(img.clone(), Rotation::DEG90);
    let twice = apply_rotation(once, Rotation::DEG90);
    assert_eq!(twice.width(), 64);  // 90+90=180, dimensions preserved
    assert_eq!(twice.height(), 48);
}

// command/state.rs — dispatch + undo/redo
#[test]
fn dispatch_then_undo_restores_state() {
    let (state, _rx) = AppState::new(project_with_page());
    state.dispatch(Command::SetBrightness(Brightness::new(0.5)));
    assert_eq!(state.project().brightness, Brightness::new(0.5));
    assert!(state.undo());
    assert_eq!(state.project().brightness, Brightness::ZERO);
}

// command/state.rs — events emitted
#[test]
fn add_pages_emits_indices() {
    let (state, rx) = AppState::new(Project::default());
    state.dispatch(Command::AddPages(vec![PathBuf::from("a.png")]));
    assert_eq!(rx.try_recv(), Ok(AppEvent::PagesAdded(vec![0])));
}
```

### Integration tests (in `core`)

Test the full pipeline against real temp files:

```rust
#[test]
fn full_pipeline_smoke() {
    let tmp = tempfile::tempdir().unwrap();
    let img_path = tmp.path().join("test.png");
    create_test_image(128, 96).save(&img_path).unwrap();

    let project = Project::default();
    let page = Page::new(img_path);
    let ctx = PipelineContext::from_page(&project, &page);
    let result = transform_page(&ctx).unwrap();

    assert_eq!(result.width(), 128);
    assert_eq!(result.height(), 96);
}

#[test]
fn pipeline_with_crop_and_resize() {
    // Crop 64x64 from 128x128, resize to 32x32
    let result = ...;
    assert_eq!(result.width(), 32);
    assert_eq!(result.height(), 32);
}

#[test]
fn run_batch_exports_all_pages() {
    let tmp = tempfile::tempdir().unwrap();
    // 3 test images, PNG export
    let results = run_batch(&project, |_, _| {});
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.is_ok()));
}

#[test]
fn batch_rejects_pdf_format() {
    let project = Project { export: ExportSettings::Pdf { quality: JpegQuality::new(90) }, .. };
    let results = run_batch(&project, |_, _| {});
    assert!(results[0].is_err());
}
```

### UI tests

GTK widget testing in Rust is limited. Focus on:

- **Headless mode** (`--headless` flag in `main.rs` via `clap`): Test the full
  pipeline end-to-end without a display server.
- **Manual test plan**: Widget interactions (drag-to-crop, zoom/pan, slider
  debounce) validated by a checklist in `tests/manual.md`.

---

## Future Extensibility

### Adding a new transform (e.g. watermark)

1. Create `core/transform/watermark.rs`:
   ```rust
   #[must_use]
   pub fn apply(img: DynamicImage, text: &str, opacity: f32) -> DynamicImage {
       // ...
   }
   ```
2. Add the watermark parameter to `Project` and `PipelineContext`.
3. Insert one line into `transform_page()`:
   ```rust
   let img = watermark::apply(img, &ctx.watermark_text, ctx.watermark_opacity);
   ```
4. No trait to implement, no registry to update, no pipeline builder to wire.

### Adding a new export format (e.g. WebP)

1. Add `ExportSettings::Webp { quality: JpegQuality }` to `domain/export.rs`.
2. Add `save_webp()` to `io/image.rs`.
3. Add a match arm in `export/batch.rs`'s format-dispatch.
4. Add a radio button in `views/export.rs`.
5. Done.

### Adding undo/redo menu items

The undo/redo stacks already exist:
- `Ctrl+Z` → `gio::Action` → `state.undo()`
- `Ctrl+Shift+Z` → `gio::Action` → `state.redo()`

Wire in `window.rs`:
```rust
action_undo.connect_activate(clone!(@weak session => move |_, _| {
    let st = session.state();
    st.undo();
}));
```

### Auto-crop with OpenCV

Isolated in `crates/core/src/autodetect.rs`:

```rust
pub fn detect_bounds(path: &Path) -> Result<Vec<CropBox>> {
    // OpenCV logic — never pollutes domain or transform modules
}
```

---

## Anti-Patterns to Avoid

| Anti-pattern | Why it hurts | The fix |
|---|---|---|
| `Rc<RefCell<T>>` sprawl | Implicit mutation, no undo, coupled to every consumer | `AppState` with `dispatch(Command)`; widgets receive data, don't hold mutable refs |
| `Box<dyn Trait>` for a single impl | Vtable overhead, heap allocation, harder to trace | Plain function unless you genuinely have 2+ implementations |
| `.unwrap()` / `.expect()` in non-test code | User-facing crash for a recoverable error | `Result` with error dialog; `anyhow` for I/O, `thiserror` for library |
| Business logic inside GTK signal handlers | Untestable, duplicates logic across views | Dispatch a `Command`, react to `AppEvent` |
| `std::thread::spawn()` scattered in views | Leaked threads, no unified cancellation | `Worker::spawn(f)` for every background task |
| `String` where an enum would do | Invalid states possible, runtime checks instead of compile-time | `enum ExportSettings` with `#[non_exhaustive]` |
| `u16` for rotation, `f32` for brightness | `if rotation == 360` checks scattered everywhere | `Rotation(u16)`, `Brightness(f32)` newtypes |
| `Box<dyn Error>` / generic error erasure | Loses type information for error handling | Concrete `enum Error { Io(io::Error), Image(image::ImageError), ... }` with `thiserror` |
| Premature abstraction | Complexity without measured benefit | Write the simplest thing first. Extract abstractions only when a second use case appears. |
| `Arc<RwLock<T>>` for shared state in GTK code | Conflicts with GTK's single-threaded main event loop | `Rc<RefCell<T>>` is correct for GTK (single-threaded by design). `Arc` is for data crossing thread boundaries. |

---

## Migration Path from Current Architecture

The current architecture is not broken. The existing code already implements
many of the patterns described here (Command/Event, Worker, PipelineContext).
The migration is incremental:

### Phase 1 — Introduce newtype wrappers

Create `core/domain/values.rs` with `Rotation`, `Brightness`, `Contrast`,
`JpegQuality`, `Dpi`. Wire them into `Project`, `Page`, `Command`, and
`PipelineContext` one at a time. Each change is mechanical: replace `u16` with
`Rotation`, replace `f32` with `Brightness`, remove scattered `.clamp()` calls.

### Phase 2 — Split monolith files into directories

| Before | After |
|---|---|
| `command.rs` (467 lines) | `command/mod.rs` + `state.rs` + `event.rs` |
| `pipeline/mod.rs` + `transforms.rs` | `transform/mod.rs` + `transform/{context,load,exif,rotate,crop,color,resize}.rs` |
| `io.rs` | `io/{atomic,image,exif,project,config}.rs` |
| `project/types.rs` | `domain/{project,crop,export,geometry,values}.rs` + migrate `geometry.rs` in |
| `app.rs` | `session/mod.rs` + `session/session.rs` |

No behavioral change. Pure file-splitting. Tests pass at every step.

### Phase 3 — Rename UI directories

| Before | After |
|---|---|
| `steps/` | `views/` (start.rs, workspace.rs, export.rs) |
| `steps/import.rs` | `widgets/thumbnail_loader.rs` |
| `steps/crop/{picker,overlay,presets}` | `widgets/{crop_picker,crop_overlay,preset_chips}.rs` |
| `steps/colors.rs` | `widgets/color_preview.rs` |
| `steps/workspace/{css,helpers}` | `views/workspace/{css,helpers}.rs` |
| `steps/grid.rs` + `steps/page_item.rs` | `widgets/{thumbnail_grid,page_item}.rs` |

No behavioral change. Just moving files and updating `mod` declarations.

### Phase 4 — Extract composables

Move repetitive signal-wiring patterns from `workspace.rs` into dedicated
composable functions (`bind_brightness`, `bind_page_count`, etc.). Views shrink
to layout + composable calls.

### Phase 5 — Enforce widget passivity

Audit all `widgets/` files for direct `Rc<AppState>` usage. Replace with setter
methods + signals. The `preset_chips` and `crop_picker` widgets are the likely
offenders.

Each phase is independently shippable and testable. No phase requires touching
public API consumers outside `recto-ui` or `recto-core`.
