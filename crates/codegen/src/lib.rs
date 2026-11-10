#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The Cranelift lowering: one native function for the reachable state graph.
//!
//! [`run`] executes it in-process through the JIT; [`emit_object`] writes a
//! relocatable object that the `aot` crate packages into a standalone
//! executable.
//! Each `(block, DP, CC)` state becomes a basic block. The value stack lives in
//! the shared context; `pointer` and `switch` choose their successor from the
//! popped value, and the remaining commands are straight-line IR. Host calls
//! handle I/O and `roll`; the stack itself is inline.

use std::collections::HashMap;
use std::mem::offset_of;

use canvas::Blocks;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, InstBuilder, MemFlagsData, Signature, Value, types,
};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
#[cfg(feature = "object")]
use cranelift_object::{ObjectBuilder, ObjectModule};
use runtime::{BufferIo, Division, Input, Output};
use tessera::{Command, Edge, State, StateGraph};

mod diagnostics;

const REASON_TERMINATED: u32 = 0;
const REASON_STEP_LIMIT: u32 = 1;
const REASON_OVERFLOW: u32 = 2;

/// How the JIT bounds a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub stack_capacity: usize,
    pub max_steps: Option<u64>,
    pub division: Division,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            stack_capacity: 1 << 20,
            max_steps: None,
            division: Division::Floored,
        }
    }
}

/// Why a JIT run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Terminated,
    StepLimit,
    StackOverflow,
}

/// What a JIT run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    pub reason: ExitReason,
    pub steps: u64,
}

/// Why the JIT could not run.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cranelift: {0}")]
    Cranelift(String),

    #[error("the compiler requires a 64-bit host")]
    UnsupportedTarget,
}

/// The ABI the generated code and the AOT host shim share.
#[repr(C)]
struct Context {
    stack: *mut i64,
    capacity: u64,
    sp: u64,
    steps: u64,
    max_steps: u64,
    reason: u32,
}

/// The JIT's context: the shared ABI first, then the in-memory I/O.
#[repr(C)]
struct JitContext {
    context: Context,
    io: *mut BufferIo,
}

struct Offsets {
    stack: i32,
    capacity: i32,
    sp: i32,
    steps: i32,
    max_steps: i32,
    reason: i32,
}

impl Offsets {
    fn new() -> Self {
        Self {
            stack: offset_of!(Context, stack) as i32,
            capacity: offset_of!(Context, capacity) as i32,
            sp: offset_of!(Context, sp) as i32,
            steps: offset_of!(Context, steps) as i32,
            max_steps: offset_of!(Context, max_steps) as i32,
            reason: offset_of!(Context, reason) as i32,
        }
    }
}

struct HostIds {
    out_char: FuncId,
    out_number: FuncId,
    in_char: FuncId,
    in_number: FuncId,
    roll: FuncId,
}

struct HostRefs {
    out_char: FuncRef,
    out_number: FuncRef,
    in_char: FuncRef,
    in_number: FuncRef,
    roll: FuncRef,
}

/// The whole-function lowering inputs that do not change between states.
struct Program<'a> {
    graph: &'a StateGraph,
    start: Option<State>,
    blocks: &'a Blocks,
    offsets: &'a Offsets,
    host_ids: &'a HostIds,
    signature: Signature,
    frontend_config: TargetFrontendConfig,
    division: Division,
}

/// The values every command emitter needs: the context pointer, the stack
/// base, the host layout, and the overflow trap.
#[derive(Clone, Copy)]
struct Frame<'a> {
    ctx: Value,
    stack: Value,
    offsets: &'a Offsets,
    overflow: Block,
}

/// The per-state lowering inputs shared by the emitters.
struct Lowering<'a> {
    state_blocks: &'a HashMap<State, Block>,
    host: &'a HostRefs,
    frame: Frame<'a>,
}

