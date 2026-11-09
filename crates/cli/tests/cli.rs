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
const BLUE_LIGHT: [u8; 4] = [192, 192, 255, 255];
const GREEN_DARK: [u8; 4] = [0, 192, 0, 255];
const CYAN_LIGHT: [u8; 4] = [192, 255, 255, 255];
const GREEN_NORMAL: [u8; 4] = [0, 255, 0, 255];
const RED_LIGHT: [u8; 4] = [255, 192, 192, 255];
const BLUE_DARK: [u8; 4] = [0, 0, 192, 255];

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
fn selftest_compares_every_path() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    write_png(&dir, "a.png", 67, 1, &rgba);
    write_png(&dir, "single.png", 1, 1, &RED);
    let subtract: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    write_png(&dir, "subtract.png", 5, 1, &subtract);
    std::fs::write(dir.path().join("subtract.in"), "10 3").unwrap();

    let output =
        run_ok(pietrs().args(["selftest", dir.path().to_str().unwrap(), "--max-steps", "4"]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PASS: a"), "{stdout}");
    assert!(stdout.contains("PASS: single"), "{stdout}");
    assert!(stdout.contains("PASS: subtract"), "{stdout}");
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

#[test]
fn build_produces_a_runnable_executable() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    let image = write_png(&dir, "a.png", 67, 1, &rgba);
    let output = dir.path().join("a");
    run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--max-steps",
        "2",
    ]));

    let executable = dir
        .path()
        .join(format!("a{}", std::env::consts::EXE_SUFFIX));
    let run = Command::new(&executable).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "A");
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("step limit"));
}

#[test]
fn a_built_executable_reads_input() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    let image = write_png(&dir, "subtract.png", 5, 1, &rgba);
    let output = dir.path().join("subtract");
    run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--max-steps",
        "4",
    ]));

    let executable = dir
        .path()
        .join(format!("subtract{}", std::env::consts::EXE_SUFFIX));
    let input = dir.path().join("subtract.in");
    std::fs::write(&input, "10 3").unwrap();
    let run = Command::new(&executable)
        .stdin(std::process::Stdio::from(
            std::fs::File::open(&input).unwrap(),
        ))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "7");
}

#[test]
fn a_built_executable_rolls_the_stack() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [
        RED, BLUE_LIGHT, GREEN_DARK, RED, BLUE_LIGHT, GREEN_DARK, RED_LIGHT, MAGENTA, BLUE_DARK,
        CYAN_LIGHT,
    ]
    .into_iter()
    .flatten()
    .collect();
    let image = write_png(&dir, "roll.png", 10, 1, &rgba);
    let output = dir.path().join("roll");
    run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--max-steps",
        "9",
    ]));

    let executable = dir
        .path()
        .join(format!("roll{}", std::env::consts::EXE_SUFFIX));
    let input = dir.path().join("roll.in");
    std::fs::write(&input, "10 20 30 3 1").unwrap();
    let run = Command::new(&executable)
        .stdin(std::process::Stdio::from(
            std::fs::File::open(&input).unwrap(),
        ))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "201030");
}

#[test]
fn a_built_executable_round_trips_utf8() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, MAGENTA, BLUE_LIGHT].into_iter().flatten().collect();
    let image = write_png(&dir, "echo.png", 3, 1, &rgba);
    let output = dir.path().join("echo");
    run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--max-steps",
        "2",
    ]));

    let executable = dir
        .path()
        .join(format!("echo{}", std::env::consts::EXE_SUFFIX));
    let input = dir.path().join("echo.in");
    std::fs::write(&input, "λ").unwrap();
    let run = Command::new(&executable)
        .stdin(std::process::Stdio::from(
            std::fs::File::open(&input).unwrap(),
        ))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "λ");
}

#[test]
fn build_names_a_missing_linker() {
    let dir = TempDir::new().unwrap();
    let image = write_png(&dir, "one.png", 1, 1, &RED);
    let output = dir.path().join("one");
    let run = pietrs()
        .args([
            "build",
            image.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "--linker",
            "definitely-not-a-compiler",
        ])
        .output()
        .unwrap();
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("no C compiler found"));
}
