# Running full Piet from Piet

The generated interpreter in `crates/loom` replays a *trace*: the host resolves
the whole run, and the image executes the commands. This note is the scope of
running a program whose control flow is decided at run time — enough for
`pointer` and `switch`, and enough for *Piet's Quest*.

## What is actually missing

Two things, and only one of them is hard.

**The geometry.** Which block the direction pointer reaches depends on the
block's shape, the codel chooser, white slides, and the eight-attempt fallback.
All of that is a *static* property of the image. `tessera` already resolves it
once into a finite graph over `(block, DP, CC)`. Compiling it away is the same
move `codegen` makes; it is not a shortcut through the language.

**The memory.** A graph with data-dependent successors needs a place to keep the
successors and a way to index them with a value from the stack. A Piet program
has one value stack and nothing else. That is the whole problem.

The current design is one end of a spectrum:

| Level | The host does | The image does | Image size | Data on the stack |
|---|---|---|---|---|
| replay a trace (now) | decodes, partitions, walks the run | executes the commands | 96 x 15, 331 blocks | none |
| walk the state graph | decodes, partitions, resolves the graph | executes commands, chooses successors | ~3-6x the current image | map from input |
| interpret the grid | decodes the colours | everything, movement included | larger still | grid and blocks |

## What a stack-only machine gives up

Random access is the only thing a stack cannot do well. In Piet it is `roll`:
one command in the image, but the runtime rotates that many values. So an access
is O(1) *Piet commands* and O(depth) *host work*. The depth is the budget.

Measured on 2026-11-11, release, Ryzen 9 7950X, `crates/perf/tests/harnesses.rs`
case `roll_a_tape`:

| Tape depth | Rolls per second | Per roll |
|---|---|---|
| 25 | 110 734 306 | 9 ns |
| 17 964 | 557 978 | 1.79 us |
| 475 867 | 20 683 | 48.3 us |

Three sizes matter:

- **Piet's Quest's state graph** is 17 964 states and 19 352 edges. At three or
  four values per state the tape is about 54 000 values, so one full roll costs
  roughly 5 us: about 190 000 interpreted steps per second if every step moved
  the whole tape.
- **Piet's Quest's grid** is 409 x 1163 = 475 667 codels. Packing colour and
  block id into one value each gives a 475 667-value tape, and one roll per step
  costs 48 us: about 20 000 steps per second before any command work. That is
  the price of the third row of the table.
- **A 64-record window on the tape** would make a sequential step cost about one
  25-value roll, 9 ns instead of 1.8 us, if the layout keeps successors near
  their sources. Crossing a window pays a full rotation.

The other half of the budget is the interpreter's own code. `interp` runs
48.5 million transitions per second (`.agents/notes/perf.md`), so an image that
executes `c` Piet commands per interpreted step runs at 48.5 million / `c` steps
per second. At 500 commands per step that is 97 000 steps per second.

Both halves land in the same range, so the honest summary is: **a universal
interpreter is viable, at tens of thousands of interpreted steps per second,
with a footprint of order a hundred thousand values.** *Piet's Quest* reached its
first menu and printed six of them within 300 000 steps; a session is seconds,
not hours.

What it costs besides speed:

- The image is fixed but large: the current one is 331 blocks and 877 states for
  fifteen handlers; a version with a tape, a program counter and successor
  selection is plausibly three to six times that. That is thousands of codels,
  which is why it has to be generated, not drawn.
- The tape shares the value stack with the interpreted program, so the stack's
  fixed capacity becomes the program's memory limit. The default million values
  would hold a state graph for *Piet's Quest*, but not that program's grid as
  well.
- There is no second stack. Every temporary the interpreter needs has to be
  addressed by depth from one end, which is why the design below keeps the
  interpreter's own working set small and constant.

## Plan

Each stage is independently useful and independently checkable. The checkpoints
are the cases that go into the gate; a stage is not done until they pass against
`interp`, not against itself.

### S1. A structured backend

`loom` can lay out straight-line commands and one dispatch. It cannot express a
loop, a two-way branch, or a named temporary. Every later stage needs all three,
so this is the real work and the real risk.

- IR: commands, named temporaries, `if`/`else`, `while`, and a dispatch on a
  value (the existing bus).
- Layout: reuse the bus, lane, and shared-return architecture, with one lane per
  basic block and jumps as dispatch ids.

Checkpoint: generated programs that sum 1..n, run a nested loop, and branch both
ways, each compared with a Rust model of the same loop.

Risk: the layout is where this can stall. If the branch/merge geometry does not
generalise, S1 has no smaller fallback, and the honest outcome is to stop at the
current trace interpreter.

### S2. Walk the state graph

The image is now data-driven in the only way that matters: it keeps a program
counter and follows successors, so loops run at run time rather than being baked
into the input.

Checkpoint: `crates/xtask/fixtures/hello`, which loops forever, compared byte
for byte with a bounded `interp` run; plus the canonical *Hello world!*.

### S3. Choose successors

Records carry up to four successors; `pointer` picks one by the popped value
modulo four, `switch` one of two by its parity.

Checkpoint: `crates/xtask/fixtures/pointer`, and a hand-built branching program,
both compared with `interp`. **This is full Piet semantics for a compiled
program**, and it is where the image stops being a trace replayer.

### S4. Interpret the grid (optional)

Move the partition and the movement rules into the image, so the host only hands
over colours. This is the only version that needs nothing but the pixels.

Checkpoint: the canonical hello world from its grid; then *Piet's Quest*.

Cost: 20 000 steps per second for a large image, and movement work on top. Worth
doing for small images and as a completeness claim, not as the way to run a big
program.

## Recommendation

S1 and S2 together are the milestone worth committing to: they are what turns
the image from a replayer into a program that follows its own control flow. S3
then closes the gap to full Piet for compiled programs, and is small once S1
exists. S4 is a separate, optional build and should be decided on its own.

The number that decides whether this is worth it is S1's: if the structured
backend lands and the loop case passes, every later stage is engineering with a
known shape. If it does not, no amount of tape design helps.
