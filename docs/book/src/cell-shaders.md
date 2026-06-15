# Cell shaders: ramp to shape vectors

A **cell shader** is the second half of the pipeline. The renderer fills a
`SampleBuffer` at sub-cell resolution; a `CellShader` collapses each `SX × SY`
sample block into one `Cell` — a glyph plus colors — or `None` when the block is
empty (transparent, so the viewport floats over whatever is underneath). The trait
is tiny:

```rust,ignore
pub trait CellShader: Sync {
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell>;
}
```

The `Sync` bound lets `resolve_cells` shade rows in parallel; every built-in
shader is immutable data and satisfies it. The choice of shader is what gets
terminal 3D past the blocky ASCII look, and the built-ins trace a real research
progression.

## LuminanceRamp — coverage to glyph

The baseline shader (asciimare's calibrated-ramp model). It averages the cell's
filled samples to a single coverage value and picks the glyph whose ink coverage
is nearest, coloring it by the mean sample RGB:

```rust,ignore
let shader = LuminanceRamp::default();    // built-in density ramp, no font needed
```

The default ramp is `DENSITY_ORDER` — a hand-ordered printable-ASCII string from
sparsest (` `) to densest (`$`) — spaced uniformly over `0.0..=1.0`. Glyph
selection is an `O(log n)` binary search. You can bias the reduction
(`LumaBias::Mean` vs. `MaxBiased`, which favors highlights and edges), enable
depth-darkened backgrounds, or feed a **font-calibrated** ramp measured by
[glyphgen](glyphgen.md):

```rust,ignore
let shader = LuminanceRamp::from_ramp(RAMP_MENLO.to_vec())   // from glyphgen
    .bias(LumaBias::MaxBiased)
    .depth_darken(true);
```

A luminance ramp is fast and works on every terminal, but it only knows *how much*
ink a cell needs — not *where*. That blind spot is what the next shader fixes.

## ShapeVector — the alexharri technique

A ramp can't tell a top-heavy cell from a bottom-heavy one, so shallow edges
stair-step into a hard two-glyph staircase. `ShapeVector` samples ink coverage in
**six staggered regions** (2 columns × 3 rows) and picks the glyph whose own 6-D
coverage vector is nearest in Euclidean distance — so a `.`/`:`/`!` transition
resolves *smoothly*:

```rust,ignore
let shader = ShapeVector::default();          // built-in ASCII shape table
```

Two refinements from the article are baked in and matter a lot:

- **Per-component normalization.** Each of the six components is normalized across
  the glyph set; skip it and lookups collapse onto a couple of border glyphs.
  `ShapeTable::new` does this for you.
- **Contrast enhancement.** Before matching, the cell vector is normalized by its
  max, raised to an exponent, and denormalized — sharpening the dominant direction
  so edges read crisply. Tune it with `.contrast(x)`.

The default `ShapeTable::builtin_ascii()` is hand-authored and good enough to show
directional selection out of the box; a font-measured table from
[`glyphgen`](glyphgen.md) is sharper:

```rust,ignore
use xre::render::{ShapeGlyph, ShapeTable, ShapeVector};

let glyphs = SHAPES_MENLO.iter()
    .map(|&(glyph, vector)| ShapeGlyph { glyph, vector })
    .collect();
let shader = ShapeVector::new(ShapeTable::new(glyphs)).contrast(0.6);
```

## Unicode modes

When the terminal has the glyphs (see [Terminals](terminals.md)), three Unicode
shaders trade the ASCII ramp for block and dot glyphs:

- **`HalfBlock`** — `▀`, with the top half's color in the foreground and the
  bottom half's in the background. This *doubles vertical color resolution* and is
  the truecolor showpiece.
- **`BlockShades`** — the ` ░▒▓█` coverage ramp; retro and ASCII-adjacent.
- **`Braille`** — a 2×4 dot matrix per cell from a luma threshold; great paired
  with wireframe for plotter-style output.

```rust,ignore
let shader = HalfBlock;                 // or BlockShades, or Braille::default()
```

These are never the default, because terminals and fonts vary — select them from
your detected `Capabilities`, and keep `LuminanceRamp` as the universal fallback.

## Resolving a whole viewport

`Viewport3D` runs the shader for you, but you can resolve a block directly with
`resolve_cells` — the row-parallel path the viewport uses internally:

```rust,ignore
use xre::render::resolve_cells;

let mut out: Vec<Option<Cell>> = vec![None; (cols * rows) as usize];
resolve_cells(&samples, &shader, cols, rows, &mut out);   // row-parallel; bit-identical to serial
```

Because every cell is independent and read-only over the buffer, the parallel and
serial results are identical — only the speed differs.

## Trying them side by side

Run the viewer and press **`c`** to cycle shaders live on a real model:

```sh
xre view assets/cube.obj
```

Then calibrate a ramp or shape table to your own terminal font with
[`xre glyphgen`](glyphgen.md), and benchmark each shader's cost with
[`xre bench`](cli.md).
