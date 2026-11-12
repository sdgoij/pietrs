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
| `roll_a_tape` | 20 000 000 value-moves through `Stack::roll`, at three depths | the cost of random access for a Pict-in-Pict interpreter with a tape |

The graph harness reports the reachable states, not the state space: from the
upper-left codel of a random grid, few states are reachable. The compiler builds
exactly that subset, so it is the honest number.

The tape depths are the three that `.agents/notes/piet-in-piet-full.md` budgets
for: the canonical hello world's state graph, *Piet's Quest*'s state graph, and
its codel grid.

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

## 2026-11-11

AMD Ryzen 9 7950X, Windows, `rustc 1.98.0`, `--release`, single-threaded.

| Harness | Time | Allocations | Allocated | Held |
|---|---|---|---|---|
| roll a 25-value tape, 800 000 rolls | 7 224 us | 0 | 0 KiB | 0 KiB |
| roll a 17 964-value tape, 1 113 rolls | 1 994 us | 0 | 0 KiB | 0 KiB |
| roll a 475 867-value tape, 1 000 rolls | 48 347 us | 0 | 0 KiB | 0 KiB |

That is 9 ns, 1.79 us and 48.3 us per roll: linear in the depth, which is what
`Vec::rotate_right` over the tape costs. The first depth is a state-graph record
window, the second *Piet's Quest*'s state graph, the third its grid. No
allocations, so the roll itself moves memory but never grows the stack.

## 2026-11-12

AMD Ryzen 9 7950X, Windows, `rustc 1.98.0`, `--release`, single-threaded.

The woven walker (`target/weave/walker.png`, 617 x 246, 30 260 blocks, 61 987
states, 62 945 edges) is the largest image the project runs, and the JIT's
startup on it is not covered by a harness. It was measured by hand, three runs
each, with `--max-steps 1` to isolate compilation:

```sh
cargo run -r -p cli -- run target/weave/walker.png --max-steps 1
```

| Stage | Time |
|---|---|
| graph build | 16 us |
| lowering to CLIF | under 1 ms |
| explicit verification | 200 ms |
| Cranelift compilation | 2 200 ms |
| instruction cache flush and symbol resolution | 1 550 ms |

That is 1 929 000 CLIF instructions in 193 922 blocks, and Cranelift's share is
linear in it. The whole startup was 5 000 ms; turning off Cranelift's own
verification, which runs again inside `define_function` after the explicit one,
brought it to 3 600 ms and saved the same second in `pietrs build`.

Compiling the image costs more than running it. The same image executes
200 000 000 steps in 1 350 ms, 148 000 000 a second, against 1 530 ms for the
AOT executable on the same record stream: the JIT's wall clock is code
generation, not the program.

Two shapes were tried and rejected. Collapsing the per-state preamble - one
shared step-limit block and one shared abort block instead of three blocks per
state - cut the block count from 310 000 to 194 000, left the compile time
unchanged within noise, and made the running program 23% slower (1 840 ms
against 1 340 ms for the same 200 000 000 steps). Setting `opt_level=speed`
added a second to the compile and bought nothing.

The ring stage (`loom::ring_grid`, 72 x 188 codels, 2 314 blocks) is linear in
the ring: each lap pushes its record under the counters with a fixed seven-value
roll, so the cost per cell is flat at about 2 400 interpreted steps from 9 to 324
cells. On the canonical hello world's grid (30 x 29, 992 ring cells) it runs
2 403 295 steps in 45 ms and prints the stream `ring_stream` builds. That is the
whole cost of the stage, once per image, not per state.

`grid_graph` now prints that stream once and reuses it for the start and every
exit, instead of building it on the host per state. The hello world's image-built
graph takes 92.45 s either side of that change, so the 17 964 host-side streams it
removes are lost in a case whose time is 17 964 image runs; the `ring_stream`
helper stays in the tree as the oracle the image is checked against.

The partition is the splitter's answer rather than `canvas::Blocks`'s now, which
adds one splitter run per graph build: the hello world's 992 ring cells take
79 730 314 steps and 2 s, and the same graph build is 94.45 s with it and 92.45 s
without. That is the honest price of not running the partition rules twice.

A stage's cost is its layout, not its arithmetic. Every dispatch walks down a lane
and back up the return column, so a block visit costs about twice the image
height, and the height is the longest block's command count. The splitter's
twelve blocks are 2 394 commands, and block 8 is 1 981 of them, which is why
every stage that includes it is 1 989 codels tall and why a 20-million-step run
on the edge stage is 4 995 dispatches per entry-column block at 4 000 steps each.
The same stage's own work is ~5 million steps for a 992-cell ring; the 79.7
million is the partition the stage re-runs.