/// Runs the program through the JIT.
pub fn run(blocks: &Blocks, io: &mut BufferIo, options: &Options) -> Result<Run, Error> {
    if std::mem::size_of::<usize>() != 8 {
        return Err(Error::UnsupportedTarget);
    }

    let graph = StateGraph::build(blocks);
    let Some(start) = graph.start() else {
        return Ok(Run {
            reason: ExitReason::Terminated,
            steps: 0,
        });
    };

    let offsets = Offsets::new();
    let mut stack = vec![0i64; options.stack_capacity];
    let mut context = JitContext {
        context: Context {
            stack: stack.as_mut_ptr(),
            capacity: options.stack_capacity as u64,
            sp: 0,
            steps: 0,
            max_steps: options.max_steps.unwrap_or(u64::MAX),
            reason: REASON_TERMINATED,
        },
        io: io as *mut BufferIo,
    };

    let mut module = build_module()?;
    let signature = pointer_signature(&mut module);
    let frontend_config = module.target_config();
    let main_id = module
        .declare_function("piet_main", Linkage::Export, &signature)
        .map_err(|error| Error::Cranelift(error.to_string()))?;
    let host_ids = declare_hosts(&mut module, &signature)?;

    let program = Program {
        graph: &graph,
        start: Some(start),
        blocks,
        offsets: &offsets,
        host_ids: &host_ids,
        signature,
        frontend_config,
        division: options.division,
    };
    define_main(
        &mut module,
        main_id,
        &program,
        diagnostics::diagnostics().jit_disasm,
    )?;
    module
        .finalize_definitions()
        .map_err(|error| Error::Cranelift(error.to_string()))?;

    let code = module.get_finalized_function(main_id);
    let main: extern "C" fn(*mut Context) = unsafe { std::mem::transmute(code) };
    main(&mut context.context);

    Ok(Run {
        reason: match context.context.reason {
            REASON_STEP_LIMIT => ExitReason::StepLimit,
            REASON_OVERFLOW => ExitReason::StackOverflow,
            _ => ExitReason::Terminated,
        },
        steps: context.context.steps,
    })
}

/// Emits the program as a relocatable object exporting `piet_main` for the
/// host target.
///
/// The object calls the host symbols `piet_out_char`, `piet_out_number`,
/// `piet_in_char`, `piet_in_number`, and `piet_roll`, which the host must
/// provide.
#[cfg(feature = "object")]
pub fn emit_object(blocks: &Blocks, division: Division) -> Result<Vec<u8>, Error> {
    emit_object_with(blocks, division, None)
}

/// Emits the program as a relocatable object for `target`.
#[cfg(feature = "object")]
pub fn emit_object_for(
    blocks: &Blocks,
    division: Division,
    target: &str,
) -> Result<Vec<u8>, Error> {
    emit_object_with(blocks, division, Some(target))
}

#[cfg(feature = "object")]
fn emit_object_with(
    blocks: &Blocks,
    division: Division,
    target: Option<&str>,
) -> Result<Vec<u8>, Error> {
    if std::mem::size_of::<usize>() != 8 {
        return Err(Error::UnsupportedTarget);
    }
    let graph = StateGraph::build(blocks);
    let offsets = Offsets::new();
    let mut module = build_object_module(target)?;
    let signature = pointer_signature(&mut module);
    let frontend_config = module.target_config();
    let main_id = module
        .declare_function("piet_main", Linkage::Export, &signature)
        .map_err(|error| Error::Cranelift(error.to_string()))?;
    let host_ids = declare_hosts(&mut module, &signature)?;
    let program = Program {
        graph: &graph,
        start: graph.start(),
        blocks,
        offsets: &offsets,
        host_ids: &host_ids,
        signature,
        frontend_config,
        division,
    };
    define_main(&mut module, main_id, &program, false)?;
    module
        .finish()
        .emit()
        .map_err(|error| Error::Cranelift(error.to_string()))
}

fn declare_hosts<M: Module>(module: &mut M, signature: &Signature) -> Result<HostIds, Error> {
    Ok(HostIds {
        out_char: declare_host(module, "piet_out_char", signature)?,
        out_number: declare_host(module, "piet_out_number", signature)?,
        in_char: declare_host(module, "piet_in_char", signature)?,
        in_number: declare_host(module, "piet_in_number", signature)?,
        roll: declare_host(module, "piet_roll", signature)?,
    })
}

fn declare_host<M: Module>(
    module: &mut M,
    name: &str,
    signature: &Signature,
) -> Result<FuncId, Error> {
    module
        .declare_function(name, Linkage::Import, signature)
        .map_err(|error| Error::Cranelift(error.to_string()))
}

fn pointer_signature<M: Module>(module: &mut M) -> Signature {
    let mut signature = module.make_signature();
    signature.params.push(AbiParam::new(types::I64));
    signature
}

