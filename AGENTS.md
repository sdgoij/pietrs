# Working in pietrs

`pietrs` compiles the tessellation flow of a given canvas. A Piet program is an
image: `canvas` turns it into a codel grid and 4-connected colour blocks,
`tessera` resolves the movement rules into a finite `(block, DP, CC)` state
graph, and that graph is either walked by `interp` or lowered to Cranelift IR by
`codegen`. `runtime` is the stack, I/O, and arithmetic support shared by both;
`cli` is the `pietrs` binary.

The language and compiler rationale live in
`.agents/notes/init-feasibility.md`. This file is the working agreement: how
changes are tested, gated, and committed.

## The iron rule

**A passing test is not a passing suite.** The cases that name your change can
pass while one in a crate you never opened fails. Run the whole gate, in the
order CI runs it, before claiming anything is green.

**The three paths must agree — once JIT/AOT parity exists.** The same program,
run through `interp`, the JIT, and the AOT-compiled executable, must produce the
same observable output: stdout bytes, exit status, and — between `interp` and the
JIT — the same trace. Until the `codegen` track lands, `interp` is the only
execution path: every new command still needs an interpreter test and a trace
golden, and the cross-path cases are added as the backend lands. The rule becomes
binding when a parity gate shows the JIT and the AOT executable match `interp`
on the corpus; from then on, a new command is not done until all three paths have
a case.

## Change workflow

1. Make one focused change.
2. Add or update a test that fails without it. A change without a failing-first
   test is not done. Prove the test is not vacuous: revert the change (or mutate
   the branch) and watch the new test fail, then restore.
3. Refresh editor diagnostics project-wide, then run the gates.
4. Commit when everything is green. Commit green, tested changes without asking.
5. Stop and ask only when a decision, a spec ambiguity, or a missing environment
   genuinely blocks progress.

Do not claim a gate passed unless the command was run in this session and its
result was observed. If a gate fails, fix the root cause when it is clear;
otherwise report the failing command and the relevant output.

Ask before committing when the change alters the language surface (a command's
semantics, the trace format, the CLI interface) or when a required gate cannot
be run.

## Verification gates

These commands are the contract. When a `Justfile` lands, `just gate` runs them
in this order; if the two ever disagree, CI is the truth.

Before the first command, refresh editor diagnostics project-wide. A diagnostic
is a failure just like a compiler warning, in every language the project uses:
Rust, C, and any fixture tooling. It is cheap; the build is not.

```sh
# 1. Formatting and whitespace.
cargo fmt --all -- --check
git --no-pager diff --check

# 2. Lints, warnings as errors.
cargo clippy --workspace --all-targets -- -D warnings

# 3. The suite: unit and integration.
cargo test --workspace

# 4. The long cases: conformance, differential, end-to-end. CI runs these too.
cargo test --workspace -- --include-ignored

# 5. The feature shapes. The interpreter must stand alone, and the codegen
#    crate must build without its object-emission feature.
cargo test -p cli --no-default-features
cargo test -p codegen --no-default-features

# 6. Documentation.
cargo test --workspace --doc

# 7. Licences, advisories, banned crates.
cargo deny check
```

Docs-only changes require steps 1 and 6 only. `cargo-deny` is not shipped with
Cargo: `cargo install cargo-deny --locked`. `cargo-nextest` may run the same
suite with per-test timeouts; it is a convenience, not a replacement.

External gates need a tool the build does not vendor. They are developer and
release gates, never part of the shipped binary:

```sh
cargo run -p xtask -- conformance <image-or-corpus>   # differential vs npiet
cargo run -p xtask -- e2e <corpus>                    # build, link, run, compare
```

A missing `npiet` or linker is reported by name. It never silently passes.

## Test layers

Every layer has a home. Do not put an integration test in a unit module, and do
not let an end-to-end case become the only test for a pure rule.

Coverage is a review property, not a percentage. Every public item, every
command-table cell, and every branch in the movement and slide rules has a case
that names it. If a branch cannot be reached, delete it or explain why in the
test.

**Unit tests** (`#[cfg(test)] mod tests` in the module that owns the rule) cover
the arithmetic and the geometry:

