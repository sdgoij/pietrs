# Piet compiler in Rust + Cranelift: feasibility

## Verdict

**Yes, feasible.** And Cranelift is a genuinely good fit, for a specific reason:
Piet's two-dimensional control flow is *static geometry*. All of the "weird" part
of the language can be resolved at compile time into a **finite CFG**, at which
point the backend sees a completely ordinary stack-machine program.

The one open-ended part of Piet is its value stack (unbounded depth, notionally
unbounded integers). Cranelift has no opinion about that; you provide it. That is
a runtime design decision, not a codegen blocker.

**Confidence:**
- Language model / compilation strategy: high.
- Cranelift lowering: high (it only needs integers, loads/stores, calls, br_table).
- Spec compatibility: medium (multiple published interpretations; `npiet` is the
  de-facto reference).
- Arbitrary-precision integers: medium (allowed by spec, not provided by Cranelift).

Estimated effort for a working AOT compiler over the core language: **1-2 focused
weeks**, *if* you build a reference interpreter first. The Cranelift work is the
small part; the frontend and semantics are the bulk.

---

## 1. What the language actually requires

Sources: the clarified 2018 spec at `dangermouse.net/esoteric/piet.html` and the
esolangs wiki mirror. Note the author explicitly warns that implementations
differ; the spec below is the clarified one.

### Grid and colours

- Program is a grid of **codels**. In an upscaled image, one codel is an N x N
  pixel tile, not a pixel.
- 20 colours: 18 chromatic (6 hues x 3 lightnesses) plus white and black.
- Hue cycle (forward): `red -> yellow -> green -> cyan -> blue -> magenta -> red`.
- Lightness cycle (forward = "1 darker"): `light -> normal -> dark -> light`.
- Non-standard colours are implementation-defined; the spec suggests treating
  them as white.

### Blocks

- A **colour block** is a maximal 4-connected region of one colour. Diagonal
  contact does not join blocks. Blocks may have holes.
- Every non-black, non-white block has an integer value = its **codel count**.

### Execution state

- `DP` (direction pointer): `Right | Down | Left | Up`, starts `Right`.
- `CC` (codel chooser): `Left | Right`, starts `Left`, always relative to `DP`.
- Start block = block containing the upper-left-most codel.

The interpreter state at a step boundary is therefore `(block, DP, CC)`.

### Movement

1. Take the codels of the current block on its edge furthest in the `DP`
   direction (e.g. max x for `Right`; this edge may be disjoint).
2. Among those, take the codel furthest in the `CC` direction relative to `DP`:

   | DP    | CC    | Codel chosen |
   |-------|-------|--------------|
   | right | left  | uppermost    |
   | right | right | lowermost    |
   | down  | left  | rightmost    |
   | down  | right | leftmost     |
   | left  | left  | lowermost    |
   | left  | right | uppermost    |
   | up    | left  | leftmost     |
   | up    | right | rightmost    |

3. Step one codel in the `DP` direction from it.

### Obstacles and termination

- If the step lands on black or off the grid: toggle `CC`; retry.
- If that fails: rotate `DP` clockwise one step; alternate the two adjustments.
- After 8 failed attempts there is no way out: **program terminates**.
  This is the only normal termination.
- Attempt pattern is `(DP + i/2, CC xor (((i+1)/2) mod 2))` for `i = 0.. 7`.

### White

White is a free zone, not a normal block:

- Entering white makes the interpreter **slide in a straight line** in the `DP`
  direction until a non-white codel is reached.
- **No command is executed** for a transition that crosses white.
- If the slide hits black/edge: toggle `CC` (a no-op for a straight ray), rotate
  `DP` clockwise, slide again from the *current* white codel. Retracing the route
  entirely inside white means terminate.
- After a successful slide you are in a coloured block and normal movement
  resumes.

### Commands

The command is a pure function of `(hue delta, lightness delta)` between the two
coloured blocks. Deltas are forward around their cycles, modulo 6 and 3.
If the transition crosses white, the command is skipped.

| hue \ lightness | none | 1 darker | 2 darker |
|-----------------|------|----------|----------|
| none            | -    | push     | pop      |
| 1 step          | add  | subtract | multiply |
| 2 steps         | divide | mod    | not      |
| 3 steps         | greater | pointer | switch |
| 4 steps         | duplicate | roll | in(number) |
| 5 steps         | in(char) | out(number) | out(char) |

Notable semantics:

- `push` pushes the **size of the block just exited** - a compile-time constant.
- `divide` / `mod` are **floored** (Python `//`, `%`; result of mod has the sign
  of the divisor). Divide by zero is implementation-defined; ignoring the command
  is recommended.
- `pointer` pops `v`, rotates `DP` clockwise by `v` (anticlockwise if negative).
- `switch` pops `v`, toggles `CC` `v` times (absolute value if negative).
- Operations that cannot be performed (e.g. pop on an empty stack) are **ignored**.
- Integer size is "notionally infinite", but implementations may impose a finite
  limit with overflow as a runtime error.

---

