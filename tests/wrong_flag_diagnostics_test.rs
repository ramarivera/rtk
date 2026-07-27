#![cfg(unix)]
//! Diagnostics faithfulness: when a wrapped tool rejects an argument, rtk must
//! make it obvious that the *tool* refused it and what to do next — never a bare
//! usage dump the reader mistakes for "rtk is broken", and never silence.
//!
//! This is the failure mode that drives agents to `RTK_DISABLED=1`, throwing
//! away every saving rtk provides. The engine's own stderr and exit code are
//! always preserved; rtk only ever *appends*.

use std::process::Command;

fn rtk(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rr-rtk"))
        .env("LC_ALL", "C")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("rtk")
}

fn raw(tool: &str, args: &[&str]) -> std::process::Output {
    Command::new(tool)
        .env("LC_ALL", "C")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(args)
        .output()
        .expect("tool")
}

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The exact reported reproducer:
///   $ rr-rtk grep -ln "pattern" --glob '*.toml' .
/// used to emit nothing but a raw BSD grep usage dump.
#[test]
fn rg_flag_at_rtk_grep_gets_an_actionable_rtk_hint() {
    let out = rtk(&["grep", "-ln", "pattern", "--glob", "*.toml", "."]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // grep's own error survives — rtk never swallows the truth.
    assert!(
        stderr.contains("--glob"),
        "engine stderr must be preserved: {stderr}"
    );
    // ...and rtk explains it, unmistakably in rtk's own voice.
    assert!(
        stderr.contains("[rtk] '--glob' is a ripgrep flag"),
        "expected rtk hint naming the flag: {stderr}"
    );
    assert!(
        stderr.contains("rtk rg --glob"),
        "expected the fix to be spelled out: {stderr}"
    );
    // The reader's next move must be narrowing, not the nuclear option.
    assert!(
        stderr.contains("exclude_commands = [\"grep\"]"),
        "expected the granular opt-out: {stderr}"
    );
    assert!(
        stderr.contains("you do not need RTK_DISABLED=1"),
        "expected RTK_DISABLED=1 to be steered away from: {stderr}"
    );
}

/// Exit code is the engine's, untouched — the hint must not mask the failure.
#[test]
fn wrong_flag_preserves_the_engines_exit_code() {
    let mine = rtk(&["grep", "-ln", "pattern", "--glob", "*.toml", "."]);
    let theirs = raw("grep", &["-ln", "pattern", "--glob", "*.toml", "."]);
    assert_eq!(
        mine.status.code(),
        theirs.status.code(),
        "rtk grep must exit exactly as grep did"
    );
}

/// Mirror image: a grep-only flag typed at `rtk rg`.
#[test]
fn grep_flag_at_rtk_rg_gets_the_mirror_hint() {
    if !tool_available("rg") {
        eprintln!("skipping: rg not installed");
        return;
    }
    let out = rtk(&["rg", "--include", "*.toml", "pattern", "."]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("[rtk] '--include' is a grep flag"),
        "expected mirror-image hint: {stderr}"
    );
    assert!(
        stderr.contains("rtk grep --include"),
        "expected the fix to be spelled out: {stderr}"
    );
    assert!(
        stderr.contains("exclude_commands = [\"rg\"]"),
        "expected the granular opt-out scoped to rg: {stderr}"
    );
}

/// A flag belonging to neither tool must not be misattributed.
#[test]
fn unknown_flag_is_not_blamed_on_the_other_tool() {
    let out = rtk(&["grep", "--totally-made-up", "pattern", "."]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !stderr.contains("is a ripgrep flag"),
        "must not invent a tool for an unknown flag: {stderr}"
    );
    assert!(
        stderr.contains("not an rtk failure"),
        "should still clarify rtk is not at fault: {stderr}"
    );
}

/// A successful search must stay clean — no hint, no stderr noise.
#[test]
fn successful_search_emits_no_hint() {
    let out = rtk(&["grep", "-rn", "fn main", "src"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("[rtk] '"),
        "no flag hint should appear on a good run: {stderr}"
    );
}

/// Regression: `rtk wc --bogus` printed *nothing* and exited 1. `filter_stdout_only`
/// captured the tool's stderr and then dropped it, so the failure was silent.
#[test]
fn failing_tool_stderr_is_never_swallowed() {
    let mine = rtk(&["wc", "--bogus", "Cargo.toml"]);
    let theirs = raw("wc", &["--bogus", "Cargo.toml"]);

    assert_eq!(
        mine.status.code(),
        theirs.status.code(),
        "exit code must match raw wc"
    );
    assert!(
        !String::from_utf8_lossy(&mine.stderr).trim().is_empty(),
        "a failing tool must not fail silently"
    );
    // It is wc's own complaint, verbatim — rtk does not paraphrase it.
    assert!(
        String::from_utf8_lossy(&mine.stderr).contains("wc:"),
        "expected wc's own message: {}",
        String::from_utf8_lossy(&mine.stderr)
    );
}

/// The counterpart: a *successful* run must not start leaking stderr chatter,
/// or the compression savings evaporate.
#[test]
fn successful_tool_run_stays_quiet_on_stderr() {
    let out = rtk(&["wc", "-l", "Cargo.toml"]);
    assert!(out.status.success(), "wc -l Cargo.toml should succeed");
    assert!(
        String::from_utf8_lossy(&out.stderr).trim().is_empty(),
        "successful runs must not emit stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