- palette: all 20 colours, hue and lightness cycles, forward deltas;
- the 6x3 command table: every `(hue delta, lightness delta)` pair;
- edge-codel selection: all 8 `(DP, CC)` combinations, disjoint edges, holes;
- the 8-attempt fallback: every attempt index, and the terminate case;
- white slides: entry, straight run, restriction, rotation, retrace;
- block partition: 4-connectivity, diagonal non-contact, holes;
- the state graph: determinism, reachability, `pointer`/`switch` arms;
- stack: underflow ignored, `push` = exited block size, `roll`, capacity;
- arithmetic: floored division and modulo, sign of the divisor, `MIN / -1`,
  divide by zero;
- `pointer`/`switch`: negative values, `rem_euclid`, empty stack.

Exhaustive beats sampled where the domain is small. The command table, the 8
`(DP, CC)` cases, and the 20-colour palette are small enough to test
exhaustively; do that.

**Integration tests** (`crates/<crate>/tests/*.rs`) use only the public API:

- `canvas`: fixture PNG -> expected grid, block count, block sizes;
- `tessera`: hand-built grid -> expected state graph and trace;
- `interp`: fixture -> stdout, exit status, trace;
- `codegen`: graph -> verified CLIF, JIT result, object emission;
- `runtime`: stack and I/O over a scripted stream.

**Acceptance and conformance** tests pin behaviour against an oracle:

- the clarified 2018 specification (`.agents/notes/` records every deliberate
  deviation);
- `npiet`, when present, through `xtask conformance`;
- golden traces and golden stdout/exit files for the checked-in corpus.

**End-to-end** tests drive the `pietrs` binary:

- `pietrs check` on a fixture;
- `pietrs trace` against a golden trace;
- `pietrs run` (JIT) against golden stdout and exit status;
- `pietrs build` -> a real executable -> run it -> same stdout and exit status;
- `--codel-size` override, non-standard colours, malformed-PNG diagnostics.

**Property and fuzz** tests cover the open-ended input:

- proptest: block partition is total and disjoint; the state graph is
  deterministic; white-slide resolution terminates or is reported; the command
  table is total; the `rem_euclid` arms are exhaustive;
- cargo-fuzz: PNG decode/quantise/segment, arbitrary codel grids, and `tessera`
  graph construction. Codegen fuzzing verifies CLIF before execution. A crash is
  a bug: fix the root cause, then add the input to the seed corpus.

**Performance** tests are `#[ignore]`d manual harnesses, not CI correctness
gates. Use a plain wall-clock loop and a counting allocator (the `regexp`
precedent) rather than adding a benchmark framework. Record results in a perf
journal; do not gate on a machine-dependent number.

## Fixtures, goldens, and oracles

- Fixtures live in `crates/<crate>/fixtures/<case>/`, with the expected
  artefact next to them: `expected.trace`, `expected.stdout`, `expected.exit`,
  or `expected.json`. A reviewer must be able to add a case without touching
  test code.
- `testkit` owns builders, a scripted stdin/stdout harness, trace parsing, and a
  `Checks` collector. Production crates never depend on `testkit`.
- Expected values are transcribed from the specification or from `npiet`, never
  re-derived from our own implementation. If an expected value is wrong,
  investigate it; do not re-baseline it to make the suite green.
- A golden re-baseline is a deliberate act: explain why in the commit and pin
  the new value. A digest pin — a corpus manifest, the trace format — is a
  compatibility boundary; moving it is a version bump, not a fix-up.
- The trace format is a public contract. A change to it updates the goldens in
  the same commit and is called out in the body.
- Degraded inputs are part of the corpus: a 1x1 image, a single-colour image,
  an image whose upper-left codel is white, one whose upper-left codel is black,
  and a malformed PNG.

## Long tests, timeouts, and external tools

- A long test is `#[ignore]`d with a reason and the exact command that runs it.
  CI runs `--include-ignored`, so nothing is covered only when someone
  remembers a second command.
- Every subprocess the suite starts (JIT run, AOT executable, `npiet`) gets a
  timeout. A timeout is a verdict, not a reason to raise the deadline. An
  infinite Piet program gets bounded stdin or a kill, not a longer timeout.
- A test that needs a linker or `npiet` fails with a diagnostic naming the
  missing tool when it is expected to run. Do not turn a missing tool into a
  silent pass.
