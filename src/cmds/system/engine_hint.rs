//! Turns a wrapped tool's "unrecognized flag" error into actionable rtk guidance.
//!
//! RTK runs the engine the agent actually invoked and never substitutes one for
//! the other (`rtk grep` runs grep, `rtk rg` runs rg — see `Engine`). That is
//! deliberate, but it means typing a ripgrep flag at `rtk grep` produces a raw
//! BSD grep usage dump:
//!
//! ```text
//! grep: unrecognized option `--glob'
//! usage: grep [-abcdDEFGHhIiJLlMmnOopqRSsUVvwXxZz] [-A num] ...
//! ```
//!
//! A reader who sees that concludes "rtk is broken" and reaches for
//! `RTK_DISABLED=1`, throwing away every saving rtk provides. The confusing
//! error is what causes the nuclear opt-out, so the error is what we fix.
//!
//! The engine's own stderr and exit code are always preserved — the hint is
//! appended, never substituted. Truth first, guidance after.

/// Flags that only ripgrep understands. Deliberately conservative: a flag that
/// exists in both vocabularies with *different* meanings (`-r`, `-z`, `-L`) is
/// left out, because naming it would be worse than saying nothing.
const RG_ONLY_FLAGS: &[&str] = &[
    "--glob",
    "-g",
    "--iglob",
    "--type",
    "-t",
    "--type-not",
    "--type-add",
    "--type-list",
    "--files",
    "--hidden",
    "--no-ignore",
    "--no-ignore-vcs",
    "--no-ignore-dot",
    "--no-ignore-parent",
    "--ignore-file",
    "--max-depth",
    "--max-filesize",
    "--max-columns",
    "--json",
    "--pcre2",
    "--engine",
    "--sort",
    "--sortr",
    "--stats",
    "--vimgrep",
    "--heading",
    "--no-heading",
    "--column",
    "--smart-case",
    "-S",
    "--case-sensitive",
    "--trim",
    "--replace",
    "--search-zip",
    "--threads",
    "--mmap",
    "--no-mmap",
    "--crlf",
    "--follow",
    "--one-file-system",
    "--path-separator",
    "--context-separator",
    "--field-match-separator",
    "--no-require-git",
    "--pre",
];

/// Flags that only grep understands, for the mirror-image hint on `rtk rg`.
/// Same conservatism applies.
const GREP_ONLY_FLAGS: &[&str] = &[
    "--recursive",
    "-R",
    "--include",
    "--exclude",
    "--exclude-dir",
    "--exclude-from",
    "--directories",
    "-d",
    "--devices",
    "-D",
    "--group-separator",
    "--no-group-separator",
    "--label",
    "--binary-files",
    "--basic-regexp",
    "-G",
    "--initial-tab",
];

/// How to achieve the same thing *without leaving the tool the user invoked*.
///
/// The primary hint already says "switch tools". This is the second option —
/// stay put and use the native spelling — so it must be phrased for the invoked
/// tool, not the other one. Absence means there is no clean mapping, in which
/// case we say nothing rather than invent a translation.
const EQUIVALENTS: &[(&str, &str)] = &[
    // rg-only flags typed at `rtk grep` → the grep spelling.
    ("--glob", "--include=<pattern> / --exclude=<pattern>"),
    ("-g", "--include=<pattern> / --exclude=<pattern>"),
    ("--iglob", "--include=<pattern> (grep globs are case-sensitive)"),
    ("--max-depth", "no grep equivalent; use find -maxdepth ... -exec grep"),
    // grep-only flags typed at `rtk rg` → the ripgrep spelling.
    ("--include", "--glob '<pattern>'"),
    ("--exclude", "--glob '!<pattern>'"),
    ("--exclude-dir", "--glob '!<dir>/**'"),
    ("--recursive", "nothing — ripgrep recurses by default"),
    ("-R", "nothing — ripgrep recurses by default"),
];

/// Which tool a flag belongs to.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FlagOwner {
    Rg,
    Grep,
    /// Recognised by neither vocabulary — most likely a genuine typo.
    Unknown,
}

pub fn flag_owner(flag: &str) -> FlagOwner {
    // Normalise `--type=rust` to `--type` so a value-carrying form still matches.
    let bare = flag.split('=').next().unwrap_or(flag);
    if RG_ONLY_FLAGS.contains(&bare) {
        FlagOwner::Rg
    } else if GREP_ONLY_FLAGS.contains(&bare) {
        FlagOwner::Grep
    } else {
        FlagOwner::Unknown
    }
}

