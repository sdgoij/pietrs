// The AOT loader. `crates/runner` includes this file as a module; `aot/build.rs`
// includes it in the generated runner `fn main`.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Mutex;

const PAYLOAD_MAGIC: [u8; 16] = *b"PIETRS-PAYLOAD\0\0";
const FOOTER_MAGIC: [u8; 16] = *b"PIETRS-AOT\0\0\0\0\0\0";
const VERSION: u32 = 1;
const ARCH_X86_64: u32 = 0;
const ARCH_AARCH64: u32 = 1;
const RUNNER_ARCH: u32 = if cfg!(target_arch = "x86_64") {
    ARCH_X86_64
} else if cfg!(target_arch = "aarch64") {
    ARCH_AARCH64
} else {
    u32::MAX
};
const FOOTER_LEN: usize = 40;
const HEADER_LEN: usize = 40;

const SYMBOL_UNDEFINED: u8 = 0;
const SYMBOL_SECTION: u8 = 1;
const SYMBOL_ABSOLUTE: u8 = 2;

const RELOCATION_ABSOLUTE_64: u8 = 0;
const RELOCATION_ABSOLUTE_32: u8 = 1;
const RELOCATION_RELATIVE_32: u8 = 2;

const REASON_TERMINATED: u32 = 0;
const REASON_STEP_LIMIT: u32 = 1;
const REASON_OVERFLOW: u32 = 2;

const STACK_CAPACITY: usize = 1 << 20;

/// The ABI the generated code and the host share.
#[repr(C)]
pub struct Context {
    stack: *mut i64,
    capacity: u64,
    sp: u64,
    steps: u64,
    max_steps: u64,
    reason: u32,
}

const _: () = {
    assert!(std::mem::size_of::<Context>() == 48);
    assert!(std::mem::offset_of!(Context, stack) == 0);
    assert!(std::mem::offset_of!(Context, capacity) == 8);
    assert!(std::mem::offset_of!(Context, sp) == 16);
    assert!(std::mem::offset_of!(Context, steps) == 24);
    assert!(std::mem::offset_of!(Context, max_steps) == 32);
    assert!(std::mem::offset_of!(Context, reason) == 40);
};

/// A byte read past the end of a number, handed back to the next reader.
static PUSHBACK: Mutex<Vec<u8>> = Mutex::new(Vec::new());

struct Section<'a> {
    align: u64,
    size: u64,
    data: &'a [u8],
}

enum Symbol {
    Undefined { name: String },
    Section { section: u32, offset: u64 },
    Absolute { address: u64 },
}

struct Relocation {
    section: u32,
    offset: u64,
    kind: u8,
    symbol: u32,
    addend: i64,
}

struct Program<'a> {
    sections: Vec<Section<'a>>,
    symbols: Vec<Symbol>,
    relocations: Vec<Relocation>,
    entry: u32,
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "the payload length overflowed".to_owned())?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "the payload is shorter than its header says".to_owned())?;
        self.offset = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes(bytes.try_into().unwrap()))
    }
}

impl<'a> Program<'a> {
    fn parse(payload: &'a [u8]) -> Result<Self, String> {
        let mut reader = Reader::new(payload);
        if payload.len() < HEADER_LEN {
            return Err("the payload is shorter than its header".to_owned());
        }
        let magic = reader.take(16)?;
        if magic != PAYLOAD_MAGIC {
            return Err("the payload does not start with the pietrs magic".to_owned());
        }
        if reader.u32()? != VERSION {
            return Err("the payload format version is not supported".to_owned());
        }
        let arch = reader.u32()?;
        if arch != RUNNER_ARCH {
            return Err("the payload architecture does not match this runner".to_owned());
        }
        let section_count = reader.u32()? as usize;
        let symbol_count = reader.u32()? as usize;
        let relocation_count = reader.u32()? as usize;
        let entry = reader.u32()?;

        let mut sections = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            let align = u64::from(reader.u32()?);
            let size = reader.u64()?;
            let data_len = reader.u64()? as usize;
            let data = reader.take(data_len)?;
            if data.len() as u64 > size {
                return Err("a payload section is smaller than its data".to_owned());
            }
            sections.push(Section { align, size, data });
        }

