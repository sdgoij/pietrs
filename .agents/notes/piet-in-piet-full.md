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

### S1. A structured backend — done

`crates/loom/src/flow.rs` compiles `Program { blocks, entry }` to an image. The
decision that removed the risk is that **no block branches**: a block computes
the id of its successor and hands it to one shared dispatcher, which is the
temporary-chain dispatcher the trace interpreter already used, unchanged. `if`
is not a jump either — the condition is normalised with two `not`s and the
successor is selected arithmetically,

```text
next = otherwise + m * (then - otherwise)
```

so the only control-flow geometry in the image is the dispatcher. `halt` pushes
an id no block has and falls through it.

The ladder is reused as-is: a prologue between the launcher and the head carries
the entry id, each block is one lane, and the return path cancels the value the
head is entered with, exactly as the trace interpreter does.

Checkpoints, all green:

- a loop summing 1..=n against `(1..=n).sum()`, for n up to 100;
- a loop whose body takes both arms of an `if`, against a Rust map-sum;
- a nested loop, n(n+1)/2;
- constants, including zero, negatives and 1 000.

The layout costs, measured: the four-block sum lays out at 42 x 41, the
seven-block alternating sum at 57 x 38, the seven-block nested loop at 57 x 39 —
about 2 000 codels. The bus is five cells per block, and the lane height is set
by the longest block, which is longer than it looks because a constant expands
to up to twelve commands and `dup local` to twelve more.

That is the lever for later stages. A sixty-block interpreter would be roughly
320 columns and, with blocks of that complexity, 60 to 100 rows: 20 000 to
30 000 codels, ten times the current image. Of the two shortcuts I predicted,
only one paid: `dup local` at depth one is now a plain `dup` (one command
instead of ten). Pushing a constant with a multi-codel block trades one command
for one lane cell each, so it is a wash up to about twelve and a loss above it;
the arithmetic form stayed.


### S2. Walk the state graph — done, as a graph compiler

This landed differently from the plan, and the difference is the important
part. With one lane per state, a *program counter* is unnecessary: the state's
block ends with a jump to its successor, so the dispatcher is the program
counter. Nothing about the graph is baked into the *input* any more — the trace
is gone, and `pointer`/`switch` choose their successor from the value stack at
run time.

`crates/loom/src/graph.rs` compiles a `StateGraph` and the partition into a
`flow::Program`: one block per state, the command's stack effect in the block,
and either a literal jump or a chain of equality tests over the popped value.
Each successor gets a trampoline that drops the selector and jumps.

Checkpoints, all green, each against `interp`:

- the canonical *Hello world!*: 25 states, output identical, both terminate;
- `xtask/fixtures/pointer`, which uses `pointer` **and** `switch`: the image
  makes the same decisions, compared as a prefix;
- `xtask/fixtures/hello`, the loop checkpoint: the image prints its own prefix
  of the same endless run;
- `xtask/fixtures/single`, which terminates without a command.

The one real bug this caught: the equality test was written with two `not`s,
which is the *inequality* test, so every branch took the other arm. The pointer
case failed with `6` against `A` — which is exactly what an inverted selector
looks like.

Cost, measured: five cells of bus per state, so 25 states lay out at
147 x 45 (6 615 codels), the pointer program's 23 states and 39 blocks at
217 x 44 (9 548), and the five-state loop at 47 x 29 (1 363).

**Why this does not scale to *Piet's Quest*.** 17 964 states at five cells each
is a 90 000-codel-wide bus, and every jump would run 90 000 equality tests, so
the dispatch alone would be four orders of magnitude too slow. Compiling the
graph is the cheap way to reach full semantics for one program; running a big
program needs the image to be fixed and the program to be data.

### S3. The fixed-image walker (the scale answer)

The ring design is pinned, and the tape above is its first piece:

```text
  [ interpreted values … | k temporaries | ring of N cells ]
```

The ring holds the program's records and sits on top, so the head is at the top
and a jump is one `roll(N, offset)` — relative offsets are stored in each record
and no program counter has to be maintained. The temporaries and the
interpreted stack live below the ring at fixed depths `N+1 …`, which is what
makes their `dup local` depths static. A value-stack access is two rolls (bring
the cell up, restore the ring); the perf harness prices one at 1.79 µs for an
18 000-cell ring.

**The tape is built and tested** (`crates/loom/tests/tape.rs`). Its invariant is
the length on top:

```text
  [ value 1, value 2, …, value len, len ]
```

`value 1` is at depth `len + 1`; rolling exactly that many brings it up and
leaves `len` at depth two, so one pop and one decrement shrink the tape with the
length still reachable. The case fills 1..=n and prints it in order — reaching
the oldest value every time at a depth that changes as the tape drains — for n
from −3 to 40, and was proved non-vacuous by breaking the computed depth, which
reverses the output for every n above one. It lays out at 57 x 42.

The interpreter's own program is still the substack: a step loop, a dispatch on
the record's command, per-command handlers, and the jump. That is the next slice,
and it is large.

