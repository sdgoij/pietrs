# The editor in the browser

Status: implemented through step 5 of the delivery plan. `cargo run -p xtask --
wasm` writes `target/web`, `08-wasm.sh` links the wasm target in every gate job,
and the bundle has been loaded in a browser. Publishing it is still open.

`pietrs` compiles to WebAssembly and runs the same editor, the same debugger,
and the same interpreter in a browser tab. The canvas is a WebGL surface, the
file dialogs are the browser's, and the document is still a PNG. The page is a
static bundle — HTML, CSS, one `.wasm`, one JS module — served from any file
server; no backend, no runtime services, no network calls after load.

This is not a fourth execution path. The `interp` crate is compiled to wasm
unchanged; stepping, tracing, the movement rules, and the integer-only core are
the desktop editor's. The JIT and the AOT executable are **not** in the browser
build, and this note explains why that is structural, not a choice deferred.

## Decisions at a glance

| Decision | Choice | Why |
|---|---|---|
| Artefact | a static bundle built from `gui`, not a new binary | `pietrs editor` stays the only native entry point; the browser gets the same `EditorApp` |
| Shell | a `web` feature on `gui` with a `web.rs` entry, and `crate-type = ["rlib", "cdylib"]` | the wasm entry sits beside `native.rs` and shares `EditorApp`, the panels, and the theme |
| Toolkit | the same `egui` 0.37 / `eframe` 0.37, via `eframe::WebRunner` | the panels need no port; eframe's wasm dependencies are already target-gated |
| Renderer | `glow` (WebGL2, falling back to WebGL1) | already the desktop backend; WebGPU is a second pipeline for no benefit |
| File I/O | the **shell** owns dialogs and the filesystem; the app emits `FileRequest` and applies `IoResult` | async on the web, blocking on the desktop, one document model, no `cfg` in the app |
| Open | `rfd::AsyncFileDialog::pick_file` → `FileHandle::read` | rfd's wasm backend renders its own `<input type=file>` overlay and resolves `None` on cancel |
| Save | `rfd::AsyncFileDialog::save_file` → `FileHandle::write` (browser download) | no paths exist on the web; "Save" is always a download |
| Time | `web-time` inside `editor` | `std::time::Instant::now()` panics on `wasm32-unknown-unknown`; `Duration` is the same type |
| Codegen | the `web` feature never enables `codegen` | `cranelift-jit` does not build for wasm32 (it needs executable memory and `region`) |
| Tooling | `wasm-bindgen-cli` at the crate's exact version, driven by `cargo run -p xtask -- wasm` | one pinned tool, an explicit failure when the versions differ; Trunk's auto-matching is convenience we do not need |
| Tests | headless `Context::run_ui` for the app; a wasm **build** in the gate; no browser in CI | the wasm shell is thin, and chromedriver/geckodriver on every runner is a cost with no coverage |

## Non-goals

- No JIT comparison and no AOT build in the browser. `cranelift-jit` cannot
  compile for `wasm32-unknown-unknown`, and a `.wasm` cannot write an ELF/Mach-O/
  PE executable. The two buttons are `cfg`-gated on `codegen` and simply do not
  exist in the web bundle.
- No publish step. The proposal produces `target/web/`; where it is hosted (a
  GitHub Pages job, a CDN, a tarball on the release) is a separate decision.
- No `pietrs editor` behaviour change. The native shapes, the CLI, and the
  release archives are untouched.
- No mobile or touch-specific design. Pointer events already paint with a
  finger, but there is no pinch-zoom, no soft-keyboard handling, and no
  responsive layout.
- No drag-and-drop import in v1. `egui` receives dropped files on the web with
  an async `bytes_async()`, so it is a small follow-up, not a redesign.
- No offline service worker, no PWA manifest, no multi-document tabs, no
  reverse stepping (still deferred in `gui.md`).
- No browser integration tests in the gate. See "Testing".

## What the spike proved

Every claim below was checked in this session, not inferred from documentation.

