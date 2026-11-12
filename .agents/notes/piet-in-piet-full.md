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

**Superseded by the four-cell record.** `skeleton()` now takes no ring argument:
the stream's first number is the ring's size, the loader counts it down, and the
offsets are `4 * k` because a record is four cells. `Error::Ring` is gone — there
is nothing left to refuse, since the image reads the size instead of being told
it. The cases above keep their shapes with the constants moved from three to
four.

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

**Underflow is handled.** A command that needs more values than the stack holds
is ignored, as `interp` has it; the runtime would ignore the over-deep roll
instead, and the handler would then pop a ring cell. The fix is a depth counter.

A counter has to live at a depth a jump does not change, and the obvious answer —
a fourth cell in each record — does not work. The depth is global, so a jump
would have to migrate it into the new head's cell, and that cell sits at the
depth the record's *offset* names, which is data. The shape that does work is a
**fixed register block between the values and the ring**: the ring is rotated by
whole records, so a block below it keeps its depth across jumps, and a push
lands between the block and the ring. The stack is then

```text
  [ value 1 … value k | depth | head record ]
```

with the depth at `N+1` and the values below it, so reading the counter costs
`roll(N+1,N)` and writing it two of those. Each handler is two blocks — a guard
that tests the counter, then the body — and `push` is the only unguarded one.

`divide` and `modulo` need one more branch. A zero divisor consumes both operands
and pushes nothing, so the depth falls by two rather than one, and the handler
has to know which happened before the operands are gone: it tests the divisor and
the two paths differ only in the amount they subtract from the counter.

Checkpoints, all against a reference:

- every underflowing command (`pop`, `dup`, `not`, both outputs, both binary
  forms) is ignored, and the program still prints `A`;
- `7 / 0` and `7 % 0` consume both operands, and a following subtract that needs
  two values is skipped — the counter is exact, not merely close;
- 32 generated 24-instruction programs, each compared against the *same*
  instruction sequence assembled into a real image and run by `interp`
  (`assemble`, the straight-line backend). That is the differential case that
  catches a counter drifting anywhere.

Non-vacuity, each observed then reverted: removing the guard fails three cases;
leaving binary operations' depth unchanged fails two, including a generated one;
sending a zero divisor down the normal path fails the exactness case.

Cost: 232 x 85 for a two-record ring and 232 x 157 for a sixty-five-record one.
The width is 42 blocks of dispatch table and does not move with the program; the
height moves only with the digits of the ring size.

**Input landed with the same twelve commands.** The interpreted program's stream
follows the ring, one event per read, in the order the program runs them: a space
and then the value. A character is written as its **code point**, not as itself,
so that the one handler serves a graph, whose reads are chosen at run time; for a
straight line the builder knows the order and refuses a program whose reads
outrun the input (`Error::InputExhausted`).

The handlers are small: `in number` is `in number` plus an insert; `in char` is
`in char`, `pop`, `in char`, insert. Neither needs a guard, and both raise the
depth counter by one. `walker` consumes the input in instruction order and refuses
a program whose reads outrun it (`Error::InputExhausted`), so the handlers can
assume the read happened.

Checkpoints, all against the straight-line reference with the same raw input: a
character reads and prints back; a digit reads as a character; a negative number
reads; a character, then a number, then a character arrive in order; and 24
generated reading programs agree with the reference, on an input built alongside
so every read has something to read.

Non-vacuity: leaving the depth counter unchanged for the reads fails five cases.

**The graph is the program.** `graph_walker(&graph, &blocks, input)` compiles a
state graph into records, which is what makes the image follow a program it was
not woven around. One record per state, with the state's outgoing command; a
deterministic state's offset is the cell distance to its successor; a branch
state's trampolines — one per successor — go directly after its own record, so
the handler only widens the popped value into `3 * (1 + index)` and replaces the
offset cell, which is the top cell and therefore a two-command poke. Three codes
are not Piet commands: `nop` for a trampoline, `pointer` and `switch` for the
branches. An empty stack is the one place a branch is not a skip — the reference
pops nothing and uses zero — so the guard's other side takes the first
successor.

Checkpoints, all against `interp` on the original image: the canonical *Hello
world!*; the `pointer` fixture, which uses both `pointer` and `switch` and so
selects successors at run time; the single-codel fixture, which halts without
running anything; and the looping fixture, as a matching prefix.

