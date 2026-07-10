# Plan: Author `ARCHITECTURE_2.md`

## Context

The user wants an opinionated `ARCHITECTURE.md` for `recto` (pagecutter2): a
GTK4 + libadwaita Rust app for batch post-processing of scanned book pages
(crop, rotate, colour, export to PNG/JPEG/TIFF/PDF).

The document is a **guideline, not a contract** — it should describe the
target shape, justify the paradigms chosen, and call out anti-patterns. It is
*not* a refactor plan; the user can reach for it whenever they touch a
sub-system.

The user also asked for an honest take on the current state. The codebase is
~7,300 LoC in a two-crate workspace and is already much more deliberate than
the prompt implied — most of the right ideas are present; one or two are
half-implemented; the UI shell has the usual god-function problem.

## Current state — honest assessment

### What is already right (keep & lean on)

- **Workspace split** (`recto-core`, `recto-ui`). Core has no GTK dep.
- **Value newtypes** (`Rotation`, `Brightness`, `Contrast`, `JpegQuality`,
  `Dpi`) with clamping constructors and `#[serde(transparent)]`. Textbook.
- **Pure functional pipeline** in `crates/core/src/transform/` — each step
  is a tiny `DynamicImage -> DynamicImage` function, `#[must_use]`, composed
  by `transform_page` and `transform_page_preview`. This is the heart of the
  app and it's done correctly.
- **`PipelineContext`** as an immutable snapshot taken on the main thread
  before any worker job runs. The right way to bridge mutable UI state into
  worker threads in Rust.
- **`Command` + `AppEvent` bus** in `crates/core/src/command/` with snapshot
  undo. Elm/Redux shape. Sound.
- **Atomic writes** via `tempfile.persist` for both project files and
  exported images.
- **Schema versioning** on `Project` with a `migrate_project` hook and an
  `UnsupportedProjectVersion` error.
- **`#[non_exhaustive]`** on `Error`, `ExportSettings`, `Orientation` — no
  SemVer accidents on variant additions.
- **`thiserror` in core, `anyhow` in UI boundary** — the correct split.
- **`PageId`** stable identity for thumbnails so a "load finished" message
  arriving after a delete can no-op cleanly.
- **Per-page `Result` from `run_batch`** — partial successes survive.
- **Tests** on every value newtype, geometry, the pipeline, and project
  round-trip.

### What is wrong (don't preserve)

1. **Dual source of truth.** `Project::pages` (canonical) and
   `gio::ListStore<PageItem>` (UI mirror) drift apart and need
   `sync_page_metadata` to reconcile. UI sites currently mutate *both*
   (`page.apply_rotation_delta(delta); state.dispatch(SetRotation{…})`).
   The Command bus is a sidecar, not the channel.
2. **`workspace/mod.rs` is a 1073-line god-function.** A single `build()`
   wires sidebar, three modes, two preview canvases, the picker, the grid,
   selection handling, two background threads, and three async receivers.
   The selection handler alone has ~14 cloned captures.
3. **Two ad-hoc `std::thread::spawn` calls inside `workspace/mod.rs`** (one
   for the rotation preview, one for the colour preview) despite a `Worker`
   abstraction whose own doc-comment says *"Every `std::thread::spawn` in
   the codebase should go through this type."* The `Worker` is used
   nowhere except export.
4. **Two parallel preview pipelines.** `transform::color::apply` (rayon,
   `image`) and `widgets::color_preview::adjust_colors` (scalar loop,
   `gdk_pixbuf`) implement the same brightness/contrast formula in two
   places against two pixel-buffer types.
5. **`AppState::project_mut() -> RefMut<Project>` is public.** Anyone can
   bypass the Command bus and silently break undo.
6. **`current_index: Cell<i32>` with `-1` as a sentinel.** That's a
   `Cell<Option<usize>>` written in C.
7. **Open-coded latest-wins channel.** `req_id: Cell<u64>` plus
   `wrapping_add(1)` plus `while let Ok(next) = rx.try_recv() { req = next; }`
   appears at least three times. Begging to be a primitive.
8. **Backward-compat re-exports** in `crates/core/src/lib.rs`
   (`pub mod project { pub use crate::domain::… }`, etc.) are post-refactor
   shims with no remaining caller and should go.
9. **Lossy boundary types.** `PageItem::rotation` is a bare `u32` so the
   `Rotation` newtype must be re-validated on every dispatch. Same story
   for crop coordinates — the comment "stored post-EXIF + post-user-rotation"
   is a load-bearing convention with no type protection.
10. **`MarkDirty: Rc<dyn Fn()>`** is a side-channel to express what
    `AppEvent::ProjectChanged` already says.
