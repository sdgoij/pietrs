# Divergences from npiet and the clarified specification

`pietrs` targets the clarified 2018 Piet specification
(`dangermouse.net/esoteric/piet.html`). npiet v1.3h is the de-facto oracle and
`xtask conformance` compares against it, but npiet is not the specification.
This note records every place where an embedder or a user can observe a
difference. Gate procedure and corpus policy live in
`.agents/notes/npiet-oracle.md`; the rounding design lives in
`.agents/notes/division.md`.

The specification is the authority where the two disagree. A conformance
golden is never re-baselined to hide one of the differences below.

## Division and modulo rounding

The clarified specification floors division and modulo (Python `//` and `%`);
npiet truncates toward zero. `pietrs` floors by default. `--division
truncating` selects npiet's rounding on `run`, `trace`, `build`, and
`selftest`. `xtask conformance` defaults to `truncating` because its oracle
does; `xtask e2e` and `selftest` default to `floored`.

Machine-checked by `runtime` unit tests, `interp`/`codegen` differential tests,
and a CLI test that runs both modes through the JIT and a linked executable.

## Zero divisors and `MIN / -1`

Both modes treat an impossible divide or modulo as an ignored command: the two
operands are consumed and nothing is pushed. `MIN / -1` is ignored and
`MIN % -1` is `0`, so neither traps. npiet instead leaves `99999999` on the
stack after a zero divisor.

## Integer width

npiet stores values in C `long`, which is 32-bit on Windows and 64-bit on LP64
hosts. `pietrs` uses `i64` on every target. Arithmetic wraps; there is no
arbitrary precision.

## Character I/O

npiet treats a character as a byte (`& 0xff` on output, `% 0xff` on input).
`pietrs` treats it as a Unicode codepoint encoded as UTF-8. ASCII is
identical; anything above U+00FF is not.

## Stack capacity

npiet grows its stack dynamically. `pietrs` has a fixed capacity (`1 << 20` by
default) and reports overflow as an explicit error on every path.

## Unknown colours

npiet maps unknown colours to white by default (`-ub` and `-uu` change that).
`pietrs` always maps a non-standard colour to white, following the
specification's suggestion; there is no error or black mode.

Every command that loads an image warns on stderr when it used one, naming each
colour and its codel count, because the mapping silently changes what the
program does. An image whose palette was mangled — re-encoded through a colour
quantiser, for instance — otherwise looks like a program that loops for ever.

## Codel-size detection

`pietrs` picks the largest uniform tile and accepts `--codel-size`. npiet
guesses from the shortest run of identical pixels. On an ambiguous image the
detected size can differ; pass `--codel-size` to `pietrs` and `-cs` to npiet
(the conformance gate pins both to the codel grid it decoded).

## Input prompt

npiet prints `? ` before reading unless `-q` is given. `pietrs` never prompts;
the conformance gate passes `-q`.

## Step limit and exit status

`pietrs run` reports a step limit on stderr and exits `1`; the linked
executable does the same. npiet prints its own message and exits `0`. The
conformance gate treats the message as the limit signal and does not compare
npiet's exit status in that case.

## White-slide legacy modes

npiet keeps `-v1`, `-v11`, and `-dpbug` for older interpreters. `pietrs`
implements only the clarified 2018 white-slide behaviour.