**`roll`, with a bound.** The depth is the one operand that decides the work, and
doing it with a depth known only at run time is the case that needs a put-back
amount which cannot survive the lift. With the depth known at weave time there is
no such problem: `roll(d+N+3, N+1)` lifts the values and both operands above the
ring, the `roll` command rotates the values, and `roll(d+N+1, d)` puts them back —
three constant rotations. So the handler is a table with one case per depth from
two to sixty-four, selected arithmetically from the popped value, and a deeper
roll stops the walker instead of rotating ring cells. The bound is a trade-off,
not a limit of the design: Piet's Quest's bedroom needs 29 to print its room
description, so eight was too shallow.

Checkpoints, against the straight-line reference: rotations of one, two and three
of three values; every depth from two to sixty-four; a depth of one, which is a
no-op that still consumes its operands; a depth beyond the stack, which is
ignored; and a depth past the table, which stops. Non-vacuity, observed and
reverted: shortening the lift window by one cell fails both rotation cases.

**Piet's Quest runs.** Measured on 2026-11-11, debug build, this machine: 17 964
states and 19 352 edges become a ring of about 30 000 records — a 158 KB stream —
and the image is 337 x 394. Run from the ring it prints its title screen and seven
menus before the input runs out: 205 116 868 machine steps in 54.5 s, which is
about 0.3 million interpreted commands at 5 300 per second. That matches the
estimate of 300 000 steps for six menus, and it is the first time the project has
run that program from anything but `interp`.

**`roll` is not implemented, and it is one dynamic amount away.** The operands
are easy: they are the top two values, so one rotation each brings them up.
Lowering them back into the value stack with two `insert`s and then rotating the
window

```text
  [ w1 … wd | depth | rolls | regs | ring ]
```

by exactly `R+N` puts the `d` values and both operands above the ring, in the
order `Roll` wants — values, then depth, then rolls on top — so a single
`Roll(d, r)` does the value rotation itself. That lift is one rotation of a
constant amount, and the window size `d+R+N+2` is computable because `d` is still
at a static depth before the lift.

The blocker is the **put-back**. After the roll the stack is `[regs | ring | w']`,
and returning to the steady state is a rotation of the window `d+R+N` by `d` —
again a data-dependent amount. `d` was consumed by `Roll`, and everything below
the rotated values now sits at a depth that depends on `d`, so it cannot be
re-read from a register or a record cell: a dynamic-depth read needs a value at a
static depth, and the only static depth left is the top of the stack, which holds
a value the roll has just moved.

Dead ends, checked so they are not retried:

- a fourth record cell for `d`: it moves with the head, and the depth it would
  have to be migrated into is the record's *offset* — data;
- keeping the value count on top of the values (the tape invariant): the
  conversion amounts become static, but the count then sits inside the rolled
  window;
- a fixed-capacity value area: every window becomes constant, but each push and
  pop then rotates the whole area, whose size is the stack limit;
- one rotation that both lifts and reorders: a rotation of `[w, regs, ring]`
  moves the values as a block and cannot rotate them internally, because the
  required permutation is not a cyclic shift of any window containing the ring.

The way through is a **page of values above the ring**, and it is not cheap: a
jump rolls exactly the ring, so a page above it would be torn unless it is pushed
into the value stack before every jump — one ring rotation per page cell, `P` per
dispatch. What it buys is that every in-page access becomes a small roll, and
`roll` with `d <= P` becomes a plain `Roll(d, r)` with the values already at the
top. A page *below* the ring does not do this: the depths are static, but the
values are not at the top, so `Roll` still cannot act on them.

**A fixed ring size does not work either, and this is worth recording because it
looks as though it should.** The idea is to weave the image for one capacity and
pad or tile the stream up to it, so every roll amount is a constant and one image
runs every program. It fails on the jump. The ring is a cycle, so a jump is a
`roll` whose amount is taken modulo the program's own record count; for the head
to land on the right record after wrapping, the ring's size has to be a **multiple
of that count**. A fixed capacity is a multiple of one program's count and not of
another's, so a cyclic program drifts one record per wrap and stops on a record
that was never its successor. Measured: the loop fixture and the branching
fixtures pass, and *Piet's Quest* — 20 110 records, ring 49 152 — prints three
characters and halts.

The invariant to remember: after a data-dependent lift, nothing below the lifted
values can be addressed, so any amount needed for the put-back must live above the
ring and survive the `Roll` — and a `Roll` consumes its operands. For the jump,
any fixed-capacity ring has to be a multiple of the program's record count, which
is what makes the image per-program in the first place.

**So if one image has to serve every program, the ring size must be data.** That
is the design to build, and it is a rework rather than an addition:

