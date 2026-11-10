#![allow(clippy::unwrap_used)]
//! Integration tests for the Rust host shim and the link step.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use canvas::{Blocks, Color, Hue, Lightness};
use testkit::blocks;

fn single_block() -> Blocks {
    blocks(vec![Color::chromatic(Hue::Red, Lightness::Normal)], 1)
}

fn looping_pair() -> Blocks {
    blocks(
        vec![
            Color::chromatic(Hue::Red, Lightness::Normal),
            Color::chromatic(Hue::Green, Lightness::Normal),
        ],
        2,
    )
}

fn compiler() -> String {
    aot::find_compiler(None).expect("no Rust compiler found: tried rustc")
}

fn executable(directory: &Path) -> std::path::PathBuf {
    directory.join(format!("case{}", std::env::consts::EXE_SUFFIX))
}

fn run(executable: &Path) -> Output {
    let mut child = Command::new(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the executable timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_explicit_compiler_is_returned_as_given() {
    assert_eq!(
        aot::find_compiler(Some("definitely-not-a-compiler")),
        Some("definitely-not-a-compiler".to_owned())
    );
}

#[test]
fn a_missing_compiler_is_named() {
    let directory = tempfile::tempdir().unwrap();
    let error = aot::build(
        &single_block(),
        &executable(directory.path()),
        Some(1),
        runtime::Division::Floored,
        Some("definitely-not-a-compiler"),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("definitely-not-a-compiler"),
        "{error}"
    );
}

#[test]
fn a_built_program_terminates_without_output() {
    let directory = tempfile::tempdir().unwrap();
    let output = executable(directory.path());
    aot::build(
        &single_block(),
        &output,
        Some(1),
        runtime::Division::Floored,
        Some(&compiler()),
    )
    .unwrap();

    let run = run(&output);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stdout.is_empty());
}

#[test]
fn a_built_program_reports_the_step_limit() {
    let directory = tempfile::tempdir().unwrap();
    let output = executable(directory.path());
    aot::build(
        &looping_pair(),
        &output,
        Some(1),
        runtime::Division::Floored,
        Some(&compiler()),
    )
    .unwrap();

    let run = run(&output);
    assert!(!run.status.success());
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("step limit reached after 1 steps"),
        "{stderr}"
    );
}
