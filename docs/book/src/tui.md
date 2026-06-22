# TUI guide

`xre-tui` is the panels-and-widgets layer. It draws into a `CellBuffer` — the same
row-major grid of `Cell`s the 3D renderer targets — through a clipped `Frame`, lays
regions out with an integer constraint solver, and ships a small widget set styled
by a swappable `Theme`. Everything degrades cleanly to plain ASCII for VTs and
minimal terminals.

```rust,ignore
use xre::prelude::*; // Frame, Layout, Constraint, Panel, the widgets, Theme, ...
```

This guide tracks the canonical demo. To see it live:

```sh
cargo run -p xre-tui --example dashboard
cargo run -p xre-tui --example dashboard -- --ascii   # degraded ASCII/mono
```

## The frame model

Widgets never touch the `CellBuffer` directly. They draw through a `Frame<'a>` — a
clipped view that carries a clip `Rect`. Every write is intersected with that clip,
so a child can *never* paint outside the region its parent handed it. That single
guarantee is what makes nesting safe.

Build the root frame over your buffer, then carve out sub-regions:

```rust,ignore
let mut buf = CellBuffer::new(caps.size);
buf.fill(Style::DEFAULT.cell(' '));
let mut frame = Frame::root(&mut buf);   // clip = whole buffer
let area = frame.area();                 // the current clip Rect
```

`Frame::region(rect)` borrows a sub-frame clipped to `rect ∩ self.area()`;
`Frame::with_clip(rect, |f| …)` runs a closure under a tightened clip and restores
afterward. Coordinates passed to draw methods are *absolute* buffer coordinates —
the clip decides what actually lands. The primitives:

- `set(x, y, cell)` — write one cell if inside the clip.
- `fill(cell)` / `fill_rect(rect, cell)` — fill the clip, or a clipped sub-rect.
- `print(x, y, text, style)` — single-line text, no wrap.
- `put_str(x, y, text, style, wrap)` — returns the advanced `(x, y)`; honours
  `WrappingMode::{Ignore, Wrap, Clamp}` and handles wide glyphs (a wide glyph that
  would straddle the right edge is dropped).
- `style_rect(rect, style)` — re-color a region, leaving glyphs untouched.
- `overlay_glyph(x, y, glyph)` — replace only the glyph, keeping existing colors.

A widget is anything implementing the immediate-mode `Widget` trait:

```rust,ignore
pub trait Widget {
    fn render(&self, area: Rect, frame: &mut Frame);
}
```

Render borrows the widget immutably — persistent state (a list selection, an input
cursor) lives in the widget value and is mutated through inherent `handle_*`
methods, keeping the trait object-safe. Implementations rely on the frame's
clipping rather than bounds-checking themselves.

## Layout

`Layout` divides a `Rect` along one axis with a single-pass integer solver.
Construct one with `Layout::horizontal(constraints)` (columns) or
`Layout::vertical(constraints)` (rows), optionally `.gap(n)` for cells between
segments, then call `.split(area)` to get one `Rect` per constraint.

The `Constraint` variants:

| Variant | Meaning |
|---------|---------|
| `Len(u32)` | exact number of cells |
| `Pct(u16)` | percentage `0..=100` of available length |
| `Ratio(u32, u32)` | a `num/den` fraction |
| `Min(u32)` | at least this many; grows with slack |
| `Max(u32)` | at most this many; grows up to the cap |
| `Fill(u16)` | a weighted share of the slack |

Solving is deterministic and remainder-exact — the off-by-one row is the classic
TUI bug, so the rules are fixed. Fixed constraints (`Len`/`Pct`/`Ratio`) claim
their size first; the remaining slack is shared among the flexible ones
(`Fill`/`Min`/`Max`) by weight, clamped to bounds; any rounding remainder is handed
out one cell at a time, left-to-right. Two `Len(5)` over width 20 give `[5, 5]`,
not `[5, 15]` — there is no flexible segment to absorb the rest. `Fill(2)` next to
`Fill(1)` over 30 cells gives `[20, 10]`.

The dashboard splits the screen into a tab bar, a flexible body, and a fixed
command panel:

```rust,ignore
let rows = Layout::vertical([
    Constraint::Len(1),   // tab bar
    Constraint::Fill(1),  // body
    Constraint::Len(3),   // command panel
])
.split(area);
```

