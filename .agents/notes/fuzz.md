# Fuzzing

The `fuzz/` crate is its own Cargo workspace, so `cargo test --workspace` and
`just gate` never build it: the targets need nightly and a sanitizer, and none of
that belongs in the compiler's build. `cargo-fuzz` is a developer tool, not a
dependency of `pietrs`.

## Targets

| Target | Input | Invariants |
|---|---|---|
| `png` | arbitrary bytes | `CodelGrid::from_png` returns a diagnostic, never panics or allocates without a bound; a decoded grid partitions totally and disjointly |
| `codel_grid` | two shape bytes plus colour codes | `CodelGrid::from_colors` -> `Blocks::partition` is total, disjoint, and self-consistent |
| `graph` | a grid of all twenty colours | every edge joins two states of the graph, the start state is a state, at most eight states per block, and rebuilding yields the same graph |
| `codegen` | a small grid | `emit_object` succeeds for both division modes; the CLIF verifier runs inside it, so the verifier is fuzzed too |

`emit_object` calls `FunctionBuilder` and verifies the function before object
emission, which is why the `codegen` target doubles as the verifier fuzz.

## Running

```sh
cargo +nightly fuzz build
cargo +nightly fuzz run png -- -runs=100000
cargo +nightly fuzz run codel_grid -- -runs=100000
cargo +nightly fuzz run graph -- -runs=100000
cargo +nightly fuzz run codegen -- -runs=3000
```

`just fuzz <target> <runs>` runs one from the repository root. `codegen` is the
slow target (Cranelift per execution, roughly 50 executions/s); the others run in
the tens of thousands per minute.

The working corpus is `fuzz/corpus/<target>/` and is ignored. To start from the
curated seeds:

```sh
cp fuzz/seeds/png/* fuzz/corpus/png/
```

libFuzzer writes new inputs into the last corpus directory it is given, so keep
`seeds/` out of the corpus argument list and copy from it instead.

## When a target finds something

An artifact lands in `fuzz/artifacts/<target>/`. Reproduce it:

```sh
cargo +nightly fuzz run <target> artifacts/<target>/<file>
```

A crash is a bug. Fix the root cause, add a regression test in the owning
crate's suite, and copy the artifact into `fuzz/seeds/<target>/` so the target
starts from it next time. Do not delete the artifact and move on.

## Findings so far

- `png`, 2026-11-10: a 55-byte PNG declaring `2 x 2181038141` pixels made
  `decode_rgba` try to allocate 8 GiB, which aborts the process. `canvas` now
  bounds the decoded buffer at 256 MiB (an 8192 x 8192 RGBA image) and reports
  `ImageTooLarge`;
  `canvas::tests::a_png_that_declares_too_many_pixels_is_a_diagnostic` keeps the
  case, and `fuzz/seeds/png/huge-header.png` is the artifact.

## What fuzzing does not cover

- The JIT and the packaged executable are not fuzzed: they execute generated
  code, and the corpus already compares `interp`, the JIT, and the AOT
  executable on every case (`pietrs selftest`).
- `cargo fuzz` is not installed in CI. The targets are a developer gate; the
  gate that CI runs is `tools/gates.sh`.
