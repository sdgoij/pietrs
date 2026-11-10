# The GUI: visual editor and debugger

Status: implemented through the headless editor, the debug session, the
`pietrs editor` entry point, and the first GUI milestone (open, render, check,
theme). The editing tools, debugger panels, and import/export dialogs are the
remaining phases.

A desktop editor and debugger, reached with `pietrs editor`, that opens a Piet
image, edits it codel by codel, and runs it under the reference interpreter one
transition at a time. The UI is `egui`/`eframe`; it supports a light, a dark, and
a system theme, and it imports and exports PNG images.

The GUI is not a fourth execution path. Stepping uses `interp`; the JIT and the
AOT executable are invoked only to compare output and to build a program. The
trace format, the movement rules, and the integer-only core stay untouched.

## Decisions at a glance

| Decision | Choice | Why |
|---|---|---|
| Toolkit | `egui` 0.37, `eframe` 0.37, `rfd` 0.17 | immediate mode needs no retained tree; one binary; a frame can be driven headless |
| Shape | new `editor` crate (headless) + new `gui` crate (egui) | the model, undo, and debugger are tested without a window |
| Entry point | `pietrs editor`, a subcommand of the existing binary | one artefact, one version, one set of commands to learn |
| Execution | `interp::Machine`, one transition per `step` | the debugger and `pietrs trace` become one implementation, not two |
| Document | the PNG is the document | no private project format to design, version, and migrate |
| Export | PNG at the document's codel size | the palette is exact; GIF import remains |
| Theme | `System`, `Light`, `Dark`, persisted | system default with a manual override |
| Release | the editor is a default feature; the spike decides the musl artefact | the shipped Linux build is static musl, which a GUI has to prove |

## Non-goals for v1

- No graph drawing (force-directed or layered). The state list, the successor
  list, and the termination verdict are enough; a graph view is a follow-up.
- No reverse stepping. The execution is deterministic, so "replay to step N"
  is feasible, but it needs a second machine and a recorded input; deferred.
- No GIF export, no multiple documents, no tabs, no crash recovery.
- No wasm build. `egui` supports it; file dialogs and the build action do not
  fit the same shape, so it is a separate proposal.
- No second binary. Everything the editor needs is reached through `pietrs
  editor`; `pietrs` without the `editor` feature is still a complete compiler
  and interpreter.

## Crates and layering

```
canvas ── tessera ── interp ── runtime
   │         │          │
   └──────── editor ─────┘
                │
               gui ── egui ── eframe + rfd   (gui/native)
                │
               cli                          (cli/editor)
                │
          pietrs editor
```

| Crate | Role | Depends on |
|---|---|---|
| `editor` | document, undo, analysis cache, debug session | `canvas`, `tessera`, `interp`, `runtime` |
| `gui` | panels, canvas rendering, input, theme, file dialogs; **library only** | `editor`, `egui`; `eframe`/`rfd` behind the `native` feature |
| `cli` | the `pietrs` binary and the `editor` subcommand | `gui` behind the `editor` feature |

`editor` is integer-only and inherits the workspace lints unchanged. `gui` is
the only crate that performs float arithmetic, because egui's geometry is
`f32`; it takes a local `allow` (see "Lints").

## Changes to existing crates

The language crates need three additions; the CLI entry point is a fourth and
is described after the `editor` and `gui` crates. Each is small and
independently testable.

### `canvas`: encode, and a codel size that is not 1

`canvas` decodes; it cannot encode, and `CodelGrid::from_colors` forces
`codel_size = 1`, so the editor cannot round-trip a 4-codel image.

```rust
impl CodelGrid {
    /// Builds a grid with an explicit codel size. Codels are one per tile.
    pub fn from_colors_with_codel_size(
        width: u32,
        height: u32,
        codel_size: u32,
        codels: Vec<Color>,
    ) -> Result<Self, Error>;

    /// Expands each codel to a `codel_size` x `codel_size` RGBA tile.
    pub fn to_rgba(&self) -> Result<Vec<u8>, Error>;

    /// Encodes `to_rgba` as an 8-bit RGBA PNG.
    pub fn encode_png(&self) -> Result<Vec<u8>, Error>;
}
```