        let mut symbols = Vec::with_capacity(symbol_count);
        for _ in 0..symbol_count {
            let kind = reader.u8()?;
            let section = reader.u32()?;
            let value = reader.u64()?;
            let name_len = reader.u32()? as usize;
            let name = std::str::from_utf8(reader.take(name_len)?)
                .map_err(|_| "a symbol name is not UTF-8".to_owned())?
                .to_owned();
            symbols.push(match kind {
                SYMBOL_UNDEFINED => Symbol::Undefined { name },
                SYMBOL_SECTION => Symbol::Section {
                    section,
                    offset: value,
                },
                SYMBOL_ABSOLUTE => Symbol::Absolute { address: value },
                other => return Err(format!("unknown payload symbol kind {other}")),
            });
        }

        let mut relocations = Vec::with_capacity(relocation_count);
        for _ in 0..relocation_count {
            relocations.push(Relocation {
                section: reader.u32()?,
                offset: reader.u64()?,
                kind: reader.u8()?,
                symbol: reader.u32()?,
                addend: reader.i64()?,
            });
        }
        if reader.offset != payload.len() {
            return Err("the payload has trailing bytes".to_owned());
        }
        Ok(Self {
            sections,
            symbols,
            relocations,
            entry,
        })
    }

    fn execute(&self, max_steps: u64) -> Result<(u32, u64), String> {
        let mut total = 0usize;
        let mut bases = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            total = align_up(total, section.align)?;
            bases.push(total);
            total = total
                .checked_add(section.size as usize)
                .ok_or_else(|| "the payload sections are too large".to_owned())?;
        }
        if total == 0 {
            return Err("the payload has no executable sections".to_owned());
        }

        let memory = allocate(total)?;
        for (section, &offset) in self.sections.iter().zip(&bases) {
            if !section.data.is_empty() {
                // SAFETY: the region holds `total` bytes and `offset + size`
                // stays inside it; `data.len() <= size` was checked when
                // parsing.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        section.data.as_ptr(),
                        memory.add(offset),
                        section.data.len(),
                    );
                }
            }
        }

        let mut addresses = Vec::with_capacity(self.symbols.len());
        for symbol in &self.symbols {
            let address = match symbol {
                Symbol::Undefined { name } => host_function(name)?,
                Symbol::Section { section, offset } => {
                    let base = bases.get(*section as usize).ok_or_else(|| {
                        format!("symbol names section {section}, which is missing")
                    })?;
                    // SAFETY: `base + offset` is a valid address inside the
                    // mapped region; `offset` comes from the trusted payload.
                    unsafe { memory.add(base + *offset as usize) as usize }
                }
                Symbol::Absolute { address } => *address as usize,
            };
            addresses.push(address);
        }

        for relocation in &self.relocations {
            let section = self
                .sections
                .get(relocation.section as usize)
                .ok_or_else(|| "a relocation names a missing section".to_owned())?;
            let base = *bases
                .get(relocation.section as usize)
                .ok_or_else(|| "a relocation names a missing section".to_owned())?;
            let size = match relocation.kind {
                RELOCATION_ABSOLUTE_64 => 8,
                RELOCATION_ABSOLUTE_32 | RELOCATION_RELATIVE_32 => 4,
                other => return Err(format!("unknown relocation kind {other}")),
            };
            let end = relocation
                .offset
                .checked_add(size)
                .ok_or_else(|| "a relocation offset overflowed".to_owned())?;
            if end > section.size {
                return Err("a relocation is outside its section".to_owned());
            }
            let target = *addresses
                .get(relocation.symbol as usize)
                .ok_or_else(|| "a relocation names a missing symbol".to_owned())?
                as i64;
            let value = target.wrapping_add(relocation.addend);
            let place = base + relocation.offset as usize;
            let address = place as i64;
            // SAFETY: the relocation was bounds-checked against the section,
            // and the region is writable.
            unsafe {
                match relocation.kind {
                    RELOCATION_ABSOLUTE_64 => memory
                        .add(place)
                        .cast::<u64>()
                        .write_unaligned(value as u64),
                    RELOCATION_ABSOLUTE_32 => memory
                        .add(place)
                        .cast::<u32>()
                        .write_unaligned(value as u32),
                    RELOCATION_RELATIVE_32 => {
                        let delta = value.wrapping_sub(address.wrapping_add(4));
                        memory
                            .add(place)
                            .cast::<i32>()
                            .write_unaligned(delta as i32);
                    }
                    _ => return Err("unknown relocation kind".to_owned()),
                }
            }
        }

        let entry = *addresses
            .get(self.entry as usize)
            .ok_or_else(|| "the payload entry symbol is missing".to_owned())?;
        flush_instruction_cache(memory, total);
        let mut stack = Vec::new();
        if let Err(error) = stack.try_reserve_exact(STACK_CAPACITY) {
            return Err(format!("cannot allocate the stack: {error}"));
        }
        stack.resize(STACK_CAPACITY, 0);
        let mut context = Context {
            stack: stack.as_mut_ptr(),
            capacity: STACK_CAPACITY as u64,
            sp: 0,
            steps: 0,
            max_steps,
            reason: REASON_TERMINATED,
        };
        // SAFETY: the payload's entry follows the host ABI, and the context
        // and its stack outlive the call.
        let entry: extern "C" fn(*mut Context) = unsafe { std::mem::transmute(entry) };
        entry(&mut context);
        Ok((context.reason, context.steps))
    }
}

