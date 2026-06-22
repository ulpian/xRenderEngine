# Terminal compatibility

xRenderEngine assumes nothing about the host terminal. At startup it probes what
the terminal can do, then degrades gracefully — color, Unicode, tear-free flushing,
and size all have a safe fallback. The goal: a recognizable picture everywhere, a
beautiful one where the terminal earns it.

## The capability probe

`Capabilities::probe()` reads environment heuristics; it never fails, and anything
undetectable falls back to a conservative default. The decision logic is factored
into pure functions, so it is unit-testable without a live terminal.

| Capability | Detected from | Values | Fallback |
|---|---|---|---|
| **Color depth** (`ColorDepth`) | `NO_COLOR`, `COLORTERM`, `TERM` | `TrueColor` / `Ansi256` / `Ansi16` / `Mono` | `Ansi16` |
| **Unicode** (`UnicodeLevel`) | `LC_ALL`, `LC_CTYPE`, `LANG` | `Full` / `HalfBlocks` / `AsciiOnly` | `AsciiOnly` |
| **Synchronized output** | `TERM_PROGRAM` | `bool` (DEC mode 2026) | `false` |
| **Size** | `crossterm::terminal::size()` | `UVec2 { x: cols, y: rows }` | `80×24` |

The full conservative fallback is `Capabilities::FALLBACK`: 16 colors, ASCII only,
`80×24`, no synchronized output.

**Color depth.** `NO_COLOR` (any value) forces `Mono`. `COLORTERM=truecolor`/`24bit`
gives `TrueColor`; a `TERM` containing `256` gives `Ansi256`; `TERM=dumb` forces
`Mono`; anything else color-ish is `Ansi16`. Colors are authored at full fidelity
(`Color::Rgb`) and downgraded *at present time* in the presenter via `Color::resolve`,
following the chain `Rgb → Ansi256 → Ansi16 → mono`. Under `Mono`, every concrete
color collapses to `Color::Default`.

**Unicode.** A `UTF-8`/`UTF8` codeset in any locale variable enables `Full`
(half-blocks, braille, box-drawing). Otherwise the renderer stays on the ASCII-safe
path.

**Synchronized output.** Absent a runtime query, the probe allow-lists terminals
known to honour DEC mode 2026 (via `TERM_PROGRAM`) and defaults to off. When
enabled, the presenter brackets each frame with the sync escapes so the terminal
swaps the whole frame at once instead of showing a half-drawn update; when off,
frames flush normally and a fast diff keeps tearing minimal.

## The `--ascii` flag and mono theme

The viewer accepts `--ascii` to force the lowest-common-denominator look regardless
of what the probe found:

```sh
xre view model.obj --ascii
```

This swaps box-drawing borders for `BorderSet::ASCII` (`+`, `-`, `|`) — guaranteed
to render in any terminal or locale — and drops the dark background fill. For color,
`Theme::mono()` conveys structure through attributes only (bold, dim, underline)
rather than RGB, suitable for 16-color or ASCII terminals. Pair it with
`BorderSet::ASCII` when `UnicodeLevel::AsciiOnly` or `ColorDepth::Mono` is detected.
The richer `BorderSet::LIGHT` / `ROUNDED` / `DOUBLE` / `HEAVY` sets require Unicode
box-drawing.

## Guaranteed restore: RAII guard + panic hook

A terminal app puts the screen into a hostile state — raw mode, alternate screen,
hidden cursor. If the process exits without undoing that, the user is left with a
wrecked shell. `TerminalGuard` makes restore unconditional:

```rust,ignore
use xre::term::TerminalGuard;

let _guard = TerminalGuard::enter()?; // raw mode + alt screen, cursor hidden
// ... run the loop ...
// guard drops here → terminal restored
```

`enter()` enables raw mode, switches to the alternate screen, hides the cursor,
enables **mouse capture**, and — where the terminal supports it — turns on the
**keyboard enhancement protocol** (see below). `Drop` reverses them all (mouse
capture and the enhancement flags are disabled *before* leaving the alternate
screen so no stray reporting escapes leak into the shell). Critically, `enter()`
also installs (once) a **panic hook** that restores the terminal — capture and
enhancement included — *before* delegating to the previous hook, so even an
unwinding panic leaves a clean screen with the panic message still printed. The
guard is `#[must_use]`: bind it to a name, not `_`, or it drops immediately.