| Check | Command | Result |
|---|---|---|
| the model, analysis, and session compile for wasm | `cargo check -p editor --target wasm32-unknown-unknown` | compiles, 4.6 s after dependencies |
| the panels and egui compile for wasm with no shell | `cargo check -p gui --no-default-features --target wasm32-unknown-unknown` | compiles |
| no RNG or temp-file crates enter the wasm graph | `cargo tree -p gui --no-default-features --target wasm32-unknown-unknown -e normal` | no `getrandom`, `rand`, `tempfile`, or `jobserver` |
| `cranelift-jit` does **not** build for wasm | `cargo check -p codegen --target wasm32-unknown-unknown` | fails: `region 3.0.2` has 11 errors (`QueryIter` is absent in its wasm path) |
| eframe + glow + persistence + rfd + wasm-bindgen link for wasm | a scratch cdylib with those dependencies, `cargo check` and `cargo build` for wasm32 | compiles; links a 2.9 MB debug `.wasm` before `wasm-bindgen`/`wasm-opt` |
| `unsafe_code = "forbid"` accepts macro output | same scratch crate with the lint and a `#[wasm_bindgen]` struct | no diagnostic; the crate's lint table needs no change |
| the CLI version must match the crate exactly | `wasm-bindgen` 0.2.129 on `PATH` against a 0.2.130 crate | hard error: `rust Wasm file schema version: 0.2.130 / this binary schema version: 0.2.129` |
| eframe's native dependencies are target-gated | `eframe-0.37.0/Cargo.toml` | `egui-winit`, `glutin`, `winit`, `image`, `pollster` are all under `cfg(not(target_arch = "wasm32"))` |
| the web shell API exists and matches the desktop trait | `eframe-0.37.0/src/web/{web_runner,panic_handler,storage}.rs`, `epi.rs` | `WebRunner::{new,start,destroy,has_panicked,panic_summary,app_mut}`, one `App` trait, `cc.storage` populated by `LocalStorage` |
| rfd's wasm surface | `rfd-0.17.2/src/{lib,file_dialog}.rs`, `file_handle/web.rs`, `backend/wasm.rs` | `AsyncFileDialog` is exported on wasm; `FileDialog` (blocking) is not; `FileHandle::{file_name,read,write}`, no `path` |
| `web-time` is a re-export off wasm | `web-time-1.1.0/src/lib.rs`, `src/time/mod.rs` | `pub use std::time::*`; only `Instant` and `SystemTime` are replaced, `Duration` is `std::time::Duration` |
| CI can install the toolchain pieces | `taiki-e/install-action` `TOOLS.md` | `wasm-bindgen` (alias `wasm-bindgen-cli`) and `trunk` are both supported from GitHub Releases |

The pinned versions in the lock are `wasm-bindgen 0.2.130`, `js-sys 0.3.107`,
`web-sys 0.3.107`, `wasm-bindgen-futures 0.4.80`, `web-time 1.1.0`.

## Crates and targets

```
canvas ── tessera ── interp ── runtime
   │         │          │
   └──────── editor ─────┘
                │            ▲
               gui ──────────┘
              /   \
   native.rs       web.rs
   eframe + rfd    eframe + rfd + wasm-bindgen
        │                │
   pietrs editor    target/web/{index.html,pietrs.js,pietrs_bg.wasm}
```

| Target | Command | Shape |
|---|---|---|
| desktop | `pietrs editor` | default features: `native` + `codegen` |
| browser | `cargo run -p xtask -- wasm` | `--no-default-features --features web` for `wasm32-unknown-unknown` |
| headless tests | `cargo test -p gui --no-default-features` | no shell at all; the panels and the app |

`cli` never builds for wasm: it is a binary with `clap`, filesystem arguments,
and `std::process`. The browser bundle is built from `gui`, which is already a
library. That is why "no second binary" survives this proposal: there is no
second binary, only a second target for one library.

## The time source

`editor::session` measures its per-call budget with `std::time::Instant`:

```rust
let started = Instant::now();
// ...
if steps >= budget.steps || started.elapsed() >= budget.time {
    return Ok(Stop::Budget);
}
```

`Instant::now()` panics on `wasm32-unknown-unknown`. The fix is one import:

```rust
// editor/src/session.rs
use web_time::{Duration, Instant};
```

`web-time` re-exports `std::time::*` off wasm, so the desktop behaviour and the
public type of `Budget { time: Duration }` are unchanged — `Duration` is
verbatim `std::time::Duration` on both sides, and only `Instant` is
substituted. `web-time` is already in the lock as an `eframe` dependency, so the
native dependency graph gains no new crate and no new build.

