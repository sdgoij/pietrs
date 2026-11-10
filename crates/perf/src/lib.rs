//! Support for the ignored wall-clock harnesses in `tests/`.
//!
//! This crate exists only to hold performance tests. It is the one place besides
//! `runner` and `codegen` that allows `unsafe`, because [`GlobalAlloc`] needs it;
//! nothing depends on `perf`, so the counting allocator reaches only its own test
//! binaries.
//!
//! The harnesses print numbers and assert nothing: a duration is a property of
//! the machine, not of the code. Record results in `.agents/notes/perf.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use canvas::{CodelGrid, Color};

/// The counters are process wide, so only one measurement runs at a time.
static MEASURING: Mutex<()> = Mutex::new(());

/// The allocation counters at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// The number of allocations since the last reset.
    pub allocations: u64,
    /// The bytes requested by those allocations.
    pub bytes: u64,
    /// The bytes still held: allocations minus frees.
    pub live: u64,
}

/// A global allocator that counts what it hands out, and otherwise delegates to
/// the system allocator.
///
/// A test binary installs one with `#[global_allocator]`. Counters are process
/// wide, so [`measure`] resets them immediately before the work it times.
#[derive(Debug, Default)]
pub struct Counter {
    allocations: AtomicU64,
    bytes: AtomicU64,
    live: AtomicU64,
}

impl Counter {
    /// A fresh counter, for a `#[global_allocator]` static.
    pub const fn new() -> Self {
        Self {
            allocations: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            live: AtomicU64::new(0),
        }
    }

    /// Clears the counters.
    pub fn reset(&self) {
        self.allocations.store(0, Ordering::Relaxed);
        self.bytes.store(0, Ordering::Relaxed);
        self.live.store(0, Ordering::Relaxed);
    }

    /// Reads the counters.
    pub fn read(&self) -> Counts {
        Counts {
            allocations: self.allocations.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            live: self.live.load(Ordering::Relaxed),
        }
    }

    fn record(&self, size: usize, live_delta: i128) {
        self.allocations.fetch_add(1, Ordering::Relaxed);
        self.bytes
            .fetch_add(u64::try_from(size).unwrap_or(u64::MAX), Ordering::Relaxed);
        if live_delta >= 0 {
            self.live.fetch_add(
                u64::try_from(live_delta).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
        } else {
            self.live.fetch_sub(
                u64::try_from(-live_delta).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
        }
    }
}

// SAFETY: every method delegates to `System`, which is a correct implementation
// of `GlobalAlloc`; the counters only observe the sizes that already passed
// through it.
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which this
        // forwards unchanged to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            self.record(layout.size(), layout.size() as i128);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as above.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            self.record(layout.size(), layout.size() as i128);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        let new = unsafe { System.realloc(pointer, layout, new_size) };
        if !new.is_null() {
            self.record(new_size, new_size as i128 - layout.size() as i128);
        }
        new
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // A buffer allocated before the last reset can be freed after it, so the
        // live count saturates instead of wrapping.
        let size = u64::try_from(layout.size()).unwrap_or(u64::MAX);
        let _ = self
            .live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
                Some(live.saturating_sub(size))
            });
        // SAFETY: the pointer came from this allocator, which forwarded to
        // `System`, and the layout is the one it was allocated with.
        unsafe { System.dealloc(pointer, layout) }
    }
}

/// What one measured run cost.
#[derive(Debug, Clone, Copy)]
pub struct Measurement {
    /// Wall-clock time spent in the closure.
    pub elapsed: Duration,
    /// Allocations made during the closure.
    pub counts: Counts,
}

/// Runs `work` after resetting `counter`, and reports what it cost.
///
/// Measurements are serialised, because the counters are process wide and
/// `cargo test` runs cases in parallel by default. Setup done before the call is
/// still counted if another case is between its own calls, which is why
/// `just perf` pins one thread.
pub fn measure<T>(counter: &Counter, work: impl FnOnce() -> T) -> (T, Measurement) {
    let guard = MEASURING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    counter.reset();
    let start = Instant::now();
    let value = work();
    let elapsed = start.elapsed();
    drop(guard);
    (
        value,
        Measurement {
            elapsed,
            counts: counter.read(),
        },
    )
}

/// Prints a measurement in integer units, because this repo denies float
/// arithmetic.
pub fn report(label: &str, measurement: &Measurement) {
    println!(
        "{label}: {} us | {} allocations | {} KiB allocated | {} KiB live",
        measurement.elapsed.as_micros(),
        measurement.counts.allocations,
        measurement.counts.bytes / 1024,
        measurement.counts.live / 1024,
    );
}

/// A `side` x `side` grid over all twenty colours.
///
/// The index goes through a linear congruential generator, so the image is wide
/// and varied and every run measures the same one.
pub fn grid(side: u32) -> CodelGrid {
    let count = (side as usize) * (side as usize);
    let codels: Vec<Color> = (0..count)
        .map(|index| {
            let mixed = (index as u64)
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            Color::ALL[(mixed >> 33) as usize % Color::ALL.len()]
        })
        .collect();
    CodelGrid::from_colors(side, side, codels).expect("a full grid is a valid grid")
}

/// The integer rate `amount / duration` in whole units per second.
pub fn per_second(amount: u64, elapsed: Duration) -> u64 {
    let nanos = elapsed.as_nanos();
    if nanos == 0 {
        return 0;
    }
    u64::try_from(u128::from(amount) * 1_000_000_000 / nanos).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[global_allocator]
static TEST_COUNTER: Counter = Counter::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counter_sees_its_own_allocations() {
        let (value, measurement) = measure(&TEST_COUNTER, || vec![0u8; 4096]);
        assert_eq!(value.len(), 4096);
        assert!(measurement.counts.allocations >= 1);
        assert!(measurement.counts.bytes >= 4096);
        assert!(measurement.counts.live >= 4096);
    }

    #[test]
    fn a_grid_is_square_and_uses_every_colour() {
        let grid = grid(16);
        assert_eq!(grid.width(), 16);
        assert_eq!(grid.height(), 16);
        let mut seen = [false; 20];
        for color in grid.codels() {
            let index = Color::ALL
                .iter()
                .position(|candidate| candidate == color)
                .expect("every codel is one of the twenty colours");
            seen[index] = true;
        }
        assert!(seen.iter().all(|used| *used), "the grid uses all twenty");
    }

    #[test]
    fn a_rate_is_integer_and_zero_safe() {
        assert_eq!(
            per_second(1_000_000, Duration::from_millis(1000)),
            1_000_000
        );
        assert_eq!(per_second(1_000_000, Duration::from_millis(500)), 2_000_000);
        assert_eq!(per_second(10, Duration::ZERO), 0);
    }
}