#[cfg(target_arch = "aarch64")]
fn flush_instruction_cache(base: *mut u8, size: usize) {
    use std::arch::asm;

    let start = base as usize;
    let end = start + size;
    // Clean the data cache to the point of unification, then invalidate the
    // instruction cache. `ctr_el0` reports the line sizes.
    let ctr: u64;
    // SAFETY: reading ctr_el0 is allowed in user mode on aarch64.
    unsafe { asm!("mrs {ctr}, ctr_el0", ctr = out(reg) ctr, options(nostack)) };
    let data_line = 4usize << ((ctr >> 16) & 0xf);
    let instruction_line = 4usize << (ctr & 0xf);

    let mut address = start & !(data_line - 1);
    while address < end {
        // SAFETY: the address is inside the mapped region.
        unsafe { asm!("dc cvau, {address}", address = in(reg) address, options(nostack)) };
        address += data_line;
    }
    // SAFETY: a barrier is safe in user mode.
    unsafe { asm!("dsb ish", options(nostack)) };

    let mut address = start & !(instruction_line - 1);
    while address < end {
        // SAFETY: the address is inside the mapped region.
        unsafe { asm!("ic ivau, {address}", address = in(reg) address, options(nostack)) };
        address += instruction_line;
    }
    // SAFETY: barriers are safe in user mode.
    unsafe { asm!("dsb ish", "isb", options(nostack)) };
}

#[cfg(not(target_arch = "aarch64"))]
fn flush_instruction_cache(_base: *mut u8, _size: usize) {}

fn align_up(value: usize, align: u64) -> Result<usize, String> {
    let align = align.max(1);
    if !align.is_power_of_two() {
        return Err(format!("section alignment {align} is not a power of two"));
    }
    let align = align as usize;
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
        .ok_or_else(|| "the payload layout overflowed".to_owned())
}

fn host_function(name: &str) -> Result<usize, String> {
    let function: Option<extern "C" fn(*mut Context)> = match name {
        "piet_out_char" => Some(piet_out_char),
        "piet_out_number" => Some(piet_out_number),
        "piet_in_char" => Some(piet_in_char),
        "piet_in_number" => Some(piet_in_number),
        "piet_roll" => Some(piet_roll),
        _ => None,
    };
    function
        .map(|function| function as usize)
        .ok_or_else(|| format!("the payload imports unknown host symbol {name:?}"))
}

#[cfg(windows)]
fn allocate(size: usize) -> Result<*mut u8, String> {
    const MEM_COMMIT_OR_RESERVE: u32 = 0x0000_3000;
    const PAGE_EXECUTE_READ_WRITE: u32 = 0x40;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut u8, size: usize, allocation: u32, protection: u32)
        -> *mut u8;
    }

    // SAFETY: a null address asks the system to choose the region.
    let pointer = unsafe {
        VirtualAlloc(
            std::ptr::null_mut(),
            size,
            MEM_COMMIT_OR_RESERVE,
            PAGE_EXECUTE_READ_WRITE,
        )
    };
    if pointer.is_null() {
        Err("VirtualAlloc could not allocate executable memory".to_owned())
    } else {
        Ok(pointer)
    }
}