`perf` keeps `std::time`: it is a manual, `#[ignore]`d harness on the host and
is never in the wasm graph.

**Rejected:** a `Clock` trait threaded through `Budget`. It is more code, more
API, and more test surface than the problem earns; `web-time` is precisely the
shim for this.

## File I/O: the shell owns the dialog, the app owns the document

Today `EditorApp` calls `rfd::FileDialog` directly — blocking, behind
`#[cfg(feature = "native")]`, with a `#[cfg(not(...))]` stub that reports
"file dialogs need the native feature". The web cannot use that shape: its
dialog is asynchronous, and there is no `FileDialog` type on wasm at all.

The app stops talking to the OS. It states what it needs and applies what came
back.

### The two types

```rust
/// A file operation the app cannot perform itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRequest {
    /// Pick an image and hand back its bytes.
    Open,
    /// Write these bytes; `path` is the current file, if there is one.
    ///
    /// The PNG is encoded when the request is made, not when the dialog
    /// closes, so a slow dialog cannot change what gets saved.
    Save { path: Option<PathBuf>, bytes: Vec<u8> },
}

/// The outcome of a request, applied on a later frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoResult {
    Opened { bytes: Vec<u8>, path: Option<PathBuf> },
    Saved { path: Option<PathBuf> },
    /// The user dismissed the dialog; the document is unchanged.
    Cancelled,
    Failed(String),
}
```

`EditorApp` gains `request: Option<FileRequest>` and `io_pending: bool`, and
two methods:

- `take_request() -> Option<FileRequest>` — drained by the shell each frame;
  sets `io_pending`. `Save` carries the encoded PNG.
- `apply_io(IoResult)` — routes `Opened` through the existing `open_bytes`,
  `Saved` through the existing "this is the current file and it is clean" path,
  `Failed` into `last_error`; clears `io_pending`.

A failed encode while building a `Save` request becomes `last_error` and no
request is emitted.

`io_pending` is the web's race fix: while a request is outstanding the canvas
ignores edits, so a file picked a second later cannot silently replace work
typed in the meantime. On the desktop the flag is set and cleared inside one
call, so it is inert.

`open_file`, `save_to`, and `FileError` move to `native.rs`: they are the
desktop's filesystem, and nothing outside `gui` uses them. `app.rs` loses
`open_dialog`/`save_dialog` and both of their `cfg` arms. The shell finishes
every operation through `apply_io`, including `IoResult::Saved`, so one method
owns `io_pending`.

### The native shell

Unchanged in behaviour, and synchronous:

```rust
fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    self.app.ui_on(ui);
    match self.app.take_request() {
        Some(FileRequest::Open) => { /* rfd::FileDialog::pick_file + open_file */ }
        Some(FileRequest::Save { path, bytes }) => {
            /* `path`, or `FileDialog::save_file()`; then write_to + note_saved */
        }
        None => {}
    }
}
```

The temp-file round-trip test ("save to a path, reopen it, same grid") moves to
`native.rs` and becomes a `#[cfg(all(test, feature = "native"))]` case, which is
where the filesystem now lives.

### The web shell

The dialog resolves on a later tick, so the result comes back through an
`Rc<RefCell<Vec<IoResult>>>` that `ui` drains before it draws:

```rust
impl eframe::App for WebApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for result in self.results.borrow_mut().drain(..) {
            self.app.apply_io(result);
        }
        self.app.ui_on(ui);

        if let Some(request) = self.app.take_request() {
            let results = Rc::clone(&self.results);
            let ctx = ui.ctx().clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(result) = service(request).await {
                    results.borrow_mut().push(result);
                }
                ctx.request_repaint();
            });
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "theme", &self.app.theme().as_str());
    }
}
```

`service` is the only browser-specific code, and it never touches the app:

```rust
async fn service(request: FileRequest) -> Option<IoResult> {
    match request {
        FileRequest::Open => {
            let file = rfd::AsyncFileDialog::new()
                .add_filter("Piet image", &["png", "gif"])
                .pick_file()
                .await?;                       // None when the user cancels
            let name = file.file_name();
            let bytes = file.read().await;
            Some(IoResult::Opened { bytes, path: Some(name.into()) })
        }
        FileRequest::Save { path, bytes } => {
            let name = /* the path's file name, or "untitled.png" */;
            let file = rfd::AsyncFileDialog::new()
                .set_file_name(name)
                .save_file()
                .await?;                       // on wasm: resolves immediately
            match file.write(&bytes).await {   // triggers the browser download
                Ok(()) => Some(IoResult::Saved { path }),
                Err(error) => Some(IoResult::Failed(error.to_string())),
            }
        }
    }
}
```