#[cfg(feature = "object")]
fn build_object_module(target: Option<&str>) -> Result<ObjectModule, Error> {
    let flags = settings::Flags::new(settings::builder());
    let isa = match target {
        Some(target) => cranelift_codegen::isa::lookup_by_name(target)
            .map_err(|error| Error::Cranelift(error.to_string()))?
            .finish(flags)
            .map_err(|error| Error::Cranelift(error.to_string()))?,
        None => cranelift_native::builder()
            .map_err(|error| Error::Cranelift(error.to_string()))?
            .finish(flags)
            .map_err(|error| Error::Cranelift(error.to_string()))?,
    };
    let builder = ObjectBuilder::new(isa, "pietrs", default_libcall_names())
        .map_err(|error| Error::Cranelift(error.to_string()))?;
    Ok(ObjectModule::new(builder))
}

fn build_module() -> Result<JITModule, Error> {
    let isa_builder =
        cranelift_native::builder().map_err(|error| Error::Cranelift(error.to_string()))?;
    let isa = isa_builder
        .finish(settings::Flags::new(settings::builder()))
        .map_err(|error| Error::Cranelift(error.to_string()))?;
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    builder.symbol("piet_out_char", piet_out_char as *const u8);
    builder.symbol("piet_out_number", piet_out_number as *const u8);
    builder.symbol("piet_in_char", piet_in_char as *const u8);
    builder.symbol("piet_in_number", piet_in_number as *const u8);
    builder.symbol("piet_roll", piet_roll as *const u8);
    Ok(JITModule::new(builder))
}

fn define_main<M: Module>(
    module: &mut M,
    main_id: FuncId,
    program: &Program<'_>,
    disasm: bool,
) -> Result<(), Error> {
    let mut context = module.make_context();
    context.func.signature = program.signature.clone();
    let host = HostRefs {
        out_char: module.declare_func_in_func(program.host_ids.out_char, &mut context.func),
        out_number: module.declare_func_in_func(program.host_ids.out_number, &mut context.func),
        in_char: module.declare_func_in_func(program.host_ids.in_char, &mut context.func),
        in_number: module.declare_func_in_func(program.host_ids.in_number, &mut context.func),
        roll: module.declare_func_in_func(program.host_ids.roll, &mut context.func),
    };
    let mut builder_context = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);

    let state_blocks: HashMap<State, Block> = program
        .graph
        .states()
        .iter()
        .map(|state| (*state, builder.create_block()))
        .collect();
    let overflow_block = builder.create_block();
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let params = builder.block_params(entry);
    let ctx_ptr = params[0];
    let stack_ptr = load_field(&mut builder, ctx_ptr, types::I64, program.offsets.stack);
    match program.start {
        Some(start) => {
            builder.ins().jump(state_blocks[&start], &[]);
        }
        None => {
            builder.ins().return_(&[]);
        }
    }

    let lowering = Lowering {
        state_blocks: &state_blocks,
        host: &host,
        frame: Frame {
            ctx: ctx_ptr,
            stack: stack_ptr,
            offsets: program.offsets,
            overflow: overflow_block,
        },
    };
    let frame = lowering.frame;

    for state in program.graph.states() {
        builder.switch_to_block(state_blocks[state]);

        let reason = load_field(&mut builder, frame.ctx, types::I32, frame.offsets.reason);
        let failed = builder.ins().icmp_imm_s(IntCC::NotEqual, reason, 0);
        let failed_block = builder.create_block();
        let proceed = builder.create_block();
        builder.ins().brif(failed, failed_block, &[], proceed, &[]);
        builder.switch_to_block(failed_block);
        builder.ins().return_(&[]);
        builder.switch_to_block(proceed);

        let steps = load_field(&mut builder, frame.ctx, types::I64, frame.offsets.steps);
        let max_steps = load_field(&mut builder, frame.ctx, types::I64, frame.offsets.max_steps);
        let exhausted = builder
            .ins()
            .icmp(IntCC::UnsignedGreaterThanOrEqual, steps, max_steps);
        let limit_block = builder.create_block();
        let work = builder.create_block();
        builder.ins().brif(exhausted, limit_block, &[], work, &[]);
        builder.switch_to_block(limit_block);
        let step_limit = builder
            .ins()
            .iconst(types::I32, i64::from(REASON_STEP_LIMIT));
        store_field(&mut builder, frame.ctx, step_limit, frame.offsets.reason);
        builder.ins().return_(&[]);
        builder.switch_to_block(work);

        let edges: Vec<&Edge> = program.graph.successors(*state).collect();
        if edges.is_empty() {
            builder.ins().return_(&[]);
            continue;
        }
        let block_size = program
            .blocks
            .block(state.block)
            .map_or(0, |block| block.size() as i64);
        bump_steps(&mut builder, frame.ctx, frame.offsets);
        emit_command(
            &mut builder,
            &lowering,
            &edges,
            block_size,
            program.division,
        );
    }

    builder.switch_to_block(overflow_block);
    let overflow = builder.ins().iconst(types::I32, i64::from(REASON_OVERFLOW));
    store_field(&mut builder, frame.ctx, overflow, frame.offsets.reason);
    builder.ins().return_(&[]);
    builder.seal_all_blocks();
    builder.finalize(program.frontend_config);
    context
        .verify(module.isa())
        .map_err(|error| Error::Cranelift(error.to_string()))?;
    if diagnostics::diagnostics().dump_clif {
        eprintln!("pietrs: clif");
        eprintln!("{}", context.func.display());
    }
    if disasm {
        context.set_disasm(true);
    }
    module
        .define_function(main_id, &mut context)
        .map_err(|error| Error::Cranelift(error.to_string()))?;
    if disasm {
        match context
            .compiled_code()
            .and_then(|code| code.vcode.as_deref())
        {
            Some(text) => {
                eprintln!("pietrs: jit disassembly");
                eprintln!("{text}");
            }
            None => eprintln!("pietrs: no disassembly is available for this target"),
        }
    }
    Ok(())
}