#[cfg(unix)]
fn allocate(size: usize) -> Result<*mut u8, String> {
    const PROT_READ_WRITE_EXECUTE: i32 = 7;
    const MAP_PRIVATE: i32 = 2;
    #[cfg(target_os = "linux")]
    const MAP_ANONYMOUS: i32 = 0x20;
    #[cfg(not(target_os = "linux"))]
    const MAP_ANONYMOUS: i32 = 0x1000;

    unsafe extern "C" {
        fn mmap(
            address: *mut u8,
            length: usize,
            protection: i32,
            flags: i32,
            descriptor: i32,
            offset: i64,
        ) -> *mut u8;
    }

    // SAFETY: the arguments request an anonymous, private, executable mapping.
    let pointer = unsafe {
        mmap(
            std::ptr::null_mut(),
            size,
            PROT_READ_WRITE_EXECUTE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if pointer as isize == -1 {
        Err("mmap could not allocate executable memory".to_owned())
    } else {
        Ok(pointer)
    }
}

#[cfg(not(any(windows, unix)))]
fn allocate(_size: usize) -> Result<*mut u8, String> {
    Err("this platform has no executable-memory allocator".to_owned())
}

/// The packaged-program entry point.
pub fn start() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let invocation = match parse_args(&args) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("pietrs-runner: {error}\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    if invocation.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if let Err(error) = install_records(invocation.input.as_deref()) {
        eprintln!("pietrs-runner: {error}");
        return ExitCode::FAILURE;
    }
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            eprintln!("pietrs-runner: cannot find this executable: {error}");
            return ExitCode::FAILURE;
        }
    };
    match run_file_with(&executable, invocation.max_steps) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pietrs-runner: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs a packaged executable's embedded payload with the step budget its footer
/// carries.
pub fn run_file(path: &Path) -> Result<ExitCode, String> {
    run_file_with(path, None)
}

/// Runs a packaged executable's embedded payload.
///
/// `max_steps` replaces the budget in the footer. The footer's value is fixed at
/// build time, which suits a single program; a walker is built once and runs
/// programs of very different lengths, so its budget belongs on the command
/// line.
pub fn run_file_with(path: &Path, max_steps: Option<u64>) -> Result<ExitCode, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let (payload, baked) = split(&bytes)?;
    let (reason, steps) = execute(payload, max_steps.unwrap_or(baked))?;
    match reason {
        REASON_TERMINATED => Ok(ExitCode::SUCCESS),
        REASON_STEP_LIMIT => {
            eprintln!("pietrs: step limit reached after {steps} steps");
            Ok(ExitCode::FAILURE)
        }
        REASON_OVERFLOW => {
            eprintln!("pietrs: the stack overflowed");
            Ok(ExitCode::FAILURE)
        }
        _ => Ok(ExitCode::FAILURE),
    }
}

/// Loads and runs a payload, returning its reason and step count.
pub fn execute(payload: &[u8], max_steps: u64) -> Result<(u32, u64), String> {
    Program::parse(payload)?.execute(max_steps)
}

fn split(bytes: &[u8]) -> Result<(&[u8], u64), String> {
    let footer = bytes
        .get(bytes.len().saturating_sub(FOOTER_LEN)..)
        .filter(|footer| footer.len() == FOOTER_LEN)
        .ok_or_else(|| "this runner has no embedded program".to_owned())?;
    if footer[..16] != FOOTER_MAGIC {
        return Err("this runner has no embedded program".to_owned());
    }
    let payload_len = u64::from_le_bytes(footer[16..24].try_into().unwrap());
    let max_steps = u64::from_le_bytes(footer[24..32].try_into().unwrap());
    let arch = u32::from_le_bytes(footer[32..36].try_into().unwrap());
    let version = u32::from_le_bytes(footer[36..40].try_into().unwrap());
    if version != VERSION {
        return Err("the embedded program uses an unsupported format".to_owned());
    }
    if arch != ARCH_X86_64 && arch != ARCH_AARCH64 {
        return Err("the embedded program has an unknown architecture".to_owned());
    }
    if arch != RUNNER_ARCH {
        return Err("the embedded program does not match this runner".to_owned());
    }
    let end = bytes.len() - FOOTER_LEN;
    let payload_len =
        usize::try_from(payload_len).map_err(|_| "the embedded program is too large".to_owned())?;
    let start = end
        .checked_sub(payload_len)
        .ok_or_else(|| "the embedded program is shorter than its footer says".to_owned())?;
    Ok((&bytes[start..end], max_steps))
}