## 2. Why this is compiler-friendly

Define a compile-time function over the static image:

```
exit(block, dp, cc)
    -> Terminate
     | Move { target: Block, dp: Dir, cc: Cc, cmd: Option<Command> }
```

This function resolves, with no runtime input:

- the edge-codel choice,
- the 8-attempt `CC`/`DP` fallback loop,
- **white sliding** (folded into the same step, because a slide executes no
  command).

It is pure and total. Therefore:

- The state space is `(coloured block, DP, CC)` - at most `8 x #blocks` states.
  White codels never need to be resting states: slides always end in a coloured
  block, so `exit()` can fold them.
- Every state has exactly **one** static successor, except two instructions that
  read the stack:
  - `pointer` -> up to 4 successors, indexed by `v mod 4`,
  - `switch`  -> up to 2 successors, indexed by `v mod 2`.
  (On an empty stack both behave as index 0.)

So the entire program lowers to one function with:

- one basic block per reachable state,
- straight-line stack/arithmetic/IO code per command,
- unconditional `br` for deterministic transitions,
- `br_table` (jump table) for `pointer`/`switch`,
- `return` at termination.

There is no dynamic code generation, no self-modification, no indirect jump to an
unknown target, and no unbounded branching factor. Cranelift's input model
(SSA CFG + integer ops + memory + calls + `br_table`) matches this exactly.

**The 2-D geometry is weird, but it is static.** It is fully solved in the
frontend. The backend sees a boring finite CFG.

---

## 3. What Cranelift gives you, and what it does not

Crates (all share one version; current line is `0.137.x` stable, `0.138.0-rc.1`
pre-release - pin one version for the whole family):

| Crate | Role |
|---|---|
| `cranelift-codegen` | IR + machine codegen |
| `cranelift-frontend` | `FunctionBuilder`, SSA construction, jump tables |
| `cranelift-module` | Name resolution, imports/exports |
| `cranelift-object` | Emit a native `.o` |
| `cranelift-jit` | In-process execution (ideal for tests) |
| `cranelift-native` | Host CPU ISA |
| `cranelift-interpreter` | Execute IR without JIT (useful to debug IR) |

It gives you `br_table`, calls to external symbols, and object emission. It does
**not** give you:

- a language-level stack, heap, or big integers - all runtime state is yours;
- a stable API guarantee - wrap it behind one module and pin the version;
- a linker - emit an object and invoke `cc`/`clang`/`lld` (or `link.exe`), or link
  a Rust `staticlib` runtime.

### Codegen notes that matter

- Keep the Piet stack in **memory**: a context with `base: *mut i64`, `sp: usize`,
  `cap: usize`. A state's stack height differs by incoming path, so `sp` is
  runtime state, not a block parameter. SSA only within a basic block.
- Every binary op checks `sp >= 2`; pops/pushes guard `sp`.
- `sdiv`/`srem` are truncated and trap on zero. Emit guards and a floored
  adjustment; special-case `MIN / -1`.
- `roll` is easiest as a runtime call.
- `push` is an immediate `i64` equal to the block size.
- Optional later passes: constant-fold stack values across straight-line runs,
  and prune `br_table` arms when the popped value is statically known.

Sketch of the `pointer` dispatch, after the static `exit()` has decided the
target block and the post-fallback `DP`/`CC`:

```
; sp = stack pointer, stack = *mut i64
  brif sp, 0, no_pop, do_pop
do_pop:
  sp1 = sp - 1
  v   = load.i64 stack + sp1*8
  jump dispatch(v)
no_pop:
  jump dispatch(0)
dispatch(v):
  ; rem_euclid(4) so negatives rotate anticlockwise
  r = srem v, 4
  r = select (r < 0), r + 4, r
  br_table r, [S0, S1, S2, S3]   ; default unreachable
S0: jump state(B, dp + 0, cc)
S1: jump state(B, dp + 1, cc)
S2: jump state(B, dp + 2, cc)
S3: jump state(B, dp + 3, cc)
```

---

## 4. Where the real difficulty is (it is not Cranelift)

1. **Frontend / image handling**
   - Palette quantisation: exact 20-colour PNGs vs. antialiased or arbitrary
     images. Nearest-colour mapping; non-standard -> white per spec.
   - **Codel size detection** for upscaled programs: try divisors of width/height
     and pick the largest N where every N x N tile is monochrome; or GCD of run
     lengths; or require `--codel-size`.
   - 4-connected component labelling, block sizes, and cached edge-codel lists.

2. **Spec ambiguity** - the author says implementations differ. Decisions that
   affect observable behaviour:
   - white slide + restriction/retrace termination,
   - floored division and modulo,
   - negative `pointer`/`switch`,
   - upper-left codel being white or black,
   - `roll` depth limits and stack overflow,
   - non-standard colours.
   Mitigation: implement the clarified spec, document deviations, and
   **differential-test against `npiet`** on the sample programs.