**The control path is now built and tested** (`crates/loom/src/walker.rs`,
`crates/loom/tests/walker.rs`). `skeleton(ring)` loads a ring from the input
stream and follows it: read `N` numbers, report the head record's command, halt
on a zero command, otherwise roll the ring by the record's offset. The stream is
read **highest record first**, so the last record read is record zero and it is
the initial head — that is what makes the stored offsets plain, `3 * k` forward
and `-3 * k` back, with no program counter and no initial roll. Records have to
be a whole number of `command, block size, offset` triples; anything else is
`Error::Ring`.

The cases: a two-record ring reports `1 0 `; a four-record chain reports every
command in order; a forward offset of six skips the record that would print
nine; a negative offset wraps to the last record; a ten-record ring keeps the
head aligned across repeated rolls; rings of 0, 1, 3, 4, 5 and 7 cells are
refused. Breaking the jump so it reads the record's block size instead of its
offset fails three of them, which is the non-vacuity check.

What the skeleton does *not* do: it carries the block size without using it, and
it cannot tell a misaligned offset from an aligned one. Both are the handlers'
problem. The image is 57 x 52, and it does not grow with the number of records:
the ring is data, not geometry.

**The handlers are built and tested** (`walker(&instructions)`, same file and
`crates/loom/tests/handlers.rs`). It weaves the fixed image and returns the
stream that loads the ring. Twelve of the fifteen commands have handlers: push,
pop, the arithmetic set including divide and modulo (the image's own runtime
supplies the floored semantics), not, greater, duplicate, out number and out
char. `roll`, `in number` and `in char` are refused when the ring is built with
`Error::NoHandler` — the first because it needs a dynamic rotation of the
*values*, the other two because the interpreted program's input is not carried
after the ring yet.

The memory model is the one pinned above, made concrete: the values sit *below*
the ring, so the head's fields stay at fixed depths and every value access is a
rotation of the ring past the values — `roll(N+1,1)` to insert, `roll(N+1,N)` to
bring one value up, `roll(N+2,N)` to bring two up in their original order, and
`roll(N+2,2)` after a `dup` to put both copies back. Every amount is a multiple
of the record width, so the head stays on a record boundary. A handler is then
one block, and the command code indexes the handler table arithmetically
(`next = 7 + code`), which needed one addition to the structured backend: a block
that leaves a *computed* successor instead of a literal one (`End::Computed`).

Checkpoints, all against `interp`:

- a push/out char pair prints `A`, and the pair really is `push 65`;
- subtract reads its operands in Piet order: `10 4 -` is 6, `4 10 -` is -6;
- `((2+3)*4)/2` prints 10, so values survive a chain of handlers;
- `dup` leaves exactly one extra copy: two prints of 7, and `dup add` on 7 is 14;
- `pop` drops only the top value;
- an empty program halts;
- the canonical *Hello world!*'s own instruction stream runs from a ring with
  identical output.

Non-vacuity: making the binary access bring up one value instead of two fails
three cases, and reading the block size one cell too deep (the offset) fails six,
including *Hello world!*.

Cost: the walker is 137 x 56 for a two-record ring and 137 x 93 for a
sixty-five-record one. The width is the dispatch table and does not move with the
program; the height moves only because a constant for a bigger ring takes more
cells to build.

The known divergence, and the next piece: **underflow**. The runtime ignores a
`roll` deeper than the stack, so a handler that reaches for a value which is not
there silently pops a ring cell, where `interp` ignores the underflowing command.

A depth counter has to live at a depth a jump does not change, and the obvious
answer — a fourth cell in each record — does not work. The depth is global, so a
jump would have to migrate it into the new head's cell, and that cell sits at the
depth the record's *offset* names, which is data. The workable shape is a fixed
register block between the values and the ring: the ring is rotated by whole
records, so a block below it keeps its depth across jumps, and pushes land
between the block and the ring. Reading the counter then costs two ring
rotations, and poking it two more, which is the price of matching `interp` on
underflow.

### S4. Interpret the grid (optional)

Move the partition and the movement rules into the image, so the host only hands
over colours. This is the only version that needs nothing but the pixels.

Checkpoint: the canonical hello world from its grid; then *Piet's Quest*.

Cost: 20 000 steps per second for a large image, and movement work on top. Worth
doing for small images and as a completeness claim, not as the way to run a big
program.

## Recommendation

S1, S2 and the walker's read-execute-jump loop all landed. The image now follows
a program held in its input, runs twelve of the fifteen commands on a value
stack below the ring, and matches `interp` on the canonical *Hello world!* from a
fixed 137-column image.

The remaining gaps, in order of size:

- **Underflow** is a divergence, not a missing feature: an access from an empty
  value stack pops a ring cell instead of being ignored. A depth cell in the
  record fixes it.
- **`roll`** needs a dynamic rotation of the values, which means rotating the
  ring out of the way and back; it is the last of the fifteen.
- **Input** needs the interpreted program's stream carried after the ring, and a
  `in char` encoding that does not collide with the numbers.
- **Scale** is then a matter of putting a real program's graph into records, with
  offsets that are multiples of three, and measuring against *Piet's Quest*.

S4 is a separate, optional build and should be decided on its own.
