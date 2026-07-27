//! Translates a raw shell command into its RTK-optimized equivalent.

use super::permissions::{check_command, PermissionVerdict};
use crate::discover::registry;
use std::io::Write;

/// Run the `rtk rewrite` command.
///
/// Prints the RTK-rewritten command to stdout and exits with a code that tells
/// the caller how to handle permissions:
///
/// | Exit | Stdout   | Meaning                                                      |
/// |------|----------|--------------------------------------------------------------|
/// | 0    | rewritten| Rewrite allowed — hook may auto-allow the rewritten command. |
/// | 1    | (none)   | No RTK equivalent — hook passes through unchanged.           |
/// | 2    | (none)   | Deny rule matched — hook defers to Claude Code native deny.  |
/// | 3    | rewritten| Ask rule matched — hook rewrites but lets Claude Code prompt.|
pub fn run(cmd: &str) -> anyhow::Result<()> {
    let (excluded, transparent_prefixes) = crate::core::config::Config::load()
        .map(|c| (c.hooks.exclude_commands, c.hooks.transparent_prefixes))
        .unwrap_or_default();

    match evaluate(cmd, &excluded, &transparent_prefixes) {
        RewriteOutcome::Allow(rewritten) => {
            print!(
                "{}",
                rewrite_for_invoked_binary(&rewritten, &invoked_binary_name())
            );
            let _ = std::io::stdout().flush();
            Ok(())
        }
        RewriteOutcome::Ask(rewritten) => {
            print!(
                "{}",
                rewrite_for_invoked_binary(&rewritten, &invoked_binary_name())
            );
            let _ = std::io::stdout().flush();
            std::process::exit(3);
        }
        RewriteOutcome::Deny => std::process::exit(2),
        RewriteOutcome::Passthrough => std::process::exit(1),
    }
}