3. **Integer model.** `i64` (wrapping) is spec-compliant ("finite maximum
   integer"), simple, and fast. Arbitrary precision would turn every arithmetic
   op into a runtime call on heap handles - feasible, but a different project.
   Recommendation: `i64` for v1 behind a `Value` abstraction.

4. **I/O runtime.** Unicode codepoints, integer parse/format, "no input waiting
   -> ignore", output flushing. Small, but easy to get subtly wrong.

5. **Linking.** `cranelift-object` + system linker is standard; make the linker
   command configurable, and keep JIT for tests so the project is usable even
   where no linker is configured.

---

## 5. Proposed crate layout

Names follow the `slag` conventions: unprefixed, lowercase, one word, named for
the layer each crate owns rather than the technology it uses. The semantic core
gets the one evocative name (`tessera`, a tile in a mosaic), as `crux` does in
`slag`. The CLI package is `cli`; the product name lives on the binary, as
`slag` does with `crates/cli`.

```
crates/
  canvas    frontend: PNG -> palette -> codel grid -> 4-connected blocks
  tessera   semantic core: command table, exit()/white-slide resolution,
            state graph (no I/O, no codegen)
  runtime   stack, I/O, roll, checked/floored arithmetic, exit
  interp    reference interpreter over `tessera` + `runtime` (oracle + trace)
  codegen   Cranelift lowering; JIT and object-emission backends
  cli       package `cli`, binary `pietrs`; sub-commands: build / run / trace / check
```

Pipeline:

```
PNG -> quantise -> codel grid -> blocks
    -> exit table -> reachable state graph
    -> Cranelift IR -> object -> link -> executable
                    \-> JIT (run + tests)
```

Having `tessera` own `exit()` and the state graph is the key separation: the
interpreter walks the graph, the compiler emits it, and tests compare the two.

---

## 6. De-risking milestones

Each step is independently useful and produces a testable artifact.

- **S0 (0.5 day).** PNG -> codel grid -> blocks. Assert colours/sizes on a known
  tiny image. Handles codel-size detection.
- **S1 (1-2 days).** Reference interpreter for the full clarified spec. Validate
  on the dangermouse sample programs and against `npiet`. This nails semantics
  *before* codegen exists.
- **S2 (0.5 day).** Implement `exit()` and the reachable state graph; dump it as
  text. Compare the interpreter's block trace to a graph walk.
- **S3 (1 day).** Cranelift **JIT** spike: only `push` / `pop` / `add` /
  `out(char)` on a hand-made grid, printing a string. Proves IR plumbing,
  runtime calls, and the stack context.
- **S4 (2-3 days).** Full command set, `pointer`/`switch` via `br_table`, white
  slides, termination. Run classic Hello World under JIT; diff with `npiet`.
- **S5 (1-2 days).** `cranelift-object` + linker -> standalone executable. Test
  truth machine, looping counter, etc.
- **S6 (ongoing).** Differential harness: for every program, run interpreter,
  JIT, and compiled binary under a timeout; compare stdout/stderr/exit.

The S1 interpreter is the highest-leverage move: it resolves the spec, gives the
trace the state-graph builder is tested against, and becomes the oracle for the
compiled output.

---

## 7. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| Divergence from `npiet` / other interpreters | Medium | clarified spec + differential tests; document deviations |
| White-slide edge cases (retrace, restriction) | Medium | interpreter first; property tests on random grids |
| Caller expects arbitrary-precision ints | Medium | choose `i64` explicitly; keep a `Value` seam |
| Codel-size detection heuristics | Low-Med | explicit `--codel-size` override + autodetect |
| Cranelift API churn | Low | pin `cranelift-*` 0.137.x; isolate in one module |
| No linker configured (esp. Windows) | Low-Med | configurable linker command; JIT `run` always works |
| Huge images / state blow-up | Low | Piet programs are small; cap states and report |
| Turing-completeness / loops | None | `pointer`/`switch` give data-dependent dispatch; compiled code just loops |

Nothing here is a fundamental blocker.

---

## 8. Decisions to lock before writing code

1. **Spec target:** clarified 2018 spec, `npiet`-compatible where they differ.
2. **Integer model:** `i64` wrapping by default; checked optional.
3. **Codel size:** autodetect with `--codel-size` override.
4. **Non-standard colours:** map to white (spec suggestion).
5. **First execution mode:** JIT `run` (tests), then AOT `build`.
6. **I/O:** Unicode codepoints over a UTF-8 stream.
7. **Stack:** fixed capacity (e.g. `1 << 20`) with an explicit overflow error.
8. **Start codel white/black:** define, test, and document.

---

## 9. Bottom line

Nothing in Piet resists compilation. The 2-D geometry - the part that looks
intractable - is static, so it collapses at compile time into a finite state
machine over `(block, DP, CC)`. The only data-dependent control flow is
`pointer`/`switch`, and each maps to a jump table over 2-4 statically known
successors. That is exactly the shape Cranelift consumes.

The risks are in *semantics and runtime design* - spec variation, white slides,
and the integer model - not in the choice of backend. Build the reference
interpreter first, then lower the same state graph to Cranelift.