Two browser facts to record in the code, not discover later:

- **There are no paths.** A `FileHandle` has `file_name()` and no `path()`. The
  document's `path` becomes a display name: the title bar, the save dialog's
  suggested name, and nothing else. Opening `report.png` twice looks identical,
  and there is no "same file on disk" notion.
- **Cancelling a save is invisible.** `save_file` resolves before the browser
  asks where to put the file, and `write` cannot report a cancel. The document
  is marked clean after `write` returns `Ok`. This is documented in the shell;
  it is the price of the download model.

## The web entry point

`gui/src/web.rs` holds the `#[wasm_bindgen]` handle. `gui` gains
`crate-type = ["rlib", "cdylib"]`; a dependency of `cli` is still built as an
rlib, and only the wasm build links the cdylib.

```rust
// gui/src/web.rs
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
    results: Rc<RefCell<Vec<IoResult>>>,
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self { /* WebRunner::new() installs the panic handler */ }

    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        self.runner.start(canvas, eframe::WebOptions::default(), /* app creator */).await
    }

    pub fn destroy(&self) { self.runner.destroy(); }
    pub fn has_panicked(&self) -> bool { self.runner.has_panicked() }
    pub fn panic_message(&self) -> Option<String> {
        self.runner.panic_summary().map(|summary| summary.message())
    }
}
```

The app creator mirrors `native.rs`: `EditorApp::new(Document::new(32, 32)?, None)?`,
then the theme from `cc.storage`. eframe provides `Some(storage)` on the web when
`persistence` is on, backed by `localStorage`, so the theme survives a reload
with the same `eframe::get_value`/`set_value` code the desktop already uses.

`WebLogger` is not installed: the crate has no `log` dependency today, and
eframe's panic handler already reports to the console. No `console_error_panic_hook`
either — `WebRunner::new()` installs a handler that keeps the panic summary for
`has_panicked`/`panic_message`, which is strictly better.

### `crates/gui/web/index.html`

Checked in, no templating, no build-time directive processing:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>pietrs</title>
    <link rel="stylesheet" href="pietrs.css">
  </head>
  <body>
    <canvas id="pietrs"></canvas>
    <div id="error" hidden></div>
    <script type="module">
      import init, { WebHandle } from "./pietrs.js";

      const show = (message) => {
        const box = document.getElementById("error");
        box.hidden = false;
        box.textContent = message;
      };

      await init();
      const handle = new WebHandle();
      window.pietrs = handle;
      try {
        await handle.start(document.getElementById("pietrs"));
      } catch (error) {
        show(String(error));
      }
      window.addEventListener("error", (event) => show(event.message));
    </script>
  </body>
</html>
```

`pietrs.css` is three rules: the body fills the viewport, the canvas is a block
that fills it, and `#error` is a readable overlay. The canvas needs an explicit
size because eframe observes it with a `ResizeObserver`; without CSS the canvas
is 300x150 and the editor renders into that box.

`ViewportCommand::Title` is still sent by `update_title`. eframe's web backend
has no title arm, so it falls into the catch-all that logs
`Unhandled egui viewport command`; no logger is installed, so nothing is
printed, and the page title stays the `<title>` element. `title()` still drives
the toolbar label, because the opened file's name is stored as the document's
path.

### Why not Trunk

Trunk is a fine tool, and it would remove one problem: it downloads the
`wasm-bindgen` CLI that matches the built wasm. The cost is a second build
orchestrator with its own config format, its own `data-trunk` directives, its
own asset naming, and its own upgrade cadence — for a bundle of four files that
one `wasm-bindgen` invocation and one `cp` produce.

The project already pins every version it depends on and already has an `xtask`
with external-tool subcommands (`conformance`, `e2e`) that fail by name when the
tool is missing. `cargo run -p xtask -- wasm` is that pattern applied to a
second external tool.

**Rejected:** `wasm-pack`, which wraps `wasm-bindgen` and adds a `package.json`
and a bundler target we do not need.

## What the web build cannot do

