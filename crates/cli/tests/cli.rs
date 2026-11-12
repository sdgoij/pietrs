#![allow(clippy::unwrap_used)]
//! End-to-end tests for the `pietrs` binary.

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::process::{Command, Output};

use tempfile::TempDir;
use testkit::{
    BLUE, BLUE_DARK, BLUE_LIGHT, CYAN_LIGHT, GREEN, GREEN_DARK, GREEN_NORMAL, MAGENTA, RED,
    RED_DARK, RED_LIGHT, WHITE, format_trace, parse_trace, write_png,
};

fn pietrs() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pietrs"));
    for name in ["PIETRS_DUMP_GRAPH", "PIETRS_DUMP_CLIF", "PIETRS_JIT_DISASM"] {
        command.env_remove(name);
    }
    command
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
fn a_bare_invocation_prints_help_and_succeeds() {
    let output = run_ok(&mut pietrs());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: pietrs"), "{stdout}");
    assert!(stdout.contains("selftest"), "{stdout}");
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
    let path = write_png(dir.path(), "four.png", 4, 4, &rgba);
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
fn the_graph_dump_switch_writes_the_graph_to_stderr() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, GREEN].into_iter().flatten().collect();
    let path = write_png(dir.path(), "two.png", 2, 1, &rgba);
    let output = run_ok(
        pietrs()
            .env("PIETRS_DUMP_GRAPH", "1")
            .args(["check", path.to_str().unwrap()]),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("graph: states=3 edges=3"), "{stderr}");
    assert!(
        stderr.contains("state: block=0 dp=Right cc=Left"),
        "{stderr}"
    );
    assert!(
        stderr.contains("edge: command=Divide branch=deterministic"),
        "{stderr}"
    );
}

#[test]
fn the_graph_dump_is_off_unless_set() {
    let dir = TempDir::new().unwrap();
    let path = write_png(dir.path(), "one.png", 1, 1, &RED);
    let output = run_ok(pietrs().args(["check", path.to_str().unwrap()]));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
}