11. **`Worker` is single-threaded and FIFO**, which is why interactive
    previews bypass it. The abstraction doesn't fit two distinct workloads
    (long batch vs. latest-wins preview).
12. **`run_batch` returns `vec![Err(PdfNotSupportedInBatch)]`** for PDF
    instead of `export()` dispatching to the right backend.
13. **`--headless` flag is stubbed.** The cleanest possible test that the
    core/UI split is honest is to actually run the pipeline without GTK,
    and we don't.

---

## Proposed `ARCHITECTURE.md` (draft to write to repo on approval)

The text below is what gets written, verbatim, to `/ARCHITECTURE.md`. It is
the only artefact this task produces — no code is changed.

````markdown
# Architecture

> A guideline for how `recto` is shaped. Not a contract — deviate when the
> code reads better, but say *why* in the diff.

## 1. Goals

`recto` is a desktop app for post-processing scanned book pages: crop,
rotate, colour-adjust, and export batches to PNG / JPEG / TIFF / PDF.

The architecture must:

- keep image processing **pure and testable** — no GTK on the hot path;
- keep the **UI declarative** with respect to a single source of truth;
- **lean into Rust**: newtypes for invariants, exhaustive enums for state,
  ownership for resource lifetimes, `Send` boundaries for threading;
- stay **honest about a desktop app**: no microservices, no event sourcing,
  no DDD aggregates for what is a `Vec<Page>`. KISS first.

## 2. Paradigms used (and why)

We mix paradigms deliberately. Every choice below is load-bearing.

| Layer        | Paradigm                                  | Why                                                               |
|--------------|-------------------------------------------|-------------------------------------------------------------------|
| Engine       | Functional core, imperative shell         | Image transforms are pure; threading is the dirty boundary.       |
| Domain       | Newtype-driven; "make illegal states unrepresentable" | A `Rotation` cannot be 73°. A `JpegQuality` cannot be 200.        |
| Application  | Command bus + event stream (CQRS-lite)    | Single mutation channel, free undo, easy to log/replay.           |
| Persistence  | Plain serde + atomic file writes          | One file, one user, one machine. A repository pattern is overkill.|
| UI           | Hexagonal (ports & adapters)              | GTK is an adapter; swap for a CLI without touching the core.      |
| Concurrency  | Two job models, not one                   | Long batch and latest-wins preview have *opposite* requirements.  |

We do **not** use:

