# Division rounding

The clarified specification floors division and modulo; npiet truncates toward
zero. `runtime::Division` selects the mode:

- `Floored` (default): Python `//` and `%`, the result of modulo has the sign of
  the divisor.
- `Truncating`: C and npiet, the result rounds toward zero.

The mode is threaded through `interp::Options`, `codegen::Options`,
`codegen::emit_object`, and `aot::build`. `pietrs run`, `trace`, `build`, and
`selftest` default to `floored`; `xtask conformance` defaults to `truncating`
because its oracle does; `xtask e2e` defaults to `floored`.

Both modes handle the edge cases without trapping: division with a zero divisor
or `MIN / -1` is ignored (both operands are consumed and nothing is pushed),
modulo by zero is ignored, and `MIN % -1` is `0`.

The full list of observable differences from npiet and the gate policy is in
`.agents/notes/divergences.md`.
