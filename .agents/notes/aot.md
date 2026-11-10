# The AOT executable

## Shape

`codegen::emit_object` lowers the state graph exactly as the JIT does and
returns a relocatable object. The object exports `piet_main`, imports the five
host symbols, and reads and writes only the shared `Context` prefix.

`aot::build` does not link it. It reads the object, translates the allocatable
sections, symbols, and relocations into a small payload, and writes three parts
into the output file:

```
[runner bytes][payload][footer]
```

The runner is the `pietrs-runner` binary, installed next to `pietrs`. Building a
program therefore invokes no compiler and no linker: `pietrs build` reads the
sibling runner, appends the payload and footer, and writes the file.

The runner maps the payload sections into executable memory, resolves the five
host symbols to its own functions, applies the payload's relocations, and calls
`piet_main`. The payload format is deliberately smaller than COFF or ELF: the
object format is read once, when the program is built, by the `object` crate.

The footer records the payload length, the baked-in step limit, the
architecture, and the format version. The runner finds it at the end of its own
file; a runner without a footer refuses to run.

The loader supports x86-64 COFF and ELF objects. The payload encoder rejects an
unsupported relocation or architecture with a named diagnostic at build time.

## Context layout

`codegen`'s `Context` and the runner's `Context` must agree byte for byte. Both
sides assert it:

- `codegen` has `the_context_layout_matches_the_host_shim`;
- `crates/runner` has `const` assertions on the size and every field offset.

The generated code loads the stack base from `Context.stack`, so the payload
takes only the context pointer.

## Runtime behaviour

- Exit status: `0` on termination, `1` on the step limit or stack overflow.
  The step limit prints the same diagnostic as `pietrs run`.
- The step limit is baked into the footer at build time; the stack capacity is
  `1 << 20`, matching the interpreter and the JIT defaults.
- Character I/O is UTF-8; numbers are parsed and formatted by the runner.
- `unsafe` is confined to `crates/runner`: the executable-memory allocation, the
  relocation writes, and the entry call. Every other crate keeps the workspace
  `unsafe_code = "forbid"`.

## Testing

`pietrs selftest` runs every case through `interp`, the JIT, and the packaged
executable. `xtask e2e` builds each case, runs it with a timeout, and compares
`expected.stdout`, `expected.exit`, and `expected.trace`. The `aot` tests cover
the payload header, the footer, the host imports, and in-process execution of a
terminating program and a step-limited one. `xtask` finds the runner next to
itself (or in the `deps` parent) and names the missing path when it cannot. The
runner is a third implementation of the host semantics, so any new command must
add an AOT case; differential agreement is the only proof that the three paths
stay together.