fn emit_command(
    builder: &mut FunctionBuilder,
    lowering: &Lowering<'_>,
    edges: &[&Edge],
    block_size: i64,
    division: Division,
) {
    let frame = lowering.frame;
    let successor = lowering.state_blocks[&edges[0].to];
    match edges[0].command {
        None => {
            builder.ins().jump(successor, &[]);
        }
        Some(Command::Push) => {
            let value = builder.ins().iconst(types::I64, block_size);
            emit_push(builder, frame, value);
            builder.ins().jump(successor, &[]);
        }
        Some(Command::Pop) => emit_pop(builder, frame, successor),
        Some(Command::Add) => {
            emit_binary(builder, frame, successor, |builder, a, b| {
                builder.ins().iadd(a, b)
            });
        }
        Some(Command::Subtract) => {
            emit_binary(builder, frame, successor, |builder, a, b| {
                builder.ins().isub(a, b)
            });
        }
        Some(Command::Multiply) => {
            emit_binary(builder, frame, successor, |builder, a, b| {
                builder.ins().imul(a, b)
            });
        }
        Some(Command::Divide) => emit_divmod(builder, frame, successor, true, division),
        Some(Command::Modulo) => emit_divmod(builder, frame, successor, false, division),
        Some(Command::Not) => emit_not(builder, frame, successor),
        Some(Command::Greater) => {
            emit_binary(builder, frame, successor, |builder, a, b| {
                let comparison = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
                builder.ins().uextend(types::I64, comparison)
            });
        }
        Some(Command::Pointer) => emit_pointer(builder, lowering, edges),
        Some(Command::Switch) => emit_switch(builder, lowering, edges),
        Some(Command::Duplicate) => emit_duplicate(builder, frame, successor),
        Some(Command::Roll) => {
            let _ = builder.ins().call(lowering.host.roll, &[frame.ctx]);
            builder.ins().jump(successor, &[]);
        }
        Some(Command::InNumber) => {
            let _ = builder.ins().call(lowering.host.in_number, &[frame.ctx]);
            builder.ins().jump(successor, &[]);
        }
        Some(Command::InChar) => {
            let _ = builder.ins().call(lowering.host.in_char, &[frame.ctx]);
            builder.ins().jump(successor, &[]);
        }
        Some(Command::OutNumber) => {
            let _ = builder.ins().call(lowering.host.out_number, &[frame.ctx]);
            builder.ins().jump(successor, &[]);
        }
        Some(Command::OutChar) => {
            let _ = builder.ins().call(lowering.host.out_char, &[frame.ctx]);
            builder.ins().jump(successor, &[]);
        }
    }
}

fn emit_push(builder: &mut FunctionBuilder, frame: Frame<'_>, value: Value) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let capacity = load_field(builder, frame.ctx, types::I64, frame.offsets.capacity);
    let overflow = builder
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, sp, capacity);
    let store_block = builder.create_block();
    builder
        .ins()
        .brif(overflow, frame.overflow, &[], store_block, &[]);
    builder.switch_to_block(store_block);
    let address = stack_address(builder, frame.stack, sp);
    builder.ins().store(MemFlagsData::new(), value, address, 0);
    let next = builder.ins().iadd_imm_s(sp, 1);
    store_field(builder, frame.ctx, next, frame.offsets.sp);
}

