#![allow(clippy::unwrap_used)]
//! End-to-end tests for the conformance gate.

use std::process::Command;

use tempfile::TempDir;

const RED: [u8; 4] = [255, 0, 0, 255];
const RED_DARK: [u8; 4] = [192, 0, 0, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const BLUE_LIGHT: [u8; 4] = [192, 192, 255, 255];
const GREEN_DARK: [u8; 4] = [0, 192, 0, 255];
const CYAN_LIGHT: [u8; 4] = [192, 255, 255, 255];
const GREEN_NORMAL: [u8; 4] = [0, 255, 0, 255];

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
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

#[test]
fn conformance_agrees_with_npiet_on_a_pushed_character() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    std::fs::write(dir.path().join("a.png"), png_bytes(67, 1, &rgba)).unwrap();

    let output = xtask()
        .args([
            "conformance",
            dir.path().to_str().unwrap(),
            "--max-steps",
            "2",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("OK: a"), "{stdout}");
}

#[test]
fn conformance_agrees_on_input_and_arithmetic() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    std::fs::write(dir.path().join("subtract.png"), png_bytes(5, 1, &rgba)).unwrap();
    std::fs::write(dir.path().join("subtract.in"), "10 3").unwrap();

    let output = xtask()
        .args([
            "conformance",
            dir.path().to_str().unwrap(),
            "--max-steps",
            "4",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert!(stdout.contains("OK: subtract"), "{stdout}");
}

#[test]
fn conformance_names_an_empty_corpus() {
    let dir = TempDir::new().unwrap();
    let output = xtask()
        .args(["conformance", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no .png cases"));
}