- a record becomes four cells, `[command, block size, ring cells, offset]`, so the
  offset stays at depth 1 (the branch poke) and the ring size at depth 2, where
  `dup_local(2)` reads it with two *depth-two* rolls — cheap, unlike a ring-sized
  rotation;
- the jump is `dup_local(2)` twice — the size, then the offset, now two slots
  down — then `roll`, with no constants at all; and because the ring is exactly
  the program's records, the offset modulo the record count wraps correctly,
  which is what the fixed capacity broke;
- every value access replaces `roll(N + j, k)` with the size pushed, `j` added,
  then the roll: four or five extra commands per access and no extra rotations;
- `MAX_RECORDS` and its error go away, because nothing is padded or tiled.

The hard part, and the reason this is not a small edit: the guard reads the depth
register, which sits at depth `N + 1` — a *computed* depth, so `dup_local` cannot
express it. It has to be written as `roll(N+1, N)`, `dup`, `roll(N+2, 1)`, and the
second amount needs `N` again *after* the first roll has displaced the head. The
workable shape is a copy of `N` pushed above the ring before the first roll and
tracked through the rotation, since the rotations are by known amounts; every
site that reads the register must do the same, and each one is easy to get wrong
by one cell.

**Built, and the image no longer depends on the program.** `walker.rs` carries the
ring size as the record's third cell, and the stream begins with the cell count,
which the loader reads and counts down — so neither the roll amounts nor the
number of records reaches the geometry. `program()` takes no argument, and
`one_image_serves_every_program` pins one 617x246 image for a two-command
program, a sixty-five-record one, and a state graph alike.

*Piet's Quest* — 17 964 states, 20 110 records, an 80 440-cell ring — weaves to a
282 027-byte stream. The same `walker.png` prints the title and nine menus and
halts when the eight-value input runs out: 135M interpreted steps, about 4.5 s
for the whole example case. The fixed-capacity build recorded 205M steps and
54 s for seven menus, but it padded the ring and was timed through a different
path, so treat the pair as anecdote rather than a benchmark.

The branch poke still works because the offset stays at depth one, and the guard
still reads the depth register the same way: `dup_size` is `dup_local` with a
depth computed from the size instead of a literal.

### S4. Interpret the grid (optional)

Move the partition and the movement rules into the image, so the host only hands
over colours. This is the only version that needs nothing but the pixels.

Checkpoint: the canonical hello world from its grid; then *Piet's Quest*.

Cost: 20 000 steps per second for a large image, and movement work on top. Worth
doing for small images and as a completeness claim, not as the way to run a big
program.

**The partition is built** (`crates/loom/src/grid.rs`, `crates/loom/tests/grid.rs`).
The splitter reads a grid's colour codes and reports, for every ring cell, the
lowest ring position in its 4-connected same-colour component — the same answer
`canvas::Blocks` gives, with the ring position standing in for the block id.

The layout is the trick that makes the movement rules tractable later. The grid
sits in a ring of four-cell records `[label, colour, stride, ring size]`, padded
with a guard row on top and a guard column on the left, so *every* neighbour is a
ring cell: left is `-4` cells, up is `-stride`, and no edge test or wrap case is
needed. The guards carry colour codes no codel and no other guard has, so they
can never merge. `stride` and the ring size ride in every record, so the roll
amounts are data and the image is the same for every grid — `splitter` takes the
grid only to build the stream.

Labels are minima, propagated to a fixpoint. Each pass visits every cell and,
when a neighbour's colour matches, keeps the smaller label. **All four
neighbours are compared**, and that is not an optimisation: with only the left
and up ones a minimum can never travel backwards, and a black L-shape converges
to a label that is merely the smallest reachable from the north-west. The pass
repeats while the `changed` accumulator is non-zero, so the number of passes is
the component's diameter rather than a constant.

The measured cost: the canonical hello world (30 x 29 codels, 39 blocks, a
3 720-cell ring) partitions correctly in 1.9 s in release. `Piet's Quest` is not
worth partitioning this way — the ring is 240 000 cells and every neighbour roll
rotates all of it — so the completeness claim stops at the small image.

**The edge codel is built too** (`loom::block_edge`). Given a block and a state
it prints the codel that state's own direction and chooser select — the single
attempt, which is `tessera::block_edge` and not `tessera::edge_codel`, because
the latter keeps trying until a move leaves the block. Every block of a row, a
diagonal, a ring with a hole, a mixed 4x5 grid and a plus shape agrees with
`tessera` in all eight `(dp, cc)` states.