fn emit_pop(builder: &mut FunctionBuilder, frame: Frame<'_>, successor: Block) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let empty = builder.ins().icmp_imm_s(IntCC::Equal, sp, 0);
    let pop_block = builder.create_block();
    builder.ins().brif(empty, successor, &[], pop_block, &[]);
    builder.switch_to_block(pop_block);
    let next = builder.ins().iadd_imm_s(sp, -1);
    store_field(builder, frame.ctx, next, frame.offsets.sp);
    builder.ins().jump(successor, &[]);
}

fn emit_binary(
    builder: &mut FunctionBuilder,
    frame: Frame<'_>,
    successor: Block,
    operation: impl FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let short = builder.ins().icmp_imm_s(IntCC::SignedLessThan, sp, 2);
    let operation_block = builder.create_block();
    builder
        .ins()
        .brif(short, successor, &[], operation_block, &[]);
    builder.switch_to_block(operation_block);
    let second = builder.ins().iadd_imm_s(sp, -2);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let a = load_stack(builder, frame.stack, second);
    let b = load_stack(builder, frame.stack, top);
    let result = operation(builder, a, b);
    let address = stack_address(builder, frame.stack, second);
    builder.ins().store(MemFlagsData::new(), result, address, 0);
    store_field(builder, frame.ctx, top, frame.offsets.sp);
    builder.ins().jump(successor, &[]);
}

fn emit_divmod(
    builder: &mut FunctionBuilder,
    frame: Frame<'_>,
    successor: Block,
    divide: bool,
    mode: Division,
) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let short = builder.ins().icmp_imm_s(IntCC::SignedLessThan, sp, 2);
    let operation_block = builder.create_block();
    builder
        .ins()
        .brif(short, successor, &[], operation_block, &[]);
    builder.switch_to_block(operation_block);
    let second = builder.ins().iadd_imm_s(sp, -2);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let a = load_stack(builder, frame.stack, second);
    let b = load_stack(builder, frame.stack, top);
    let zero = builder.ins().icmp_imm_s(IntCC::Equal, b, 0);
    let minimum = builder.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
    let negative_one = builder.ins().icmp_imm_s(IntCC::Equal, b, -1);
    let overflow = builder.ins().band(minimum, negative_one);

    if divide {
        // `MIN / -1` overflows and a zero divisor has no quotient. In both cases
        // the command consumes its operands and pushes nothing, like the
        // interpreter's `Division::divide` returning `None`.
        let skip = builder.ins().bor(zero, overflow);
        let consume = builder.create_block();
        let compute = builder.create_block();
        builder.ins().brif(skip, consume, &[], compute, &[]);
        builder.switch_to_block(compute);

        let quotient = builder.ins().sdiv(a, b);
        let result = if mode == Division::Floored {
            let remainder = builder.ins().srem(a, b);
            let adjust = adjust_floored(builder, remainder, b);
            let adjusted = builder.ins().iadd_imm_s(quotient, -1);
            builder.ins().select(adjust, adjusted, quotient)
        } else {
            quotient
        };
        store_binary_result(builder, frame, second, top, result);
        builder.ins().jump(successor, &[]);

        builder.switch_to_block(consume);
        store_field(builder, frame.ctx, second, frame.offsets.sp);
        builder.ins().jump(successor, &[]);
        return;
    }

    // A zero divisor consumes both operands and pushes nothing. `MIN % -1`
    // never traps: it is defined as zero, and `srem` must not run for it.
    let nonzero_block = builder.create_block();
    let consume = builder.create_block();
    builder.ins().brif(zero, consume, &[], nonzero_block, &[]);
    builder.switch_to_block(nonzero_block);
    let compute = builder.create_block();
    let zero_result = builder.create_block();
    builder.ins().brif(overflow, zero_result, &[], compute, &[]);

    builder.switch_to_block(compute);
    let result = if mode == Division::Floored {
        let remainder = builder.ins().srem(a, b);
        let adjust = adjust_floored(builder, remainder, b);
        let adjusted = builder.ins().iadd(remainder, b);
        builder.ins().select(adjust, adjusted, remainder)
    } else {
        builder.ins().srem(a, b)
    };
    store_binary_result(builder, frame, second, top, result);
    builder.ins().jump(successor, &[]);

    builder.switch_to_block(zero_result);
    let zero_value = builder.ins().iconst(types::I64, 0);
    store_binary_result(builder, frame, second, top, zero_value);
    builder.ins().jump(successor, &[]);

    builder.switch_to_block(consume);
    store_field(builder, frame.ctx, second, frame.offsets.sp);
    builder.ins().jump(successor, &[]);
}