- DDD aggregates (the project *is* an aggregate, but there's only one);
- Event sourcing (snapshot undo is sufficient and trivial);
- Async/await for I/O (GTK's main loop already gives us cooperative
  scheduling; `glib::MainContext::spawn_local` is enough);
- Trait objects for things that have one implementation;
- Macros that hide control flow.

## 3. Crate layout

```
crates/
├── core/   recto-core    — domain, commands, pipeline, persistence
├── ui/     recto-ui      — GTK4 + libadwaita binary
└── cli/    recto-cli     — headless binary (future; proves the split)
```

`core` has **zero** GTK / glib / gdk dependencies. This is enforced by
inspection — any `use gtk` import in `core/` is a bug.

When a third consumer appears (CLI, plugin host, fuzzing harness) we split
the engine off as `recto-pipeline` and let `core` shrink back to domain +
application.

## 4. Layered responsibilities

```
              ┌────────────────────────────────────────────────┐
   GTK ←────  │  Presentation       widgets/, views/           │
              │  - Renders state, dispatches Commands          │
              │  - Subscribes to AppEvent                      │
              └────────────────────────────────────────────────┘
                              │ Command          ▲ AppEvent
                              ▼                  │
              ┌────────────────────────────────────────────────┐
              │  Application        command/, AppState         │
              │  - Owns the Project                            │
              │  - Dispatches commands, emits events           │
              │  - Snapshot undo / redo                        │
              └────────────────────────────────────────────────┘
                              │ &Project, &Page
                              ▼
              ┌────────────────────────────────────────────────┐
              │  Engine             transform/, export/        │
              │  - Pure functions: image in, image out         │
              │  - Composed via PipelineContext                │
              │  - rayon for data parallelism                  │
              └────────────────────────────────────────────────┘
                              │ load / save
                              ▼
              ┌────────────────────────────────────────────────┐
   FS  ←────  │  Infrastructure     io/                        │
              │  - Atomic writes, EXIF, image codecs, config   │
              └────────────────────────────────────────────────┘
                              ▲
              ┌───────────────┴────────────────────────────────┐
              │  Domain             domain/                    │
              │  - Project, Page, Rotation, CropBox, Rect, …   │
              │  - No I/O, no GTK, no threads                  │
              └────────────────────────────────────────────────┘
```

Direction of dependency: arrows always point downward in the diagram. The
Domain depends on nothing. The Engine depends on Domain + I/O. Application
depends on Domain + Engine + I/O. Presentation depends on Application only —
**never on Engine or I/O directly**.

## 5. The Command / Event pipeline (single source of truth)

There is exactly one `Project` per session. It lives in `AppState` behind a
`RefCell`. **The only way to mutate it is `AppState::dispatch(Command)`.**

```rust
state.dispatch(Command::SetRotation { index, rotation });
//        │
//        ├── pushes a snapshot onto the undo stack
//        ├── clears redo
//        ├── applies the mutation
//        └── emits AppEvent::PageChanged(index)
```

Rules:

1. **No `&mut Project` ever leaves `AppState`.** `project_mut()` is private
   to the `command::` module; presentation code reads through `project()`
   and writes through `dispatch()`. There are no exceptions.
2. **UI state derived from `Project` is a projection, not a mirror.** The
   `gio::ListStore<PageItem>` is recomputed from `AppEvent::PagesAdded /
   PagesRemoved / PageChanged`. UI code never writes to a `PageItem` and
   *also* dispatches a `Command` — only the latter, and the projector
   reacts.
3. **`AppEvent` is the bus, period.** No side-channel `MarkDirty`
   callbacks; "session is dirty" is a subscriber to `ProjectChanged`.
4. **Commands carry validated newtypes**, not raw primitives.
   `Command::SetRotation { rotation: Rotation::DEG90 }`, never a `u16` to be
   re-validated on the application side.

## 6. The functional pipeline

`transform_page` is the spec for "how a page becomes a file":

```rust
load(path) → exif_correct(path) → rotate(rotation)
           → crop(crop_box)     → colour(b, c)
           → resize(output)
```

Each step is a `fn(DynamicImage, …) -> DynamicImage`, `#[must_use]`, no I/O
except `load`, no global state. New transforms (deskew, despeckle, OCR
overlay) are added by writing one file in `transform/` and slotting it into
the composition. The compiler tells you the rest.

The preview pipeline is the export pipeline minus `resize` — it is
**literally the same code path**. There is exactly one implementation of
brightness/contrast in the codebase. (Today there are two; that's a bug, not
a feature.)

`PipelineContext` is the immutable snapshot of all parameters for one page,
constructed on the main thread before the job is handed off. Once it
exists, the job is independent of any UI mutation.

## 7. Concurrency

GTK is single-threaded; image work is CPU-bound. We use **two distinct job
models** because they have opposite requirements:

### Batch executor (`recto-core::JobQueue`)

- Backed by `rayon` for data parallelism within a job.
- One job at a time on the orchestration side; jobs run to completion.
- Cancellable via an `AtomicBool` flag in `JobToken`.
- Used by: export, project load (thumbnail decode for *N* pages).

### Preview executor (`recto-ui::PreviewService`)

- Single dedicated worker thread per preview kind (arrange, colour).
- **Latest-wins**: a new request supersedes the in-flight one.
- Implemented as a `LatestOnly<Req>` channel primitive — extracted once,
  reused everywhere a preview exists.
- Used by: arrange-mode preview, colour-mode preview, crop-mode overlay
  background renders.

Both surface results to the GTK main loop via `async-channel` +
`glib::MainContext::spawn_local`. Receivers compare a request id against
the latest issued id and drop stale results.

**No `std::thread::spawn` outside these two services.** If you need a
background thread, you need one of these.

## 8. Type-state where it pays for itself

A few invariants are enforced by the type system rather than by convention:

- **`Rotation`, `Brightness`, `Contrast`, `JpegQuality`, `Dpi`** — clamped
  in their constructors; the inner field is private. You cannot construct a
  bad one without `unsafe` (and we have no `unsafe`).
- **`Rect` vs `CropBox`** — `Rect` is `f64` for interactive editing,
  `CropBox` is `u32` for persistence. Conversion is explicit at the
  boundary.
- **(Proposed) `Rect<Space>`** — phantom-tag coordinate space (`ImageSpace`,
  `CanvasSpace`, `OrientedImageSpace`) so the post-EXIF / post-rotation
  convention is a compile error to mix up. Today this is a comment in the
  project memory; it should be a type.

The pattern: any time a comment explains what coordinate frame / unit / state
a value is in, escalate that comment to a marker type when it crosses a
function boundary more than twice.

## 9. Errors

- `recto-core::Error` (thiserror, `#[non_exhaustive]`) for typed errors at
  module boundaries within core.
- `anyhow` only at the UI / CLI boundary, where errors become user-visible
  strings.
- **No `unwrap` / `expect` outside tests and `OnceCell`-style infallible
  initialization.** GTK callbacks return `Result<()>` to a dialog, never
  panic.
- I/O errors carry the path: wrap with `with_context` at the call site, not
  in the library.

## 10. Persistence

- Project files (`*.recto`) are JSON written atomically through a tempfile
  rename.
- Image paths are stored relative to the project file when possible.
- `version: u32` lives at the top of the file. `migrate_project` handles
  forward upgrades; `Error::UnsupportedProjectVersion` rejects newer files
  loudly.
- `#[serde(default)]` on every new optional field. Adding a field is
  backward-compatible by construction.
- PDF metadata lives in `~/.config/recto/pdf_meta.json` — *user* config, not
  *project* config. Don't conflate the two.

## 11. UI composition

The workspace is a composition of mode controllers, not a single function:

```
Workspace
├── ArrangeController   (sidebar + preview + delete/rotate handlers)
├── CropController      (preset chips + CropPicker + overlay)
└── ColorController     (sliders + reset + colour preview)
```

Each controller:

- is a GObject (so it can be cloned, weak-ref'd, signal-connected);
- owns its sidebar widgets and its preview widget;
- subscribes to the `AppEvent` stream it cares about;
- dispatches `Command`s; never mutates `Project` directly.

Adding a `DeskewController` is a new file plus one line in `Workspace::new`.

The shared chrome (header bar, hamburger menu, save/open actions, close
confirmation) lives in `window.rs` and remains framework-shaped — that's
fine, it's intrinsic GTK glue.

## 12. Testing strategy

- **Domain & engine**: unit tests in-module. Newtypes test their clamps,
  geometry tests its corner cases, the pipeline has end-to-end smoke tests
  with `tempfile`.
- **Application**: unit tests on `AppState::dispatch` for every `Command`
  variant — assert the resulting state and the emitted event.
- **Presentation**: not unit-tested in Rust; covered by manual smoke runs
  and (eventually) by the headless CLI exercising the pipeline end-to-end.
- **Headless CLI as integration test**: `recto-cli --project foo.recto
  --output out/` is the cheapest possible "does the engine work without
  GTK" assertion. Build it. Run it in CI.

## 13. Conventions

- Comments only when the *why* is non-obvious. No "// loop over pages".
- No re-exports from `lib.rs` whose only purpose is to soften a refactor.
  Move the call sites or land the rename properly.
- One concept per file in `domain/` and `transform/`. Files growing past
  ~300 lines are usually carrying two concepts.
- `#[must_use]` on any function returning a value the caller might forget
  to use (transforms, builders, validators).
- `Default` impls only when the default is meaningful, not as a syntactic
  convenience.
- Sentinels (`-1`, `u32::MAX`, magic strings) are forbidden when `Option`
  exists.

## 14. Anti-patterns specific to this codebase

These are mistakes the codebase has already made or is tempted to make.
List them so they stay made-once.

- **Don't** mutate a `PageItem` and then dispatch a `Command`. Dispatch the
  `Command`; let the projector update the `PageItem`.
- **Don't** add a side-channel `Rc<dyn Fn()>` callback for "something
  changed" — emit an `AppEvent` and subscribe.
- **Don't** spawn a thread inline. Use `JobQueue` or `PreviewService`.
- **Don't** reimplement a transform in pixbuf-land for previews. Convert
  pixbuf ↔ `DynamicImage` at the boundary and call the same engine code.
- **Don't** widen `AppState`'s API to leak `&mut Project`. If a use case
  feels like it needs that, it needs a new `Command` variant instead.
- **Don't** introduce a god-function `build(state, store, sel, sync,
  spinner, count, load1, load2, mark_dirty, worker, rx)`. That signature
  is the smell that you missed a controller boundary.

## 15. Future-proof gates

The architecture is sized for the present scope but admits these without
restructuring:

- **More transforms** (deskew, despeckle, denoise) — drop a file in
  `transform/`, slot it into `transform_page`.
- **More export formats** (WebP, JXL) — extend `ExportSettings`, add an
  arm in `process_one`. `#[non_exhaustive]` keeps it SemVer-clean.
- **Headless / scripting** — already foreseen; the CLI binary makes it
  real.
- **Plugin host** — when this is needed, the trait boundary is
  `fn(DynamicImage, &Params) -> DynamicImage`; everything else is in place.
- **Per-page colour adjustments** — today brightness/contrast are
  project-global. Migrating to per-page is a `Command` variant change and a
  schema bump (`migrate_project` already exists).
````

---

## Phase 5

After this plan is approved, the only execution step is to write the
fenced markdown content above to `/home/sotiris/p/sotirismorf/pagecutter2/ARCHITECTURE.md`
verbatim (without the outer fence).

No source files will be modified. No commits will be made.