`GridLayout` is the 2D form: independent column and row constraints plus per-axis
gaps. `GridLayout::new(cols, rows).gaps(col_gap, row_gap).split(area)` returns a
row-major matrix `cells[row][col]`; `dims()` reports `(cols, rows)`, and
`span(area, (c0, c1), (r0, r1))` returns the bounding rect over an inclusive cell
range for cell-spanning panels.

```rust,ignore
let grid = GridLayout::new(
    [Constraint::Fill(1), Constraint::Fill(1)],
    [Constraint::Fill(1), Constraint::Fill(1)],
)
.gaps(1, 0)
.split(area);
let top_left = grid[0][0];
```

## Panels and borders

`Panel` is a bordered, titled, padded container. It is a builder: `Panel::new()`
starts with a `LIGHT` border and no title, and you chain `.title(s)`,
`.title_align(TitleAlign::{Left, Center, Right})`, `.border(Some(set))` (or
`.border(None)` for borderless), `.border_style(style)`, `.title_style(style)`,
`.fill(cell)`, and `.padding(n)`.

`render(area, frame)` draws the border, title and fill, and **returns the inner
`Rect`** children should draw into; `inner(area)` computes that rect without
drawing. Because the panel renders through `frame.region(area)`, its children are
clipped to the panel — they cannot escape.

```rust,ignore
let panel = Panel::new()
    .border(Some(BorderSet::ROUNDED))
    .border_style(Style::fg(Color::Rgb(90, 110, 140)))
    .title("command");
let inner = panel.render(rows[2], &mut frame);
input.render(inner, &mut frame);
```

`BorderSet` is the six glyphs (corners, horizontal, vertical). Built-in sets:
`BorderSet::ASCII` (`+-|`, guaranteed in any terminal/locale), `LIGHT`, `ROUNDED`,
`DOUBLE`, `HEAVY`. The dashboard picks `ASCII` in degraded mode and `ROUNDED`
otherwise.

## Widget gallery

Every widget implements `Widget`. Stateful ones keep their state in the value.

**Text** — `Text::raw(s)` or `Text::styled(s, style)`, with
`.align(Align::{Left, Center, Right})`, `.style(s)` and `.wrap(true)` (greedy word
wrap). `render_into(area, frame)` is an alias for the `Widget::render` call.

```rust,ignore
Text::styled("switch to the Logs tab", Style::DEFAULT.with_attrs(Attrs::DIM))
    .wrap(true)
    .render_into(inner, &mut frame);
```

**List + ListState** — `ListState` holds the selection and scroll offset across
frames; drive it with `select_next()`, `select_prev()`, `select(i)`, `set_len(n)`.
`List::new(&items)` (`items: &[String]`) takes `.item_style`, `.selected_style`,
`.highlight_symbol("> ")`. The stateful entry point is
`render_stateful(area, frame, &mut state)`, which clamps the selection, scrolls to
keep it visible, and draws the highlight row.

```rust,ignore
let mut state = ListState::new();
state.set_len(items.len());
state.select_next();
List::new(&items).render_stateful(area, &mut frame, &mut state);
```

**Table** — `Table::new(&rows, widths)` where `rows: &[Vec<String>]` and `widths:
impl Into<Vec<Constraint>>`. Columns are laid out by the same solver. Builders:
`.header(vec![...])`, `.col_gap(n)`, `.header_style`, `.cell_style`, `.align`.

**Gauge** — `Gauge::new(ratio)` (clamped `0.0..=1.0`), a horizontal bar with
sub-cell eighths. Builders: `.label("80%")` (centered overlay), `.bar_style`,
`.bg_style`, `.ascii(true)` for `#`/`-` blocks in degraded mode.

```rust,ignore
Gauge::new(ratio).ascii(self.ascii).label(format!("{:.0}%", ratio * 100.0))
    .render(split[0], &mut frame);
```

**Sparkline** — `Sparkline::new(data)` (`impl Into<Vec<f32>>`), a one-row bar chart
using block eighths. `.style(s)` and `.max(v)` fix the full-height value (it
auto-scales to the data max otherwise) and it shows the most recent `width` samples.