`from_colors` keeps its current meaning (`codel_size` 1) so no caller changes.
`Error` gains `PngEncode(String)`; `Error::Png` is the decoder's.

The encoder lives in `canvas`, not in `gui`, because it is the inverse of the
decoder and because `testkit` already has the only production-adjacent encoder
today and production crates must not depend on `testkit`.

Tests: every one of the twenty colours round-trips through
`encode_png` -> `from_image`; a 3-codel grid encodes to `3 * width` by
`3 * height` pixels; every codel size in 1..=4 tiles correctly; a size that
does not divide the codels is rejected.

### `interp`: a machine that can stop after one transition

`run` and `run_traced` share a private `run_inner` loop and cannot be entered
stepwise; there is no way to observe the stack between transitions. Refactor
the loop into a `Machine` and keep `run` as a driver.

```rust
/// Why one call to `step` returned.
pub enum Progress {
    Moved {
        from: State,
        to: State,
        command: Option<Command>,
        target: canvas::BlockId,
    },
    /// The program stopped. `from` is `None` when it could not start at all
    /// (a black or white-dead end upper-left codel), which records no step.
    Terminated { from: Option<State> },
    StepLimit,
}

/// The execution state, independent of the blocks it walks.
pub struct Machine {
    state: Option<State>,
    stack: Stack,
    steps: u64,
    options: Options,
    finished: bool,
}

impl Machine {
    pub fn new(blocks: &Blocks, options: Options) -> Self;

    /// One transition. `Ok(None)` means the machine had already finished.
    ///
    /// `blocks` must be the partition the machine was created from.
    pub fn step(&mut self, blocks: &Blocks, io: &mut dyn Io)
        -> Result<Option<Progress>, Error>;

    pub fn state(&self) -> Option<State>;
    pub fn stack(&self) -> &Stack;
    pub fn steps(&self) -> u64;
    pub fn is_finished(&self) -> bool;
}
```

`Progress::step()` returns the `Step` the trace records, or `None` for a
step-limit or an unstartable program:

```rust
impl Progress {
    pub fn step(&self) -> Option<Step>;
}
```

`run` becomes:

```
machine = Machine::new(blocks, options)
loop {
    match machine.step(blocks, io)? {
        None => return Terminated at 0 steps,
        Some(Moved) => continue,
        Some(Terminated { from }) => return Terminated,
        Some(StepLimit) => return StepLimit,
    }
}
```

`run_traced` is the same loop, pushing `progress.step()` when it is `Some`.
The order of checks is load-bearing: the step limit is tested before start
termination, exactly as `run_inner` does today, so `--max-steps 0` still
reports the limit for a program that could not start. `Machine` owns no
borrow of `Blocks`; `step` takes the partition, which is what makes a
`Session` that owns both possible without a self-reference.

The proof that this is a refactor and not a rewrite is the existing corpus:
the trace goldens, `expected.stdout`, and `expected.exit` must be unchanged.

Tests: for every fixture, stepping with `Machine` yields the same `Vec<Step>`,
step count, and reason as `run_traced`; a step limit of zero matches the
current output; a start-terminated program produces no step and no error; a
stack overflow still surfaces as `interp::Error`.

### `tessera`: the edge codel of a resolved movement

The debugger draws the direction pointer against the block's edge. That codel
is computed inside `exit` and thrown away. Extract the resolution and expose
the codel without changing `Exit`:

```rust
/// The codel the movement rules leave from, or `None` when no attempt finds
/// an exit.
pub fn edge_codel(blocks: &Blocks, block: BlockId, dp: Direction, cc: Chooser)
    -> Option<Point>;
```

`exit` and `edge_codel` both call one private `resolve` function, so the arrow
and the executed move cannot disagree.

## The `editor` crate

Three types, no UI.

### `Document`

