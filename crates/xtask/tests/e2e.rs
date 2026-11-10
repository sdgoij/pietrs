#![allow(clippy::unwrap_used)]
//! End-to-end tests for the build, link, and run gate.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use testkit::{BLUE_LIGHT, CYAN_LIGHT, GREEN_DARK, GREEN_NORMAL, RED, write_png};

fn xtask() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
}

fn write_case(directory: &Path, expected_stdout: &str) {
    let case = directory.join("subtract");
    std::fs::create_dir_all(&case).unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    write_png(&case, "program.png", 5, 1, &rgba);
    std::fs::write(case.join("stdin"), "10 3").unwrap();
    std::fs::write(case.join("max_steps"), "4").unwrap();
    std::fs::write(case.join("expected.stdout"), expected_stdout).unwrap();
    std::fs::write(case.join("expected.exit"), "1").unwrap();
    std::fs::write(
        case.join("expected.trace"),
        concat!(
            "block=0 dp=Right cc=Left command=InNumber target=1\n",
            "block=1 dp=Right cc=Left command=InNumber target=2\n",
            "block=2 dp=Right cc=Left command=Subtract target=3\n",
            "block=3 dp=Right cc=Left command=OutNumber target=4\n",
        ),
    )
    .unwrap();
}

#[test]
fn e2e_compares_the_executable_with_its_goldens() {
    let directory = TempDir::new().unwrap();
    write_case(directory.path(), "7");

    let output = xtask()
        .args(["e2e", directory.path().to_str().unwrap()])
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
fn e2e_reports_a_mismatch() {
    let directory = TempDir::new().unwrap();
    write_case(directory.path(), "8");

    let output = xtask()
        .args(["e2e", directory.path().to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success());
    assert!(stdout.contains("FAIL: subtract"), "{stdout}");
    assert!(stdout.contains("stdout differs"), "{stdout}");
}

#[test]
fn e2e_reports_a_trace_mismatch() {
    let directory = TempDir::new().unwrap();
    write_case(directory.path(), "7");
    std::fs::write(
        directory.path().join("subtract").join("expected.trace"),
        "block=9 dp=Up cc=Right command=none target=none\n",
    )
    .unwrap();

    let output = xtask()
        .args(["e2e", directory.path().to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success());
    assert!(stdout.contains("FAIL: subtract"), "{stdout}");
    assert!(stdout.contains("trace differs at line 1"), "{stdout}");
}

#[test]
fn e2e_runs_the_checked_in_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let output = xtask()
        .args(["e2e", corpus.to_str().unwrap()])
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
fn e2e_names_an_empty_corpus() {
    let directory = TempDir::new().unwrap();
    let output = xtask()
        .args(["e2e", directory.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no case directories"));
}