The trick is the score. Each cell of the block is worth
`primary * big + secondary`, where `big` is the ring's cell count, so the two
halves cannot carry into each other. Each half is flipped (`row cells - value`)
when the state wants a minimum, which turns max-primary/min-secondary,
min-primary/max-secondary and the rest into the same single rule: keep the
larger score. The eight cases then differ only in which half is flipped and
whether the row or column is the primary, so each is one small block of constants
and the scan itself is shared. Decoding the winner is a division and a remainder.

That also settled an interface question: `tessera::edge_codel` is not "the edge
for this state" but "the codel a move leaves from", fallback included. The raw
selection is now public as `tessera::block_edge` so both levels can be tested
against the same oracle.

**The step needs a guard on every side, and now has one.** Stepping right from
the last column used to wrap onto the first codel of the next row, because the
ring is a cycle and only the top and left were guarded. The border is now one cell
wide all round, so a step in any direction is `±4` or `±stride` and lands on a
guard when it leaves the grid. A guard's colour code is `22 + index`, so `colour
>= 22` is the same signal as black, and the partition is unaffected.

**The step is built** (`loom::step_state`). It decodes the winning score back to
a codel, seeks the ring to it with a one-cell loop, steps once in the direction,
and reads the neighbour's colour: a guard or black is `-1` (the attempt fails),
white is `-2` (the slide, still to build), and a chromatic neighbour is the move
— its label and the attempt's `(dp, cc)`. Every block of five fixtures agrees
with `tessera::block_edge` plus `tessera::step_codel` in all eight states.

Two things the build settled. The scan stops on the last cell it *scored*, not on
cell zero, so the step stage advances once before seeking; without it the whole
ring is off by one cell and every move lands on the block's own codel. And the
decode has to drop the raw score halves from under the two coordinates it just
pushed, which is a `roll(4, 2)` and two pops — a plain pair of pops takes the
coordinates instead.

**The exit and the slide are built** (`loom::exit_state`). The eight attempts
run the same scan in a loop. The exit keeps `attempt`, `cc` and `dp` under the
scan's six live values, and `ring_by` finds the record's ring size from the
*total* live count, so the scan's data roll is told the total (nine). After a
failed step the continue block drops the scan's three, counts the attempt up,
rebuilds `case` and prints nothing; after the eighth failure it prints `-1`.

Two conventions the build settled. `case_nodes` computes a throwaway `q` and
`dp_i` on top of the live values, so it must dip the case below them before
popping — the first version popped one value too many and silently ate `big`.
And the scan derives a cell's coordinates from the loop index, not from the
ring's position, so every attempt has to start with the ring back at cell zero:
a short loop advances until the head's label is zero, which only cell zero can
be, because it is a guard and guards never merge.

The slide is a state machine over `(codel, dp)` held in `case`: step, read the
neighbour's colour, and on a guard or black step back, toggle the chooser and
rotate the direction clockwise. It needs a bound instead of the host's visited
set: more than `4 * cells` turns without reaching a colour is a retrace, and the
answer is `-1`. A chromatic neighbour is the move, and the final `case` is the
`(dp, cc)` reported, which is what the slide's last rotation selected.
The edge codel's colour is the block's colour, so one comparison after the seek
rules out white and black blocks, which the movement rules never leave.

Every block of nine fixtures agrees with `tessera::exit` in all eight states,
in release: the eight-attempt fallback, the slide to a colour, the slide that
retraces, the slide that restarts from its current codel, and non-chromatic
blocks.

**The command is built too.** `exit_state` prints a fourth field: the walker's
record code for the colour transition. The source block's colour is stashed in
the dead `best` slot before the step, and the target's is the ring head's colour
after it, so the exit computes the hue and lightness deltas and maps
`hue * 3 + lightness` to the walker's codes — 1 to 15, 17 and 18 for `pointer`
and `switch`, and 16 (`nop`) when the transition runs nothing. A slide's move is
not a transition, so it has its own move block and prints 16; that is the
interpreter's semantics, not an image shortcut.

The **start state is built** (`loom::start_state`). The partition ends with the
ring at cell zero, so the upper-left codel is one row down and one cell right:
`row cells + 1`, the ring index `stride/4 + 1` the note predicted. Its colour
decides: black is `-1`, a chromatic codel is the state `(label, right, left)`, and
white hands the same slide blocks the exit stage uses a nine-value live layout
with `case` at right/left and the turn counter at zero. The slide is now one
builder (`slide_blocks`) with derived ids, shared by both programs.

Every start fixture agrees with `tessera::start`, including the canonical hello
world, which starts in its upper-left block without sliding.

**The hand-over is built** (`loom::grid_graph`). It walks the state graph with
the image — one `start_state` run and one `exit_state` run per state — and
assembles the same `StateGraph` that `tessera::StateGraph::build` would, so
`graph_walker` takes the records unchanged. At this stage the host still supplied
the bookkeeping the image's four fields did not: the block sizes, the
label-to-block map and the offsets; S9 moved the first two into the splitter. A
`Resolver` trait in `tessera` is the seam; `Blocks` implements it, so both
builders share one walk.

That closes the checkpoint: the canonical hello world, decoded from its grid,
partitioned and moved by the images, hands the walker its records and prints
`Hello world!`. One full image run per state makes that an ignored release
harness; the default suite compares the small fixtures with `tessera`.

The staging is deliberate. Each step above is a scan over the ring plus a batch
of comparisons, and each can be checked against `tessera::block_edge`,
`tessera::edge_codel` or `tessera::exit` on the small fixtures before the next is
added, which is the same way the walker was built.

### S5. Read the pixels (done, for PPM)

The last thing the host still did that the image can do for itself.
`ppm_grid` (`crates/loom/src/ppm.rs`) reads a P3 PPM from its input and prints
the grid: the width, the height, then one colour code per codel in row-major
order. The codes are `canvas::Color::ALL`'s numbering, which is the one the
splitter and the movement stages already speak, so the host's remaining job is
to turn the codes back into a grid and assemble the ring.

A component is `0`, `192` or `255` in the palette and something else otherwise,
so each one becomes the digit `0`, `1`, `2`, or `3` for "not a palette value",
and the three digits index a 64-entry table of colour codes. The base is four,
not the palette's three, so a `3` digit stays unambiguous. Anything holding a
`3`, and the six cells of colour space the palette leaves empty, is white: the
specification's answer for a non-standard colour, and the answer
`canvas::Color::quantize` gives.

The digits come from three equality tests per channel rather than from
arithmetic. Division and remainder cannot tell `0` from `64`, so a component
that is not a palette value has to be recognised, not approximated.

```text
P3 2 1 255  255 0 0  255 255 255   ->   2 1 / 1 18
```

The checkpoint is `crates/loom/tests/ppm.rs`. The palette round-trips through
pixels; a non-standard colour and an empty palette cell are both white; a `P6`
and a scaled PPM are refused. The last case runs the canonical hello world end
to end: its 30 x 29 grid written as a PPM, decoded by the image, turned back
into a grid, and run by the walker, which prints `Hello world!`. The image is
512 x 72 and 7 610 blocks, because the dispatcher's five cells per block dominate
a program with 98 of them.

What it does not do is a picture format: `P6`, a maxval other than 255, comments,
and a codel size above one are all outside it. `P6` is refused outright because
its bytes are not a UTF-8 stream; a scaled PPM prints the dimensions and then
`-1`, which no colour code can be. Codel size detection and PNG both stay with
`canvas`.

### S6. From a picture to a running program (done, for PPM)

S5 plus the movement stages closes the frontend. `ppm_to_grid` runs `ppm_grid`
and turns the codes back into a grid; `grid_graph` resolves every move with the
start and exit images; `graph_walker` hands the walker its records. The whole
path is one command:

```sh
cargo run -p loom --example run-ppm -- program.ppm "10 3"
```

What the host still does is what cannot be an image's output: it carries text
between stages, and fills in the ring offsets the records carry. The block ids and
sizes it also needed at the time are the splitter's since S9, and the ring stream
has been the image's since S7.

The cost is the one S4 measured: `grid_graph` is one image run per state.
*Hello world!* from its 30 x 29 PPM through the image-resolved graph takes 96 s
in release (`the_canonical_hello_world_graph_runs`, which now starts from the
PPM), against 1.4 s when the host resolves the graph
(`the_hello_world_runs_from_its_pixels`). Both print `Hello world!`. One image
run per state is why the first is an ignored release case and the second is in
the default suite.

The step to a *single* image — one that reads a PPM and prints the walker's
records in one run — needs the ring's reconstruction, the partition's fixpoint,
the graph's queue and the records' assembly inside the one image, with
dynamic-depth rolls where the stages now take a host-built stream. That is S4's
third row of the table, not a plumbing change; the five stages above are each
checkable against `tessera` or against the host's own helper, which is why they
were worth doing separately first.

### S7. The ring in the image (done)

`ring_stream` in `crates/loom/src/grid.rs` was the last input the movement stages
took from the host: the ring size, then one `ring stride colour index` record per
cell, highest index first. `loom::ring_grid` (`crates/loom/src/ring.rs`) now
prints that from the colour codes `ppm_grid` emits, so the two frontend images
speak nothing but text and the host only carries it:

```text
2 1 / 1 18   ->   12 48 16 33 11 48 16 32 10 ... 48 16 22 0
```

`loom::grid_codes` writes the input side — the width, the height, one code per
codel — and `loom::ring_to_stream` runs the image and returns what it printed.

The ring is the grid with a one-cell border, four values per cell. Border cells
are guards carrying `22 + index`; the interior consumes the colour codes in ring
order, which is row-major over the grid. Guards never merge, so a guard's colour
is its index and the label the splitter reads is already in the stream.

Two pieces of arithmetic do the work. A cell is a guard exactly when
`row * (row - h - 1) * col * (col - w - 1)` is zero, which is three divisions and
remainders per lap instead of a chain of comparisons against the borders. And the
records have to come out highest index first, so each lap pushes its record
*below* the counters — one fixed-window roll of seven values — which leaves the
newest record nearest the top and the sentinel `-1` underneath all of them. The
print loop halts when the sentinel surfaces.

The counters never move relative to the records, so a lap is a fixed number of
commands and the stage is linear in the ring. Measured from 9 to 324 cells the
cost is flat at about 2 400 interpreted steps per cell; the growth is the fixed
prologue, not the loop. `ring_to_stream`'s 20 million-step budget therefore covers
a few thousand cells, which is the scale this stage is for — *Piet's Quest*'s
475 667-codel grid is not an input to it.

Measured on 2026-11-12, release, the canonical hello world (30 x 29 grid, 992 ring
cells): 2 403 295 interpreted steps, 45 ms, matching `ring_stream` token for
token. The image is 72 x 188 codels and 2 314 blocks.

The checkpoint is `crates/loom/tests/ring.rs`: a 1 x 1 codel, a 2 x 2 grid, a
3 x 3 grid with white and black, and the hello world's own grid, each compared
with `ring_stream`; `the_stream_is_the_one_the_splitter_reads` pins the ordering
against `loom::splitter`'s stream, so a disagreement cannot hide behind two
agreements with the same wrong helper.

### S8. The stages take the image's stream (done)

`ring_stream` in `grid.rs` built the splitter's input on the host, which left the
ring image beside the path rather than on it. Every stage now takes the text
`ring_grid` prints: `grid_ring` prints it once for a grid, `with_step_query` and
`with_exit_query` append a state and a label, and `splitter`, `block_edge`,
`step_state`, `exit_state` and `start_state` are thin wrappers over those.
`ppm_codes` exposes the PPM image's own output, so the frontend is three images
joined by characters — PPM, then colour codes, then the ring, then labels — and
the only stream a stage takes from the host is the walker's records.

`grid_graph` prints the ring once and reuses it for the start and every exit.
That is also where the saving would be: one ring run replaces 17 964 host-side
`ring_stream` calls on the hello world. Measured, it is 92.45 s against the 96 s
recorded for the same case before — inside the noise, because a case that spends
its time in 17 964 image runs does not notice the host's string building.

What the host still does is bookkeeping: `Blocks::partition` names the blocks the
graph is keyed by, and their sizes are what the walker's `push` records. Moving
that is a different shape of work — the sizes are a property of the partition the
splitter already resolves, so its output would have to carry them, not only the
labels.

### S9. The partition in the image (done)

The last thing the host still partitioned for itself was the walker's
bookkeeping: `grid_graph` asked `Blocks::partition` for the ids its states are
keyed by, and `graph_walker` asked it for each block's size, which is what a
`push` costs. Both now come from the labels the splitter was already printing.

A block's id is `canvas::Blocks`'s numbering, and that numbering is reproducible
from labels alone: `Blocks` numbers blocks in row-major order of their first
codel, and the ring visits the codels in the same order, so the host assigns
`0, 1, 2, …` as each new label appears. A block's size is how many cells carry
its label. `Partition` holds the result — label to id, id to label, id to size —
and implements `Sizes`, the one thing the walker asks for; `Blocks` implements it
too, so the host-resolved and image-resolved paths share the same walker.

`grid_graph` returns the graph and the partition, and runs the splitter once over
the ring: the hello world's 992 cells take 79.7 million steps, 2 s, on top of the
states' 92 s. `the_image_s_partition_sizes_the_walker_the_way_the_host_does` runs
the same program through both partitions and compares the record streams byte for
byte, which is the check that the reproduced numbering is the host's.

So a grid no longer needs a host partition to run: the images name the blocks,
size them and resolve every move. `Blocks` stays for tiling the images themselves
— any image has to be partitioned to be run — and for the host-resolved graph the
walker's tests build directly.

### S10. One image from pixels to output (measured, not built)

The last row of S4's table is one image that reads a PPM, partitions it and runs
it. The measurements say where the cost is, and it is not where the staged plan
assumed.

**Every query re-runs the partition.** Each stage here is "the splitter plus
something": `edge_program`, `step_program` and `exit_program` all start from
`blocks()` and jump past the partition only if they could trust someone else's
labels. On the canonical hello world the stages decompose as: splitter
79 730 314 steps, edge 84 381 247, step 84 772 539, exit 84 785 578. So a move
is about 5 million steps and the other 79.7 million are the partition being redone
for the twenty-fifth time. That is why the image-resolved graph costs 94 s for 25
states: 25 x 3.2 s of partition plus 25 moves.

**A block visit costs the image's height.** The count above is dominated by
transport, not by arithmetic. The layout is one bus row, one lane per block, and
one return column; every dispatch walks down a lane and back up the return path,
so a block visit costs about twice the image's height. On the edge stage a
20-million-step run visits each of 12 entry-column blocks 4 995 times — about
4 000 steps a dispatch, against an image 1 989 codels tall. The height itself is
set by the longest block: the splitter's twelve blocks total 2 394 commands and
block 8 (`BODY`, the partition's per-cell rule) is **1 981** of them, so every
block in every stage that includes it pays for those 1 981 codels on the way up
and down.

**So the machine is the cheap path, not the dear one.** A single program that
partitions once and then loops — scan, step, slide, command — spends on the hello
world 79.7 million steps for the partition, 5.1 million on the first move and
about 21 million on each later one: a move often needs several of the eight
attempts, and every attempt rescans the ring. The whole walk of 24 moves is
roughly 600 million steps, twelve seconds in release, against the eager graph's
94 s. For a grid of N codels the partition is ~80 000 x N and a move ~20 000 x N,
so the machine is usable for the sizes this repository tests with and out of reach
for *Piet's Quest*: 20 000 x 478 815 is 9.6 billion steps *per move*, and that
program has 300 000 of them.

**The storage.** The movement stages keep four values per ring cell, so *Piet's
Quest*'s 478 815 cells are 1 915 260 values — over the runtime's million-value
default before a value is pushed. Packing a cell into one value, colour and label
in the two halves, brings it to 478 815; that is a change to the ring layout
itself, and the machine can start without it.

**The shape.** One program, three phases, one stack:

1. `ppm`'s decoder runs as a subroutine, one pixel per call, over the header and
   then the pixels;
2. the ring is built once, as [`crate::ring_grid`] builds it, and the splitter's
   fixpoint runs on it once;
3. the loop: read the state `(label, dp, cc)`, scan the ring for that label's
   edge codel, step and slide, take the command from the colour transition, run
   it on the value stack, repeat.

Phases 1 and 2 exist as programs and phase 3's pieces exist as `grid.rs`'s scan,
step and slide blocks and `walker.rs`'s command handlers. What is new is one
stack layout that all three agree on — the grid ring at the bottom, a live count,
then the values — and the loop that hands the state from the movement to the
command and back.

**The slices.** (1) the loop over a ring the caller supplies, printing the state
trace, checked against `tessera`'s walk — **done**, as [`crate::machine`]; (2) the
command handlers on the value stack, so the hello world prints — **done**, as
[`crate::run`], with a value stack below the ring (below); (3) slides and
`pointer`/`switch` — **done**, slides in the loop since slice 1 and the two branch
handlers in the run image; (4) the PPM and partition phases inside the same
program.
Each slice is checkable against the stage it replaces, which is how S1 to S9 were
built.

Slice 1 is the exit stage with its two tails looped back: the move prints, then
rebuilds the state the move reached and re-enters the scan. The new state is
`(ring head's label, case / 2, case % 2)` — a move's `(dp, cc)` are the attempt's,
and attempt zero's case is that same number — with `index = 0`, `best = -1` and
the attempt counter back to zero. Two things make the rebuild delicate: a move
leaves the command's two colour temporaries above the nine values, so the slide's
tail pops two and the move's four; and the ring's head must be brought back to
cell zero before the scan, because the scan's coordinates are its loop index.
`the_restart_rebuilds_the_scan_s_nine` checks the rebuild on a `Machine` over
nodes, with no image at all — which is what made the two swaps in the first
version findable in a second instead of ninety.

**Slice 2's open question was where the value stack lives**, and the answer is
**below the ring, always** — not the two-header-slot scheme sketched here before
the image was built. The movement only ever rolls the top `nine + ring values`,
so anything under the ring is invisible to it and `lift_ring` keeps reading the
ring size at depth thirteen. That makes the layout

```text
  [index best case big row label attempt cc dp | ring | k | V1 .. Vk]
```

stable across a move: there is no phase boundary, no whole-stack roll per move,
and no spare field per record (the `PER_CELL = 5` attempt, reverted). The one
trick is how a handler reaches a value. With the nine on top the ring size `N` is
at depth thirteen, so `k` is at `N + 10` and `V1` at `N + 11`; a handler builds
those depths at run time and rolls the values above the ring, applies the command,
and rolls them back. `k` never has to be known in advance, which is what the
three earlier layouts were all fighting. `roll` dispatches on its depth to a table
of blocks (depth two to sixty-four, as the walker does), so the lift and put-back
amounts are constants; a deeper roll consumes its operands and does nothing.

The commands run on the machine stack directly (the image's own `add`, `divide`,
and so on), so the arithmetic and the zero-divisor rule are the runtime's. The
hello world — `push`, `duplicate`, `out char` — prints `Hello world!` from the one
image, checked against `interp`.

**One image's first two seams are done.** `compose::ppm_ring` puts the PPM
parser and the ring builder in one image. The parser leaves `[width, height,
last pixel .. first pixel]` on the stack, keeping `[counter, width, height]`
above the accumulator so each finished code is one fixed `roll(4, 1)` below the
header. The ring builder walks the ring **backwards**, popping one pixel per
interior cell: the interior cells in descending order are the pixels in
descending order, so a plain LIFO pop matches them and the records come out
highest-index-first. No FIFO tape and no dynamic-depth roll are needed for this
seam, which was not obvious before it was built — a forward builder would have
needed one.

`compose::ppm_ring_splitter` adds the splitter. The ring builder pushes each
record `[index, colour, stride, ring]` below its registers `[i, w2, h2]`, then
hands over `[count, records ..]` at the block after the splitter's read loop.
The registers end up under the ring, which is exactly where the movement's
value stack lives, so they are inert. The one dynamic-depth operation is the
pixel pop: the records sit between the registers and the remaining pixels, so
the next pixel is at depth `4 + 4 * records`. A single `roll(depth, depth - 1)`
is a right rotation by one, which lifts the pixel and leaves the registers and
records in order — no put-back, no second roll. Both images are checked against
the two-image chain on the small fixtures and the canonical hello world.

The remaining seam is `splitter -> movement`. The splitter is already one
`Program` from the `blocks()` loader onward, so the ring builder can hand the
records to `exit_blocks()`'s block four instead; what is left is deriving the
start state inside the image (the `start_blocks` stage) rather than reading
`dp cc label` from a host query.

## Recommendation

Everything landed. The image follows a program held in its input — a straight line
or a state graph — runs all fifteen commands, and reaches *Piet's Quest*: 17 964
states, one 617 x 246 image, nine menus in about 4.5 s for the example case.

The frontend is images too, as far as text goes: a PPM becomes colour codes, and
the ring the movement stages read is printed from those codes rather than built
by the host. The only stream a stage still takes from the host is the walker's
records, which the host assembles from the graph.

What is left is not capability but cost, and it is worth saying which is which.

- **`roll`'s bound.** The table covers depths two to sixty-four. Extending it is one
  block per depth; removing the bound needs the values addressable at the top of
  the stack, which no layout that keeps the ring on top allows (the analysis is
  in the S3 section). A deeper roll stops rather than lying.
- **Speed.** The dispatcher's five cells per block and the ring rotation behind
  every value access dominate. A page above the ring would fix both and cost a
  flush before every jump.
- **`in char` from a graph** takes a code point rather than a character, because
  the graph decides its reads at run time and the stream has to be unambiguous.
  That is an interface wart, not a semantic one.

S4 is built end to end: the partition, the edge codel, the step, the
eight-attempt fallback, the white slide, the start state, the colour transition
and the hand-over to the walker are all in the image, and the host only
assembles the records. Every fixture agrees with `canvas::Blocks` and `tessera`,
and the canonical hello world runs through the image-built graph to
`Hello world!`. The cost is one full image run per state, so the big case is a
release harness; the small cases are in the default suite.

S7 to S9 then took the last three host streams away: the ring, the splitter's
labels and the block sizes are all printed by images. What remains of the host is
the graph, and S10 measures what moving it would take: the eager stages re-run
the partition on every query, so the hello world's graph costs 94 s where a
machine that partitions once and then loops would cost about 4 s. The single
image is worth building, with the lazy loop rather than the graph.
