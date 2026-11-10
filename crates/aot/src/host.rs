//! The AOT host: the process entry point and the functions the lowered object calls.
//!
//! `aot::build` compiles this file with `rustc` and links the emitted object into
//! the same executable. The step limit is read at compile time from
//! `PIET_AOT_MAX_STEPS`; without it the file still compiles, with no limit.
//!
//! The `Context` layout must match `codegen`'s `Context`. The assertions below
//! fail the compilation if it ever drifts.

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::sync::Mutex;

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

const REASON_TERMINATED: u32 = 0;
const REASON_STEP_LIMIT: u32 = 1;
const REASON_OVERFLOW: u32 = 2;

const STACK_CAPACITY: usize = 1 << 20;

/// A byte read past the end of a number, handed back to the next reader.
static PUSHBACK: Mutex<Vec<u8>> = Mutex::new(Vec::new());

unsafe extern "C" {
    fn piet_main(context: *mut Context);
}

fn main() -> ExitCode {
    let max_steps = match option_env!("PIET_AOT_MAX_STEPS") {
        Some(text) => text.parse().expect("PIET_AOT_MAX_STEPS is not a u64"),
        None => u64::MAX,
    };
    let mut stack = Vec::new();
    if let Err(error) = stack.try_reserve_exact(STACK_CAPACITY) {
        eprintln!("pietrs: cannot allocate the stack: {error}");
        return ExitCode::FAILURE;
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
    // SAFETY: the context is live for the call, and `stack` outlives it.
    unsafe { piet_main(&mut context) };
    match context.reason {
        REASON_STEP_LIMIT => {
            eprintln!("pietrs: step limit reached after {} steps", context.steps);
            ExitCode::FAILURE
        }
        REASON_OVERFLOW => {
            eprintln!("pietrs: the stack overflowed");
            ExitCode::FAILURE
        }
        REASON_TERMINATED => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
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

fn read_byte() -> Option<u8> {
    if let Some(byte) = PUSHBACK.lock().ok()?.pop() {
        return Some(byte);
    }
    let mut byte = [0u8; 1];
    match io::stdin().read(&mut byte) {
        Ok(1) => Some(byte[0]),
        _ => None,
    }
}

fn unread(byte: u8) {
    if let Ok(mut pushback) = PUSHBACK.lock() {
        pushback.push(byte);
    }
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Reads a decimal integer the way `runtime::BufferIo` does: leading whitespace
/// is consumed, a lone sign is handed back, and a value that does not fit is
/// consumed but not pushed.
fn read_number() -> Option<i64> {
    let mut byte = loop {
        let byte = read_byte()?;
        if !is_space(byte) {
            break byte;
        }
    };
    let mut text = String::new();
    if byte == b'+' || byte == b'-' {
        text.push(char::from(byte));
        let Some(next) = read_byte() else {
            unread(byte);
            return None;
        };
        byte = next;
    }
    let mut digits = 0;
    loop {
        if byte.is_ascii_digit() {
            text.push(char::from(byte));
            digits += 1;
            let Some(next) = read_byte() else {
                break;
            };
            byte = next;
        } else {
            unread(byte);
            break;
        }
    }
    if digits == 0 {
        if let Some(sign) = text.chars().next() {
            unread(sign as u8);
        }
        return None;
    }
    text.parse().ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn piet_out_char(context: *mut Context) {
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

#[unsafe(no_mangle)]
pub extern "C" fn piet_out_number(context: *mut Context) {
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    let Some(value) = pop(context) else {
        return;
    };
    let mut stdout = io::stdout();
    let _ = write!(stdout, "{value}");
    let _ = stdout.flush();
}

#[unsafe(no_mangle)]
pub extern "C" fn piet_in_char(context: *mut Context) {
    let Some(first) = read_byte() else {
        return;
    };
    let (mut code, extra) = match first {
        0x00..=0x7f => (u32::from(first), 0),
        byte if byte & 0xe0 == 0xc0 => (u32::from(byte & 0x1f), 1),
        byte if byte & 0xf0 == 0xe0 => (u32::from(byte & 0x0f), 2),
        byte if byte & 0xf8 == 0xf0 => (u32::from(byte & 0x07), 3),
        _ => return,
    };
    for _ in 0..extra {
        let Some(next) = read_byte() else {
            return;
        };
        if next & 0xc0 != 0x80 {
            return;
        }
        code = (code << 6) | u32::from(next & 0x3f);
    }
    if code > 0x10ffff || (0xd800..=0xdfff).contains(&code) {
        return;
    }
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    push(context, i64::from(code));
}

#[unsafe(no_mangle)]
pub extern "C" fn piet_in_number(context: *mut Context) {
    let Some(value) = read_number() else {
        return;
    };
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    push(context, value);
}

#[unsafe(no_mangle)]
pub extern "C" fn piet_roll(context: *mut Context) {
    // SAFETY: the generated code passes the live context pointer.
    let context = unsafe { &mut *context };
    let Some((depth, rolls)) = pop_two(context) else {
        return;
    };
    if depth < 0 || depth <= 1 || (depth as u64) > context.sp {
        return;
    }
    let count = depth as usize;
    let start = context.sp as usize - count;
    let mut shift = rolls % depth;
    if shift < 0 {
        shift += depth;
    }
    // SAFETY: `sp` slots starting at `stack` are initialised.
    let stack = unsafe { std::slice::from_raw_parts_mut(context.stack, context.sp as usize) };
    for _ in 0..shift {
        let last = stack[stack.len() - 1];
        stack.copy_within(start..stack.len() - 1, start + 1);
        stack[start] = last;
    }
}
