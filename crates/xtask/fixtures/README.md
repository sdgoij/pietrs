# Conformance corpus

Each case is a directory with:

- `program.png` — the Piet program, the only required file;
- `stdin` — optional program input;
- `max_steps` — optional transition limit, baked into the AOT executable;
- `expected.stdout` — the exact bytes the program prints;
- `expected.exit` — optional exit status, default `0`.

The same corpus drives all three gates:

```sh
cargo run -p cli   -- selftest    crates/xtask/fixtures
cargo run -p xtask -- conformance crates/xtask/fixtures
cargo run -p xtask -- e2e         crates/xtask/fixtures
```

`selftest` compares `interp`, the JIT, and the linked executable; `conformance`
compares the interpreter with npiet v1.3h; `e2e` compares the linked executable
with the goldens. `conformance` defaults to `--division truncating` to match
npiet; `selftest` and `e2e` default to `floored`. A case whose output depends on
rounding needs a mode-specific golden or should stay out of the shared corpus.
Values outside 32-bit range and non-ASCII character I/O remain npiet
deviations. The recorded deviations are in `.agents/notes/divergences.md`.