**Tabs** — `Tabs::new(&titles, selected)`. The active tab is bold+underline by
default; `.active_style`, `.inactive_style`, `.divider(char)` customize it.
Switching tabs is the app's job (track an index, advance it on `KeyCode::Tab`).

**Input** — a single-line editor holding buffer, cursor and history. Feed keys with
`handle_key(key)`, which returns `Some(line)` when Enter commits. It handles
`Char`, `Backspace`, `Delete`, arrows, `Home`/`End`, and `Up`/`Down` for history;
`value()`, `set_value(s)`, `clear()`, `is_empty()`, and `.focused(true)` round it
out.

**Log** — `Log::new(capacity)`, a capped ring buffer rendered newest-at-bottom.
`push(line)` appends (evicting the oldest at capacity); in *follow* mode (default)
the view sticks to the latest. `scroll_up(n)` turns follow off; `scroll_down(n)`
re-enables it at the bottom.

**Separator** — `Separator::horizontal()` (`─`) or `Separator::vertical()` (`│`), a
one-cell rule. `.ascii()` swaps in `-` / `|`; `.glyph(c)` and `.style(s)` override.

## Theme and style

A `Cell` carries a `glyph`, `fg`/`bg` `Color`, and `Attrs`. `Style` bundles `fg`,
`bg`, `attrs`: build with `Style::DEFAULT`, `Style::fg(color)`, then `.with_bg`,
`.with_fg`, `.with_attrs`. `style.cell(glyph)` produces a `Cell`; `style.apply(cell)`
re-styles one. `Attrs` is a bitset — `Attrs::BOLD | Attrs::UNDERLINE`, with `BOLD`,
`DIM`, `ITALIC`, `UNDERLINE`. `Color` is `Default`, `Ansi16(u8)`, `Ansi256(u8)`, or
`Rgb(u8, u8, u8)`, and resolves down the `Rgb → 256 → 16 → mono` chain at present
time.

`Theme` maps dotted keys (`"panel.title"`, `"list.selected"`, `"gauge.bar"`,
`"tabs.active"`, …) to `Style`, with a fallback for unknown keys. Widgets resolve
`theme.style("key")` instead of hard-coding colors, so the whole UI restyles by
swapping one `Theme`. Two built-ins ship: `Theme::default()` (truecolor) and
`Theme::mono()` (attributes only — for 16-color/ASCII terminals).

```rust,ignore
let theme = if ascii { Theme::mono() } else { Theme::default() };
let title = theme.style("panel.title");
```

ASCII/mono degradation is a deliberate, app-driven choice: pick `Theme::mono()`,
`BorderSet::ASCII`, `Gauge::ascii(true)`, and `Separator::ascii()` when the target
terminal can't do better. The dashboard threads one `ascii` flag through all of
these.

## Focus

A `FocusManager` tracks tab order and the focused widget. Register each focusable
with a `FocusId(u32)` (the first registered becomes focused); cycle with
`focus_next()` / `focus_prev()` (both wrap), jump with `focus(id)`, and route input
by checking `is_focused(id)` / `focused()`.

```rust,ignore
let mut fm = FocusManager::new();
fm.register(FocusId(0));   // input
fm.register(FocusId(1));   // list
// on KeyCode::Tab: fm.focus_next();
if fm.is_focused(FocusId(0)) { input.handle_key(key); }
```

## Scrollbars

`Scrollbar` is a track-and-thumb indicator for content taller (or wider) than its
viewport. It is stateless; the scroll metrics live in a `ScrollbarState`
(`content_length`, `viewport_length`, `position`, all in content units — items or
lines) that the application owns across frames. The thumb geometry is computed with
integer-only math, so it stays bit-identical across platforms.

A scrollbar does **not** lay itself out. Reserve a one-cell strip next to the
content with the layout solver and render into it:

```rust,ignore
let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Len(1)]).split(inner);
let (content, track) = (cols[0], cols[1]);
list.render_stateful(content, &mut frame, &mut list_state);

let mut sb = list_state.scrollbar_state(content.height() as u16);
Scrollbar::new(ScrollbarOrientation::VerticalRight)
    .ascii(ascii)                                   // '|'/'#' instead of '│'/'█'
    .track_style(theme.style("scrollbar.track"))
    .thumb_style(theme.style("scrollbar.thumb"))
    .render_stateful(track, &mut frame, &mut sb);
```