## Mouse capture and text selection

Mouse capture is on by default so any UI element can be driven with the mouse. The
catch: while captured, the terminal forwards mouse actions to the application instead
of performing its own click-drag text selection. Users can still select and copy by
holding **Shift** (xterm, GNOME Terminal, most others) or **Option** (macOS Terminal,
iTerm2), which bypasses the application. If an app would rather keep native selection,
opt out with `TerminalGuard::enter_with(GuardOptions { mouse: false, ..Default::default() })`.
The `Capabilities` probe also exposes an advisory `mouse` flag (true for any non-`dumb`
`TERM`); the guard, not the probe, decides whether to actually capture.

## Keyboard protocol and simultaneous keys

In their default mode terminals never report key *releases* and only auto-repeat
the **last** key held, so two keys at once (e.g. **W+D** for a diagonal) can't be
tracked. The fix is the **kitty keyboard protocol**: when `enter()` finds the
terminal supports it (`crossterm::terminal::supports_keyboard_enhancement`), it
pushes `REPORT_EVENT_TYPES | REPORT_ALL_KEYS_AS_ESCAPE_CODES`, so every key
arrives as a distinct press, auto-repeat, or release. Each `Event::Key` then
carries a `KeyState` (`Press` / `Repeat` / `Release`); without the protocol every
event is a `Press`. Query whether it took effect with
`TerminalGuard::keyboard_enhanced()`.

Because releases are now delivered, **every key consumer except the input map must
ignore `KeyState::Release`** (and usually `Repeat` for discrete actions), or a
single keypress fires twice. Widgets like `Input` already do this internally.

`xre-engine`'s `InputMap` consumes all three states to maintain a per-key
*down-set*, so `held`/`axis` report genuinely simultaneous keys. Call
`InputMap::set_release_reporting(guard.keyboard_enhanced())`: with releases a key
is held from press to release; without them, held is synthesised with a short
grace window (it smooths single-key movement but can't sustain a true multi-key
hold). For those terminals, pair it with `LatchAxis` — a press *sets* a sticky
direction (idempotent, so holding a key keeps moving rather than flipping it, and
W+D stay set together for a sustained diagonal); a stop key clears it. Protocol
support today: kitty, ghostty, foot, WezTerm, recent Alacritty; **not** Apple
Terminal.app or stock xterm.

## Per-terminal notes

These are general expectations from the probe heuristics; exact behavior depends on
configuration and version.

- **xterm** — typically `Ansi16`, or `Ansi256` when `TERM=xterm-256color`. Set
  `COLORTERM=truecolor` for 24-bit. No synchronized output by default.
- **kitty, WezTerm, iTerm2, ghostty** — generally truecolor, full Unicode, and on
  the synchronized-output allow-list, so you get tear-free frames.
- **alacritty** — truecolor and full Unicode in a UTF-8 locale; not on the sync
  allow-list (it sets `TERM`, not `TERM_PROGRAM`), so frames flush via the diff path.
- **Windows Terminal** — truecolor and full Unicode; not allow-listed for sync by
  `TERM_PROGRAM`, so expect the diffed flush.
- **conhost (legacy Windows console)** — treat as the conservative floor: 16 colors
  and ASCII. Use `--ascii` / `Theme::mono()`.
- **tmux / screen** — `TERM` often reports `screen*` (16 colors) unless configured
  for `*-256color`; truecolor and synchronized output may be intercepted by the
  multiplexer. Verify before relying on either.
- **ssh** — capabilities follow whatever `TERM`/`COLORTERM`/locale the session
  forwards; a bare login may land on the `Ansi16` / `AsciiOnly` fallback.
- **Linux virtual terminal (VT)** — limited color, restricted glyph coverage, no
  synchronized output. The ASCII + mono path is the safe choice.

When in doubt, the engine's own defaults already target the floor: 16 colors, ASCII
borders, `80×24`, normal flushing. Everything above that is an upgrade the probe
grants only when it can confirm support.
