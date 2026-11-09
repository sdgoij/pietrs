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
with the goldens. Keep every case within the npiet-safe subset: no negative
division, no values outside 32-bit range, and ASCII character I/O. The recorded
deviations are in `.agents/notes/npiet-oracle.md`.