```rust
pub struct Document {
    width: u32,
    height: u32,
    codel_size: u32,
    codels: Vec<Color>,
    revision: u64,
    // undo and redo stacks of private `Change` entries
}
```

A `Change` is the state to restore: a paint records the colour each point had
when the stroke began, a resize snapshots the cropped grid, and a codel-size
change records the old scale. The public surface is `undo`/`redo`/`can_undo`;
the edit representation stays private.

- `from_grid(&CodelGrid)` and `to_grid(&self) -> Result<CodelGrid, Error>`.
  Import carries the detected codel size; export preserves it.
- `set`, `fill` (4-connected, matching block semantics), `resize` (top-left
  anchored, new codels white), `set_codel_size`.
- `begin_stroke`/`end_stroke` coalesce a drag into one entry; a stroke records
  each point once, so painting a codel twice does not duplicate the delta, and
  undo restores the colour from before the stroke, not an intermediate one.
- `undo`/`redo` swap the change with its inverse, so both directions share one
  implementation; a new edit clears the redo stack; the cap is 256 entries.
- Every mutation bumps `revision`. The analysis cache keys on it.
- `codel_size` is display and export only. It does not affect the program: the
  partition is over codels, which is why a 3-codel and a 6-codel render of the
  same picture are the same program.

Tests: each edit kind round-trips through undo/redo; a coalesced stroke is one
undo entry; redo is cleared by a new edit; resize anchors and fills correctly;
a proptest applies a random edit sequence and undoes it all, asserting the
document equals the original (codels, size, codel size).

### `Analysis`

```rust
pub struct Analysis {
    revision: u64,
    blocks: Arc<Blocks>,
    graph: StateGraph,
}
impl Analysis {
    pub fn refresh(&mut self, doc: &Document);
    pub fn blocks(&self) -> &Arc<Blocks>;
    pub fn graph(&self) -> &StateGraph;
    pub fn report(&self) -> Report;
}
```

`refresh` returns early when `revision` matches, so a frame with no edit does
no work. `Report` mirrors `pietrs check` as data, not text: grid size, codel
size, block count, state and edge counts, `can_terminate`, the start state,
unknown colours, and a palette histogram. The panel renders it. The CLI's
stdout wording is a contract and is deliberately left alone; a test compares
the numbers, not the strings.

### `Session`

```rust
pub struct Session {
    blocks: Arc<Blocks>,
    machine: Machine,
    io: BufferIo,
    input: String,
    trace: Vec<Step>,
    trail: Vec<Point>,
    stop: Option<Stop>,
    revision: u64,
}
pub enum Stop { Terminated, StepLimit, StackOverflow, Breakpoint(Breakpoint) }
```

- `new(&Analysis, input: String, options)` snapshots the revision.
- `step`, `run_until(&Breakpoints, budget)`, `reset`.
- `is_valid_for(&Document)`: a session is bound to the revision it was created
  from. Any edit abandons it. Editing while paused is allowed; editing while
  running stops the run first.
- The trace and the trail are the session's, not the machine's, so the machine
  stays the minimal execution primitive.
- `export_trace(&self) -> String` uses `Step`'s `Display`, which is the same
  formatter `pietrs trace` writes. There is no second trace formatter.

Breakpoints are a `BTreeSet` per kind: block ids, `(block, dp, cc)` states,
commands, and flags for "before input" and "after output". `run_until` checks
the *current* state before each step, and the pending command through
`tessera::exit`, so a breakpoint fires before the command runs.

Budget: `run_until` takes a step count and a `Duration` and returns why it
stopped. It is called once per frame, on the UI thread; see "Threading".

Tests: a session stepped to completion matches `run_traced` for every fixture;
a breakpoint on a block, a state, and a command each stop at the expected step;
reset restores step 0 and an empty stack; the exported trace is byte-identical
to `pietrs trace`'s stdout for the same fixture and limit.

## The CLI entry point

`pietrs editor` is a subcommand of the existing binary, not a second binary.

```
pietrs editor [IMAGE] [--codel-size N] [--input FILE] [--max-steps N]
              [--division floored|truncating]
```

