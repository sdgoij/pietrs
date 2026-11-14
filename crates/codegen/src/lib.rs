#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The Cranelift lowering: native functions for the reachable state graph.
//!
//! A program whose state graph is small enough is one native function with a
//! basic block per `(block, DP, CC)` state: [`run`] executes it in-process through
//! the JIT, and [`emit_object`] writes it as a relocatable object. Above
//! `WHOLE_FUNCTION_LIMIT` one function stops being practical — its layout and
//! register allocation grow faster than the state count — so [`run`] switches to
//! lazy lowering: each state's function is compiled the first time the driver
//! enters it. The composed machine image (1.2M states, 409k visited) is the
//! motivating case; the CLI still prefers the interpreter there, because lazy
//! lowering wins on reachability but not on time (see `.agents/notes/aot.md`).
//! For packaging, [`graph_payload`] writes the graph as data for the AOT runner
//! instead, which is practical at every size.
//!
//! In both shapes the value stack lives in the shared context; `pointer` and
//! `switch` choose their successor from the popped value, and the remaining
//! commands are straight-line IR. Host calls handle I/O and `roll`; the stack
//! itself is inline.

use std::collections::HashMap;
use std::mem::offset_of;

use canvas::{BlockId, Blocks};
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, InstBuilder, MemFlagsData, Signature, Value, types,
};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings;
use cranelift_codegen::settings::Configurable;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module, default_libcall_names};
#[cfg(feature = "object")]
use cranelift_object::{ObjectBuilder, ObjectModule};
use runtime::{Division, Io};
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
struct JitContext<'io> {
    context: Context,
    io: *mut (dyn Io + 'io),
}

/// One compiled state: it runs the state's command and returns the successor's
/// index, or `-1` when the run stops.
type StateFunction = extern "C" fn(*mut Context) -> i64;

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

/// Where a finished state's code goes.
#[derive(Clone, Copy)]
enum Leave {
    /// Jump to another basic block of the same function.
    Block(Block),
    /// Return the successor's index: the driver compiles that state if it has not
    /// been compiled yet.
    Index(u32),
}

/// How a state names its successor, which is the one thing the two emitters do
/// differently.
enum Successors<'a> {
    /// One function, one Cranelift block per state.
    Blocks(&'a HashMap<State, Block>),
    /// One function per state, compiled the first time it runs.
    Indices(&'a HashMap<State, u32>),
}

impl Successors<'_> {
    fn leave(&self, state: State) -> Leave {
        match self {
            Self::Blocks(blocks) => Leave::Block(blocks[&state]),
            Self::Indices(indices) => Leave::Index(indices[&state]),
        }
    }
}

/// The per-state lowering inputs shared by the emitters.
struct Lowering<'a> {
    successors: Successors<'a>,
    host: &'a HostRefs,
    frame: Frame<'a>,
}

/// Above this many states one function with a block per state stops being a
/// good lowering: its layout and register allocation grow faster than the state
/// count, and a 62 000-state image already takes about four seconds. Past the
/// bound [`run`] compiles a state's function the first time it is entered.
///
/// The CLI intercepts before this: it runs the interpreter above the same count,
/// because on the composed machine image the interpreter (44 s) beats lazy
/// lowering (about seven minutes). The lazy path is still the JIT's answer for an
/// image the whole-function lowering cannot compile at all.
const WHOLE_FUNCTION_LIMIT: usize = 100_000;

/// Runs the program through the JIT.
pub fn run(blocks: &Blocks, io: &mut dyn Io, options: &Options) -> Result<Run, Error> {
    run_with_limit(blocks, io, options, WHOLE_FUNCTION_LIMIT)
}

