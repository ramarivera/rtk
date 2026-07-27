//! Filters environment variables, hiding secrets and compacting noise.
//!
//! # Fork divergence: masking is on by default
//!
//! Upstream removed secret masking from this filter in
//! `fix(env): clean up feature from secrets rewrite`. This fork deliberately
//! keeps it. `rtk env` is run *by agents*, and its output lands verbatim in a
//! transcript that gets stored, shared, and replayed. A development shell
//! routinely holds dozens of live credentials, so an unmasked `rtk env` is a
//! one-command credential dump.
//!
//! Masking is therefore the default, disabled only by an explicit
//! per-invocation `--show-all` that a human has to type on purpose. Erring
//! toward over-masking is correct here: a masked value the user wanted is a
//! minor annoyance; a leaked API key is not recoverable.

use crate::core::guard::never_worse;
use crate::core::tracking;
use crate::core::truncate::{CAP_LIST, CAP_WARNINGS};
use anyhow::Result;
use std::collections::HashSet;
use std::env;
use std::fmt::Write;

/// Show filtered environment variables, masking secret-looking values.
///
/// `show_all` reveals values that would otherwise be masked. It is opt-in per
/// invocation and never defaulted on — see the module docs.
pub fn run(filter: Option<&str>, show_all: bool, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if verbose > 0 {
        eprintln!("Environment variables:");
    }

    let sensitive_patterns = get_sensitive_patterns();
    let mut vars: Vec<(String, String)> = env::vars().collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));

    // Interesting categories
    let mut path_vars = Vec::new();
    let mut lang_vars = Vec::new();
    let mut cloud_vars = Vec::new();
    let mut tool_vars = Vec::new();
    let mut other_vars = Vec::new();

    for (key, value) in &vars {
        // Apply filter if provided
        if let Some(f) = filter {
            if !key.to_lowercase().contains(&f.to_lowercase()) {
                continue;
            }
        }

        // Masking wins over truncation: a 50-char "preview" of an API key is
        // still the API key.
        let is_sensitive = sensitive_patterns
            .iter()
            .any(|p| key.to_lowercase().contains(p));

        let display_value = if is_sensitive && !show_all {
            mask_value(value)
        } else if value.len() > 100 {
            let preview: String = value.chars().take(50).collect();
            format!("{}... ({} chars)", preview, value.chars().count())
        } else {
            value.clone()
        };

        let entry = (key.clone(), display_value);

        // Categorize
        if key.contains("PATH") {
            path_vars.push(entry);
        } else if is_lang_var(key) {
            lang_vars.push(entry);
        } else if is_cloud_var(key) {
            cloud_vars.push(entry);
        } else if is_tool_var(key) {
            tool_vars.push(entry);
        } else if filter.is_some() || is_interesting_var(key) {
            other_vars.push(entry);
        }
    }

    let mut body = String::new();
    if !path_vars.is_empty() {
        let _ = writeln!(body, "PATH Variables:");
        for (k, v) in &path_vars {
            if k == "PATH" {
                // Split PATH for readability
                let paths: Vec<&str> = v.split(':').collect();
                let _ = writeln!(body, "  PATH ({} entries):", paths.len());
                const MAX_PATH_ENTRIES: usize = CAP_WARNINGS;
                for p in paths.iter().take(MAX_PATH_ENTRIES) {
                    let _ = writeln!(body, "    {}", p);
                }
                if paths.len() > MAX_PATH_ENTRIES {
                    let _ = writeln!(body, "    ... +{} more", paths.len() - MAX_PATH_ENTRIES);
                }
            } else {
                let _ = writeln!(body, "  {}={}", k, v);
            }
        }
    }

    if !lang_vars.is_empty() {
        let _ = writeln!(body, "\nLanguage/Runtime:");
        for (k, v) in &lang_vars {
            let _ = writeln!(body, "  {}={}", k, v);
        }
    }

    if !cloud_vars.is_empty() {
        let _ = writeln!(body, "\nCloud/Services:");
        for (k, v) in &cloud_vars {
            let _ = writeln!(body, "  {}={}", k, v);
        }
    }

    if !tool_vars.is_empty() {
        let _ = writeln!(body, "\nTools:");
        for (k, v) in &tool_vars {
            let _ = writeln!(body, "  {}={}", k, v);
        }
    }

    if !other_vars.is_empty() {
        const MAX_OTHER_VARS: usize = CAP_LIST;
        let _ = writeln!(body, "\nOther:");
        for (k, v) in other_vars.iter().take(MAX_OTHER_VARS) {
            let _ = writeln!(body, "  {}={}", k, v);
        }
        if other_vars.len() > MAX_OTHER_VARS {
            let _ = writeln!(body, "  ... +{} more", other_vars.len() - MAX_OTHER_VARS);
        }
    }

    let total = vars.len();
    let shown = path_vars.len()
        + lang_vars.len()
        + cloud_vars.len()
        + tool_vars.len()
        + other_vars.len().min(20);
    if filter.is_none() {
        let _ = writeln!(body, "\nTotal: {} vars (showing {} relevant)", total, shown);
    }

    let raw: String = vars.iter().fold(String::new(), |mut output, (k, v)| {
        let _ = writeln!(output, "{}={}", k, v);
        output
    });
    let shown_body = never_worse(&raw, &body);
    print!("{}", shown_body);
    timer.track("env", "rtk env", &raw, shown_body);
    Ok(())
}

