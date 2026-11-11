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
encoder inlines the program's own input at each `in number` or `in char`. An
`in char` writes a separator before the character and the handler reads the
separator and throws it away, so a character that is also a digit cannot be
mistaken for part of the opcode before it.

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
- A lane's first transition pops the zero the test left behind, and the rest
  runs the opcode's handler, so the interpreter's stack *is* the interpreted
  stack. A handler is a command sequence, not one command: `push` runs
  `in number` to read its operand, `in char` runs `in char, pop, in char` to
  discard the separator, and every other opcode runs itself.
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

## The sample programs

`assemble(instructions)` builds an image for any instruction sequence: one
block per instruction, sized to its `push` operand, whose exit runs the
instruction. A three-codel launcher exits with a `pop` that the empty start
stack ignores, and a three-codel bar terminates the program.

`linear_program(text)` is the same builder over instructions that print text.
One character costs two instructions: `push` of the character code, then
`out char`.

## The canonical Hello World

`fixtures/hello-world/program.png` is the 30 x 29 canonical *Hello world!*
program that the editor ships with. Its trace is twenty-four instructions —
`push`, `duplicate` and `out char`, no `pointer` or `switch` — so it is inside
the linear fragment, and the generated interpreter runs it:

```sh
cargo run -p loom --example weave -- target/weave crates/gui/web/hello.png
target/debug/pietrs run target/weave/interpreter.png --input target/weave/program.txt
# Hello world!
```

The test `the_interpreter_runs_the_canonical_hello_world` decodes the fixture,
checks the reference interpreter prints the committed `expected.stdout`, and
then checks the generated image prints the same thing from the encoded trace.

## Refusing a program

`instructions` walks the machine one step at a time, so the first thing it
cannot model is reported at once, before the rest of the program runs:

```text
weave: pietquest.png: block 12 runs Pointer, so the program is not linear
```

*Piet's Quest* is 409 x 1163 codels and 13 912 blocks, and it branches in its
menu loop; running it to the end first, as the example used to, overflowed the
value stack with no explanation.

A slide through white is not refused: it runs no command, so it becomes no
instruction. Movement does not touch the stack and the interpreter never sees
it, which means a linear program may use white freely. A run that hits the step
limit is reported, because a program that does not terminate has no finite
instruction sequence to encode.

## Tests

`crates/loom/tests/self_interpreter.rs` runs a generated program directly
through `interp`, reads its trace back with `instructions`, encodes that trace,
and runs the *interpreter image* on the encoding. The two outputs must agree,
which checks the image against the reference semantics rather than against
itself.

A deterministic sweep does that for forty-eight pseudo-random programs over the
whole opcode set, including arithmetic, `roll`, output and number input, and
compares stdout for every one. Separate cases name every opcode the interpreter
claims to run, the halt on an unknown opcode, and character input including a
digit. One case runs the canonical *Hello world!* image end to end, one builds a
program that slides through white by hand, and two check the refusals: a
`pointer` (the interpreter image itself is the subject) and a program that never
terminates.

## Limits

- Only the linear fragment. A program whose trace contains `pointer` or
  `switch` is refused, naming the block; it is never mis-executed.
- The interpreter does not model the interpreted program's direction pointer or
  codel chooser, so it cannot run programs that use them to branch. Doing so
  needs the interpreted program's block geometry and random access to it, and
  the interpreter has one stack and no memory beyond it.
- The interpreted stack is the interpreter's stack, so a program that overflows
  the runtime's stack overflows while being interpreted too.
- A truncated stream is not a supported input: `in char` reads its separator
  unconditionally, so a stream that ends at an `in char` leaves the interpreted
  stack one value short.