/// Whether a truncated remainder needs the floored adjustment: it is non-zero
/// and its sign differs from the divisor's.
fn adjust_floored(builder: &mut FunctionBuilder, remainder: Value, divisor: Value) -> Value {
    let nonzero = builder.ins().icmp_imm_s(IntCC::NotEqual, remainder, 0);
    let remainder_negative = builder
        .ins()
        .icmp_imm_s(IntCC::SignedLessThan, remainder, 0);
    let divisor_negative = builder.ins().icmp_imm_s(IntCC::SignedLessThan, divisor, 0);
    let signs_differ = builder.ins().bxor(remainder_negative, divisor_negative);
    builder.ins().band(nonzero, signs_differ)
}

fn store_binary_result(
    builder: &mut FunctionBuilder,
    frame: Frame<'_>,
    second: Value,
    top: Value,
    result: Value,
) {
    let address = stack_address(builder, frame.stack, second);
    builder.ins().store(MemFlagsData::new(), result, address, 0);
    store_field(builder, frame.ctx, top, frame.offsets.sp);
}

fn emit_not(builder: &mut FunctionBuilder, frame: Frame<'_>, successor: Block) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let empty = builder.ins().icmp_imm_s(IntCC::Equal, sp, 0);
    let operation_block = builder.create_block();
    builder
        .ins()
        .brif(empty, successor, &[], operation_block, &[]);
    builder.switch_to_block(operation_block);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let value = load_stack(builder, frame.stack, top);
    let is_zero = builder.ins().icmp_imm_s(IntCC::Equal, value, 0);
    let one = builder.ins().iconst(types::I64, 1);
    let zero = builder.ins().iconst(types::I64, 0);
    let result = builder.ins().select(is_zero, one, zero);
    let address = stack_address(builder, frame.stack, top);
    builder.ins().store(MemFlagsData::new(), result, address, 0);
    builder.ins().jump(successor, &[]);
}

fn emit_duplicate(builder: &mut FunctionBuilder, frame: Frame<'_>, successor: Block) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let empty = builder.ins().icmp_imm_s(IntCC::Equal, sp, 0);
    let duplicate_block = builder.create_block();
    builder
        .ins()
        .brif(empty, successor, &[], duplicate_block, &[]);
    builder.switch_to_block(duplicate_block);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let value = load_stack(builder, frame.stack, top);
    emit_push(builder, frame, value);
    builder.ins().jump(successor, &[]);
}

fn emit_pointer(builder: &mut FunctionBuilder, lowering: &Lowering<'_>, edges: &[&Edge]) {
    let frame = lowering.frame;
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let present = builder.ins().icmp_imm_s(IntCC::NotEqual, sp, 0);
    let pop_block = builder.create_block();
    let empty_block = builder.create_block();
    builder
        .ins()
        .brif(present, pop_block, &[], empty_block, &[]);
    builder.switch_to_block(pop_block);
    let top = builder.ins().iadd_imm_s(sp, -1);
    store_field(builder, frame.ctx, top, frame.offsets.sp);
    let value = load_stack(builder, frame.stack, top);
    let index = rem_euclid(builder, value, 4);
    emit_branch_by_index(builder, index, edges, lowering.state_blocks);
    builder.switch_to_block(empty_block);
    builder.ins().jump(lowering.state_blocks[&edges[0].to], &[]);
}

fn emit_switch(builder: &mut FunctionBuilder, lowering: &Lowering<'_>, edges: &[&Edge]) {
    let frame = lowering.frame;
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let present = builder.ins().icmp_imm_s(IntCC::NotEqual, sp, 0);
    let pop_block = builder.create_block();
    let empty_block = builder.create_block();
    builder
        .ins()
        .brif(present, pop_block, &[], empty_block, &[]);
    builder.switch_to_block(pop_block);
    let top = builder.ins().iadd_imm_s(sp, -1);
    store_field(builder, frame.ctx, top, frame.offsets.sp);
    let value = load_stack(builder, frame.stack, top);
    let bit = builder.ins().band_imm_u(value, 1);
    let index = builder.ins().ireduce(types::I32, bit);
    emit_branch_by_index(builder, index, edges, lowering.state_blocks);
    builder.switch_to_block(empty_block);
    builder.ins().jump(lowering.state_blocks[&edges[0].to], &[]);
}

