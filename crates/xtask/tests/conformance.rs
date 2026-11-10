#![allow(clippy::unwrap_used)]
//! End-to-end tests for the conformance gate.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use testkit::{
    BLUE_LIGHT, CYAN_LIGHT, GREEN_DARK, GREEN_NORMAL, MAGENTA, RED, RED_DARK, write_png,
};

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
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
    write_png(dir.path(), "a.png", 67, 1, &rgba);

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
    write_png(dir.path(), "subtract.png", 5, 1, &rgba);
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
fn conformance_agrees_with_the_checked_in_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let output = xtask()
        .args(["conformance", corpus.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    for name in [
        "hello",
        "subtract",
        "roll",
        "single",
        "white",
        "pointer",
        "black-start",
        "white-start",
    ] {
        assert!(stdout.contains(&format!("OK: {name}")), "{stdout}");
    }
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