#[cfg(feature = "codegen")]
#[test]
fn the_clif_dump_switch_writes_the_lowered_function() {
    let dir = TempDir::new().unwrap();
    let path = write_png(dir.path(), "one.png", 1, 1, &RED);
    let output = run_ok(pietrs().env("PIETRS_DUMP_CLIF", "1").args([
        "run",
        path.to_str().unwrap(),
        "--max-steps",
        "1",
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pietrs: clif"), "{stderr}");
    assert!(stderr.contains("function"), "{stderr}");
    assert!(stderr.contains("block0:"), "{stderr}");
}

#[cfg(feature = "codegen")]
#[test]
fn run_uses_the_jit() {
    // `run` is the JIT path, so the disassembly exists there and nowhere else.
    let dir = TempDir::new().unwrap();
    let path = write_png(dir.path(), "one.png", 1, 1, &RED);
    let output = run_ok(pietrs().env("PIETRS_JIT_DISASM", "1").args([
        "run",
        path.to_str().unwrap(),
        "--max-steps",
        "1",
    ]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pietrs: jit disassembly"), "{stderr}");
    assert!(stderr.contains("block0:"), "{stderr}");
}

#[test]
fn a_single_block_terminates() {
    let dir = TempDir::new().unwrap();
    let path = write_png(dir.path(), "one.png", 1, 1, &RED);
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
    let path = write_png(dir.path(), "a.png", 67, 1, &rgba);
    let output = pietrs()
        .args(["run", path.to_str().unwrap(), "--max-steps", "2"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "A");
    assert!(!output.status.success(), "the step limit is a failure");
    assert!(String::from_utf8_lossy(&output.stderr).contains("step limit"));
}

#[test]
fn run_reads_standard_input_when_no_file_is_given() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("pointer.png");
    std::fs::write(
        &path,
        include_bytes!("../../xtask/fixtures/pointer/program.png"),
    )
    .unwrap();
    let mut child = pietrs()
        .args(["run", path.to_str().unwrap(), "--max-steps", "5000"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"A0").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "A");
    assert!(!output.status.success(), "the step limit is a failure");
}

#[test]
fn run_continues_from_standard_input_after_the_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("subtract.png");
    std::fs::write(
        &path,
        include_bytes!("../../xtask/fixtures/subtract/program.png"),
    )
    .unwrap();
    let input = dir.path().join("first.txt");
    std::fs::write(&input, "10 ").unwrap();
    let mut child = pietrs()
        .args([
            "run",
            path.to_str().unwrap(),
            "--input",
            input.to_str().unwrap(),
            "--max-steps",
            "200000",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"3").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7");
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
    let path = write_png(dir.path(), "a.png", 67, 1, &rgba);
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
fn trace_matches_every_corpus_golden() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../xtask/fixtures");
    let mut cases: Vec<std::path::PathBuf> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("program.png").is_file())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no corpus cases");

    for case in cases {
        let name = case.file_name().unwrap().to_string_lossy().into_owned();
        let program = case.join("program.png");
        let expected = std::fs::read(case.join("expected.trace"))
            .unwrap_or_else(|error| panic!("{name}: cannot read expected.trace: {error}"));
        let expected_text = String::from_utf8(expected.clone()).unwrap();
        let lines = parse_trace(&expected_text)
            .unwrap_or_else(|error| panic!("{name}: invalid expected.trace: {error}"));
        assert_eq!(
            format_trace(&lines),
            expected_text,
            "{name}: expected.trace does not round-trip"
        );
        let max_steps = std::fs::read_to_string(case.join("max_steps"))
            .map_or_else(|_| "100000".to_owned(), |text| text.trim().to_owned());
        let output = run_ok(pietrs().args([
            "trace",
            program.to_str().unwrap(),
            "--max-steps",
            &max_steps,
        ]));
        assert_eq!(output.stdout, expected, "trace differs for {name}");
    }
}

#[test]
fn the_codel_size_override_is_honoured() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, GREEN, BLUE, WHITE].into_iter().flatten().collect();
    let path = write_png(dir.path(), "pixels.png", 2, 2, &rgba);
    let output = pietrs()
        .args(["check", path.to_str().unwrap(), "--codel-size", "2"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a single colour"));
}

#[test]
fn a_malformed_image_is_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let garbage = dir.path().join("garbage.png");
    std::fs::write(&garbage, b"not an image").unwrap();
    let truncated = dir.path().join("truncated.png");
    std::fs::write(&truncated, b"\x89PNG\r\n\x1a\nnot a png").unwrap();

    for (path, expected) in [
        (&garbage, "neither PNG nor GIF"),
        (&truncated, "PNG decoding failed"),
    ] {
        let output = pietrs()
            .args(["check", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{} succeeded", path.display());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
    write_png(dir.path(), "a.png", 67, 1, &rgba);
    write_png(dir.path(), "single.png", 1, 1, &RED);
    let subtract: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    write_png(dir.path(), "subtract.png", 5, 1, &subtract);
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
fn build_help_names_the_cross_targets() {
    let output = run_ok(pietrs().args(["build", "--help"]));
    let text = String::from_utf8_lossy(&output.stdout);
    for target in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
        assert!(text.contains(target), "{text}");
    }
}

#[test]
fn check_reports_a_program_that_cannot_terminate() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, GREEN].into_iter().flatten().collect();
    let image = write_png(dir.path(), "loop.png", 2, 1, &rgba);
    let output = run_ok(pietrs().args(["check", image.to_str().unwrap()]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("termination: never"), "{stdout}");
}

#[test]
fn check_warns_about_a_non_standard_colour() {
    let dir = TempDir::new().unwrap();
    let strange = [176, 36, 240, 255];
    let rgba: Vec<u8> = [RED, strange].into_iter().flatten().collect();
    let image = write_png(dir.path(), "strange.png", 2, 1, &rgba);
    let output = run_ok(pietrs().args(["check", image.to_str().unwrap()]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("1 codels use 1 non-standard colours"),
        "{stderr}"
    );
    assert!(stderr.contains("#b024f0 x1"), "{stderr}");
}

#[cfg(feature = "codegen")]
#[test]
fn build_warns_when_the_program_cannot_terminate() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, GREEN].into_iter().flatten().collect();
    let image = write_png(dir.path(), "loop.png", 2, 1, &rgba);
    let output = dir.path().join("loop");
    let run = run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]));
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("no terminating state"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[cfg(feature = "codegen")]
#[test]
fn run_is_quiet_about_termination_when_bounded() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, GREEN].into_iter().flatten().collect();
    let image = write_png(dir.path(), "loop.png", 2, 1, &rgba);
    let run = pietrs()
        .args(["run", image.to_str().unwrap(), "--max-steps", "5"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&run.stderr).contains("no terminating state"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[cfg(feature = "codegen")]
#[test]
fn build_produces_a_runnable_executable() {
    let dir = TempDir::new().unwrap();
    let mut rgba = Vec::new();
    for _ in 0..65 {
        rgba.extend_from_slice(&RED);
    }
    rgba.extend_from_slice(&RED_DARK);
    rgba.extend_from_slice(&MAGENTA);
    let image = write_png(dir.path(), "a.png", 67, 1, &rgba);
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

/// Reads `e_machine` from an ELF header (offset 18, two bytes, little endian).
#[cfg(feature = "codegen")]
fn elf_machine(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"\x7fELF", "not an ELF file");
    u16::from_le_bytes([bytes[18], bytes[19]])
}

#[cfg(feature = "codegen")]
#[test]
fn build_for_x86_64_linux_writes_an_x86_64_elf() {
    let dir = TempDir::new().unwrap();
    let image = write_png(dir.path(), "one.png", 1, 1, &RED);
    let output = dir.path().join("one");
    run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--target",
        "x86_64-unknown-linux-gnu",
    ]));

    assert!(output.is_file(), "the executable has no .exe suffix");
    assert_eq!(elf_machine(&output), 62, "e_machine is x86-64");
}

#[cfg(feature = "codegen")]
#[test]
fn build_for_aarch64_linux_writes_an_aarch64_elf() {
    let dir = TempDir::new().unwrap();
    let image = write_png(dir.path(), "one.png", 1, 1, &RED);
    let output = dir.path().join("one");
    run_ok(pietrs().args([
        "build",
        image.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--target",
        "aarch64-unknown-linux-gnu",
    ]));

    assert!(output.is_file(), "the executable has no .exe suffix");
    assert_eq!(elf_machine(&output), 183, "e_machine is AArch64");
}

#[cfg(feature = "codegen")]
#[test]
fn an_unknown_target_is_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let image = write_png(dir.path(), "one.png", 1, 1, &RED);
    let run = pietrs()
        .args([
            "build",
            image.to_str().unwrap(),
            "-o",
            dir.path().join("one").to_str().unwrap(),
            "--target",
            "riscv64-unknown-none",
        ])
        .output()
        .unwrap();
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("the AOT compiler does not support"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[cfg(feature = "codegen")]
#[test]
fn a_built_executable_reads_input() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    let image = write_png(dir.path(), "subtract.png", 5, 1, &rgba);
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

#[cfg(feature = "codegen")]
#[test]
fn a_built_executable_chains_a_record_file_then_stdin() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    let image = write_png(dir.path(), "chained.png", 5, 1, &rgba);
    let output = dir.path().join("chained");
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
        .join(format!("chained{}", std::env::consts::EXE_SUFFIX));
    // A walker's record stream has no trailing newline, so the tenth of the file
    // and the third typed at the terminal must stay two numbers.
    let records = dir.path().join("chained.input");
    std::fs::write(&records, "10").unwrap();
    let typed = dir.path().join("chained.typed");
    std::fs::write(&typed, "3").unwrap();
    let run = Command::new(&executable)
        .arg(&records)
        .stdin(Stdio::from(std::fs::File::open(&typed).unwrap()))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "7");
}

