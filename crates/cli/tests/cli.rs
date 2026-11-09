#![allow(clippy::unwrap_used)]
//! End-to-end tests for the `pietrs` binary.

use std::process::{Command, Output};

use tempfile::TempDir;

const RED: [u8; 4] = [255, 0, 0, 255];
const RED_DARK: [u8; 4] = [192, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn pietrs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pietrs"))
}

fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(rgba).unwrap();
    }
    out
}

fn write_png(
    dir: &TempDir,
    name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, png_bytes(width, height, rgba)).unwrap();
    path
}

fn run_ok(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn check_reports_the_grid_and_blocks() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            let pixel = match (x / 2, y / 2) {
                (0, 0) => RED,
                (1, 0) => GREEN,
                (0, 1) => BLUE,
                _ => WHITE,
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    let path = write_png(&dir, "four.png", 4, 4, &rgba);
    let output = run_ok(pietrs().args(["check", path.to_str().unwrap()]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("grid: 2x2 codels (codel size 2)"),
        "{stdout}"
    );
    assert!(stdout.contains("blocks: 4"), "{stdout}");
    assert!(stdout.contains("states: "), "{stdout}");
    assert!(stdout.contains("start: block 0"), "{stdout}");
}

#[test]
fn a_single_block_terminates() {
    let dir = TempDir::new().unwrap();
    let path = write_png(&dir, "one.png", 1, 1, &RED);
    let output = run_ok(pietrs().args(["run", path.to_str().unwrap()]));
    assert!(output.stdout.is_empty());
}

#[test]
fn run_prints_a_pushed_character() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    let path = write_png(&dir, "a.png", 67, 1, &rgba);
    let output = pietrs()
        .args(["run", path.to_str().unwrap(), "--max-steps", "2"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "A");
    assert!(!output.status.success(), "the step limit is a failure");
    assert!(String::from_utf8_lossy(&output.stderr).contains("step limit"));
}

#[test]
fn trace_prints_the_transitions() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    let path = write_png(&dir, "a.png", 67, 1, &rgba);
    let output = run_ok(pietrs().args(["trace", path.to_str().unwrap(), "--max-steps", "2"]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("block=0 dp=Right cc=Left command=Push target=1"),
        "{stdout}"
    );
    assert!(
        stdout.contains("block=1 dp=Right cc=Left command=OutChar target=2"),
        "{stdout}"
    );
}

#[test]
fn the_codel_size_override_is_honoured() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, GREEN, BLUE, WHITE].into_iter().flatten().collect();
    let path = write_png(&dir, "pixels.png", 2, 2, &rgba);
    let output = pietrs()
        .args(["check", path.to_str().unwrap(), "--codel-size", "2"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a single colour"));
}

#[test]
fn a_malformed_png_is_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bad.png");
    std::fs::write(&path, b"not a png").unwrap();
    let output = pietrs()
        .args(["check", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("PNG decoding failed"));
}

#[test]
fn selftest_compares_the_interpreter_and_the_jit() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    write_png(&dir, "a.png", 67, 1, &rgba);
    write_png(&dir, "single.png", 1, 1, &RED);

    let output =
        run_ok(pietrs().args(["selftest", dir.path().to_str().unwrap(), "--max-steps", "2"]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS: a"), "{stdout}");
    assert!(stdout.contains("PASS: single"), "{stdout}");
}

#[test]
fn selftest_without_cases_is_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let output = pietrs()
        .args(["selftest", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no .png cases"));
}