/// Pull the offending flag out of an engine's stderr.
///
/// Handles the three shapes seen in the wild:
///   BSD grep : ``grep: unrecognized option `--glob'``
///   GNU grep : `grep: unrecognized option '--glob'` / `grep: invalid option -- 'Q'`
///   ripgrep  : `rg: unrecognized flag --glob`
///
/// Returns the flag with its leading dashes, e.g. `--glob` or `-Q`.
pub fn extract_unrecognized_flag(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        let line = line.trim();

        // "... unrecognized option `--glob'" / "'--glob'" / "--glob"
        // "... unrecognized flag --glob"
        for marker in ["unrecognized option", "unrecognized flag", "unknown option"] {
            if let Some(rest) = line.split_once(marker).map(|(_, r)| r) {
                if let Some(flag) = first_flag_token(rest) {
                    return Some(flag);
                }
            }
        }

        // GNU/BSD short-flag form: "grep: invalid option -- 'Q'" or "-- Q"
        for marker in ["invalid option --", "illegal option --"] {
            if let Some(rest) = line.split_once(marker).map(|(_, r)| r) {
                let ch = rest.trim().trim_matches(|c| c == '\'' || c == '`');
                if let Some(first) = ch.chars().next() {
                    if first.is_ascii_alphanumeric() {
                        return Some(format!("-{first}"));
                    }
                }
            }
        }
    }
    None
}

/// First dash-prefixed token in `s`, stripped of surrounding quotes/backticks.
fn first_flag_token(s: &str) -> Option<String> {
    s.split_whitespace()
        .map(|tok| tok.trim_matches(|c| c == '\'' || c == '`' || c == '"' || c == ','))
        .find(|tok| tok.starts_with('-') && tok.len() > 1)
        .map(str::to_string)
}

/// Build the rtk-level hint for a rejected flag.
///
/// `invoked` is the rtk subcommand the user ran (`grep` or `rg`); `engine` is
/// the binary rtk executed on their behalf — the same thing today, but passed
/// explicitly so the message never lies if that ever stops being true.
pub fn hint_lines(invoked: &str, engine: &str, flag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let owner = flag_owner(flag);

    match (owner, invoked) {
        (FlagOwner::Rg, "grep") => {
            out.push(format!(
                "[rtk] '{flag}' is a ripgrep flag; 'rtk grep' runs {engine}. \
                 Use 'rtk rg {flag} ...' instead."
            ));
        }
        (FlagOwner::Grep, "rg") => {
            out.push(format!(
                "[rtk] '{flag}' is a grep flag; 'rtk rg' runs {engine}. \
                 Use 'rtk grep {flag} ...' instead."
            ));
        }
        _ => {
            out.push(format!(
                "[rtk] {engine} rejected '{flag}'. rtk ran {engine} verbatim — \
                 the error above is {engine}'s own, not an rtk failure."
            ));
        }
    }

    if let Some((_, equivalent)) = EQUIVALENTS.iter().find(|(f, _)| *f == flag) {
        out.push(format!(
            "[rtk] Or stay in '{invoked}' and use: {equivalent}"
        ));
    }

    out.push(format!(
        "[rtk] rtk did not alter your flags. To stop rtk wrapping this one command, \
         add exclude_commands = [\"{invoked}\"] to config.toml — \
         you do not need RTK_DISABLED=1."
    ));

    out
}