/// [`run`], with the whole-function bound exposed so the lazy driver can be
/// exercised on a small graph.
fn run_with_limit<'io>(
    blocks: &Blocks,
    io: &'io mut dyn Io,
    options: &Options,
    whole_function_limit: usize,
) -> Result<Run, Error> {
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
        io: io as *mut (dyn Io + 'io),
    };

    let mut module = build_module()?;
    let signature = pointer_signature(&mut module);
    let frontend_config = module.target_config();
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
    let disasm = diagnostics::diagnostics().jit_disasm;

    if graph.states().len() <= whole_function_limit {
        let main_id = module
            .declare_function("piet_main", Linkage::Export, &program.signature)
            .map_err(|error| Error::Cranelift(error.to_string()))?;
        define_main(&mut module, main_id, &program, disasm)?;
        module
            .finalize_definitions()
            .map_err(|error| Error::Cranelift(error.to_string()))?;
        let code = module.get_finalized_function(main_id);
        // SAFETY: `define_main` compiled the signature `pointer_signature` declared.
        let main: extern "C" fn(*mut Context) = unsafe { std::mem::transmute(code) };
        main(&mut context.context);
    } else {
        run_lazy(&mut module, &program, start, &mut context.context, disasm)?;
    }

    Ok(Run {
        reason: match context.context.reason {
            REASON_STEP_LIMIT => ExitReason::StepLimit,
            REASON_OVERFLOW => ExitReason::StackOverflow,
            _ => ExitReason::Terminated,
        },
        steps: context.context.steps,
    })
}

/// Runs a graph too large for one function: each state is compiled the first
/// time the driver enters it.
fn run_lazy(
    module: &mut JITModule,
    program: &Program<'_>,
    start: State,
    context: &mut Context,
    disasm: bool,
) -> Result<(), Error> {
    let states = program.graph.states();
    let indices: HashMap<State, u32> = states
        .iter()
        .enumerate()
        .map(|(index, state)| (*state, index as u32))
        .collect();
    let mut state_signature = module.make_signature();
    state_signature.params.push(AbiParam::new(types::I64));
    state_signature.returns.push(AbiParam::new(types::I64));
    let mut functions: Vec<Option<StateFunction>> = vec![None; states.len()];
    let mut current = indices[&start] as usize;
    loop {
        let function = match functions[current] {
            Some(function) => function,
            None => {
                let id = module
                    .declare_function(
                        &format!("piet_state_{current}"),
                        Linkage::Local,
                        &state_signature,
                    )
                    .map_err(|error| Error::Cranelift(error.to_string()))?;
                define_state(
                    module,
                    id,
                    program,
                    &state_signature,
                    states[current],
                    &indices,
                    disasm,
                )?;
                module
                    .finalize_definitions()
                    .map_err(|error| Error::Cranelift(error.to_string()))?;
                let code = module.get_finalized_function(id);
                // SAFETY: `define_state` compiled this function with this signature.
                let function: StateFunction = unsafe { std::mem::transmute(code) };
                functions[current] = Some(function);
                function
            }
        };
        let next = function(context);
        let Ok(next) = usize::try_from(next) else {
            break;
        };
        current = next;
    }
    Ok(())
}

/// The magic the AOT runner recognises as a state-graph payload.
const GRAPH_MAGIC: [u8; 16] = *b"PIETRS-GRAPH\0\0\0\0";
/// The graph payload's format version.
const GRAPH_VERSION: u32 = 1;

/// The graph payload, if the image is too large to compile: `None` means the
/// caller should emit an object instead.
///
/// Above [`WHOLE_FUNCTION_LIMIT`] the object is not practical (one block per
/// state in one function), but the graph itself is small enough to carry as
/// data: about fifteen megabytes for the composed machine image, against an
/// object that would not finish emitting. The AOT runner interprets it.
pub fn graph_payload(blocks: &Blocks, division: Division) -> Option<Vec<u8>> {
    let graph = StateGraph::build(blocks);
    if graph.states().len() <= WHOLE_FUNCTION_LIMIT {
        return None;
    }
    Some(encode_graph(blocks, &graph, division))
}

