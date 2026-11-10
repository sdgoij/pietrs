# The AOT executable

## Shape

`codegen::emit_object` lowers the state graph exactly as the JIT does and
returns a relocatable object. The object:

- exports `piet_main(Context *)` with the C calling convention;
- imports `piet_out_char`, `piet_out_number`, `piet_in_char`,
  `piet_in_number`, and `piet_roll`;
- reads and writes only the shared `Context` prefix.

The `aot` crate writes that object next to `crates/aot/src/host.rs` (embedded
with `include_str!`) and invokes `rustc` to compile the host and link the
object into one executable. The host provides `main`, the stack allocation, and
the five functions the object imports. `pietrs` therefore needs a Rust compiler
to build an executable, not a C compiler; the C toolchain is only a developer
dependency for building the vendored npiet oracle.

The compilation is `rustc --edition=2024 -C opt-level=2 -D warnings -o <output>
piet_host.rs -C link-arg=<object>`. `-D warnings` makes a host warning a build
failure, matching the old C shim's `-Werror`.

The step limit is passed in the environment as `PIET_AOT_MAX_STEPS`; the host
reads it with `option_env!`, so the source still compiles when opened on its
own, with no limit.

## Context layout

`codegen`'s `Context` and the host's `Context` must agree byte for byte. Both
sides assert it:

- `codegen` has `the_context_layout_matches_the_host_shim`;
- `host.rs` has a `const` assertion on the size and every field offset.

The generated code loads the stack base from `Context.stack`, so the object
takes only the context pointer. The JIT wraps the same `Context` in a
`JitContext` whose first field is the context, and its host functions cast the
pointer back to recover the in-memory `BufferIo`; the AOT host reads the same
prefix from its own stack.

## Runtime behaviour

- Exit status: `0` on termination, `1` on the step limit or stack overflow.
  The step limit prints the same diagnostic as `pietrs run`.
- The step limit is baked into the executable at build time; the stack capacity
  is `1 << 20`, matching the interpreter and the JIT defaults.
- Character I/O is UTF-8; numbers are parsed and formatted by the host.
- The host is compiled as its own crate, not as a module of `aot`, so the
  workspace `unsafe_code = "forbid"` is untouched. Its `unsafe` is confined to
  the FFI boundary and the stack pointer.

## Testing

`pietrs selftest` runs every case through `interp`, the JIT, and, when a Rust
compiler is available, the linked executable. `xtask e2e` builds each case,
runs it with a timeout, and compares `expected.stdout` and `expected.exit`. The
`aot` integration tests build and run a terminating program and a step-limited
one, and name a missing compiler. The host is a third implementation of the
host semantics, so any new command must add an AOT case; differential agreement
is the only proof that the three paths stay together.
