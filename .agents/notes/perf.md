# Performance journal

The harnesses live in `crates/perf/tests/harnesses.rs` and are `#[ignore]`d. They
assert nothing: a duration is a property of the machine, not of the code, so
nothing here is a gate. They print wall-clock time, allocation counts, allocated
bytes, and bytes still held.

```sh
just perf
# cargo test -p perf --release -- --ignored --nocapture --test-threads=1
```

Release, one thread. The allocator counter is process wide, so a parallel run
mixes cases; `measure` serialises the measurement window, but setup done before
it can still land in another case's window.

Units are integers: microseconds, allocation counts, and KiB. The repository
denies float arithmetic, including in the harnesses.

## What is measured

| Harness | Work | Why |
|---|---|---|
| `partition_a_large_grid` | `Blocks::partition` on a 320 x 320 grid of all twenty colours | the frontend is the bulk of compile time |
| `build_a_large_graph` | `StateGraph::build` on a 96 x 96 grid | one traversal of the reachable `(block, DP, CC)` states |
| `emit_an_object` | `codegen::emit_object` on a 32 x 32 grid | Cranelift lowering plus object emission for one function |
| `simulate_a_looping_program` | 1 000 000 transitions through `interp` | the interpreter's per-step cost, which the JIT has to beat |

The graph harness reports the reachable states, not the state space: from the
upper-left codel of a random grid, few states are reachable. The compiler builds
exactly that subset, so it is the honest number.

## 2026-11-10

AMD Ryzen 9 7950X, Windows, `rustc 1.98.0`, `--release`, single-threaded.

| Harness | Time | Allocations | Allocated | Held |
|---|---|---|---|---|
| partition 320x320, 67 062 blocks | 7 600 us | 134 141 | 13 183 KiB | 6 991 KiB |
| graph 96x96, 9 216 blocks, 39 states | 23 us | 57 | 7 KiB | 2 KiB |
| codegen 32x32, 1 024 blocks, 3 KiB object | 1 627 us | 1 753 | 1 336 KiB | 3 KiB |
| interp 1 000 000 transitions | 20 584 us | 0 | 0 KiB | 0 KiB |

The interpreter's zero allocations per million transitions is the number to
watch: a per-step allocation would show up here immediately. `partition`
allocates once per block or so, which is the known cost of the partition map;
`codegen` allocates inside Cranelift, not in our lowering.

Do not compare runs across machines, and do not add an assertion to a harness.
If a number moves, record it here with the change that moved it.
