# The gate cases

`tools/gates.sh` runs every `*.sh` in this directory in lexical order. Each
case is self-contained and follows one rule:

- print exactly one verdict as its last line: `OK: <name>` or `FAIL: <name>`;
- exit zero only when that verdict is `OK`.

The runner writes the case's full output to `target/gate-logs/<case>.log`. A
case that prints `OK` and then exits non-zero is a failure, and so is a case
that never prints a verdict. The runner itself exits non-zero if any case fails.

The order is the CI order described in `AGENTS.md`:

| Case | Command |
|---|---|
| `01-fmt` | `cargo fmt --all -- --check`, `git diff --check` |
| `02-clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `03-test` | `cargo test --workspace` |
| `04-long-tests` | `cargo test --workspace -- --include-ignored` |
| `05-features` | `cargo test -p cli --no-default-features`, `cargo test -p codegen --no-default-features` |
| `06-doc` | `cargo test --workspace --doc` |
| `07-deny` | `cargo deny check` |

`just gate` runs the same runner; `.github/workflows/ci.yml` runs it on Linux
and Windows. The external `xtask conformance` and `xtask e2e` gates are recipes
in the `Justfile` and are also exercised by the long tests.

To exercise the runner without touching the real cases, point `GATE_CASES` at
another directory:

```sh
GATE_CASES=target/scratch-cases sh tools/gates.sh
```
