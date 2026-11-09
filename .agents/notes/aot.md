# The AOT executable

## Shape

`codegen::emit_object` lowers the state graph exactly as the JIT does and
returns a relocatable object. The object:

- exports `piet_main(Context *)` with the C calling convention;
- imports `piet_out_char`, `piet_out_number`, `piet_in_char`,
  `piet_in_number`, and `piet_roll`;
- reads and writes only the shared `Context` prefix.

The `aot` crate compiles `crates/aot/src/aot.c` with the emitted object and
links both with `cc`, `gcc`, or `clang`. The C shim provides `main`, the host
functions, and the stack allocation. No Rust static library or Cargo target
path is involved, so the link step is the same on every host with a C compiler.
`pietrs build`, `pietrs selftest`, and `xtask e2e` all call `aot::build`.

The step limit is passed as `-DPIET_AOT_MAX_STEPS=UINT64_C(n)`; the shim's
`#ifndef` default keeps the file valid C when opened on its own. The link uses
`-Wall -Wextra -Werror`, so a C warning is a build failure.

## Context layout

`codegen`'s `Context` and the C `Context` must agree byte for byte. Both sides
assert it:

- `codegen` has `the_context_layout_matches_the_c_shim`;
- `aot.c` has `_Static_assert` on the size and every field offset.

The generated code loads the stack base from `Context.stack`, so the object
takes only the context pointer. The JIT wraps the same `Context` in a
`JitContext` whose first field is the context; the host functions cast the
pointer back to recover the in-memory `BufferIo`.

## Runtime behaviour

- Exit status: `0` on termination, `1` on the step limit or stack overflow.
  The step limit prints the same diagnostic as `pietrs run`.
- The step limit is baked into the executable at build time; the stack capacity
  is `1 << 20`, matching the interpreter and the JIT defaults.
- Character I/O is UTF-8; numbers use `scanf`/`printf`.
- `unsafe` stays within `codegen`'s host functions and the JIT entry point. The
  shim is C, not Rust, so the workspace `unsafe_code = "forbid"` is untouched.

## Testing

`pietrs selftest` runs every case through `interp`, the JIT, and, when a C
compiler is available, the linked executable. `xtask e2e` builds each case,
runs it with a timeout, and compares `expected.stdout` and `expected.exit`.
The build tests cover push, input plus arithmetic, roll, UTF-8 round-tripping,
and the missing-linker diagnostic. The C shim is a third implementation of the
host semantics, so any new command must add an AOT case; differential agreement
is the only proof that the three paths stay together.