- `Commands::Editor(EditorArgs)` is in the clap surface in every feature shape,
  so `pietrs --help` names it and shell completions do not change with the
  build.
- With `--features editor` the arm decodes the image when one is given, then
  calls `gui::run(gui::Args { .. })`. The image is decoded **before** a window
  opens, so a bad path or a malformed PNG is a one-line CLI error on stderr and
  a non-zero exit, not a blank window. No image starts a new 32x32 document.
- Without the feature the arm returns `Error::NoEditor` ("this pietrs was built
  without the editor feature"), the way `build` already reports a missing
  `codegen`. The stub compiles in every shape, which keeps the subcommand list
  stable.
- `gui::run` owns the window and returns `Result<(), gui::Error>`; `cli::Error`
  wraps it transparently. Closing the window exits 0; an eframe failure (no
  display, no GPU) is a named error, not a panic.
- `--division` and `--max-steps` seed the session defaults; the GUI can change
  them afterwards. `--codel-size` overrides detection exactly as the other
  commands do. `--input FILE` is read by the CLI, like `run`, so a bad path is
  also a pre-window error; the GUI receives the text.
- The window title is `pietrs - <file>`, with `*` for unsaved edits.

The feature is named after the command, not the crate it enables: `cli/editor`
depends on `gui`, and `gui/native` selects the window. A reader of `pietrs
--help` never needs to know the crate graph.

`gui` stays a library: `EditorApp` (egui only), `NativeApp` (eframe), and `run`
(the entry point). The CLI supplies the process, the exit code, and the argument
parsing; the GUI supplies the window. There is no `[[bin]]` in `gui`.

The feature graph has one deliberate weak edge:

```toml
# cli
[features]
default = ["codegen", "editor"]
codegen = ["dep:codegen", "dep:aot", "dep:tempfile", "gui?/codegen"]
editor  = ["dep:gui", "gui/native"]
```

`gui?/codegen` enables the GUI's JIT/build actions only when the GUI is enabled
at all, so `--no-default-features --features codegen` stays exactly today's
headless compiler. All four shapes are meaningful: interpreter only,
interpreter plus codegen, interpreter plus editor, and the full binary. The
workspace dependency on `gui` sets `default-features = false`; `cli` inherits
it. Without that, adding the optional dependency would silently turn on `gui`'s
defaults and there would be no editor-without-JIT shape to test.

## The `gui` crate

### Shape

```
gui::EditorApp          egui-only state and panels; no eframe types
gui::NativeApp          eframe::App, wraps EditorApp; the only window-aware code
gui::run(Args)          the entry point `pietrs editor` calls (native)
```

`EditorApp::ui_on(&mut self, ui: &mut egui::Ui)` is the whole interface. It
applies the theme and adds the panels. eframe 0.37's `App` trait hands the app
a `&mut Ui` (`fn ui(&mut self, ui, frame)`), so the shell is three lines and
has no context plumbing. Tests call `ui_on` through `egui::Context::run_ui`;
no window, no GPU, no `eframe::Frame`. The frame's `textures_delta` is cleared
in the test, because egui insists the backend consume it.

### Layout

```
┌─────────────────────────────────────────────────────────────────────────┐
│ File  Edit  View  Run  Help                                             │
│ [New] [Open] [Save] [Export PNG] │ [Pencil] [Rect] [Fill] [Pick]        │
│ [Undo] [Redo] │ 20 swatches │ [System] [Light] [Dark]                   │
├──────────────┬───────────────────────────────────────┬──────────────────┤
│ Document     │                                       │ Check            │
│  size        │            canvas + overlays          │  grid/blocks     │
│  codel size  │                                       │  states/edges    │
│ Palette      │                                       │  termination     │
│ Messages     │                                       │ Debugger         │
│              │                                       │  state, stack    │
│              │                                       │  output, controls│
│              │                                       │ Breakpoints      │
├──────────────┴───────────────────────────────────────┴──────────────────┤
│ codel (x, y) · colour · block 7 (size 12) · step 1042 · running         │
└─────────────────────────────────────────────────────────────────────────┘
```

`egui::Panel::{top,left,right,bottom}` around a `CentralPanel`. The 0.37 docs
use the unified `egui::Panel`; older examples use `SidePanel` and
`TopBottomPanel`, which this code does not.

### Canvas rendering

- One `egui::ColorImage` at one texel per codel, `TextureOptions::NEAREST`,
  rebuilt and uploaded only when `Document::revision` changes. Scaling one
  texture avoids the seams that adjacent per-codel quads produce.
- Zoom is points per codel. Wheel zooms around the pointer; middle-drag or
  space-drag pans; `0` fits the document. Grid lines are drawn only when a
  codel is at least 8 points, so a large image stays readable.
- A frame around the canvas in both themes: white codels on a light panel are
  otherwise invisible.
- Overlays, each a toggle in View:
  - block boundaries: edges between codels of different blocks, cached per
    revision;
  - current block: a tinted overlay texture, rebuilt on state change;
  - direction pointer: an arrow from `tessera::edge_codel`, with the chooser
    shown by which side of the arrow the arrowhead sits on;
  - trail: a polyline through the entry codel of each visited block, capped at
    16 384 points with a "trail truncated" note;
  - block ids and sizes, at zoom;
  - breakpoints, as markers in the panel and on the block.
- No overlay relies on colour alone: the current state, the breakpoint set, and
  the exit reason are also in text.

### Editing

| Tool | Input | Undo unit |
|---|---|---|
| Pencil | left drag | one stroke |
| Rectangle | left drag, preview | one edit |
| Fill | left click, 4-connected | one edit |
| Eyedropper | left click, right click in Pencil | none |

`Left` paints, `Middle`/`Space+Left` pans, wheel zooms. Palette shortcuts select
the current colour; `E` picks. Resize and codel size are dialogs with explicit
Apply, one undo entry each.

Warnings, not errors, because each is a legal program:

- the upper-left codel is white (the program starts with a slide) or black (it
  terminates immediately);
- the image has no chromatic block;
- the graph cannot terminate (`!can_terminate`), same verdict as `pietrs check`;
- the document is larger than 262 144 codels, at which point interaction may
  stutter; the hard cap is 1 048 576, matching `canvas`'s decode limit.

### Debugger

Controls: Step (F10), Run/Pause (F5), Run to step limit, Reset (Ctrl+R), step
limit field, division mode (`Floored`/`Truncating`), stack capacity.

Panels:

- **State**: block id and colour, block size, DP, CC, the pending command from
  `tessera::exit` (before it runs), the target block, step count, exit reason.
- **Stack**: depth and capacity, the top 64 values with indices, the rest
  collapsed. Read via `Machine::stack`.
- **Output**: the stdout buffer, selectable and copyable; a count of characters
  written.
- **Input**: an editable string, applied on reset only. Live stdin while paused
  would need a different `Io` contract; it is not in v1.
- **History**: the last N recorded steps in `Step` form, newest last; clicking
  a row selects the block it ran in. It is a log, not a time machine.

Breakpoints: click a block to toggle a block breakpoint; a state breakpoint is
added from the state panel; a command breakpoint from a command list. "Run to
next breakpoint" is `run_until`, bounded by the same step limit.

Editing invalidates the session and the trail; the controls say so before the
edit, not after.

### Theme

`enum ThemeChoice { System, Light, Dark }`, persisted. `System` sets
`ctx.set_theme(ThemePreference::System)`; the explicit choices set
`Light`/`Dark`. The canvas palette is fixed by the language; only the chrome,
grid lines, overlays, and the frame follow the theme. Overlay colours are
derived from the active `Visuals` so both themes keep contrast.

### Import, export, run, build

- Open: `rfd` filter `png`/`gif`; decode through `canvas`, with an optional
  codel-size override; unknown colours are reported with counts exactly as the
  CLI reports them.
- Save: writes the PNG (`.png` appended if absent). The document is the image,
  so there is no separate save format and no "save as project".
- Export trace: the session's `Step` lines, for producing a golden.
- Run (JIT): `codegen::run` on the same blocks and the same input, then a diff
  of stdout and exit reason against the session. This is the parity check made
  convenient; disagreement is shown as an error, never hidden.
- Build: `aot::build` with the chosen target, step limit, and division, on a
  worker thread (see below), writing where the user asks.

### Persistence

`eframe::Storage` (the `persistence` feature) holds the theme, window size,
panel widths, overlay toggles, recent directory, and last target. It does not
hold the document.

## Threading

- The editor, the debugger, and rendering run on the UI thread. A frame runs at
  most `4 ms` or `100_000` steps, whichever comes first, then requests a
  repaint. No thread, no channel, no shared state, and the same step semantics
  as `pietrs trace`.
- JIT runs and AOT builds can take seconds. They run on a worker with a cloned
  `Arc<Blocks>` and an owned input string, and return through an `mpsc` channel;
  the worker calls `ctx.request_repaint()` when it finishes. Only this path is
  threaded.
- `rfd`'s synchronous dialog blocks the UI thread for the duration of the
  dialog. That is acceptable for a modal dialog; it is documented, not hidden.

## Dependencies

Workspace additions, pinned like Cranelift:

```toml
egui = "0.37"
eframe = { version = "0.37", default-features = false, features = [
    "glow", "default_fonts", "persistence", "accesskit", "clipboard",
    "links", "x11", "wayland",
] }
rfd = "0.17"
```

`default-features = false` avoids the `wgpu` backend and its build cost; `glow`
is enough for 2D. `x11` and `wayland` are enabled explicitly (winit's `default`
feature is off), and `accesskit`, `clipboard`, and `links` keep the desktop
behaviour. The features are target-independent; on Windows the Linux-only
dependency edges are not compiled.

`egui_kittest` is deliberately not a dependency yet. The first milestone's
frame test uses `egui::Context::run_ui` directly, which needs no display and no
accessibility tree; `egui_kittest` is added with the first click-driven widget
test.

`cargo deny` was run at the spike, not left for later. It needed two deliberate
changes, both recorded in `deny.toml` with the GPL-2.0-only reasoning: `BSL-1.0`,
`ISC`, `OFL-1.1`, and `Ubuntu-font-1.0` are allowed (the last two are the
default fonts, which are data), and `RUSTSEC-2026-0370/0371/0372` are ignored —
the `zbus` split crates are unmaintained, not vulnerable, and egui 0.37 has no
newer `accesskit`.

### Lints

The workspace denies `clippy::float_arithmetic`, which is right for the
compiler pipeline and wrong for a UI: egui's public API is `f32` geometry.
Cargo does not let a package merge a local lint table with
`lints.workspace = true`, so `gui` restates the two workspace lints next to the
one override:

```toml
[lints.rust]
unsafe_code = "forbid"

[lints.clippy]
float_arithmetic = "allow"
unwrap_used = "warn"
```

The restatement is the price of the exemption. `cargo clippy --workspace
--all-targets -- -D warnings` proves it: the crate's float geometry passes and
the other lints are still errors.

`editor` takes no override. If a float appears there, the lint is the reviewer.

`unwrap_used` stays a warning workspace-wide and is therefore an error under
`-D warnings`. Neither new crate unwraps outside `#[cfg(test)]`.

### Cargo features

```toml
# gui
[features]
default = ["native", "codegen"]
native  = ["dep:eframe", "dep:rfd"]     # the window, the dialogs
codegen = ["dep:codegen", "dep:aot"]    # JIT comparison and `build`
```

No feature selects a binary; `gui` is a library in every shape. The panels need
only `egui`, so they are testable with `--no-default-features`. The shapes the
gate runs are:

| Shape | Command | What it proves |
|---|---|---|
| interpreter only | `cargo test -p cli --no-default-features` | the language runs with no codegen and no GUI (today's shape) |
| headless compiler | `cargo test -p cli --no-default-features --features codegen` | the release fallback shape: everything but the editor |
| editor, no JIT | `cargo test -p cli --no-default-features --features editor` | the editor and the debugger need no codegen |
| panels, no shell | `cargo test -p gui --no-default-features` | egui panels and stepping work with no eframe |
| full | `cargo test --workspace` | the default binary and the native shell |

All but the last are added to `tools/gates/05-features.sh`; the last is the
normal test gate. The two CLI subcommand assertions in the testing plan are
`cfg`-gated against each other, because one asserts that the editor opens and
the other asserts that it reports the missing feature.

### CI

`cargo test --workspace` now compiles `winit`/`glutin` on Linux, because it
builds the default `pietrs` with the `editor` feature. Neither should need a
development package at build time (`x11-dl` and the Wayland client load at
runtime), but this is the first thing the spike verifies on all three runners.
If a package is genuinely required, the gate installs it explicitly, the way it
already installs the musl targets; it does not drop the crate from the gate.

The GUI tests never open a window, so the Linux gate needs no display server.
The `build` job is unchanged: `cargo build --release --locked -p cli --target
<triple>` now produces a `pietrs` with the editor, and the spike proves that on
musl before it is promised.

## Testing plan

Following the layers in `AGENTS.md`:

| Layer | Home | Cases |
|---|---|---|
| Unit | `canvas` | encode round trip, all 20 colours, codel sizes, dimensions |
| Unit | `tessera` | `edge_codel` agrees with `exit` on all 8 attempts, including holes and slides |
| Unit | `interp` | `Machine` equals `run_traced` step for step; limit, termination, overflow |
| Unit | `editor` | edit kinds, undo/redo, stroke coalescing, analysis cache, breakpoints, trace export |
| Property | `editor` | random edits undo to the original; fill is total over the reachable region |
| Integration | `editor/fixtures/` | the checked-in corpus: step counts, traces, output, exit |
| Integration | `gui` (`Context::run_ui`) | open a fixture, one frame, panels show the expected numbers |
| Integration | `gui` | a computed canvas click paints one codel; export decodes back to the document |
| Integration | `gui` | theme choice sets `ctx.theme()` and survives a fake `Storage` |
| Integration | `cli` | `editor --help` is listed in every feature shape; a bad image path fails before a window opens |
| Integration | `cli` | without the `editor` feature the subcommand exits 1 with the named error |
| Degraded | `editor` | 1x1, single colour, white upper-left, black upper-left, malformed PNG |

`egui_kittest` drives widgets by their accessible label, so the palette buttons
and the step buttons must have accessible names in the first implementation, not
in a later accessibility pass.

A new `#[ignore]`d perf case in `editor` measures partition plus analysis for a
512x512 document and records the number in `.agents/notes/perf.md`. It does not
gate.

## Risks

1. **System libraries on Linux CI.** The GUI compiles and its tests pass on
   Windows. The Linux runners are the remaining check: if a package is
   genuinely required it is installed explicitly, the way the gate already
   installs the musl targets; the crate is not dropped from the gate.
2. **`cargo deny` licences.** New transitive crates can add a licence class.
   The gate is run in the spike, before the bulk of the UI exists.
3. **egui churn.** 0.37 is pinned for the workspace, like Cranelift. An upgrade
   is one deliberate commit that runs the full gate.
4. **Panel API naming.** The panel names in this note come from the 0.37 docs;
   the shell is written against 0.37, not against older `SidePanel` examples.
5. **The editor in the release binary.** `pietrs` links `winit`/`glow` in its
   default build, and the Linux artefacts are static musl. The spike is the
   gate: build the release command for all three targets and start `pietrs
   editor` on a stock desktop. If that fails, the release job builds
   `--no-default-features --features codegen`; the shipped binary keeps every
   compiler command, loses only the editor, and `pietrs editor` says exactly
   that. It is a recorded packaging decision, not a silent `cfg`.
6. **Large documents.** A 1024x1024 grid is a 4 MiB texture and a 1 MiB codel
   vector; undo deltas are bounded by the edit, not the document. The soft
   warning is there because a 512x512 partition is still fast but a repaint is
   not free.
7. **The `Machine` refactor.** The trace is a public contract. The corpus
   goldens are the proof that the refactor changed no output; if they move, the
   refactor is wrong.

## Delivery plan

Each step lands green with its tests; no step depends on an unmerged later one.

1. **Spike.** A `gui` crate with `run()`, the `pietrs editor` subcommand behind
   the `editor` feature, a blank window, and a nearest-neighbour texture.
   `cargo deny`, CI, and `cargo build --release -p cli` green on all three
   runners. Confirms the dependency, system-library, and packaging assumptions
   before any UI exists.
2. **`canvas`.** `from_colors_with_codel_size`, `to_rgba`, `encode_png`, tests.
3. **`interp`.** `Machine` and `Progress`; `run` and `run_traced` become
   drivers. Goldens unchanged.
4. **`tessera`.** `edge_codel` sharing `resolve` with `exit`.
5. **`editor`.** Document and undo, with the property test.
6. **`editor`.** Analysis, report, and the cache.
7. **`editor`.** Session, breakpoints, trace export.
8. **`gui` + CLI.** Shell, the `editor` subcommand and its arguments, layout,
   canvas rendering and overlays, theme persistence.
9. **`gui`.** Editing tools, palette, resize, codel size.
10. **`gui`.** Debugger panels, breakpoints, run budget.
11. **`gui`.** Import, export, JIT comparison, AOT build worker.
12. **Wiring.** The feature shapes in `05-features.sh`, the release decision
    from the spike, a README section and crate table row, this note marked
    implemented, perf number recorded.

## Implementation status

Landed, with tests, in the first implementation pass:

- `canvas`: `from_colors_with_codel_size`, `to_rgba`, `encode_png`; a round trip
  covers all twenty colours, codel sizes, and tile layout.
- `interp`: `Machine` and `Progress`; `run` and `run_traced` are drivers, and the
  corpus goldens did not move.
- `tessera`: public `edge_codel`, sharing `resolve` with `exit`.
- `editor`: `Document` with stroke-coalesced undo and a proptest round trip,
  `Analysis` with a revision-keyed cache and `Report`, `Session` with
  breakpoints, budgets, reset, trail, and trace export.
- `gui`: `EditorApp` with the palette-fixed canvas, fit/pan/zoom, the check
  panel, and the `System`/`Light`/`Dark` switch; a headless frame test.
- `cli`: `pietrs editor [IMAGE] [--codel-size N]`, the `editor` feature, the
  named missing-feature error, and tests in both shapes.
- Gates: `05-features.sh` runs the six shapes; `deny.toml` records the new
  licences and the three unmaintained-advisory ignores.

Still to build: the editing tools and palette, the debugger panels and
breakpoint UI, import/export and the JIT/build actions, theme persistence, and
the README's editor section. The release question in the risks is still open:
the GUI has been compiled and tested on Windows; the static musl artefact and
the two Linux runners have not been exercised here, so CI is the first proof.

## Invariants this proposal keeps

- The debugger is the interpreter. No GUI-only execution semantics.
- The trace format is unchanged; the GUI consumes `Step` and writes `Step`.
- The language surface is unchanged: no new command, no new arithmetic.
- `pietrs` remains a complete compiler with `--no-default-features`; the
  editor is a feature, not a dependency of the language crates.
- `interp` remains usable with no GUI, no JIT, and no window.
- The integer-only rule holds everywhere except `gui`, where it is explicitly
  and narrowly waived.
- No `unsafe`, no `TODO`, no generated artefact in the tree.

## Open questions

1. **Does the released `pietrs` include the editor?** Recommendation: yes for
   Windows and macOS, and for Linux if the spike's static musl build starts on
   a stock desktop. If it does not, the release is the headless shape and the
   editor ships from source until a glibc desktop artefact is packaged. The
   subcommand exists either way.
2. **Reverse stepping in v1?** Recommendation: no. "Replay to step N" is the
   honest version, and it is a second machine plus a recorded input.
3. **GIF export?** Recommendation: no. PNG is lossless and the palette is
   exact.