fn emit_branch_by_index(
    builder: &mut FunctionBuilder,
    index: Value,
    edges: &[&Edge],
    state_blocks: &HashMap<State, Block>,
) {
    for (position, edge) in edges.iter().enumerate() {
        let target = state_blocks[&edge.to];
        if position + 1 == edges.len() {
            builder.ins().jump(target, &[]);
            return;
        }
        let next = builder.create_block();
        let matches = builder
            .ins()
            .icmp_imm_s(IntCC::Equal, index, position as i64);
        builder.ins().brif(matches, target, &[], next, &[]);
        builder.switch_to_block(next);
    }
}

fn rem_euclid(builder: &mut FunctionBuilder, value: Value, modulus: i64) -> Value {
    let remainder = builder.ins().srem_imm_s(value, modulus);
    let negative = builder
        .ins()
        .icmp_imm_s(IntCC::SignedLessThan, remainder, 0);
    let positive = builder.ins().iadd_imm_s(remainder, modulus);
    let adjusted = builder.ins().select(negative, positive, remainder);
    builder.ins().ireduce(types::I32, adjusted)
}

fn bump_steps(builder: &mut FunctionBuilder, ctx: Value, offsets: &Offsets) {
    let steps = load_field(builder, ctx, types::I64, offsets.steps);
    let next = builder.ins().iadd_imm_s(steps, 1);
    store_field(builder, ctx, next, offsets.steps);
}

fn stack_address(builder: &mut FunctionBuilder, stack: Value, index: Value) -> Value {
    let offset = builder.ins().ishl_imm_s(index, 3);
    builder.ins().iadd(stack, offset)
}

fn load_stack(builder: &mut FunctionBuilder, stack: Value, index: Value) -> Value {
    let address = stack_address(builder, stack, index);
    builder
        .ins()
        .load(types::I64, MemFlagsData::new(), address, 0)
}

fn load_field(
    builder: &mut FunctionBuilder,
    base: Value,
    ty: cranelift_codegen::ir::Type,
    offset: i32,
) -> Value {
    builder.ins().load(ty, MemFlagsData::new(), base, offset)
}

fn store_field(builder: &mut FunctionBuilder, base: Value, value: Value, offset: i32) {
    builder
        .ins()
        .store(MemFlagsData::new(), value, base, offset);
}

fn push(ctx: &mut JitContext, value: i64) {
    if ctx.context.sp < ctx.context.capacity {
        // SAFETY: the context's stack points at a buffer of `capacity` values.
        unsafe { *ctx.context.stack.add(ctx.context.sp as usize) = value };
        ctx.context.sp += 1;
    } else {
        ctx.context.reason = REASON_OVERFLOW;
    }
}

fn pop(ctx: &mut JitContext) -> Option<i64> {
    if ctx.context.sp == 0 {
        return None;
    }
    ctx.context.sp -= 1;
    // SAFETY: the context's stack points at a buffer of `capacity` values.
    Some(unsafe { *ctx.context.stack.add(ctx.context.sp as usize) })
}

fn pop_two(ctx: &mut JitContext) -> Option<(i64, i64)> {
    if ctx.context.sp < 2 {
        return None;
    }
    let top = pop(ctx)?;
    let second = pop(ctx)?;
    Some((second, top))
}

fn roll(ctx: &mut JitContext, depth: i64, rolls: i64) {
    if depth < 0 {
        return;
    }
    let Ok(depth) = usize::try_from(depth) else {
        return;
    };
    if depth > ctx.context.sp as usize || depth <= 1 {
        return;
    }
    let start = ctx.context.sp as usize - depth;
    let shift = rolls.rem_euclid(depth as i64) as usize;
    // SAFETY: the context's stack points at a buffer of `capacity` values and
    // `start..sp` is within it.
    let values = unsafe { std::slice::from_raw_parts_mut(ctx.context.stack.add(start), depth) };
    values.rotate_right(shift);
}

extern "C" fn piet_out_char(context: *mut Context) {
    // SAFETY: `context` points at the first field of the `JitContext` built by
    // `run`; the generated code only sees the shared prefix.
    let ctx = unsafe { &mut *(context as *mut JitContext) };
    if let Some(value) = pop(ctx)
        && let Ok(code) = u32::try_from(value)
        && let Some(character) = char::from_u32(code)
    {
        let io = unsafe { &mut *ctx.io };
        io.write_char(character);
    }
}

extern "C" fn piet_out_number(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext) };
    if let Some(value) = pop(ctx) {
        let io = unsafe { &mut *ctx.io };
        io.write_number(value);
    }
}

extern "C" fn piet_in_char(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext) };
    let character = {
        let io = unsafe { &mut *ctx.io };
        io.read_char()
    };
    if let Some(character) = character {
        push(ctx, i64::from(character as u32));
    }
}