fn push(context: &mut Context, value: i64) {
    if context.sp < context.capacity {
        // SAFETY: `sp < capacity` and the stack has `capacity` slots.
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
    // SAFETY: the slot was written before `sp` was incremented.
    Some(unsafe { *context.stack.add(context.sp as usize) })
}

fn pop_two(context: &mut Context) -> Option<(i64, i64)> {
    if context.sp < 2 {
        return None;
    }
    context.sp -= 1;
    // SAFETY: two slots below the old `sp` are live.
    let top = unsafe { *context.stack.add(context.sp as usize) };
    context.sp -= 1;
    // SAFETY: the remaining slot is below the new `sp`.
    let second = unsafe { *context.stack.add(context.sp as usize) };
    Some((second, top))
}

/// A byte source: the record stream named on the command line, or the terminal.
///
/// Chaining happens per token, not per byte. A number that ends at the end of
/// the file must not run into the first digit the terminal supplies: the loader
/// reads `80440 148 1` from a stream with no trailing newline, and a byte-level
/// hand-over would turn a typed `2` into `12`.
trait Bytes {
    /// The next byte, or `None` at the end of the source.
    fn byte(&mut self) -> Option<u8>;

    /// Hands `byte` back to this source.
    fn unread(&mut self, byte: u8);
}

struct RecordFile {
    reader: io::BufReader<std::fs::File>,
    pushback: Vec<u8>,
}

/// The record stream the executable was given, if any.
static RECORDS: Mutex<Option<RecordFile>> = Mutex::new(None);

struct Records;

impl Bytes for Records {
    fn byte(&mut self) -> Option<u8> {
        let mut guard = RECORDS.lock().ok()?;
        let records = guard.as_mut()?;
        if let Some(byte) = records.pushback.pop() {
            return Some(byte);
        }
        let mut byte = [0u8; 1];
        match records.reader.read(&mut byte) {
            Ok(1) => Some(byte[0]),
            _ => None,
        }
    }

    fn unread(&mut self, byte: u8) {
        if let Ok(mut guard) = RECORDS.lock()
            && let Some(records) = guard.as_mut()
        {
            records.pushback.push(byte);
        }
    }
}

struct Terminal;

impl Bytes for Terminal {
    fn byte(&mut self) -> Option<u8> {
        if let Some(byte) = PUSHBACK.lock().ok()?.pop() {
            return Some(byte);
        }
        let mut byte = [0u8; 1];
        match io::stdin().read(&mut byte) {
            Ok(1) => Some(byte[0]),
            _ => None,
        }
    }

    fn unread(&mut self, byte: u8) {
        if let Ok(mut pushback) = PUSHBACK.lock() {
            pushback.push(byte);
        }
    }
}

/// What the command line asked the executable to do.
#[derive(Default)]
struct Invocation {
    /// The record stream to read before the terminal.
    input: Option<std::path::PathBuf>,
    /// A step budget that replaces the one baked in at build time.
    max_steps: Option<u64>,
    /// Whether `-h` or `--help` was given.
    help: bool,
}

const USAGE: &str = "\
usage: <program> [input-file] [--max-steps N]

The program reads the input file first, then standard input. A step budget given
here replaces the one baked in when the program was built.";

/// Parses the executable's own arguments.
fn parse_args(args: &[std::ffi::OsString]) -> Result<Invocation, String> {
    let mut invocation = Invocation::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let text = arg.to_string_lossy();
        if text == "-h" || text == "--help" {
            invocation.help = true;
        } else if text == "--max-steps" {
            let Some(value) = rest.next() else {
                return Err("--max-steps needs a number".to_owned());
            };
            invocation.max_steps = Some(parse_steps(&value.to_string_lossy())?);
        } else if let Some(value) = text.strip_prefix("--max-steps=") {
            invocation.max_steps = Some(parse_steps(value)?);
        } else if text != "-" && text.starts_with('-') {
            return Err(format!("unknown option {text}"));
        } else if invocation.input.is_some() {
            return Err("expected at most one input file".to_owned());
        } else if text != "-" {
            // A lone `-` names standard input, which is what the runner reads
            // with no file at all, so it installs nothing.
            invocation.input = Some(std::path::PathBuf::from(arg));
        }
    }
    Ok(invocation)
}

