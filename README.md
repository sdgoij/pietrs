# pietrs

Compiles the tessellation flow of a given canvas.

`pietrs` compiles a [Piet](https://www.dangermouse.net/esoteric/piet.html)
program — an image — into a standalone native executable. The program's control
flow is resolved at compile time into a finite state graph, Cranelift lowers that
graph, and `pietrs build` writes the executable itself: no C compiler, no linker,
and no `rustc` runs on the machine that builds a program.

```
program.png ─> codel grid ─> colour blocks ─> (block, DP, CC) graph
                                              ├─> interpreter    (trace, oracle)
                                              ├─> Cranelift JIT  (`run`)
                                              └─> object + runner (`build`)
```

Piet's two-dimensional geometry looks like the hard part of the language. It is
not: for a given image, the movement rules resolve into a finite graph over
`(colour block, direction pointer, codel chooser)`, one unconditional edge per
state. Only `pointer` and `switch` read the stack, and each lowers to a jump
table over two or four statically known successors — exactly the shape a
compiler backend expects.

## Install

Download the archive for your platform from the
[releases](https://github.com/sdgoij/pietrs/releases) page, unpack it, and put
`pietrs` somewhere on your `PATH`. The archives hold the binary, this README, and
the licence.

Or build from source (Rust 1.98, pinned in `rust-toolchain.toml`):

```sh
cargo build --release
target/release/pietrs --help
```

## Use

```sh
# What did the image decode into? Grid, blocks, state graph, and whether the
# program can ever terminate.
pietrs check program.png

# Run it: the JIT, in process.
pietrs run program.png

# Same, with input and a step bound.
pietrs run adder.png --input numbers.txt --max-steps 100000

# One line per transition, for debugging or diffing.
pietrs trace program.png --max-steps 200

# Compile it to a standalone executable.
pietrs build program.png -o program
./program
```

`pietrs build` writes the whole executable: the program, plus a runner that is
compiled into `pietrs` itself and embedded at build time. Nothing is linked when
you build a program, which is why a cross build needs no target toolchain.

### Commands

| Command | What it does |
|---|---|
| `check` | decodes, partitions, and builds the state graph without executing; reports whether any reachable state terminates |
| `trace` | runs the reference interpreter and prints every transition |
| `run` | runs the program through the JIT and prints its output |
| `build` | writes a standalone executable for the host or `--target` |
| `selftest` | runs a corpus through every execution path and compares them |

### Images

PNG and GIF (the first frame) are decoded, at any codel size: the size is
detected from the image, and `--codel-size N` overrides it. A colour outside the
twenty is reported on stderr and treated as white, as the specification suggests —
an image whose palette has been mangled otherwise looks like a program that loops.

### Cross-compiling

```sh
pietrs build program.png -o program.exe --target x86_64-unknown-linux-gnu
```

The embedded runners cover the host and, on every host, `x86_64-unknown-linux-*`
and `aarch64-unknown-linux-*` (the GNU triples run the static musl artefacts
unchanged). Adding a target means adding a runner and an ISA to the compiler, not
a toolchain to the user's machine.

### Bounds and diagnostics

- `--max-steps N` stops a run and reports the limit; without it an unbounded
  program runs for ever, and `check` tells you in advance when that is true.
- `PIETRS_DUMP_GRAPH=1` writes the state graph to stderr; `PIETRS_DUMP_CLIF=1`
  writes the lowered function; `PIETRS_JIT_DISASM=1` writes the machine code.

## Semantics

The target is the clarified 2018 Piet specification, with npiet v1.3h as the
conformance oracle. Integers are `i64`; division and modulo are floored by
default (`--division truncating` selects npiet's rounding); I/O is Unicode
codepoints over UTF-8; the value stack has a fixed capacity and reports overflow.
Every place where `pietrs` observably differs from npiet is recorded in
[`.agents/notes/divergences.md`](.agents/notes/divergences.md), and `pietrs
selftest` checks the interpreter, the JIT, and a packaged executable against each
other on the corpus.

## Development

```sh
just gate   # fmt, clippy, tests, long tests, feature shapes, doc, licences
just perf   # the ignored performance harnesses
just fuzz png 100000   # one fuzz target, needs cargo-fuzz and nightly
```

The working agreement — test layers, verification gates, and the invariants a
change must not break — is [`AGENTS.md`](AGENTS.md). Design notes live in
[`.agents/notes/`](.agents/notes/).

| Crate | Role |
|---|---|
| `canvas` | image → codel grid → 4-connected colour blocks |
| `tessera` | movement, white slides, and the `(block, DP, CC)` state graph |
| `runtime` | stack, I/O, floored arithmetic |
| `interp` | the reference interpreter: trace, oracle, no codegen |
| `codegen` | Cranelift lowering; JIT and object emission |
| `aot` | packages an object plus an embedded runner into an executable |
| `runner` | the embedded runner: maps the payload and calls `piet_main` |
| `cli` | the `pietrs` binary |
| `testkit`, `xtask`, `perf` | fixtures, developer gates, ignored harnesses |

## Licence

GPL-2.0-only. See [`LICENSE.md`](LICENSE.md).