fn invoked_binary_name() -> String {
    let runtime_name = std::env::args_os()
        .next()
        .as_ref()
        .and_then(|path| std::path::Path::new(path).file_stem())
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| {
            std::env::current_exe().ok().and_then(|path| {
                path.file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "rtk".to_string());

    let package_name = env!("CARGO_PKG_NAME");
    if runtime_name == "rtk" && package_name != "rtk" {
        package_name.to_string()
    } else {
        runtime_name
    }
}

fn rewrite_for_invoked_binary(rewritten: &str, binary_name: &str) -> String {
    if binary_name == "rtk" {
        return rewritten.to_string();
    }

    let tokens = crate::discover::lexer::tokenize(rewritten);
    let mut output = String::with_capacity(rewritten.len() + binary_name.len());
    let mut cursor = 0;
    let mut expect_command = true;

    for token in tokens {
        if token.offset < cursor {
            continue;
        }
        output.push_str(&rewritten[cursor..token.offset]);

        match token.kind {
            crate::discover::lexer::TokenKind::Operator
            | crate::discover::lexer::TokenKind::Pipe(_) => {
                output.push_str(&token.value);
                expect_command = true;
            }
            crate::discover::lexer::TokenKind::Shellism if token.value == "&" => {
                output.push_str(&token.value);
                expect_command = true;
            }
            crate::discover::lexer::TokenKind::Arg if expect_command && token.value == "rtk" => {
                output.push_str(binary_name);
                expect_command = false;
            }
            crate::discover::lexer::TokenKind::Arg if expect_command => {
                output.push_str(&token.value);
                if !is_env_assignment(&token.value) && !is_shell_prefix_builtin(&token.value) {
                    expect_command = false;
                }
            }
            _ => output.push_str(&token.value),
        }

        cursor = token.offset + token.value.len();
    }

    output.push_str(&rewritten[cursor..]);
    output
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && !name.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn is_shell_prefix_builtin(token: &str) -> bool {
    matches!(
        token,
        "noglob" | "command" | "builtin" | "exec" | "nocorrect"
    )
}

#[derive(Debug, PartialEq)]
enum RewriteOutcome {
    Allow(String),
    Passthrough,
    Deny,
    Ask(String),
}

fn evaluate(cmd: &str, excluded: &[String], transparent_prefixes: &[String]) -> RewriteOutcome {
    let verdict = check_command(cmd);

    if verdict == PermissionVerdict::Deny {
        return RewriteOutcome::Deny;
    }

    if crate::discover::lexer::contains_unattestable_construct(cmd) {
        return RewriteOutcome::Passthrough;
    }

    match registry::rewrite_command(cmd, excluded, transparent_prefixes) {
        Some(rewritten) => match verdict {
            PermissionVerdict::Allow => RewriteOutcome::Allow(rewritten),
            _ => RewriteOutcome::Ask(rewritten),
        },
        None => RewriteOutcome::Passthrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite_command_no_prefixes(cmd: &str) -> Option<String> {
        registry::rewrite_command(cmd, &[], &[])
    }

    #[test]
    fn test_run_supported_command_succeeds() {
        assert!(rewrite_command_no_prefixes("git status").is_some());
    }

    #[test]
    fn test_run_unsupported_returns_none() {
        assert!(rewrite_command_no_prefixes("htop").is_none());
    }

    #[test]
    fn test_run_already_rtk_returns_some() {
        assert_eq!(
            rewrite_command_no_prefixes("rtk git status"),
            Some("rtk git status".into())
        );
    }

    #[test]
    fn test_rewrite_for_invoked_binary_uses_renamed_binary() {
        assert_eq!(
            rewrite_for_invoked_binary("rtk git status --short", "rr-rtk"),
            "rr-rtk git status --short"
        );
    }

    #[test]
    fn test_rewrite_for_invoked_binary_updates_compound_segments() {
        assert_eq!(
            rewrite_for_invoked_binary("rtk cargo test && rtk git status", "rr-rtk"),
            "rr-rtk cargo test && rr-rtk git status"
        );
    }

    #[test]
    fn test_rewrite_for_invoked_binary_preserves_rtk_arguments() {
        assert_eq!(
            rewrite_for_invoked_binary("rtk cargo install rtk", "rr-rtk"),
            "rr-rtk cargo install rtk"
        );
    }

    #[test]
    fn test_rewrite_for_invoked_binary_handles_env_and_shell_prefixes() {
        assert_eq!(
            rewrite_for_invoked_binary("FOO=1 command rtk git status", "rr-rtk"),
            "FOO=1 command rr-rtk git status"
        );
    }

    mod unattestable_passthrough {
        use super::super::{evaluate, RewriteOutcome};

        /// `evaluate` consults the host's Claude permission rules, so a developer
        /// whose real settings already allow `git status` would otherwise see
        /// `Allow` where these tests expect `Ask`. Pin the lookup at an empty
        /// config directory so the outcome depends only on the code under test.
        fn with_empty_claude_config<T>(f: impl FnOnce() -> T) -> T {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

            let tmp = tempfile::tempdir().expect("tempdir");
            let orig = std::env::var_os("CLAUDE_CONFIG_DIR");
            std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path());
            let out = f();
            match orig {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
            out
        }

        #[test]
        fn test_backtick_substitution_passthrough() {
            assert_eq!(
                evaluate("git status `rm -rf /tmp/x`", &[], &[]),
                RewriteOutcome::Passthrough
            );
        }

        #[test]
        fn test_dollar_substitution_passthrough() {
            assert_eq!(
                evaluate("git status $(rm -rf /tmp/x)", &[], &[]),
                RewriteOutcome::Passthrough
            );
        }

        #[test]
        fn test_double_quoted_substitution_passthrough() {
            assert_eq!(
                evaluate("git log --pretty=\"$(rm -rf /tmp/x)\"", &[], &[]),
                RewriteOutcome::Passthrough
            );
        }

        #[test]
        fn test_file_redirect_passthrough() {
            assert_eq!(
                evaluate("git log > /tmp/out.txt", &[], &[]),
                RewriteOutcome::Passthrough
            );
        }

        #[test]
        fn test_fd_dup_redirect_still_rewrites() {
            let outcome = with_empty_claude_config(|| evaluate("git status 2>&1", &[], &[]));
            assert!(matches!(outcome, RewriteOutcome::Ask(_)), "got {outcome:?}");
        }

        #[test]
        fn test_plain_command_still_rewrites() {
            let outcome = with_empty_claude_config(|| evaluate("git status", &[], &[]));
            assert!(matches!(outcome, RewriteOutcome::Ask(_)), "got {outcome:?}");
        }
    }

    /// SECURITY: Verify the exit code protocol for permission verdicts.
    ///
    /// The bash hook (.claude/hooks/rtk-rewrite.sh) interprets exit codes as:
    ///   0 → auto-allow (sets permissionDecision: "allow")
    ///   1 → passthrough (no RTK equivalent)
    ///   2 → deny (let Claude Code handle natively)
    ///   3 → ask (rewrite but omit permissionDecision, forcing user prompt)
    ///
    /// CRITICAL: PermissionVerdict::Default MUST map to exit 3 (ask), NOT exit 0.
    /// If Default were mapped to exit 0, any command without an explicit permission
    /// rule would be auto-allowed — bypassing Claude Code's least-privilege default.
    /// See: https://github.com/rtk-ai/rtk/issues/1155
    mod exit_code_protocol {
        use super::registry;
        use crate::hooks::permissions::{check_command_with_rules, PermissionVerdict};

        /// Exit code that `run()` returns for each verdict:
        ///   Allow  → 0 (exit Ok(()))
        ///   Ask    → 3 (process::exit(3))
        ///   Default→ 3 (process::exit(3)) — grouped with Ask
        ///   Deny   → 2 (process::exit(2)) — handled before rewrite match
        fn expected_exit_code(verdict: &PermissionVerdict) -> i32 {
            match verdict {
                PermissionVerdict::Allow => 0,
                PermissionVerdict::Deny => 2,
                PermissionVerdict::Ask => 3,
                PermissionVerdict::Default => 3, // MUST be 3, not 0!
            }
        }

        #[test]
        fn test_default_verdict_maps_to_ask_exit_code() {
            // When no rules match, verdict is Default → exit code must be 3 (ask).
            let verdict = check_command_with_rules("git status", &[], &[], &[]);
            assert_eq!(verdict, PermissionVerdict::Default);
            assert_eq!(
                expected_exit_code(&verdict),
                3,
                "Default verdict MUST exit with code 3 (ask), not 0 (allow)"
            );
        }

        #[test]
        fn test_allow_verdict_maps_to_allow_exit_code() {
            let allow = vec!["git *".to_string()];
            let verdict = check_command_with_rules("git status", &[], &[], &allow);
            assert_eq!(verdict, PermissionVerdict::Allow);
            assert_eq!(expected_exit_code(&verdict), 0);
        }

        #[test]
        fn test_ask_verdict_maps_to_ask_exit_code() {
            let ask = vec!["git push".to_string()];
            let verdict = check_command_with_rules("git push origin main", &[], &ask, &[]);
            assert_eq!(verdict, PermissionVerdict::Ask);
            assert_eq!(expected_exit_code(&verdict), 3);
        }

        #[test]
        fn test_deny_verdict_maps_to_deny_exit_code() {
            let deny = vec!["rm -rf".to_string()];
            let verdict = check_command_with_rules("rm -rf /tmp/test", &deny, &[], &[]);
            assert_eq!(verdict, PermissionVerdict::Deny);
            assert_eq!(expected_exit_code(&verdict), 2);
        }

        #[test]
        fn test_no_auto_allow_bypass_for_unrecognized_commands() {
            // SECURITY: A command with no permission rules and no matching allow rule
            // must NOT be auto-allowed. This is the core of issue #1155.
            // Even though `git status` can be rewritten to `rtk git status`,
            // the absence of an allow rule means Default → exit 3 → ask.
            let verdict = check_command_with_rules("git status", &[], &[], &[]);
            assert_eq!(verdict, PermissionVerdict::Default);

            // Verify the rewrite exists (so the hook would output it),
            // but the exit code forces user confirmation.
            assert!(registry::rewrite_command("git status", &[], &[]).is_some());
            assert_eq!(expected_exit_code(&verdict), 3);
        }

        #[test]
        fn test_default_never_equals_allow() {
            // Sentinel: ensure Default and Allow are distinct enum variants.
            // If this ever fails, the entire permission model is broken.
            assert_ne!(PermissionVerdict::Default, PermissionVerdict::Allow);
        }
    }
}
