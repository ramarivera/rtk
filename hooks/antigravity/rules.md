# RTK - Rust Token Killer (Google Antigravity)

**Usage**: Token-optimized CLI proxy for shell commands.

## Rule

Always prefix shell commands with `rr-rtk` to minimize token consumption.

Examples:

```bash
rr-rtk git status
rr-rtk cargo test
rr-rtk ls src/
rr-rtk grep "pattern" src/
rr-rtk find "*.rs" .
rr-rtk docker ps
rr-rtk gh pr list
```

## Meta Commands

```bash
rr-rtk gain              # Show token savings
rr-rtk gain --history    # Command history with savings
rr-rtk discover          # Find missed RTK opportunities
rr-rtk proxy <cmd>       # Run raw (no filtering, for debugging)
```

## Why

RTK filters and compresses command output before it reaches the LLM context, saving 60-90% tokens on common operations. Always use `rr-rtk <cmd>` instead of raw commands.