extern "C" fn piet_in_number(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext) };
    let value = {
        let io = unsafe { &mut *ctx.io };
        io.read_number()
    };
    if let Some(value) = value {
        push(ctx, value);
    }
}

extern "C" fn piet_roll(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext) };
    if let Some((depth, rolls)) = pop_two(ctx) {
        roll(ctx, depth, rolls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(capacity: usize) -> (Vec<i64>, JitContext) {
        let mut stack = vec![0i64; capacity];
        let ctx = JitContext {
            context: Context {
                stack: stack.as_mut_ptr(),
                capacity: capacity as u64,
                sp: 0,
                steps: 0,
                max_steps: u64::MAX,
                reason: REASON_TERMINATED,
            },
            io: std::ptr::null_mut(),
        };
        (stack, ctx)
    }

    fn filled(values: &[i64]) -> (Vec<i64>, JitContext) {
        let (stack, mut ctx) = context(values.len() + 4);
        for &value in values {
            push(&mut ctx, value);
        }
        (stack, ctx)
    }

    fn slice(ctx: &JitContext) -> &[i64] {
        // SAFETY: the context's stack points at a buffer of `capacity` values
        // and `sp` never exceeds it.
        unsafe { std::slice::from_raw_parts(ctx.context.stack, ctx.context.sp as usize) }
    }

    #[test]
    fn push_and_pop_walk_the_stack() {
        let (_stack, mut ctx) = context(2);
        assert_eq!(pop(&mut ctx), None);
        push(&mut ctx, 1);
        push(&mut ctx, 2);
        assert_eq!(slice(&ctx), [1, 2]);
        assert_eq!(pop(&mut ctx), Some(2));
        assert_eq!(pop(&mut ctx), Some(1));
        assert_eq!(pop(&mut ctx), None);
    }

    #[test]
    fn a_full_stack_reports_overflow() {
        let (_stack, mut ctx) = context(1);
        push(&mut ctx, 1);
        push(&mut ctx, 2);
        assert_eq!(ctx.context.sp, 1);
        assert_eq!(ctx.context.reason, REASON_OVERFLOW);
    }

    #[test]
    fn pop_two_returns_second_then_top() {
        let (_stack, mut ctx) = filled(&[1, 2, 3]);
        assert_eq!(pop_two(&mut ctx), Some((2, 3)));
        assert_eq!(slice(&ctx), [1]);
        assert_eq!(pop_two(&mut ctx), None);
    }

    #[test]
    fn roll_matches_the_runtime_stack() {
        let (_stack, mut ctx) = filled(&[1, 2, 3]);
        roll(&mut ctx, 3, 1);
        assert_eq!(slice(&ctx), [3, 1, 2]);

        let (_stack, mut ctx) = filled(&[1, 2, 3]);
        roll(&mut ctx, 3, -1);
        assert_eq!(slice(&ctx), [2, 3, 1]);

        let (_stack, mut ctx) = filled(&[1, 2, 3]);
        roll(&mut ctx, 3, 4);
        assert_eq!(slice(&ctx), [3, 1, 2]);
    }

    #[test]
    fn an_invalid_roll_leaves_the_stack_alone() {
        for depth in [-1, 0, 1, 4] {
            let (_stack, mut ctx) = filled(&[1, 2, 3]);
            roll(&mut ctx, depth, 1);
            assert_eq!(slice(&ctx), [1, 2, 3], "depth {depth}");
        }
    }

    #[test]
    fn the_context_layout_matches_the_host_shim() {
        assert_eq!(std::mem::size_of::<Context>(), 48);
        assert_eq!(offset_of!(Context, stack), 0);
        assert_eq!(offset_of!(Context, capacity), 8);
        assert_eq!(offset_of!(Context, sp), 16);
        assert_eq!(offset_of!(Context, steps), 24);
        assert_eq!(offset_of!(Context, max_steps), 32);
        assert_eq!(offset_of!(Context, reason), 40);
    }

    #[test]
    #[cfg(feature = "object")]
    fn the_object_is_not_empty() {
        let blocks = canvas::Blocks::partition(
            canvas::CodelGrid::from_colors(
                1,
                1,
                vec![canvas::Color::chromatic(
                    canvas::Hue::Red,
                    canvas::Lightness::Normal,
                )],
            )
            .unwrap(),
        );
        let object = emit_object(&blocks, Division::Floored).unwrap();
        assert!(!object.is_empty());
    }
}