/// Append the hint to `stderr` if it carries an unrecognized-flag error.
/// Returns the text to print — the engine's stderr is always preserved verbatim.
pub fn annotate_stderr(invoked: &str, engine: &str, stderr: &str) -> String {
    let Some(flag) = extract_unrecognized_flag(stderr) else {
        return stderr.to_string();
    };
    let mut out = stderr.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    for line in hint_lines(invoked, engine, &flag) {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- flag extraction across the three real-world error shapes ---

    #[test]
    fn extracts_bsd_grep_backtick_form() {
        // Verbatim from macOS: grep: unrecognized option `--glob'
        assert_eq!(
            extract_unrecognized_flag("grep: unrecognized option `--glob'"),
            Some("--glob".to_string())
        );
    }

    #[test]
    fn extracts_gnu_grep_quoted_form() {
        assert_eq!(
            extract_unrecognized_flag("grep: unrecognized option '--glob'"),
            Some("--glob".to_string())
        );
    }

    #[test]
    fn extracts_ripgrep_form() {
        // Verbatim from rg 14: rg: unrecognized flag --recursive
        assert_eq!(
            extract_unrecognized_flag("rg: unrecognized flag --recursive"),
            Some("--recursive".to_string())
        );
    }

    #[test]
    fn extracts_short_flag_forms() {
        assert_eq!(
            extract_unrecognized_flag("grep: invalid option -- Q"),
            Some("-Q".to_string())
        );
        assert_eq!(
            extract_unrecognized_flag("grep: invalid option -- 'Q'"),
            Some("-Q".to_string())
        );
        assert_eq!(
            extract_unrecognized_flag("rg: unrecognized flag -Z"),
            Some("-Z".to_string())
        );
    }

    #[test]
    fn ignores_stderr_without_a_flag_error() {
        assert_eq!(extract_unrecognized_flag(""), None);
        assert_eq!(
            extract_unrecognized_flag("grep: /etc/shadow: Permission denied"),
            None
        );
        // A real no-match run must not be mistaken for a flag problem.
        assert_eq!(extract_unrecognized_flag("rg: No files were searched"), None);
    }

    #[test]
    fn finds_the_flag_on_a_multi_line_usage_dump() {
        let stderr = "grep: unrecognized option `--glob'\n\
                      usage: grep [-abcdDEFGHhIiJLlMmnOopqRSsUVvwXxZz] [-A num]\n";
        assert_eq!(
            extract_unrecognized_flag(stderr),
            Some("--glob".to_string())
        );
    }

    // --- vocabulary ownership ---

    #[test]
    fn classifies_rg_only_flags() {
        for f in ["--glob", "-g", "--type", "-t", "--files", "--hidden", "--json"] {
            assert_eq!(flag_owner(f), FlagOwner::Rg, "{f} should be rg-only");
        }
    }

    #[test]
    fn classifies_grep_only_flags() {
        for f in ["--recursive", "--include", "--exclude", "--exclude-dir"] {
            assert_eq!(flag_owner(f), FlagOwner::Grep, "{f} should be grep-only");
        }
    }

    #[test]
    fn value_carrying_form_still_classifies() {
        assert_eq!(flag_owner("--type=rust"), FlagOwner::Rg);
        assert_eq!(flag_owner("--include=*.c"), FlagOwner::Grep);
    }

    /// Flags whose meaning differs between the two tools must stay Unknown —
    /// a confidently wrong "use rg instead" is worse than a generic hint.
    #[test]
    fn ambiguous_flags_are_not_claimed_by_either_tool() {
        // -r/-z/-L/-T mean different things in each tool; -i/-v/-n/-P are shared.
        for f in ["-r", "-z", "-L", "-T", "-i", "-v", "-n", "-P"] {
            assert_eq!(flag_owner(f), FlagOwner::Unknown, "{f} must stay ambiguous");
        }
    }

    // --- the hint itself ---

    #[test]
    fn rg_flag_at_grep_names_the_flag_and_the_fix() {
        let lines = hint_lines("grep", "grep", "--glob");
        let joined = lines.join("\n");
        assert!(joined.contains("'--glob' is a ripgrep flag"), "{joined}");
        assert!(joined.contains("rtk rg --glob"), "{joined}");
        // Unmistakably rtk-authored.
        assert!(lines.iter().all(|l| l.starts_with("[rtk]")), "{joined}");
    }

    #[test]
    fn grep_flag_at_rg_gives_the_mirror_image_hint() {
        let joined = hint_lines("rg", "rg", "--include").join("\n");
        assert!(joined.contains("'--include' is a grep flag"), "{joined}");
        assert!(joined.contains("rtk grep --include"), "{joined}");
        // ...and the stay-in-rg alternative, since --include maps cleanly.
        assert!(joined.contains("Or stay in 'rg' and use: --glob"), "{joined}");
    }

    #[test]
    fn unknown_flag_does_not_claim_it_belongs_to_the_other_tool() {
        let joined = hint_lines("grep", "grep", "--totally-made-up").join("\n");
        assert!(!joined.contains("ripgrep flag"), "{joined}");
        assert!(joined.contains("not an rtk failure"), "{joined}");
    }

    /// Every hint must point at the granular opt-out, never at RTK_DISABLED=1.
    /// This is the whole point: the reader's next move should be narrowing, not
    /// switching rtk off wholesale.
    #[test]
    fn every_hint_advertises_the_granular_opt_out() {
        for (invoked, flag) in [
            ("grep", "--glob"),
            ("rg", "--include"),
            ("grep", "--nonsense"),
        ] {
            let joined = hint_lines(invoked, invoked, flag).join("\n");
            assert!(
                joined.contains(&format!("exclude_commands = [\"{invoked}\"]")),
                "{joined}"
            );
            assert!(
                joined.contains("you do not need RTK_DISABLED=1"),
                "{joined}"
            );
        }
    }

    // --- annotate_stderr: preserve the truth, append the guidance ---

    /// The exact reproducer: rtk grep -ln "pattern" --glob '*.toml' .
    #[test]
    fn annotates_the_reported_reproducer_without_swallowing_stderr() {
        let engine_stderr = "grep: unrecognized option `--glob'\n\
                             usage: grep [-abcdDEFGHhIiJLlMmnOopqRSsUVvwXxZz] [-A num]\n";
        let out = annotate_stderr("grep", "grep", engine_stderr);

        // The engine's own output survives verbatim.
        assert!(out.starts_with(engine_stderr), "{out}");
        // ...and the actionable hint follows.
        assert!(out.contains("[rtk] '--glob' is a ripgrep flag"), "{out}");
        assert!(out.contains("rtk rg --glob"), "{out}");
    }

    #[test]
    fn leaves_unrelated_stderr_completely_untouched() {
        let stderr = "grep: /etc/shadow: Permission denied\n";
        assert_eq!(annotate_stderr("grep", "grep", stderr), stderr);
        assert_eq!(annotate_stderr("grep", "grep", ""), "");
    }

    #[test]
    fn adds_a_trailing_newline_before_the_hint_when_missing() {
        // BSD grep's form is backtick-open, quote-close: `--glob'
        let out = annotate_stderr("grep", "grep", "grep: unrecognized option `--glob'");
        assert!(out.contains("`--glob'\n[rtk]"), "{out}");
    }
}
