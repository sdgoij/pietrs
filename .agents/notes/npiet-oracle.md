# npiet as the conformance oracle

## Decision

Vendor npiet's source, pinned to `v1.3h`, under `vendor/npiet/`. Build it on
demand with the system C compiler and run it from `xtask conformance`.

Why not download a release binary or expect one on `PATH`:

- no user has to find a version or trust a random binary;
- the exact source is committed, auditable, and checked by SHA-256;
- the gate works offline once the repository is cloned;
- the project is `GPL-2.0-only`, so vendoring GPL-2.0 npiet is compatible.

npiet is a developer and release gate, never part of the shipped `pietrs`
binary. A compiled npiet is a build artefact under `target/` and is never
committed.

## Invocation

- Convert the PNG fixture to PPM (`P6`, maxval 255); the vendored build has no
  libpng support.
- Pass `-q` so npiet does not print its input prompt.
- Bound the run with `-e <steps>` and treat the timeout as a verdict.
- Compare stdout, stderr diagnostics that matter, and exit status against
  `interp`. Report the missing `cc` by name when the gate is expected to run;
  never turn a missing tool into a silent pass.

## Known deviations

npiet is not the specification. `pietrs` follows the clarified 2018 spec where
they differ, and records each deviation here.

- Division and modulo: npiet truncates toward zero; `pietrs` floors
  (Python `//`, `%`).
- Divide/modulo by zero: npiet consumes both operands and pushes `99999999`;
  `pietrs` consumes both operands and pushes nothing.
- Integers: npiet uses C `long`, which is 32-bit on Windows; `pietrs` uses
  `i64` on every target.
- Character I/O: npiet treats a character as a byte (`& 0xff`, `% 0xff`);
  `pietrs` uses Unicode codepoints.

The conformance corpus must either avoid these cases or name them as expected
differences. Do not re-baseline a `pietrs` golden to match npiet in a case that
exercises a recorded deviation.