`ListState::scrollbar_state(viewport)` and `Log::scrollbar_state(viewport)` build the
state for you from a widget's current scroll position, so the bar always lines up
with what is visible. `Log::scroll_to(position, viewport)` is the inverse — use it to
drive the log from a dragged scrollbar. Orientations: `VerticalRight` / `VerticalLeft`
and `HorizontalBottom` / `HorizontalTop`. Theme keys: `scrollbar.track`,
`scrollbar.thumb`.

## Mouse support

Mouse capture is **on by default** (`TerminalGuard::enter()`); pass
`GuardOptions { mouse: false, ..Default::default() }` to `enter_with` to turn it
off. While capture is on, the terminal sends clicks, drags and the scroll wheel to
your app as `Event::Mouse(MouseEvent)` instead of doing native click-drag text
selection — users can still select text by holding **Shift** (most terminals) or
**Option** (macOS). See [Terminal compatibility](terminals.md) for the trade-off.

For a simple, single-region target (a centred menu, say) a `MouseRouter` is
overkill: mirror the render layout in a small `hit(area, col, row) -> Option<idx>`
helper — the same pattern `Tabs::hit` uses — and call it on `MouseKind::Down`. The
`rift-fps` example does exactly this for its clickable pause menu.

Because the UI is immediate-mode there is no retained widget tree, so a
`MouseRouter` does the hit-testing — the same rebuild-every-frame model as
`FocusManager`. During render, register each interactive region's `Rect` against a
`FocusId`; during input, route an event to the region under the cursor. Routing
honours **drag capture** (a press sticks to its target through the drag, so a
one-cell scrollbar thumb keeps tracking even when the cursor slides off) and
**z-order** (later registrations win overlaps). Events are routed against *last*
frame's regions — a one-frame latency that is standard for immediate-mode and
invisible at interactive rates.

```rust,ignore
// input phase — route against the regions registered last frame
if let Event::Mouse(m) = ev {
    if let Some(id) = router.route(&m) {
        if matches!(m.kind, MouseKind::Down(_)) { focus.focus(id); }
        match id {
            LIST_ID => { list.handle_mouse(&m, list_rect, &mut list_state); }
            BAR_ID  => { /* Scrollbar::handle_mouse + Log::scroll_to */ }
            _ => {}
        }
    }
}

// render phase — rebuild the regions in paint order
router.begin_frame();
router.register(LIST_ID, list_rect);
list.render_stateful(list_rect, &mut frame, &mut list_state);
```

Each interactive widget exposes an inherent `handle_mouse` (mirroring `handle_key`):
`List` selects the row under a click and scrolls on the wheel, `Log` scrolls on the
wheel, `Tabs::handle_mouse` returns the clicked tab index, `Input` positions the
cursor, and `Scrollbar` drags its thumb. For a 3D `Viewport3D`, translate events
with `viewport_gesture(ev, prev, sens)` and apply the resulting `ViewportGesture`
(orbit / zoom) to your own camera controller. The `dashboard`, `spinning-cube` and
`rift-fps` examples wire all of this up.

## Putting it together

The full pattern — a `TerminalGuard` for RAII restore, a `Capabilities::probe`, a
`Presenter` diffing frames, and an `EventQueue` draining input frame-coherently —
lives in the `dashboard` example. The loop, in essence:

```rust,ignore
let _guard = TerminalGuard::enter()?;               // raw mode + alt screen, restored on drop/panic
let caps = Capabilities::probe();
let mut presenter = Presenter::stdout(&caps);
let mut buf = CellBuffer::new(caps.size);
let mut events = EventQueue::new();

while app.running {
    events.pump(Duration::from_millis(16))?;        // block up to the frame budget, drain the rest
    for ev in events.drain() {
        if let Event::Resize(size) = ev { buf.resize(size); presenter.resize(size); }
        app.handle(&ev);
    }
    app.tick();
    app.render(&mut buf);                            // your Frame::root(&mut buf) draw code
    presenter.present(&buf)?;                        // diffs against the last frame, writes minimal bytes
}
```

Run it, press `Tab` to cycle tabs, type into the command line, and `q` / `Esc` to
quit. Add `--ascii` to see the same UI rendered through `Theme::mono()` and
`BorderSet::ASCII`.