#[cfg(feature = "codegen")]
#[test]
fn a_built_executable_takes_its_step_budget_from_the_command_line() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, CYAN_LIGHT, GREEN_NORMAL]
        .into_iter()
        .flatten()
        .collect();
    let image = write_png(dir.path(), "budget.png", 5, 1, &rgba);
    let output = dir.path().join("budget");
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
        .join(format!("budget{}", std::env::consts::EXE_SUFFIX));
    let input = dir.path().join("budget.in");
    std::fs::write(&input, "10 3").unwrap();

    // The build baked four steps. One stops the run before it can print.
    let stopped = Command::new(&executable)
        .args(["--max-steps", "1"])
        .stdin(Stdio::from(std::fs::File::open(&input).unwrap()))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&stopped.stdout), "");
    assert!(
        String::from_utf8_lossy(&stopped.stderr).contains("step limit reached after 1 steps"),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );

    // A budget beyond the baked one runs to the end.
    let finished = Command::new(&executable)
        .arg("--max-steps=100")
        .stdin(Stdio::from(std::fs::File::open(&input).unwrap()))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&finished.stdout), "7");
}

#[cfg(feature = "codegen")]
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
    let image = write_png(dir.path(), "roll.png", 10, 1, &rgba);
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

#[cfg(feature = "codegen")]
#[test]
fn a_built_executable_round_trips_utf8() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, MAGENTA, BLUE_LIGHT].into_iter().flatten().collect();
    let image = write_png(dir.path(), "echo.png", 3, 1, &rgba);
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
fn selftest_runs_the_checked_in_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../xtask/fixtures");
    let output = run_ok(pietrs().args(["selftest", corpus.to_str().unwrap()]));
    let stdout = String::from_utf8_lossy(&output.stdout);
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
        assert!(stdout.contains(&format!("PASS: {name}")), "{stdout}");
    }
}