`cranelift-jit` does not compile for `wasm32-unknown-unknown` (verified: it
fails inside `region`, which allocates executable pages). Two consequences:

- `gui/web` must not enable `gui/codegen`. The buttons behind it are already
  `#[cfg(feature = "codegen")]`, so the debugger panel loses "Compare with JIT"
  and "Build an executable" with no new `cfg` in the UI code.
- `aot` is excluded, because it depends on `codegen`. `aot/build.rs` also
  shells out to `rustc` to cross-compile runners, which could not run under wasm
  anyway.

The document model, the analysis, the session, the breakpoints, the trail, and
the step budget are all fully available. Only the two optional jobs are missing,
and their absence is a feature boundary of the browser, not a bug.

## Build tooling and version pinning

`wasm-bindgen` requires the CLI's version to match the crate's exactly. This is
not advisory: a mismatch aborts with

```
error: failed to read wasm-bindgen data from '.../gui.wasm'
    rust Wasm file schema version: 0.2.130
       this binary schema version: 0.2.129
```

Therefore:

- `[workspace.dependencies] wasm-bindgen = "=0.2.130"`, with `web-sys =
  "=0.3.107"` pinned to the release that corresponds to it; `js-sys` follows
  transitively. One deliberate bump moves all three, like the Cranelift family.
- `xtask wasm` reads the locked version, runs `wasm-bindgen --version`, and if
  they differ fails with the exact `cargo install` line to run. A missing tool
  is named, never silently skipped.
- CI installs `wasm-bindgen-cli@0.2.130` with
  `taiki-e/install-action` (a prebuilt download; `cargo install` would be a
  two-minute compile in every leg, the same trade already made for `cargo-deny`).

`cargo run -p xtask -- wasm`:

```
cargo build --locked --release -p gui --no-default-features --features web \
    --target wasm32-unknown-unknown
wasm-bindgen --target web --out-name pietrs --out-dir target/web \
    target/wasm32-unknown-unknown/release/gui.wasm
cp crates/gui/web/index.html crates/gui/web/pietrs.css target/web/
```

The output is `target/web/index.html` plus `pietrs.js`, `pietrs_bg.wasm`, and
`pietrs.css`. `target/` is already ignored, so nothing generated is committed.

## Testing

| Layer | Home | Cases |
|---|---|---|
| Unit | `editor` | the existing budget cases still stop on steps and on time; the import swap is invisible to them |
| Unit | `gui` | `request_open` yields `FileRequest::Open` when clean, `Pending::Open` when dirty; `save` yields `Save { path: Some(_), bytes }` for a named document and `Save { path: None, .. }` otherwise, with bytes that decode back to the document; `apply_io` handles `Opened`, `Saved`, and `Failed` |
| Integration | `gui`, `Context::run_ui` | a frame exposes a pending request; a result replaces the document, sets the title, and clears `dirty`; an edit is ignored while `io_pending` |
| Integration | `gui/native.rs` | the temp-file write/open round trip (moved from `app.rs`) |
| Build | gate | `cargo build -p gui --no-default-features --features web --target wasm32-unknown-unknown` (debug profile: the gate proves the target links, and a release build adds minutes to every leg for no extra coverage) |
| Manual | `xtask wasm` + a browser | paint, step, run, breakpoints, open a PNG, save a PNG, switch the theme, reload and confirm the theme persisted, resize the window |
| Smoke | `xtask wasm` + headless Edge | the page boots, sizes the canvas, and runs frames without panicking |
| Not in the gate | — | `wasm-bindgen-test` in a headless browser |

The browser test is deliberately out. It needs chromedriver or geckodriver on
every runner, it tests the same `EditorApp` the headless frame tests already
drive, and the wasm-only code is `WebHandle` (four delegating methods) and the
`service` function (two rfd calls). The honest coverage is: put every rule in
`EditorApp`, unit-test it there, and keep `web.rs` too small to hide a bug.
If browser tests are ever wanted, `cargo run -p xtask -- wasm-test` is the
natural home and still not a gate.

What was run once, by hand, is a headless smoke test. Serve `target/web` over
HTTP and let `msedge --headless=new --enable-unsafe-swiftshader
--virtual-time-budget=6000 --dump-dom` load the page: the canvas comes back with
eframe's `tabindex` and a real `width`/`height`, the `#error` element stays
hidden, and a probe that calls `has_panicked()` after three seconds reports
`false`. That covers module load, WebGL context creation, the first frames, and
the absence of a panic; it does not cover the dialogs or the painting, which
need a person and a mouse.

