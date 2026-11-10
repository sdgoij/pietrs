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

- Write the decoded codel grid to PPM (`P6`, maxval 255, one pixel per codel)
  and pass `-cs 1`; the vendored build has no libpng support.
- Pass `-q` so npiet does not print its input prompt.
- Bound the run with `-e <steps>` and treat the timeout as a verdict.
- Compare stdout and the step-limit state against `interp`. Report a missing
  `cc`/`gcc`/`clang`, a wrong npiet version, a nonzero exit, or a timeout by
  name; never turn a missing tool into a silent pass.

## Divergences

npiet is not the specification. Every observable difference is recorded in
`divergences.md` in this directory; this note only covers how the gate behaves.
Do not re-baseline a `pietrs` golden to match npiet in a case that exercises a
recorded divergence.
