# Terminal spike notes (Stage 0.3)

This document records the findings of the throwaway-but-instructive terminal
spike that precedes the real presenter (see
[`RiftEngine-Plan/05-phase-0-foundations.md`](../RiftEngine-Plan/05-phase-0-foundations.md)
§0.3). The spike lives in
[`crates/xre-term/examples/terminal_spike.rs`](../crates/xre-term/examples/terminal_spike.rs).

Run it with:

```sh
cargo run -p xre-term --example terminal-spike
```

It probes capabilities, prints a report, then animates a truecolor gradient with
**naive full redraws** at a 60 FPS target and reports the achieved FPS and the
bytes written per frame — the baseline the diffed presenter must beat in Phase 1.

## What the spike proves

1. **Raw mode + alternate screen lifecycle** via [`TerminalGuard`], restored on
   normal exit, on `?`-propagated error, **and on panic** (a panic hook is
   installed once). Leaving a terminal in raw mode is the cardinal TUI sin.
2. **Capability probing** without a live round-trip: color depth from
   `COLORTERM`/`TERM`/`NO_COLOR`, Unicode level from the locale variables, size
   from the OS, and a synchronized-output allow-list keyed on `TERM_PROGRAM`.
   The decision logic is pure and unit-tested in `capabilities.rs`.
3. **Throughput baseline**: a full 80×24 truecolor redraw is ~20 bytes/cell of
   SGR + glyph, i.e. on the order of 30–40 KB/frame with no diffing — which is
   exactly why the Phase 1 presenter diffs (spec §C budgets a full-change frame
   at ≤ 64 KB and a typical animated frame at < 40 % of cells touched).

## Measurements

Fill in per terminal as the spike is run on each (throughput, quirks):

| Terminal | OS | Color | Sync output | Notes |
|---|---|---|---|---|
| xterm | Linux | _tbd_ | _tbd_ | |
| kitty | Linux/macOS | truecolor | yes | honours DEC 2026 |
| alacritty | any | truecolor | _tbd_ | |
| Windows Terminal | Windows | truecolor | _tbd_ | needs VT processing enabled |
| tmux | any | depends on `-2`/`Tc` | often swallowed | sync-output passthrough varies |
| raw Linux VT | Linux | 16/256 | no | ASCII-safe path matters here |

## Decisions recorded

- **Backend: `crossterm`** (go) for raw mode, input, size and ANSI emission —
  cross-platform and battle-tested. A direct termios + Win32 path is kept open as
  a future `backend-raw` feature for binary-size reduction, not for 0.1.
- **Windows legacy console**: the `WriteConsoleOutput` fallback
  (Command_Line_3D's technique) is deferred to a `legacy-windows` feature in a
  later phase; modern Windows Terminal enables
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING` and uses the VT path.
- **Synchronized output**: detected by allow-list for now; a runtime DECRQM
  query is deferred to Phase 1 where the presenter can consume the reply.

[`TerminalGuard`]: ../crates/xre-term/src/guard.rs