The existing perf case in `editor` (partition plus analysis for a 512x512
document) is a host measurement and stays on the host. A browser repaint of a
512x512 texture per edit is the noticeable cost; it is listed under Risks, not
measured by a gate.

## CI and the gate

- `rust-toolchain.toml` gains `wasm32-unknown-unknown` in `targets`. `rustup
  show` already runs in every gate job and materialises the file, so no new
  install step is needed; the CI comment about the musl targets extends to the
  wasm standard library.
- A new `tools/gates/08-wasm.sh` builds the web shape and prints
  `OK: wasm` / `FAIL: wasm`. It is a compile-and-link gate, so it needs no
  `wasm-bindgen` and no browser. It runs on all three runners: the wasm
  dependency graph is disjoint from the host graph, `Swatinem/rust-cache` keys it
  by target, and a Windows-only or arm64-only failure is exactly the kind of
  thing the three-way gate exists to catch.
- `05-features.sh` is unchanged. The web shape is not a feature of the shipped
  binary; it is a different target for one library.
- `cargo test --workspace` is unchanged: the new tests are host tests.
- `cargo deny`: no `deny.toml` change is expected. `wasm-bindgen`, `js-sys`,
  `web-sys`, `wasm-bindgen-futures`, and `web-time` are already in the lock
  through `eframe` and already pass the licence gate; the spike re-runs
  `cargo deny check` to confirm rather than assume.
- Optional, last step: a `web` job that runs `xtask wasm`, tars `target/web`,
  and attaches `pietrs-web-<ref>.tar.gz` to a tag's release beside the binary
  archives. It cannot join the `build` matrix (that job is `-p cli`), and it must
  not gate `publish` unless we decide the bundle is a release artefact.

## Dependencies, lints, and features

Workspace additions:

```toml
web-time = "1.1"
wasm-bindgen = "=0.2.130"
wasm-bindgen-futures = "0.4"
web-sys = { version = "=0.3.107", features = [
    "Document", "HtmlCanvasElement", "Window",
] }
```

`gui`:

```toml
[lib]
crate-type = ["rlib", "cdylib"]

[dependencies]
eframe = { workspace = true, optional = true }
rfd = { workspace = true, optional = true }
wasm-bindgen = { workspace = true, optional = true }
wasm-bindgen-futures = { workspace = true, optional = true }
web-sys = { workspace = true, optional = true }

[features]
default = ["native", "codegen"]
native  = ["dep:eframe", "dep:rfd"]
web     = ["dep:eframe", "dep:rfd", "dep:wasm-bindgen", "dep:wasm-bindgen-futures", "dep:web-sys"]
codegen = ["dep:codegen", "dep:aot"]
```

`web` and `native` select different shells and must never be on together. A
`compile_error!` states that, and `web.rs` is additionally gated on
`target_arch = "wasm32"` so a mistaken host build fails with a sentence instead
of a wall of `web-sys` errors.

`editor` adds `web-time.workspace = true` and nothing else.

Lints: `gui` keeps `unsafe_code = "forbid"`. The wasm-bindgen macro expands to
`unsafe` code, and the spike confirmed that `forbid` does not fire on
macro-generated code, so the crate's existing three-line lint table is
unchanged. `float_arithmetic` stays allowed in `gui` for egui's geometry and
denied everywhere else; the wasm build is still float-free in the language
crates.

## Risks

1. **The `Instant` panic — addressed in step 1, not discovered at runtime.**
   Every budget check calls `Instant::now()`. Swapping to `web-time` is the first
   delivery step, with the full gate green before any wasm code exists.
2. **`wasm-bindgen` version drift.** Pinned to `=0.2.130`; `xtask wasm` fails
   with the exact version and install command; CI installs the same number. An
   `eframe` upgrade that moves the schema is one commit that changes both.
3. **Bundle size.** A debug link of the egui/eframe stack is already 2.9 MB
   before `wasm-bindgen` and `wasm-opt`; the editor adds more. The first
   measurable target is a release bundle small enough to load over a phone
   connection; if it is not, `opt-level = "s"` and `wasm-opt -Oz` are the knobs
   and neither is a structural change.
