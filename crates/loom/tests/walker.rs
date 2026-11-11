#![allow(clippy::unwrap_used)]
//! The walker: a program held in a ring, followed record by record.
//!
//! The ring is streamed highest record first, so the last record read is record
//! zero and the head starts there. Each record is `command, block size,
//! offset`, and the offset is a roll of the whole ring in cells: `3 * k` moves
//! the head forward `k` records, `-3 * k` moves it back.

use canvas::{Blocks, CodelGrid};
use interp::{ExitReason, Options, Run};
use loom::{Error, RECORD, skeleton};
use runtime::BufferIo;

fn run(grid: &CodelGrid, input: &str) -> (String, Run) {
    let blocks = Blocks::partition(grid.clone());
    let mut io = BufferIo::new(input);
    let options = Options {
        max_steps: Some(1_000_000),
        ..Options::default()
    };
    let result = interp::run(&blocks, &mut io, &options).unwrap();
    (io.into_output(), result)
}

/// A ring's input stream: highest record first, three cells per record.
fn stream(records: &[(i64, i64, i64)]) -> String {
    let mut text = String::new();
    for (command, size, offset) in records.iter().rev() {
        text.push_str(&format!("{command} {size} {offset} "));
    }
    text
}

fn walk(records: &[(i64, i64, i64)]) -> (String, Run) {
    let program = skeleton((records.len() as u32) * RECORD).unwrap();
    run(&program.image().unwrap(), &stream(records))
}

#[test]
fn the_skeleton_reports_each_record_until_a_halt() {
    // push 65, then halt: the command codes, separated by spaces.
    let (output, result) = walk(&[(1, 65, 3), (0, 0, 0)]);
    assert_eq!(output, "1 0 ");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn the_offset_moves_the_head_forward() {
    // Four records in a chain; the last one halts.
    let (output, _) = walk(&[(1, 65, 3), (2, 0, 3), (3, 0, 3), (0, 0, 0)]);
    assert_eq!(output, "1 2 3 0 ");
}

#[test]
fn an_offset_can_skip_a_record() {
    // Record one would print 9. A forward offset of 6 steps over it, so a
    // single-record step lands there and loops forever instead.
    let (output, _) = walk(&[(1, 65, 6), (9, 0, 0), (0, 0, 0)]);
    assert_eq!(output, "1 0 ");
}

#[test]
fn a_negative_offset_wraps_backwards() {
    // From record zero, -3 is one step back, which is the last record.
    let (output, _) = walk(&[(1, 65, -3), (9, 0, 0), (9, 0, 0), (0, 0, 0)]);
    assert_eq!(output, "1 0 ");
}

#[test]
fn a_longer_ring_keeps_the_head_aligned() {
    let mut records: Vec<(i64, i64, i64)> = (1..10).map(|command| (command, 0, 3)).collect();
    records.push((0, 0, 0));
    let (output, result) = walk(&records);
    assert_eq!(output, "1 2 3 4 5 6 7 8 9 0 ");
    assert_eq!(result.reason, ExitReason::Terminated);
}

#[test]
fn a_ring_lays_out_small() {
    // The layout must not blow up with the ring: the image is one fixed
    // interpreter, whatever the number of records.
    let program = skeleton(10 * RECORD).unwrap();
    let image = program.image().unwrap();
    assert!(image.width() <= 64, "width {}", image.width());
    assert!(image.height() <= 64, "height {}", image.height());
}

#[test]
fn a_ring_that_is_not_whole_records_is_refused() {
    for cells in [0, 1, RECORD, 4, 5, 7] {
        assert!(
            matches!(skeleton(cells), Err(Error::Ring { cells: refused }) if refused == cells),
            "ring of {cells} cells"
        );
    }
}

#[test]
fn an_offset_off_the_record_boundary_still_walks() {
    // The skeleton cannot refuse a misaligned offset, but the run must not
    // panic: the head is simply somewhere else, and the reported command is
    // whatever cell it lands on. This pins the current behaviour rather than
    // claiming it is right.
    let program = skeleton(2 * RECORD).unwrap();
    let (_, result) = run(&program.image().unwrap(), "0 0 0 1 65 1 ");
    assert!(matches!(
        result.reason,
        ExitReason::Terminated | ExitReason::StepLimit
    ));
}