/// Encodes the reachable state graph as the data the AOT runner interprets.
///
/// One block-size table (so `push` can push the block it left), then one record
/// per state: its block, its command, and the state indices of its successors.
/// A `pointer` state has four successors, indexed by the popped value modulo
/// four; a `switch` state has two, indexed by its parity; every other state has
/// one, or none when it terminates. The layout and [`command_code`]'s numbers
/// are repeated in `crates/runner/src/loader.rs`.
pub fn encode_graph(blocks: &Blocks, graph: &StateGraph, division: Division) -> Vec<u8> {
    let states = graph.states();
    let index: HashMap<State, u32> = states
        .iter()
        .enumerate()
        .map(|(position, state)| (*state, position as u32))
        .collect();

    // `StateGraph::successors` filters every edge per state, which is quadratic
    // for a graph this size. Group the edges once instead: one count, then one
    // flat target list, with the command of each state's first edge.
    let mut counts = vec![0u32; states.len()];
    for edge in graph.edges() {
        counts[index[&edge.from] as usize] += 1;
    }
    let mut offsets = vec![0u32; states.len() + 1];
    for (position, count) in counts.iter().enumerate() {
        offsets[position + 1] = offsets[position] + count;
    }
    let mut targets = vec![0u32; graph.edges().len()];
    let mut commands = vec![0u8; states.len()];
    let mut cursor = offsets.clone();
    for edge in graph.edges() {
        let from = index[&edge.from] as usize;
        targets[cursor[from] as usize] = index[&edge.to];
        cursor[from] += 1;
        commands[from] = command_code(edge.command);
    }

    let mut bytes =
        Vec::with_capacity(36 + blocks.len() * 4 + states.len() * 8 + targets.len() * 4);
    bytes.extend_from_slice(&GRAPH_MAGIC);
    bytes.extend_from_slice(&GRAPH_VERSION.to_le_bytes());
    let mode = match division {
        Division::Floored => 0u32,
        Division::Truncating => 1u32,
    };
    bytes.extend_from_slice(&mode.to_le_bytes());
    let start = graph.start().map_or(u32::MAX, |state| index[&state]);
    bytes.extend_from_slice(&start.to_le_bytes());
    bytes.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    for id in 0..blocks.len() as u32 {
        let size = blocks
            .block(BlockId(id))
            .map_or(0, |block| block.size() as u32);
        bytes.extend_from_slice(&size.to_le_bytes());
    }
    bytes.extend_from_slice(&(states.len() as u32).to_le_bytes());
    for (position, state) in states.iter().enumerate() {
        bytes.extend_from_slice(&state.block.0.to_le_bytes());
        bytes.push(commands[position]);
        let from = offsets[position] as usize;
        let to = offsets[position + 1] as usize;
        bytes.push((to - from) as u8);
        for target in &targets[from..to] {
            bytes.extend_from_slice(&target.to_le_bytes());
        }
    }
    bytes
}