#[test]
fn the_division_flag_selects_the_rounding_on_run() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, BLUE_DARK, CYAN_LIGHT]
        .into_iter()
        .flatten()
        .collect();
    let image = write_png(dir.path(), "divide.png", 5, 1, &rgba);
    let input = dir.path().join("divide.in");
    std::fs::write(&input, "-7 3").unwrap();

    for (mode, expected) in [("floored", "-3"), ("truncating", "-2")] {
        let run = pietrs()
            .args([
                "run",
                image.to_str().unwrap(),
                "--input",
                input.to_str().unwrap(),
                "--max-steps",
                "4",
                "--division",
                mode,
            ])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{mode}");
    }
}

#[cfg(feature = "codegen")]
#[test]
fn the_division_flag_selects_the_rounding_on_build() {
    let dir = TempDir::new().unwrap();
    let rgba: Vec<u8> = [RED, BLUE_LIGHT, GREEN_DARK, BLUE_DARK, CYAN_LIGHT]
        .into_iter()
        .flatten()
        .collect();
    let image = write_png(dir.path(), "divide.png", 5, 1, &rgba);
    let input = dir.path().join("divide.in");
    std::fs::write(&input, "-7 3").unwrap();

    for (mode, expected) in [("floored", "-3"), ("truncating", "-2")] {
        let output = dir.path().join(format!("divide-{mode}"));
        run_ok(pietrs().args([
            "build",
            image.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
            "--max-steps",
            "4",
            "--division",
            mode,
        ]));
        let executable = dir
            .path()
            .join(format!("divide-{mode}{}", std::env::consts::EXE_SUFFIX));
        let run = Command::new(&executable)
            .stdin(Stdio::from(std::fs::File::open(&input).unwrap()))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{mode}");
    }
}

#[cfg(not(feature = "codegen"))]
#[test]
fn build_reports_the_missing_feature() {
    let dir = TempDir::new().unwrap();
    let image = write_png(dir.path(), "one.png", 1, 1, &RED);
    let run = pietrs()
        .args(["build", image.to_str().unwrap(), "-o", "out"])
        .output()
        .unwrap();
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("without the codegen feature"));
}

#[test]
fn editor_is_listed_in_help() {
    let output = run_ok(pietrs().args(["editor", "--help"]));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Open the visual editor"), "{stdout}");
}

#[cfg(feature = "editor")]
#[test]
fn a_bad_editor_image_fails_before_a_window_opens() {
    let missing = "this-image-does-not-exist.png";
    let output = pietrs().args(["editor", missing]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot read"), "{stderr}");
    assert!(stderr.contains(missing), "{stderr}");
}

#[cfg(not(feature = "editor"))]
#[test]
fn editor_without_the_feature_is_a_diagnostic() {
    let output = pietrs().arg("editor").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("built without the editor feature"),
        "{stderr}"
    );
}
