---
title: Configuration
description: Customize RTK behavior via config.toml, environment variables, and per-project filters
sidebar:
  order: 4
---

# Configuration

## Config file location

| Platform | Path |
|----------|------|
| Linux | `~/.config/rtk/config.toml` |
| macOS | `~/Library/Application Support/rtk/config.toml` |

```bash
rtk config            # show current configuration
rtk config --create   # create config file with defaults
```

## Full config structure

```toml
[tracking]
enabled = true              # enable/disable token tracking
history_days = 90           # retention in days (auto-cleanup)
database_path = "/custom/path/history.db"   # optional override

[display]
colors = true               # colored output
emoji = true                # use emojis in output
max_width = 120             # maximum output width

[filters]
# These apply to file-reading commands (ls, find, grep, cat/rtk read).
# Paths matching these patterns are excluded from output, keeping noise low.
ignore_dirs = [".git", "node_modules", "target", "__pycache__", ".venv", "vendor"]
ignore_files = ["*.lock", "*.min.js", "*.min.css"]

[tee]
enabled = true              # save raw output on failure
mode = "failures"           # "failures" (default), "always", "never"
max_files = 20              # rotation: keep last N files
# directory = "/custom/tee/path"  # optional override

[telemetry]
enabled = true              # anonymous daily ping — see Telemetry & Privacy for full details

[hooks]
exclude_commands = []       # commands to never auto-rewrite
```

For full details on what is collected, opt-out options, and GDPR rights, see [Telemetry & Privacy](../resources/telemetry.md).

## Environment variables

| Variable | Description |
|----------|-------------|
| `RTK_DISABLED=1` | Disable RTK for a single command (`RTK_DISABLED=1 git status`) |
| `RTK_TEE_DIR` | Override the tee directory |
| `RTK_TELEMETRY_DISABLED=1` | Disable telemetry |
| `RTK_HOOK_AUDIT=1` | Enable hook audit logging |
| `SKIP_ENV_VALIDATION=1` | Skip env validation (useful with Next.js) |

## Tee system

When a command fails, RTK saves the full raw output to a local file and prints the path:

```
FAILED: 2/15 tests
[full output: ~/.local/share/rtk/tee/1707753600_cargo_test.log]
```

Your AI assistant can then read the file if it needs more detail, without re-running the command.

| Setting | Default | Description |
|---------|---------|-------------|
| `tee.enabled` | `true` | Enable/disable |
| `tee.mode` | `"failures"` | `"failures"`, `"always"`, `"never"` |
| `tee.max_files` | `20` | Rotation: keep last N files |
| Min size | 500 bytes | Outputs shorter than this are not saved |
| Max file size | 1 MB | Truncated above this |

## Excluding commands from auto-rewrite

Prevent specific commands from being rewritten by the hook:

```toml
[hooks]
exclude_commands = ["git rebase", "git cherry-pick", "docker exec"]
```

Patterns match against the full command after stripping env prefixes (`sudo`, `VAR=val`), so `"psql"` excludes both `psql -h localhost` and `PGPASSWORD=x psql -h localhost`.

Subcommand patterns work too: `"git push"` excludes `git push origin main` but not `git status`.

Patterns starting with `^` are treated as regex:

```toml
[hooks]
exclude_commands = ["^curl", "^wget", "git rebase"]
```

Invalid regex patterns fall back to prefix matching.

Or for a single invocation:

```bash
RTK_DISABLED=1 git rebase main
```

### Precedence

Checks run in this order; the first one that says "don't rewrite" wins, and the
original command is passed through untouched:

1. **`RTK_DISABLED=1` in the command's env prefix** — skips this one invocation entirely.
2. **`[hooks] exclude_commands`** — regex (patterns starting with `^`) or prefix match against the command with env prefixes stripped.
3. **Per-command fail-safe** — RTK declines any invocation it cannot reproduce exactly (see below).

**Reach for `exclude_commands` before `RTK_DISABLED=1`.** This is the single
most important line on this page. Excluding one command keeps every other
saving:

```toml
[hooks]
exclude_commands = ["rg"]     # rg untouched; git, grep, cargo, … still filtered
```

