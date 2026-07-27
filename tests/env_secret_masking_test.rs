#![cfg(unix)]
//! `rtk env` must never print a live credential.
//!
//! This command is run *by agents*, and its output lands verbatim in a
//! transcript that gets stored, shared, and replayed. A development shell
//! routinely holds dozens of live credentials, so an unmasked `rtk env` is a
//! one-command credential dump.
//!
//! Upstream removed masking in `fix(env): clean up feature from secrets
//! rewrite`; this fork deliberately keeps it on by default. These tests are the
//! guard on that divergence — they fail loudly if a future merge drops it again.
//!
//! Every canary value here is synthetic and shaped like the real thing. No test
//! reads the developer's actual environment.

use std::process::Command;

/// Distinctive synthetic secrets — no real credential is ever used.
const CANARY: &str = "sk-ant-CANARY0000SECRET0000VALUE0000DoNotLeak";
const SHORT_CANARY: &str = "hunter2";

fn rtk_env(extra_env: &[(&str, &str)], args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rr-rtk"));
    cmd.env("LC_ALL", "C").arg("env");
    cmd.args(args);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("rtk env");
    // Check both streams: a leak on stderr is still a leak.
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The core guarantee. This test fails on an unmasked build.
#[test]
fn secret_shaped_values_never_appear_in_output() {
    let out = rtk_env(
        &[
            ("RTK_TEST_ANTHROPIC_API_KEY", CANARY),
            ("RTK_TEST_GITHUB_TOKEN", CANARY),
            ("RTK_TEST_DB_PASSWORD", CANARY),
            ("RTK_TEST_CLIENT_SECRET", CANARY),
            ("RTK_TEST_AUTH_HEADER", CANARY),
        ],
        &["--filter", "RTK_TEST_"],
    );

    assert!(
        !out.contains(CANARY),
        "rtk env leaked a secret value verbatim.\nOutput:\n{out}"
    );
    // A truncated preview is still a leak — assert on a substring long enough
    // to be usable but short enough to survive the 50-char preview path.
    assert!(
        !out.contains("CANARY0000SECRET0000VALUE"),
        "rtk env leaked a usable prefix of a secret.\nOutput:\n{out}"
    );
}

/// The variables must still be *listed* — masking is not suppression. An agent
/// needs to know GITHUB_TOKEN is set without learning what it is.
#[test]
fn masked_variables_are_still_listed_with_a_marker() {
    let out = rtk_env(
        &[("RTK_TEST_GITHUB_TOKEN", CANARY)],
        &["--filter", "RTK_TEST_GITHUB_TOKEN"],
    );

    assert!(
        out.contains("RTK_TEST_GITHUB_TOKEN"),
        "the variable name should still be visible: {out}"
    );
    assert!(
        out.contains("****"),
        "expected a masking marker in the output: {out}"
    );
}

/// Short secrets must be replaced wholesale — there is nothing left to redact
/// if you keep two chars of a seven-char password.
#[test]
fn short_secrets_are_fully_replaced() {
    let out = rtk_env(
        &[("RTK_TEST_SHORT_PASSWORD", SHORT_CANARY)],
        &["--filter", "RTK_TEST_SHORT_PASSWORD"],
    );
    assert!(
        !out.contains(SHORT_CANARY),
        "short secret leaked verbatim: {out}"
    );
}

/// Non-secret variables are unaffected — masking must not make `rtk env`
/// useless for ordinary configuration.
#[test]
fn non_secret_values_are_not_masked() {
    let out = rtk_env(
        &[("RTK_TEST_LOG_LEVEL", "debug")],
        &["--filter", "RTK_TEST_LOG_LEVEL"],
    );
    assert!(
        out.contains("debug"),
        "ordinary values should print normally: {out}"
    );
}

/// Masking is the default: it must apply with no flags at all, not only when
/// a filter happens to be passed.
///
/// Uses an `AWS_`-prefixed name so the variable is categorised as a cloud var
/// and therefore actually *appears* in unfiltered output — otherwise the test
/// would pass vacuously against an unmasked build.
#[test]
fn masking_applies_without_any_filter_flag() {
    let out = rtk_env(&[("AWS_SECRET_ACCESS_KEY", CANARY)], &[]);
    assert!(
        out.contains("AWS_SECRET_ACCESS_KEY"),
        "guard: the canary must be listed, or this test proves nothing: {out}"
    );
    assert!(
        !out.contains(CANARY),
        "rtk env with no flags leaked a secret: {out}"
    );
}

/// The reveal escape hatch works, and is opt-in only. If this stops working the
/// flag is dead weight; if it works *without* the flag, the default is broken.
#[test]
fn show_all_is_the_only_way_to_reveal() {
    let masked = rtk_env(
        &[("RTK_TEST_REVEAL_API_KEY", CANARY)],
        &["--filter", "RTK_TEST_REVEAL_API_KEY"],
    );
    let revealed = rtk_env(
        &[("RTK_TEST_REVEAL_API_KEY", CANARY)],
        &["--filter", "RTK_TEST_REVEAL_API_KEY", "--show-all"],
    );

    assert!(!masked.contains(CANARY), "default must mask: {masked}");
    assert!(
        revealed.contains(CANARY),
        "--show-all must reveal, or the escape hatch is broken: {revealed}"
    );
}

/// Case must not matter: lowercase and mixed-case key names are just as
/// dangerous as SCREAMING_CASE ones.
#[test]
fn matching_is_case_insensitive() {
    for key in ["rtk_test_lower_token", "Rtk_Test_Mixed_Secret"] {
        let out = rtk_env(&[(key, CANARY)], &["--filter", "rtk_test_"]);
        assert!(
            !out.contains(CANARY),
            "{key} leaked — key matching must be case-insensitive: {out}"
        );
    }
}
