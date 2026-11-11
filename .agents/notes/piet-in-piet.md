# A Piet interpreter in Piet

`crates/loom` generates a Piet program that reads another Piet program from its
input and runs it. It is a self-interpreter for the language's straight-line
fragment, and it is generated rather than drawn: a Piet command *is* a colour
transition, so a program is a colour walk over a grid, and the generator fixes
the path and then solves for the colours.

## What the interpreter understands

Piet has 17 commands. Two of them, `pointer` and `switch`, move the direction
pointer or the codel chooser; the other fifteen only touch the value stack. The
interpreter models the stack and nothing else, so it interprets exactly the
fifteen commands that leave the geometry alone — the linear fragment.

```text
1 push   2 pop     3 add    4 subtract  5 multiply
6 divide 7 modulo  8 not    9 greater   10 duplicate
11 roll  12 in number       13 in char
14 out number               15 out char
```

`push` is followed by its operand, the block size to push, because that is
where a compiled `push` gets its value from. Opcode `0` halts; so does any
number outside `1..=15`. A program is therefore a whitespace-separated stream:

```text
1 72 15 1 105 15 0      # push 72, out char, push 105, out char, halt
```

The stream *is* the interpreted program's input as well as its code, so the
encoder inlines the program's own input at each `in number` or `in char`. One
consequence: `in char` cannot read an ASCII digit, because the digit would join
the opcode before it. `encode` reports that rather than mis-encoding it.

## Why the image is built, not drawn

A transition between two blocks runs the one command the palette difference
names, and the palette is a cycle in both coordinates, so a command is an
invertible translation. From a pair of colours the command is forced; from a
colour and a command the next colour is forced. Laying out a program is
therefore a colour walk, and the only freedom is the starting colour and the
filler between real commands.

The filler has to leave the value stack alone. `crates/loom/src/shade.rs` holds
eleven stack-neutral macros — `push, pop`, `duplicate, pop`,
`push, push, add, pop`, … — and a breadth-first search over the eighteen
chromatic colours finds the cheapest walk from one colour to another. Padding
with two zero-delta macros of length two and five reaches any requested filler
length.

## The layout

`interpreter()` tries starting colours, lane colours and lane heights until one
of them partitions into exactly the blocks the path needs, so no accidental
merge joins two corridors.

```text
S H [push][sub][dup][not][pointer] … [push][pointer]      row 0, the bus
| |    |                                        |
| |    +---- descent lanes, one per opcode -----+         one column each
| +--------- ascent --------------------------------------+  column 1
+----------------- one wide lane block -------------------+  the bottom row
```

- `H` is the loop head. The launcher `S` enters it at start and the ascent
  enters it at the end of every lap; both transitions run `in number`, so a lap
  reads exactly one opcode.
- Each test subtracts one from the opcode, duplicates, negates and runs
  `pointer`: a match turns the direction pointer down into that opcode's lane,
  no match walks on to the next test.
- A lane's first transition pops the zero the test left behind, and its second
  runs the opcode itself, so the interpreter's stack *is* the interpreted
  stack. The interpreted `push` is the one exception: it runs `in number`, which
  reads the block size from the stream.
- The rest of the lane is neutral filler that walks to the colour the shared
  return path needs.
- The return path is one wide block. Entering it runs one filler `push`, the
  first codel past it pops it, and the ascent climbs back to `H`. Every block a
  lap travels through is one codel wide, so the codel chooser never changes
  which codel is left from, and the layer of state the interpreter does not
  model stays irrelevant.
- Running out of tests is the halt. Two cells push and rotate, and a three-codel
  bar entered above its middle codel has no exit: the movement rules try both
  ends of every edge, and the entry codel is in the middle, so the program
  terminates.

## The sample program

`linear_program(text)` builds a straight-line program that prints `text`. One
character costs three blocks: a block of `code` codels, a one-codel block whose
entry runs `push` (pushing the previous block's size, which is the character
code), and a one-codel block whose entry runs `out char`. A launcher of three
codels puts the upper-left codel in the start block while keeping the program
row low enough for the terminator bar to reach above it.

## Tests

`crates/loom/tests/self_interpreter.rs` runs a generated program directly
through `interp`, reads its trace back with `instructions`, encodes that trace,
and runs the *interpreter image* on the encoding. The two outputs must agree,
which checks the image against the reference semantics rather than against
itself. Separate cases name every opcode the interpreter claims to run, the
halt on an unknown opcode, and the input inlining.

## Limits

- Only the linear fragment. A program whose trace contains `pointer` or
  `switch` is refused, not mis-executed.
- The interpreter does not model the interpreted program's direction pointer or
  codel chooser, so it cannot run programs that use them to branch.
- The interpreted stack is the interpreter's stack, so a program that overflows
  the runtime's stack overflows while being interpreted too.
- `in char` cannot read an ASCII digit, as above.