- A gate's own verdict is its `OK:`/`FAIL:` line; the exit status backs it up.
  A crash after the verdict is not a pass. The gate runner (`tools/gates.sh`)
  auto-discovers its cases, prints one line each, writes logs under
  `target/gate-logs/`, and exits non-zero if any case never gave a verdict.

## Diagnostics and self-testing

- `pietrs check` validates the image, the block partition, and the state graph
  without executing.
- `pietrs trace` is a stable, line-oriented dump of
  `(block, DP, CC, command, target)`. It is both the debugging tool and the
  golden-test format.
- `pietrs selftest` runs the checked-in corpus through every execution path that
  exists — `interp`, the JIT, and, when a linker is configured, the AOT
  executable — compares them, and prints one PASS/FAIL line per case. This is the
  project's self-check: the compiler booting its own corpus and judging it, the
  analogue of the Godot gates.
- Compiler errors carry the image coordinate, the codel index, the block id,
  `DP`/`CC`, and the transition. Malformed input is a diagnostic, never a panic.
- Diagnostic switches are off unless set and write to stderr:
  `PIETRS_DUMP_GRAPH`, `PIETRS_DUMP_CLIF`, `PIETRS_JIT_DISASM`. They are read
  once per process.

## Project invariants

- The target is the clarified 2018 Piet specification
  (`dangermouse.net/esoteric/piet.html`). npiet v1.3h is the conformance oracle,
  but the specification wins where they disagree. Every observable deviation is
  recorded in `.agents/notes/divergences.md`.
- Integers are `i64` in v1. `pointer`/`switch` use `rem_euclid`; division and
  modulo are floored (Python `//` and `%`) by default, and `--division
  truncating` selects C/npiet rounding; `MIN / -1` and divide-by-zero are
  handled, not traps.
- An underflowing operation is ignored. An empty stack makes `pointer` and
  `switch` behave as index 0.
- `push` pushes the codel count of the block just exited; it is a compile-time
  constant.
- Non-standard colours map to white; codel size is autodetected with
  `--codel-size` as the override.
- Termination is the 8-attempt rule, and white-slide retracing, only.
- The value stack has a fixed capacity with an explicit overflow error.
- I/O is Unicode codepoints over a UTF-8 stream.
- Arithmetic in `tessera` and `runtime` is integer-only; floating point is not
  part of the language semantics.
- Cranelift is pinned to one version for the whole `cranelift-*` family. Every
  emitted function is run through the CLIF verifier before JIT or object
  emission.
- `unsafe` is confined to the JIT entry point and the FFI boundary. Every other
  crate forbids it.
- The AOT executable is the shipped artefact; the JIT is for `run`, tests, and
  debugging. They share one lowering.
- The same input produces the same trace and the same stdout and exit status.
  Object bytes are not required to be byte-identical unless a golden says so.
- The toolchain is the `rust-version` in `Cargo.toml` (1.98, edition 2024). A
  feature that needs newer must bump the pin in the same commit.
- No crate is without tests. No `TODO` reaches `main`.

## Commit rules

- Imperative subject with a type prefix: `feat:`, `fix:`, `test:`, `docs:`,
  `refactor:`.
- Keep the subject under about 50 characters, with no trailing period.
- Explain what changed and why in the body. One unwrapped line per paragraph.
- Tracked text is LF in the index and the working tree on every platform.
- Never commit generated artefacts: `target/`, compiled executables, object
  files, downloaded corpora, CLIF dumps, or `npiet` output.
- Keep the tree clean after committing.

## Rules hygiene

- This file is the working agreement. Product and architecture decisions go in
  `.agents/notes/`; procedures and subsystem detail go in
  `.agents/skills/<name>/SKILL.md`.
- A rule is a trap a change must avoid or follow: non-obvious, repeatedly
  encountered, and actionable. It is not a map of the code and not a lesson in
  method.
- Do not add rules inline during feature work. Propose them in the PR under
  "Suggested AGENTS.md additions"; reviewers decide.

## Review checklist

- Is there a test that fails without this change?
- Did a new command or transition reach every execution path that exists —
  `interp` always, and the JIT/AOT once parity has been reached?
- Does the trace format or a golden need an update, and is it in this commit?
- Are malformed inputs diagnostic rather than panicking?
- Is the Cranelift pin still one version across the family?
- Was the whole gate run, not only the test named after the change?
