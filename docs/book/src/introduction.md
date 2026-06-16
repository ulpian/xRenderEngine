# Introduction

**xRenderEngine** is a lightweight 3D rendering engine and game framework that
runs in your terminal. It draws real 3D scenes — and full TUI dashboards — as
characters, with no GPU, no async runtime, and a short list of dependencies. The
crates carry the `xre-` prefix; the command-line tool and the facade crate are
both named `xre`.

This book is the practical guide. The design rationale and full roadmap live in
the planning archive under `RiftEngine-Plan/` in the
[repository](https://github.com/ulpian/xRenderEngine) (kept under its
original name); start with `02-architecture.md` if you want the why behind the what.

## Two ideas

Everything in xRenderEngine follows from two design commitments.

**Characters are not pixels.** A pixel has exactly one property — brightness. A
terminal cell has a *glyph*, and glyphs differ wildly in how much ink they put on
screen and *where* they put it: `.` is sparse and bottom-weighted, `'` sits high,
`@` is nearly solid. Treating one character as one pixel throws all of that away.
Instead, the renderer rasterizes each cell at **sub-cell resolution** — a 2×4 grid
of samples per cell by default — and then a pluggable [*cell shader*](cell-shaders.md)
collapses that block into the single best glyph and color. The result gets far past
the usual blocky ASCII look.

**One frame, two engines.** A TUI layer (panels, grids, widgets) and a 3D layer
(scenes inside `Viewport3D` widgets) render into *the same* cell buffer and are
flushed by *one* diffed presenter. A spinning model can float over a dashboard;
a heads-up display can overlay a first-person view. You compose 2D and 3D with the
same `Frame` and the same `Widget` trait — there is no separate "graphics mode."

## How the pipeline fits together

A frame flows top to bottom through the crates:

```text
  your app / Game ──────────────────────────────────  game loop, ECS, time, input
        │ scene + UI tree
  xre-cello / xre-tui ───────────────────────────────  SceneGraph, Camera, widgets, Viewport3D
        │ draw lists + widget draws
  xre-render ────────────────────────────────────────  rasterize → SampleBuffer → CellShader
        │ CellBuffer (glyph, fg, bg, attrs)
  xre-term ──────────────────────────────────────────  diff vs. last frame → minimal ANSI write
```

The 3D renderer fills a `SampleBuffer` at sub-cell resolution; a `CellShader`
resolves it to a `CellBuffer`; the `Presenter` diffs that against the previous
frame and writes only the bytes that changed.

## The crates

| Crate | Responsibility | Guide |
|-------|----------------|-------|
| `xre-core`   | math (glam re-exports), `Color`, `Cell`, `CellBuffer`, `Rect`, `Transform` | — |
| `xre-term`   | raw mode, capability probe, diffed `Presenter`, input | [Terminals](terminals.md) |
| `xre-tui`    | layout, panels, widgets, theme, `Viewport3D` | [TUI guide](tui.md) |
| `xre-render` | `SampleBuffer`, rasterizer, lighting, cell shaders | [3D rendering](rendering.md), [Cell shaders](cell-shaders.md) |
| `xre-cello`  | scene graph, OBJ/MTL loader, textures, camera controllers | [Scenes & assets](scenes.md) |
| `xre-engine` | fixed-timestep loop, ECS (`hecs`), animation, input map, collision | [The game loop](game-loop.md) |
| `xre`        | facade: prelude + feature flags re-exporting all of the above | — |
| `tools/xre-cli` | the `xre` binary: `view` / `bench` / `new` / `glyphgen` | [CLI reference](cli.md) |
| `tools/glyphgen` | offline font → luminance ramp / shape vectors | [Glyph calibration](glyphgen.md) |

## Using the facade

Most applications depend on a single crate, `xre`, and import its prelude:

```rust,ignore
use xre::prelude::*;
```

That brings the common types — `Vec3`, `Transform`, `Cell`, `CellBuffer`, the
TUI widgets, the renderer (`Mesh`, `Camera`, `draw_mesh`, `SampleBuffer`,
`Viewport3D`), and the engine entry points (`run`, `Game`, `Time`) — into scope.
Specialized items live under their module path (`xre::engine::Track`,
`xre::scene::Texture`, `xre::render::Rasterizer`); each guide notes which is which.

## Conventions this engine keeps

- **No `unsafe`** anywhere in the libraries (`unsafe_code = "forbid"`).
- **No panics in library code** — fallible operations return `thiserror` enums,
  and the presenter restores the terminal through an RAII guard *and* a panic hook.
- **Determinism** — a fixed `dt` and seed produce bit-identical frames across
  platforms, which is what makes the golden-frame snapshot tests possible. The
  default `parallel` rendering feature preserves this: it parallelizes over
  independent rows, so the output never changes — only the throughput does (see
  [3D rendering](rendering.md)).
- **Graceful degradation** — truecolor → 256 → 16 → mono, full Unicode → ASCII,
  detected at startup ([Terminals](terminals.md)).

Ready to draw something? Head to the [Quickstart](quickstart.md).