4. **No paths on the web.** The document's identity is a file name. Save is
   always a download. A user who edits the same file twice ends up with
   `report.png` and `report (1).png`; that is the browser's model, and the shell
   documents it rather than pretending otherwise.
5. **Cancelling a save looks like success.** `save_file` resolves before the
   browser prompts, and `write` cannot report a cancel. The document is marked
   clean. Accepted and documented; the alternative is the File System Access
   API, which is not portable and needs a user gesture per handle.
6. **The dialog race.** The native dialog blocks the frame; the web dialog does
   not. `io_pending` freezes edits while a request is outstanding, and a test
   proves the freeze.
7. **CI cost.** The wasm target adds one cold compile of the egui/eframe stack
   per runner. It is cached by target and is a build, not a test suite; the
   spike measures it before the gate adopts it.
8. **`crate-type = ["rlib", "cdylib"]` links an extra artefact** for host builds
   (`cargo clippy --workspace --all-targets` in particular). Harmless, and the
   cost of keeping the wasm entry beside the native one instead of in a second
   crate.
9. **`egui` churn.** Already pinned workspace-wide. The web shell uses only
   `WebRunner` and `WebOptions`, the two pieces eframe has kept stable across
   releases.

## Delivery plan

Each step lands green on its own; none depends on an unmerged later one.

1. **`editor`.** `web-time` for the session budget; the full gate green. No wasm
   artefact exists yet, so this step cannot break anything and nothing can
   forget it.
2. **`gui`.** `FileRequest`/`IoResult`, requests drained by `native.rs`,
   `open_file`/`save_to`/`FileError` moved to `native.rs`, the race guard, and
   the unit and frame tests. Desktop behaviour unchanged; the gate proves it,
   including the feature shapes.
3. **`gui` + wasm.** `rust-toolchain.toml` gains the target; the `web` feature,
   `web.rs`, `crate-type`, and `crates/gui/web/{index.html,pietrs.css}`;
   `cargo build -p gui --no-default-features --features web --target
   wasm32-unknown-unknown` green on all three hosts.
4. **`xtask`.** The `wasm` subcommand with the version check and the named
   failure; a release bundle in `target/web/`.
5. **Wiring.** `08-wasm.sh`, `cargo deny` re-run, a README section beside the
   editor's, this note marked implemented, and the optional `web` CI job
   decided.

## Implementation status

Landed, with the full gate green after each step:

- `editor` takes `web-time` for its stepping budget. The time branch of the
  loop had no test; it has one now, and it fails if the branch is removed.
- `gui` emits `FileRequest` and applies `IoResult`. `open_file`, `save_to`, and
  `FileError` live in `native.rs`, the document path has no `rfd` or `std::fs`
  left in `app.rs` (the AOT build dialog stays native-only, behind `codegen`),
  and every method that changes the document refuses while a request is in
  flight. Eight tests cover the request, the result, the cancel and failure
  paths, and the frozen-edit rule.
- `gui` builds for `wasm32-unknown-unknown` with the `web` feature and
  `crate-type = ["rlib", "cdylib"]`; `crates/gui/web/` holds the page.
- `cargo run -p xtask -- wasm` produces `target/web`. The release bundle is
  6.6 MB of wasm and 87 KB of JS before `wasm-opt`. Both version-check failures
  were exercised against a stub: a missing CLI and a 0.2.129 CLI each fail with
  the exact command that fixes them.
- `08-wasm.sh` joins the gate on all three runners; `cargo deny` needed no
  change, as predicted; the README has a browser section and a gate table row.
- The bundle was loaded in headless Edge: the canvas was sized by eframe, the
  error element stayed hidden, and a probe reported `panicked=false` after
  three seconds. Painting, the dialogs, and the theme round trip still want a
  person.

Still open: the two questions below.

## Open questions

- Is the web bundle a release artefact? If yes, step 5 adds the `web` job and
  `publish` waits for it; if no, it stays a documented `xtask` command.
- Is a hosted demo (GitHub Pages) wanted? It is one more CI job and a URL, and
  it makes the editor tryable without a download — but it also makes the
  project's CI depend on a deployment target, which is a deliberate choice, not
  a default.
- Drag-and-drop import (eframe delivers dropped files on the web) is small
  enough to fold into step 4 if it is wanted in v1.