/// The payload's command numbers, which the runner repeats in `loader.rs`.
fn command_code(command: Option<Command>) -> u8 {
    match command {
        None => 0,
        Some(Command::Push) => 1,
        Some(Command::Pop) => 2,
        Some(Command::Add) => 3,
        Some(Command::Subtract) => 4,
        Some(Command::Multiply) => 5,
        Some(Command::Divide) => 6,
        Some(Command::Modulo) => 7,
        Some(Command::Not) => 8,
        Some(Command::Greater) => 9,
        Some(Command::Duplicate) => 10,
        Some(Command::Roll) => 11,
        Some(Command::InNumber) => 12,
        Some(Command::InChar) => 13,
        Some(Command::OutNumber) => 14,
        Some(Command::OutChar) => 15,
        Some(Command::Pointer) => 16,
        Some(Command::Switch) => 17,
    }
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

/// The Cranelift settings both emitters compile with.
///
/// Cranelift verifies a function again inside `define_function`, but
/// [`define_main`] already runs the same verification on the finished function
/// before either path compiles it, so the second pass can only cost time: for
/// an image with 62 000 states it is about a second and a half of a five second
/// JIT startup. Everything else stays at the defaults, including unwind
/// information, which a panic in a host function needs to unwind through the
/// generated frames.
fn isa_flags() -> settings::Flags {
    let mut flags = settings::builder();
    // `set` fails only for a setting that does not exist, which a Cranelift
    // bump could cause; keeping the default verifier is the safe direction.
    let _ = flags.set("enable_verifier", "false");
    settings::Flags::new(flags)
}

#[cfg(feature = "object")]
fn build_object_module(target: Option<&str>) -> Result<ObjectModule, Error> {
    let flags = isa_flags();
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
        .finish(isa_flags())
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
        successors: Successors::Blocks(&state_blocks),
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

/// The value that tells the driver the run stopped.
fn stop(builder: &mut FunctionBuilder) -> Value {
    builder.ins().iconst(types::I64, -1)
}

/// One state's function: check the run's reason and budget, run the command, and
/// return the successor's index.
///
/// The driver compiles a state the first time it enters it. An image whose static
/// state graph is enormous but whose run touches a fraction of it therefore pays
/// only for the states it reaches, which is what makes the composed machine
/// image executable through the JIT at all.
fn define_state<M: Module>(
    module: &mut M,
    func_id: FuncId,
    program: &Program<'_>,
    signature: &Signature,
    state: State,
    indices: &HashMap<State, u32>,
    disasm: bool,
) -> Result<(), Error> {
    let mut context = module.make_context();
    context.func.signature = signature.clone();
    let host = HostRefs {
        out_char: module.declare_func_in_func(program.host_ids.out_char, &mut context.func),
        out_number: module.declare_func_in_func(program.host_ids.out_number, &mut context.func),
        in_char: module.declare_func_in_func(program.host_ids.in_char, &mut context.func),
        in_number: module.declare_func_in_func(program.host_ids.in_number, &mut context.func),
        roll: module.declare_func_in_func(program.host_ids.roll, &mut context.func),
    };
    let mut builder_context = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);

    let overflow_block = builder.create_block();
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let params = builder.block_params(entry);
    let ctx_ptr = params[0];
    let stack_ptr = load_field(&mut builder, ctx_ptr, types::I64, program.offsets.stack);

    // A failed run stops before the next command, exactly as the whole-function
    // lowering stops at the start of the next state.
    let reason = load_field(&mut builder, ctx_ptr, types::I32, program.offsets.reason);
    let failed = builder.ins().icmp_imm_s(IntCC::NotEqual, reason, 0);
    let failed_block = builder.create_block();
    let proceed = builder.create_block();
    builder.ins().brif(failed, failed_block, &[], proceed, &[]);
    builder.switch_to_block(failed_block);
    let value = stop(&mut builder);
    builder.ins().return_(&[value]);
    builder.switch_to_block(proceed);

    let steps = load_field(&mut builder, ctx_ptr, types::I64, program.offsets.steps);
    let max_steps = load_field(&mut builder, ctx_ptr, types::I64, program.offsets.max_steps);
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
    store_field(&mut builder, ctx_ptr, step_limit, program.offsets.reason);
    let value = stop(&mut builder);
    builder.ins().return_(&[value]);
    builder.switch_to_block(work);

    let edges: Vec<&Edge> = program.graph.successors(state).collect();
    if edges.is_empty() {
        let value = stop(&mut builder);
        builder.ins().return_(&[value]);
    } else {
        let block_size = program
            .blocks
            .block(state.block)
            .map_or(0, |block| block.size() as i64);
        bump_steps(&mut builder, ctx_ptr, program.offsets);
        let lowering = Lowering {
            successors: Successors::Indices(indices),
            host: &host,
            frame: Frame {
                ctx: ctx_ptr,
                stack: stack_ptr,
                offsets: program.offsets,
                overflow: overflow_block,
            },
        };
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
    store_field(&mut builder, ctx_ptr, overflow, program.offsets.reason);
    let value = stop(&mut builder);
    builder.ins().return_(&[value]);

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
        .define_function(func_id, &mut context)
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
    let next = lowering.successors.leave(edges[0].to);
    match edges[0].command {
        None => {
            leave(builder, next);
        }
        Some(Command::Push) => {
            let value = builder.ins().iconst(types::I64, block_size);
            emit_push(builder, frame, value);
            leave(builder, next);
        }
        Some(Command::Pop) => emit_pop(builder, frame, next),
        Some(Command::Add) => {
            emit_binary(builder, frame, next, |builder, a, b| {
                builder.ins().iadd(a, b)
            });
        }
        Some(Command::Subtract) => {
            emit_binary(builder, frame, next, |builder, a, b| {
                builder.ins().isub(a, b)
            });
        }
        Some(Command::Multiply) => {
            emit_binary(builder, frame, next, |builder, a, b| {
                builder.ins().imul(a, b)
            });
        }
        Some(Command::Divide) => emit_divmod(builder, frame, next, true, division),
        Some(Command::Modulo) => emit_divmod(builder, frame, next, false, division),
        Some(Command::Not) => emit_not(builder, frame, next),
        Some(Command::Greater) => {
            emit_binary(builder, frame, next, |builder, a, b| {
                let comparison = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
                builder.ins().uextend(types::I64, comparison)
            });
        }
        Some(Command::Pointer) => emit_pointer(builder, lowering, edges),
        Some(Command::Switch) => emit_switch(builder, lowering, edges),
        Some(Command::Duplicate) => emit_duplicate(builder, frame, next),
        Some(Command::Roll) => {
            let _ = builder.ins().call(lowering.host.roll, &[frame.ctx]);
            leave(builder, next);
        }
        Some(Command::InNumber) => {
            let _ = builder.ins().call(lowering.host.in_number, &[frame.ctx]);
            leave(builder, next);
        }
        Some(Command::InChar) => {
            let _ = builder.ins().call(lowering.host.in_char, &[frame.ctx]);
            leave(builder, next);
        }
        Some(Command::OutNumber) => {
            let _ = builder.ins().call(lowering.host.out_number, &[frame.ctx]);
            leave(builder, next);
        }
        Some(Command::OutChar) => {
            let _ = builder.ins().call(lowering.host.out_char, &[frame.ctx]);
            leave(builder, next);
        }
    }
}

/// Finish a state: jump inside the function, or return the successor's index.
fn leave(builder: &mut FunctionBuilder, target: Leave) {
    match target {
        Leave::Block(block) => {
            builder.ins().jump(block, &[]);
        }
        Leave::Index(index) => {
            let value = builder.ins().iconst(types::I64, i64::from(index));
            builder.ins().return_(&[value]);
        }
    }
}

/// The block an early path branches to for `target`.
///
/// A jump target is already a block. A returned index needs one that leaves, and
/// the block is filled after the work path has been terminated: the builder
/// does not allow switching away from an unfilled block.
fn exit_block(builder: &mut FunctionBuilder, target: Leave) -> Block {
    match target {
        Leave::Block(block) => block,
        Leave::Index(_) => builder.create_block(),
    }
}

/// Fill the early-exit block once the work path is finished.
fn fill_exit(builder: &mut FunctionBuilder, target: Leave, exit: Block) {
    if let Leave::Index(_) = target {
        builder.switch_to_block(exit);
        leave(builder, target);
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

fn emit_pop(builder: &mut FunctionBuilder, frame: Frame<'_>, target: Leave) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let empty = builder.ins().icmp_imm_s(IntCC::Equal, sp, 0);
    let pop_block = builder.create_block();
    let exit = exit_block(builder, target);
    builder.ins().brif(empty, exit, &[], pop_block, &[]);
    builder.switch_to_block(pop_block);
    let next = builder.ins().iadd_imm_s(sp, -1);
    store_field(builder, frame.ctx, next, frame.offsets.sp);
    leave(builder, target);
    fill_exit(builder, target, exit);
}

fn emit_binary(
    builder: &mut FunctionBuilder,
    frame: Frame<'_>,
    target: Leave,
    operation: impl FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let short = builder.ins().icmp_imm_s(IntCC::SignedLessThan, sp, 2);
    let operation_block = builder.create_block();
    let exit = exit_block(builder, target);
    builder.ins().brif(short, exit, &[], operation_block, &[]);
    builder.switch_to_block(operation_block);
    let second = builder.ins().iadd_imm_s(sp, -2);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let a = load_stack(builder, frame.stack, second);
    let b = load_stack(builder, frame.stack, top);
    let result = operation(builder, a, b);
    let address = stack_address(builder, frame.stack, second);
    builder.ins().store(MemFlagsData::new(), result, address, 0);
    store_field(builder, frame.ctx, top, frame.offsets.sp);
    leave(builder, target);
    fill_exit(builder, target, exit);
}

fn emit_divmod(
    builder: &mut FunctionBuilder,
    frame: Frame<'_>,
    target: Leave,
    divide: bool,
    mode: Division,
) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let short = builder.ins().icmp_imm_s(IntCC::SignedLessThan, sp, 2);
    let operation_block = builder.create_block();
    let exit = exit_block(builder, target);
    builder.ins().brif(short, exit, &[], operation_block, &[]);
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
        leave(builder, target);

        builder.switch_to_block(consume);
        store_field(builder, frame.ctx, second, frame.offsets.sp);
        leave(builder, target);
        fill_exit(builder, target, exit);
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
    leave(builder, target);

    builder.switch_to_block(zero_result);
    let zero_value = builder.ins().iconst(types::I64, 0);
    store_binary_result(builder, frame, second, top, zero_value);
    leave(builder, target);

    builder.switch_to_block(consume);
    store_field(builder, frame.ctx, second, frame.offsets.sp);
    leave(builder, target);
    fill_exit(builder, target, exit);
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

fn emit_not(builder: &mut FunctionBuilder, frame: Frame<'_>, target: Leave) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let empty = builder.ins().icmp_imm_s(IntCC::Equal, sp, 0);
    let operation_block = builder.create_block();
    let exit = exit_block(builder, target);
    builder.ins().brif(empty, exit, &[], operation_block, &[]);
    builder.switch_to_block(operation_block);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let value = load_stack(builder, frame.stack, top);
    let is_zero = builder.ins().icmp_imm_s(IntCC::Equal, value, 0);
    let one = builder.ins().iconst(types::I64, 1);
    let zero = builder.ins().iconst(types::I64, 0);
    let result = builder.ins().select(is_zero, one, zero);
    let address = stack_address(builder, frame.stack, top);
    builder.ins().store(MemFlagsData::new(), result, address, 0);
    leave(builder, target);
    fill_exit(builder, target, exit);
}

fn emit_duplicate(builder: &mut FunctionBuilder, frame: Frame<'_>, target: Leave) {
    let sp = load_field(builder, frame.ctx, types::I64, frame.offsets.sp);
    let empty = builder.ins().icmp_imm_s(IntCC::Equal, sp, 0);
    let duplicate_block = builder.create_block();
    let exit = exit_block(builder, target);
    builder.ins().brif(empty, exit, &[], duplicate_block, &[]);
    builder.switch_to_block(duplicate_block);
    let top = builder.ins().iadd_imm_s(sp, -1);
    let value = load_stack(builder, frame.stack, top);
    emit_push(builder, frame, value);
    leave(builder, target);
    fill_exit(builder, target, exit);
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
    emit_branch_by_index(builder, index, edges, &lowering.successors);
    builder.switch_to_block(empty_block);
    leave(builder, lowering.successors.leave(edges[0].to));
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
    emit_branch_by_index(builder, index, edges, &lowering.successors);
    builder.switch_to_block(empty_block);
    leave(builder, lowering.successors.leave(edges[0].to));
}

fn emit_branch_by_index(
    builder: &mut FunctionBuilder,
    index: Value,
    edges: &[&Edge],
    successors: &Successors<'_>,
) {
    match successors {
        Successors::Blocks(blocks) => {
            for (position, edge) in edges.iter().enumerate() {
                let target = blocks[&edge.to];
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
        Successors::Indices(indices) => {
            // The successor index as a value: one `select` per edge but the last,
            // then a single return. No block has to be left unfilled.
            let last = edges.len() - 1;
            let mut selected = builder
                .ins()
                .iconst(types::I64, i64::from(indices[&edges[last].to]));
            for (position, edge) in edges.iter().enumerate().take(last) {
                let candidate = builder
                    .ins()
                    .iconst(types::I64, i64::from(indices[&edge.to]));
                let matches = builder
                    .ins()
                    .icmp_imm_s(IntCC::Equal, index, position as i64);
                selected = builder.ins().select(matches, candidate, selected);
            }
            builder.ins().return_(&[selected]);
        }
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

fn push(context: &mut Context, value: i64) {
    if context.sp < context.capacity {
        // SAFETY: the context's stack points at a buffer of `capacity` values.
        unsafe { *context.stack.add(context.sp as usize) = value };
        context.sp += 1;
    } else {
        context.reason = REASON_OVERFLOW;
    }
}

fn pop(context: &mut Context) -> Option<i64> {
    if context.sp == 0 {
        return None;
    }
    context.sp -= 1;
    // SAFETY: the context's stack points at a buffer of `capacity` values.
    Some(unsafe { *context.stack.add(context.sp as usize) })
}

fn pop_two(context: &mut Context) -> Option<(i64, i64)> {
    if context.sp < 2 {
        return None;
    }
    let top = pop(context)?;
    let second = pop(context)?;
    Some((second, top))
}

fn roll(context: &mut Context, depth: i64, rolls: i64) {
    if depth < 0 {
        return;
    }
    let Ok(depth) = usize::try_from(depth) else {
        return;
    };
    if depth > context.sp as usize || depth <= 1 {
        return;
    }
    let start = context.sp as usize - depth;
    let shift = rolls.rem_euclid(depth as i64) as usize;
    // SAFETY: the context's stack points at a buffer of `capacity` values and
    // `start..sp` is within it.
    let values = unsafe { std::slice::from_raw_parts_mut(context.stack.add(start), depth) };
    values.rotate_right(shift);
}

extern "C" fn piet_out_char(context: *mut Context) {
    // SAFETY: `context` points at the first field of the `JitContext` built by
    // `run`; the generated code only sees the shared prefix.
    let ctx = unsafe { &mut *(context as *mut JitContext<'_>) };
    if let Some(value) = pop(&mut ctx.context)
        && let Ok(code) = u32::try_from(value)
        && let Some(character) = char::from_u32(code)
    {
        let io = unsafe { &mut *ctx.io };
        io.write_char(character);
    }
}

extern "C" fn piet_out_number(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext<'_>) };
    if let Some(value) = pop(&mut ctx.context) {
        let io = unsafe { &mut *ctx.io };
        io.write_number(value);
    }
}

extern "C" fn piet_in_char(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext<'_>) };
    let character = {
        let io = unsafe { &mut *ctx.io };
        io.read_char()
    };
    if let Some(character) = character {
        push(&mut ctx.context, i64::from(character as u32));
    }
}

extern "C" fn piet_in_number(context: *mut Context) {
    let ctx = unsafe { &mut *(context as *mut JitContext<'_>) };
    let value = {
        let io = unsafe { &mut *ctx.io };
        io.read_number()
    };
    if let Some(value) = value {
        push(&mut ctx.context, value);
    }
}

extern "C" fn piet_roll(context: *mut Context) {
    let ctx = unsafe { &mut *context };
    if let Some((depth, rolls)) = pop_two(ctx) {
        roll(ctx, depth, rolls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(capacity: usize) -> (Vec<i64>, Context) {
        let mut stack = vec![0i64; capacity];
        let context = Context {
            stack: stack.as_mut_ptr(),
            capacity: capacity as u64,
            sp: 0,
            steps: 0,
            max_steps: u64::MAX,
            reason: REASON_TERMINATED,
        };
        (stack, context)
    }

    fn filled(values: &[i64]) -> (Vec<i64>, Context) {
        let (stack, mut ctx) = context(values.len() + 4);
        for &value in values {
            push(&mut ctx, value);
        }
        (stack, ctx)
    }

    fn slice(ctx: &Context) -> &[i64] {
        // SAFETY: the context's stack points at a buffer of `capacity` values
        // and `sp` never exceeds it.
        unsafe { std::slice::from_raw_parts(ctx.stack, ctx.sp as usize) }
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
        assert_eq!(ctx.sp, 1);
        assert_eq!(ctx.reason, REASON_OVERFLOW);
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

    /// The lazy driver is the path above the whole-function bound; the bound is
    /// zero here so a small program takes it too.
    #[test]
    fn the_lazy_driver_agrees_with_the_interpreter() {
        let blocks = testkit::row(&[
            Command::Push,
            Command::Push,
            Command::Add,
            Command::OutNumber,
        ]);
        assert_lazy_agrees(&blocks, "", 16);
    }

    #[test]
    fn the_lazy_driver_honours_the_step_limit() {
        let blocks = testkit::row(&[Command::Push, Command::Push, Command::Add]);
        let mut io = runtime::BufferIo::new("");
        let options = Options {
            max_steps: Some(0),
            ..Options::default()
        };
        let run = run_with_limit(&blocks, &mut io, &options, 0).unwrap();
        assert_eq!(run.reason, ExitReason::StepLimit);
        assert_eq!(run.steps, 0);
    }

    /// `pointer` and `switch` choose their successor from the stack, so the lazy
    /// driver has to pick a state index with a `select` rather than a jump.
    #[test]
    fn the_lazy_driver_branches_on_pointer() {
        use canvas::{Color, Hue, Lightness};
        let blocks = testkit::blocks(
            vec![
                Color::chromatic(Hue::Red, Lightness::Normal),
                Color::chromatic(Hue::Magenta, Lightness::Normal),
                Color::chromatic(Hue::Cyan, Lightness::Light),
                Color::chromatic(Hue::Red, Lightness::Normal),
                Color::chromatic(Hue::Magenta, Lightness::Light),
                Color::Black,
                Color::Black,
                Color::Black,
                Color::chromatic(Hue::Magenta, Lightness::Dark),
                Color::Black,
            ],
            5,
        );
        for input in ["A0", "A1", "A2", "A3"] {
            assert_lazy_agrees(&blocks, input, 4);
        }
    }

    /// Runs both engines over the same blocks and compares what they did.
    fn assert_lazy_agrees(blocks: &Blocks, input: &str, max_steps: u64) {
        let mut lazy_io = runtime::BufferIo::new(input);
        let lazy = run_with_limit(
            blocks,
            &mut lazy_io,
            &Options {
                max_steps: Some(max_steps),
                ..Options::default()
            },
            0,
        )
        .unwrap();
        let mut interpreter_io = runtime::BufferIo::new(input);
        let interpreter = interp::run(
            blocks,
            &mut interpreter_io,
            &interp::Options {
                max_steps: Some(max_steps),
                ..interp::Options::default()
            },
        )
        .unwrap();
        assert_eq!(lazy_io.output(), interpreter_io.output(), "{input}");
        assert_eq!(lazy.steps, interpreter.steps, "{input}");
        let reason = match lazy.reason {
            ExitReason::Terminated => interp::ExitReason::Terminated,
            ExitReason::StepLimit => interp::ExitReason::StepLimit,
            ExitReason::StackOverflow => panic!("the lazy driver overflowed the stack"),
        };
        assert_eq!(reason, interpreter.reason, "{input}");
    }
}