/// Substrings that mark a variable name as secret-bearing. Matched
/// case-insensitively against the whole key, so `ANTHROPIC_API_KEY`,
/// `gh_token` and `MY_DB_PASSWORD` all hit.
///
/// This list is intentionally broad. A false positive masks a value someone
/// wanted to see and they re-run with `--show-all`; a false negative prints a
/// live credential into a transcript that is already archived.
fn get_sensitive_patterns() -> HashSet<&'static str> {
    let mut set = HashSet::new();
    for pattern in [
        "key",
        "secret",
        "password",
        "passwd",
        "token",
        "credential",
        "auth",
        "private",
        "api_key",
        "apikey",
        "access_key",
        "jwt",
        "session",
        "cookie",
        "signature",
        "salt",
        "cert",
        "dsn",
        "connection_string",
        "webhook",
        "bearer",
        "pat",
        "pin",
    ] {
        set.insert(pattern);
    }
    set
}

/// Replace a secret with a shape-preserving stub.
///
/// Keeps two leading and two trailing characters so a human can tell two keys
/// apart (`sk****4a` vs `gh****z9`) without the value being usable. Anything
/// four characters or shorter is replaced wholesale — there is nothing left to
/// redact otherwise.
fn mask_value(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 4 {
        "****".to_string()
    } else {
        let prefix: String = chars[..2].iter().collect();
        let suffix: String = chars[chars.len() - 2..].iter().collect();
        format!("{}****{}", prefix, suffix)
    }
}

fn is_lang_var(key: &str) -> bool {
    let patterns = [
        "RUST", "CARGO", "PYTHON", "PIP", "NODE", "NPM", "YARN", "DENO", "BUN", "JAVA", "MAVEN",
        "GRADLE", "GO", "GOPATH", "GOROOT", "RUBY", "GEM", "PERL", "PHP", "DOTNET", "NUGET",
    ];
    patterns.iter().any(|p| key.to_uppercase().contains(p))
}

fn is_cloud_var(key: &str) -> bool {
    let patterns = [
        "AWS",
        "AZURE",
        "GCP",
        "GOOGLE_CLOUD",
        "DOCKER",
        "KUBERNETES",
        "K8S",
        "HELM",
        "TERRAFORM",
        "VAULT",
        "CONSUL",
        "NOMAD",
    ];
    patterns.iter().any(|p| key.to_uppercase().contains(p))
}

fn is_tool_var(key: &str) -> bool {
    let patterns = [
        "EDITOR",
        "VISUAL",
        "SHELL",
        "TERM",
        "GIT",
        "SSH",
        "GPG",
        "BREW",
        "HOMEBREW",
        "XDG",
        "CLAUDE",
        "ANTHROPIC",
    ];
    patterns.iter().any(|p| key.to_uppercase().contains(p))
}

fn is_interesting_var(key: &str) -> bool {
    let patterns = ["HOME", "USER", "LANG", "LC_", "TZ", "PWD", "OLDPWD"];
    patterns.iter().any(|p| key.to_uppercase().starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_lang_var_rust() {
        assert!(is_lang_var("RUST_LOG"));
        assert!(is_lang_var("CARGO_HOME"));
        assert!(is_lang_var("GOPATH"));
        assert!(is_lang_var("NODE_ENV"));
    }

    #[test]
    fn test_is_lang_var_negative() {
        assert!(!is_lang_var("HOME"));
        assert!(!is_lang_var("PATH"));
        assert!(!is_lang_var("USER"));
    }

    #[test]
    fn test_is_cloud_var() {
        assert!(is_cloud_var("AWS_ACCESS_KEY_ID"));
        assert!(is_cloud_var("AZURE_CLIENT_ID"));
        assert!(is_cloud_var("DOCKER_HOST"));
        assert!(is_cloud_var("KUBERNETES_SERVICE_HOST"));
    }

    #[test]
    fn test_is_cloud_var_negative() {
        assert!(!is_cloud_var("HOME"));
        assert!(!is_cloud_var("RUST_LOG"));
    }

    #[test]
    fn test_is_tool_var() {
        assert!(is_tool_var("EDITOR"));
        assert!(is_tool_var("GIT_AUTHOR_NAME"));
        assert!(is_tool_var("SSH_AUTH_SOCK"));
        assert!(is_tool_var("CLAUDE_API_KEY"));
    }

    #[test]
    fn test_is_interesting_var() {
        assert!(is_interesting_var("HOME"));
        assert!(is_interesting_var("USER"));
        assert!(is_interesting_var("LANG"));
        assert!(is_interesting_var("TZ"));
        assert!(is_interesting_var("PWD"));
    }

    #[test]
    fn test_is_interesting_var_negative() {
        assert!(!is_interesting_var("RANDOM_VAR"));
        assert!(!is_interesting_var("MY_CUSTOM_VAR"));
    }
}