fn parse_steps(text: &str) -> Result<u64, String> {
    text.parse()
        .map_err(|_| format!("--max-steps needs a whole number, got {text}"))
}

/// Installs the record stream an executable was told to read first.
fn install_records(path: Option<&Path>) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut guard = RECORDS
        .lock()
        .map_err(|_| "the input lock is poisoned".to_owned())?;
    *guard = Some(RecordFile {
        reader: io::BufReader::new(file),
        pushback: Vec::new(),
    });
    Ok(())
}

/// Reads a character from the record stream, then from the terminal.
fn read_char() -> Option<char> {
    read_char_from(&mut Records).or_else(|| read_char_from(&mut Terminal))
}

/// Reads a number from the record stream, then from the terminal.
fn read_number() -> Option<i64> {
    read_number_from(&mut Records).or_else(|| read_number_from(&mut Terminal))
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Reads a decimal integer the way `runtime::BufferIo` does: leading whitespace
/// is consumed, a lone sign is handed back, and a value that does not fit is
/// consumed but not pushed.
fn read_number_from<S: Bytes>(source: &mut S) -> Option<i64> {
    let mut byte = loop {
        let byte = source.byte()?;
        if !is_space(byte) {
            break byte;
        }
    };
    let mut text = String::new();
    if byte == b'+' || byte == b'-' {
        text.push(char::from(byte));
        let Some(next) = source.byte() else {
            source.unread(byte);
            return None;
        };
        byte = next;
    }
    let mut digits = 0;
    loop {
        if byte.is_ascii_digit() {
            text.push(char::from(byte));
            digits += 1;
            let Some(next) = source.byte() else {
                break;
            };
            byte = next;
        } else {
            source.unread(byte);
            break;
        }
    }
    if digits == 0 {
        if let Some(sign) = text.chars().next() {
            source.unread(sign as u8);
        }
        return None;
    }
    text.parse().ok()
}

/// Decodes one UTF-8 character, as `runtime::StreamIo` does.
fn read_char_from<S: Bytes>(source: &mut S) -> Option<char> {
    let first = source.byte()?;
    let (mut code, extra) = match first {
        0x00..=0x7f => (u32::from(first), 0),
        byte if byte & 0xe0 == 0xc0 => (u32::from(byte & 0x1f), 1),
        byte if byte & 0xf0 == 0xe0 => (u32::from(byte & 0x0f), 2),
        byte if byte & 0xf8 == 0xf0 => (u32::from(byte & 0x07), 3),
        _ => return None,
    };
    for _ in 0..extra {
        let next = source.byte()?;
        if next & 0xc0 != 0x80 {
            return None;
        }
        code = (code << 6) | u32::from(next & 0x3f);
    }
    if code > 0x10ffff || (0xd800..=0xdfff).contains(&code) {
        return None;
    }
    char::from_u32(code)
}

extern "C" fn piet_out_char(context: *mut Context) {
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    let Some(value) = pop(context) else {
        return;
    };
    let Ok(value) = u32::try_from(value) else {
        return;
    };
    let Some(character) = char::from_u32(value) else {
        return;
    };
    let mut bytes = [0u8; 4];
    let text = character.encode_utf8(&mut bytes);
    let mut stdout = io::stdout();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
}

extern "C" fn piet_out_number(context: *mut Context) {
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    let Some(value) = pop(context) else {
        return;
    };
    let mut stdout = io::stdout();
    let _ = write!(stdout, "{value}");
    let _ = stdout.flush();
}

extern "C" fn piet_in_char(context: *mut Context) {
    let Some(character) = read_char() else {
        return;
    };
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    push(context, i64::from(character as u32));
}

extern "C" fn piet_in_number(context: *mut Context) {
    let Some(value) = read_number() else {
        return;
    };
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    push(context, value);
}

extern "C" fn piet_roll(context: *mut Context) {
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    let Some((depth, rolls)) = pop_two(context) else {
        return;
    };
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
    // `rotate_right`, not a shift-by-one loop: `depth` can be a walker's ring
    // (tens of thousands of values), and a loop is O(shift * depth) per call.
    // This mirrors `codegen`'s `roll` host function.
    // SAFETY: `sp` slots starting at `stack` are initialised, and `start..sp`
    // is inside them.
    let values = unsafe { std::slice::from_raw_parts_mut(context.stack.add(start), depth) };
    values.rotate_right(shift);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An in-memory byte source, so the readers can be tested without the
    /// process's own input.
    struct Buffer {
        input: Vec<u8>,
        at: usize,
        pushback: Vec<u8>,
    }

    impl Buffer {
        fn new(input: &str) -> Self {
            Self {
                input: input.as_bytes().to_vec(),
                at: 0,
                pushback: Vec::new(),
            }
        }
    }

    impl Bytes for Buffer {
        fn byte(&mut self) -> Option<u8> {
            if let Some(byte) = self.pushback.pop() {
                return Some(byte);
            }
            let byte = *self.input.get(self.at)?;
            self.at += 1;
            Some(byte)
        }

        fn unread(&mut self, byte: u8) {
            self.pushback.push(byte);
        }
    }

    #[test]
    fn a_number_does_not_run_past_the_end_of_its_source() {
        let mut records = Buffer::new("10");
        let mut terminal = Buffer::new("3");
        // The hand-over `read_number` makes: the terminal's first digit must not
        // continue the file's last number.
        let first = read_number_from(&mut records).or_else(|| read_number_from(&mut terminal));
        let second = read_number_from(&mut records).or_else(|| read_number_from(&mut terminal));
        assert_eq!(first, Some(10));
        assert_eq!(second, Some(3));
    }

    #[test]
    fn a_source_hands_back_the_byte_that_ends_its_number() {
        let mut records = Buffer::new("12 34");
        assert_eq!(read_number_from(&mut records), Some(12));
        assert_eq!(read_number_from(&mut records), Some(34));
        assert_eq!(records.at, 5);
    }

    #[test]
    fn a_character_chain_reads_across_the_hand_over() {
        let mut records = Buffer::new("a");
        let mut terminal = Buffer::new("λ");
        let first = read_char_from(&mut records).or_else(|| read_char_from(&mut terminal));
        let second = read_char_from(&mut records).or_else(|| read_char_from(&mut terminal));
        assert_eq!(first, Some('a'));
        assert_eq!(second, Some('λ'));
    }

    /// A context over `stack`'s live values, with room for `extra` pushes.
    fn context_for(stack: &mut Vec<i64>, extra: usize) -> Context {
        let live = stack.len();
        stack.reserve(extra);
        Context {
            stack: stack.as_mut_ptr(),
            capacity: (live + extra) as u64,
            sp: live as u64,
            steps: 0,
            max_steps: 0,
            reason: REASON_TERMINATED,
        }
    }

    /// `roll(depth, rolls)` on `values`, as the host function sees it.
    fn roll(values: &[i64], rolls: i64) -> Vec<i64> {
        let mut stack = values.to_vec();
        let mut context = context_for(&mut stack, 2);
        push(&mut context, i64::try_from(values.len()).unwrap());
        push(&mut context, rolls);
        piet_roll(&mut context as *mut Context);
        stack
    }

    #[test]
    fn a_roll_rotates_the_top_values() {
        assert_eq!(roll(&[1, 2, 3, 4], 1), [4, 1, 2, 3]);
        assert_eq!(roll(&[1, 2, 3, 4], 3), [2, 3, 4, 1]);
    }

    #[test]
    fn a_negative_roll_rotates_the_other_way() {
        assert_eq!(roll(&[1, 2, 3, 4], -1), [2, 3, 4, 1]);
        assert_eq!(roll(&[1, 2, 3, 4], 5), [4, 1, 2, 3]);
    }

    #[test]
    fn a_deep_roll_rotates_the_whole_window() {
        // The walker's ring is tens of thousands of values; depth 64 is the
        // deepest the table weaves, so this exercises the large-window path.
        let rolled = roll(&(0..64).collect::<Vec<i64>>(), 29);
        assert_eq!(rolled[0], 35);
        assert_eq!(rolled[29], 0);
        assert_eq!(rolled[63], 34);
    }

    #[test]
    fn alignment_is_up_and_power_of_two() {
        assert_eq!(align_up(0, 1).unwrap(), 0);
        assert_eq!(align_up(1, 1).unwrap(), 1);
        assert_eq!(align_up(1, 8).unwrap(), 8);
        assert_eq!(align_up(9, 8).unwrap(), 16);
        assert!(align_up(1, 3).is_err());
    }

    #[test]
    fn the_command_line_takes_a_file_and_a_budget() {
        let invocation = parse_args(&[std::ffi::OsString::from("records.txt")]).unwrap();
        assert_eq!(invocation.input.as_deref(), Some(Path::new("records.txt")));
        assert_eq!(invocation.max_steps, None);

        let invocation = parse_args(&[
            std::ffi::OsString::from("records.txt"),
            std::ffi::OsString::from("--max-steps"),
            std::ffi::OsString::from("10"),
        ])
        .unwrap();
        assert_eq!(invocation.input.as_deref(), Some(Path::new("records.txt")));
        assert_eq!(invocation.max_steps, Some(10));

        let invocation = parse_args(&[std::ffi::OsString::from("--max-steps=7")]).unwrap();
        assert_eq!(invocation.input, None);
        assert_eq!(invocation.max_steps, Some(7));
    }

    #[test]
    fn a_lone_dash_names_standard_input() {
        let invocation = parse_args(&[std::ffi::OsString::from("-")]).unwrap();
        assert_eq!(invocation.input, None);
    }

    #[test]
    fn a_bad_command_line_is_a_diagnostic() {
        let two_files = [
            std::ffi::OsString::from("a.txt"),
            std::ffi::OsString::from("b.txt"),
        ];
        assert!(parse_args(&two_files).is_err());
        assert!(parse_args(&[std::ffi::OsString::from("--max-steps")]).is_err());
        assert!(parse_args(&[std::ffi::OsString::from("--max-steps=nope")]).is_err());
        assert!(parse_args(&[std::ffi::OsString::from("--wat")]).is_err());
    }

    #[test]
    fn help_is_not_a_run() {
        let invocation = parse_args(&[std::ffi::OsString::from("--help")]).unwrap();
        assert!(invocation.help);
        assert_eq!(invocation.max_steps, None);
    }

    #[test]
    fn a_file_without_a_footer_is_a_diagnostic() {
        let error = split(b"not a packaged program").unwrap_err();
        assert!(error.contains("no embedded program"), "{error}");
    }

    #[test]
    fn a_footer_that_points_past_the_file_is_a_diagnostic() {
        let mut bytes = vec![0u8; FOOTER_LEN];
        bytes[..16].copy_from_slice(&FOOTER_MAGIC);
        bytes[16..24].copy_from_slice(&100u64.to_le_bytes());
        assert!(split(&bytes).is_err());
    }

    fn footer(arch: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; FOOTER_LEN];
        bytes[..16].copy_from_slice(&FOOTER_MAGIC);
        bytes[32..36].copy_from_slice(&arch.to_le_bytes());
        bytes[36..40].copy_from_slice(&VERSION.to_le_bytes());
        bytes
    }

    fn header(arch: u32) -> Vec<u8> {
        let mut bytes = PAYLOAD_MAGIC.to_vec();
        for value in [VERSION, arch, 0, 0, 0, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn the_host_architecture_passes_the_footer_check() {
        assert!(split(&footer(RUNNER_ARCH)).is_ok());
    }

    #[test]
    fn a_footer_for_another_architecture_is_a_diagnostic() {
        let error = split(&footer(1 - RUNNER_ARCH)).unwrap_err();
        assert!(error.contains("does not match this runner"), "{error}");
    }

    #[test]
    fn a_footer_with_an_unknown_architecture_is_a_diagnostic() {
        let error = split(&footer(7)).unwrap_err();
        assert!(error.contains("unknown architecture"), "{error}");
    }

    #[test]
    fn a_payload_for_another_architecture_is_a_diagnostic() {
        let error = execute(&header(1 - RUNNER_ARCH), 0).unwrap_err();
        assert!(error.contains("does not match this runner"), "{error}");
    }

    #[test]
    fn a_payload_with_an_unknown_version_is_a_diagnostic() {
        let mut bytes = header(RUNNER_ARCH);
        bytes[16..20].copy_from_slice(&7u32.to_le_bytes());
        let error = execute(&bytes, 0).unwrap_err();
        assert!(error.contains("version"), "{error}");
    }
}
