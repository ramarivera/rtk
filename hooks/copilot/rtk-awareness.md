# RTK — Copilot Integration (VS Code Copilot Chat + Copilot CLI)

**Usage**: Token-optimized CLI proxy (cuts up to 90% of bash output)

## What's automatic

The `.github/copilot-instructions.md` file is loaded at session start by both Copilot CLI and VS Code Copilot Chat.
It instructs Copilot to prefix commands with `rr-rtk` automatically.

The `.github/hooks/rtk-rewrite.json` hook adds a `PreToolUse` safety net via `rr-rtk hook` —
a cross-platform Rust binary that intercepts raw bash tool calls and rewrites them.
No shell scripts, no `jq` dependency, works on Windows natively.

## Meta commands (always use directly)

```bash
rr-rtk gain              # Token savings dashboard for this session
rr-rtk gain --history    # Per-command history with savings %
rr-rtk discover          # Scan session history for missed rtk opportunities
rr-rtk proxy <cmd>       # Run raw (no filtering) but still track it
```

## Installation verification

```bash
rr-rtk --version   # Should print: rr-rtk X.Y.Z
rr-rtk gain        # Should show a dashboard (not "command not found")
which rr-rtk       # Verify correct binary path
```

> ⚠️ **Name collision**: If `rr-rtk gain` fails, you may have `reachingforthejack/rtk`
> (Rust Type Kit) installed instead. Check `which rr-rtk` and reinstall from
> the personal fork (https://github.com/ramarivera/rtk, install via `cargo install --locked rr-rtk`).

## How the hook works

`rr-rtk hook` reads `PreToolUse` JSON from stdin, detects the agent format, and responds appropriately:

**VS Code Copilot Chat** (supports `updatedInput` — transparent rewrite, no denial):
1. Agent runs `git status` → `rr-rtk hook` intercepts via `PreToolUse`
2. `rr-rtk hook` detects VS Code format (`tool_name`/`tool_input` keys)
3. Returns `hookSpecificOutput.updatedInput.command = "rr-rtk git status"`
4. Agent runs the rewritten command silently — no denial, no retry

**GitHub Copilot CLI** (deny-with-suggestion — CLI ignores `updatedInput` today, see [issue #2013](https://github.com/github/copilot-cli/issues/2013)):
1. Agent runs `git status` → `rr-rtk hook` intercepts via `PreToolUse`
2. `rr-rtk hook` detects Copilot CLI format (`toolName`/`toolArgs` keys)
3. Returns `permissionDecision: deny` with reason: `"Token savings: use 'rr-rtk git status' instead"`
4. Copilot reads the reason and re-runs `rr-rtk git status`

When Copilot CLI adds `updatedInput` support, only `rr-rtk hook` needs updating — no config changes.

## Integration comparison

| Tool                  | Mechanism                               | Hook output              | File                               |
|-----------------------|-----------------------------------------|--------------------------|------------------------------------|
| Claude Code           | `PreToolUse` hook with `updatedInput`   | Transparent rewrite      | `hooks/rtk-rewrite.sh`             |
| VS Code Copilot Chat  | `PreToolUse` hook with `updatedInput`   | Transparent rewrite      | `.github/hooks/rtk-rewrite.json`   |
| GitHub Copilot CLI    | `PreToolUse` deny-with-suggestion       | Denial + retry           | `.github/hooks/rtk-rewrite.json`   |
| OpenCode              | Plugin `tool.execute.before`            | Transparent rewrite      | `hooks/opencode-rtk.ts`            |
| (any)                 | Custom instructions                     | Prompt-level guidance    | `.github/copilot-instructions.md`  |