`RTK_DISABLED=1` throws all of them away. If one rewrite misbehaves, narrow —
do not switch RTK off wholesale. Every diagnostic RTK prints points back here
for exactly this reason.

### When a tool rejects your flag

RTK runs the tool you named and never substitutes one for another: `rtk grep`
runs grep, `rtk rg` runs ripgrep. Their flag vocabularies overlap but are not
the same, so a ripgrep flag typed at `rtk grep` makes *grep* fail:

```console
$ rtk grep -ln "pattern" --glob '*.toml' .
grep: unrecognized option `--glob'
usage: grep [-abcdDEFGHhIiJLlMmnOopqRSsUVvwXxZz] [-A num] ...
[rtk] '--glob' is a ripgrep flag; 'rtk grep' runs grep. Use 'rtk rg --glob ...' instead.
[rtk] Or stay in 'grep' and use: --include=<pattern> / --exclude=<pattern>
[rtk] rtk did not alter your flags. To stop rtk wrapping this one command,
      add exclude_commands = ["grep"] to config.toml — you do not need RTK_DISABLED=1.
```

The tool's own error and exit code are always preserved — RTK only appends the
`[rtk]`-prefixed lines. The hint works in both directions: a grep-only flag such
as `--include` typed at `rtk rg` gets the mirror-image advice. A flag belonging
to neither tool is not misattributed; RTK just confirms the tool rejected it and
that RTK did not alter your arguments.

A failing tool also never fails *silently*: if a wrapped command exits non-zero
with a message, RTK surfaces that message verbatim even when it otherwise
filters only stdout.

### Fail-safe passthrough

RTK never emits a command it cannot run. When an invocation contains a flag or
predicate RTK does not model, it declines to rewrite and the original command
runs verbatim, with the real tool's exit code forwarded unchanged.

Most RTK subcommands get this for free — they delegate to the underlying binary
and only filter its output. `rtk find` is the exception: it reimplements the
directory walk, so it explicitly checks its argument list and passes through on
anything outside the modelled subset (`-name`, `-iname`, `-type`, `-maxdepth`,
`-print`). Compound predicates (`-not`, `-o`), actions (`-exec`, `-delete`),
time/size predicates (`-newer`, `-size`, `-mtime`, `-perm`, `-regex`), and any
unrecognised flag all pass through to the system `find`.

> **Known divergence.** Within its modelled subset, `rtk find` filters results
> using `.gitignore` and skips hidden entries unless the pattern starts with `.`
> — real `find` does neither. This is deliberate (it is where the token savings
> come from), but it means `rtk find . -maxdepth 1 -type f` can return fewer
> files than `find . -maxdepth 1 -type f`. Use `exclude_commands = ["find"]` if
> you need byte-identical `find` semantics.

## Telemetry

RTK sends one anonymous ping per day (23h interval). No personal data, no file paths, no command content.

Data sent: device hash, version, OS, architecture, command count/24h, top commands, savings %.

To opt out:

```bash
# Via environment variable
export RTK_TELEMETRY_DISABLED=1

# Via config.toml
[telemetry]
enabled = false
```

## Custom filters

Add your own filters (or override built-ins) in either location:

- **Project-local** — `.rtk/filters.toml` in your project root (committed with the repo)
- **User-global** — `~/.config/rtk/filters.toml` (applies to every project)

See [`src/filters/README.md`](https://github.com/rtk-ai/rtk/blob/master/src/filters/README.md) for the full TOML DSL reference.

### Trusting custom filters

Because a filter can rewrite what your AI assistant sees, custom filter files are **not applied until you trust them**. An untrusted (or edited) filter file is skipped silently on the command path. You review and manage trust with explicit commands:

```bash
rtk trust      # shows each filter and asks to confirm (--yes to skip the prompt)
rtk untrust    # revokes trust
```

`rtk init` also detects existing filters and lets you enable them — interactively, or non-interactively with `--trust-filters` / `--no-trust-filters`. Trust is tied to the file's contents (SHA-256), so editing a trusted file requires re-running `rtk trust`.

> **Upgrading:** earlier versions applied `~/.config/rtk/filters.toml` without trust. After upgrading, the user-global file is gated like project filters — if you already relied on a global filter, run `rtk trust` once to re-enable it.
