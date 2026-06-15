# 3D rendering guide

`xre-render` is the software 3D core. It is a two-stage pipeline:

1. **Rasterize** geometry into a `SampleBuffer` at sub-cell resolution — a
   `Camera`/`Projection` transforms vertices, near-plane clipping splits triangles
   that straddle the camera, and `draw_mesh` fills depth-tested,
   perspective-correct, Lambert-lit samples.
2. **Resolve** that buffer to glyphs with a [`CellShader`](cell-shaders.md).

This chapter covers stage one. Everything here is in the `xre::prelude`.

## The sample buffer

`SampleBuffer` is the renderer's true canvas: a sub-cell grid where each terminal
cell owns an `SX × SY` block of samples (2×4 by default). It is **struct-of-arrays**
(separate luma, RGB, and depth planes) and persistent — resize it on a terminal
resize, `clear` it each frame, and it allocates nothing in steady state.

```rust,ignore
use xre::prelude::*;

// A 120×36-cell viewport, 2×4 samples per cell (240×144 samples).
let mut samples = SampleBuffer::new(UVec2::new(120, 36), 2, 4);
samples.clear([0, 0, 0]);            // background RGB; depth → +∞, luma → 0
```

`resize(cells, sx, sy)` reallocates only when the total length changes, so
reacting to a `Resize` event is cheap.

## Camera and projection

A `Camera` is a `Transform` plus a `Projection`. Build one with `Camera::look_at`,
then ask for a view-projection matrix sized to your viewport:

```rust,ignore
let cam = Camera::look_at(Vec3::new(0.0, 1.6, 4.5), Vec3::ZERO);
let vp = cam.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
```

The third argument is the **cell aspect ratio**. Terminal cells are roughly twice
as tall as they are wide, so without correction a sphere renders as an ellipse.
`Projection::DEFAULT_CELL_ASPECT` is `0.5` — the right value for typical fonts.
`Projection` is `Perspective { fov_y, near, far }` or `Orthographic { height,
near, far }`; the default is a 60°-ish perspective. `view_projection(cols, rows,
aspect)` folds the viewport dimensions and cell aspect into the projection so the
image is never stretched.

## Meshes

A `Mesh` is positions, normals, UVs, and triangle indices. Procedural constructors
cover the basics:

```rust,ignore
let cube   = Mesh::cube();
let plane  = Mesh::plane(2.0);
let sphere = Mesh::uv_sphere(1.0, 32, 48);     // radius, rings, sectors
let torus  = Mesh::torus(1.2, 0.4, 32, 16);    // major, minor, segments
```

`mesh.aabb()` returns the axis-aligned bounds (handy for framing a camera),
`mesh.triangle_count()` the triangle total, and `mesh.recompute_smooth_normals()`
regenerates normals (the OBJ loader calls it when a file omits them). Load meshes
from disk with the [OBJ loader](scenes.md).

## Drawing a mesh

`draw_mesh` runs the whole geometry-plus-raster stage into the sample buffer:

```rust,ignore
draw_mesh(
    &mut samples,
    &mesh,
    model.to_mat4(),   // model matrix (a Transform → Mat4)
    vp,                // view-projection from the camera
    &rig,              // LightRig
    &material,         // Material
    ShadeMode::PerSample,
    Cull::Back,
);
```

Internally it transforms vertices, near-clips triangles that cross the camera
plane (a Sutherland–Hodgman split — the fix for the classic "garbage coordinates
behind the camera" bug), then rasterizes with an edge-function traversal, a
top-left fill rule (no cracks, no double-drawn pixels along shared edges), an f32
LESS depth test, and perspective-correct attribute interpolation.

### Shade modes

`ShadeMode` chooses where lighting is evaluated:

- **`Flat`** — one face normal, shaded once per triangle. Cheapest; faceted look.
- **`Gouraud`** — light each vertex, interpolate the colors across the triangle.
- **`PerSample`** — interpolate the normal and light *every* sample. Smoothest,
  and the default for the demos.

`Cull` is `Back` (drop back-faces, the usual choice), `Front`, or `None`.

### Reusing a rasterizer

The free `draw_mesh` is backed by a thread-local `Rasterizer` so it stays
allocation-free without you managing one. In a hot loop you can hold one
explicitly — it owns the per-frame scratch (transformed vertices, set-up
primitives, the row bins) and reuses it every frame:

```rust,ignore
use xre::render::Rasterizer;

let mut rasterizer = Rasterizer::new();
loop {
    samples.clear([0, 0, 0]);
    rasterizer.draw_mesh(&mut samples, &mesh, model, vp, &rig,
                         &material, ShadeMode::PerSample, Cull::Back);
    // ... resolve + present ...
}
```

## Lighting

A `LightRig` is a list of `Light`s plus an ambient term and optional depth fog.
A `Light` is `Directional` (a sun) or `Point` (with quadratic attenuation):

```rust,ignore
let rig = LightRig::default()                              // one directional + ambient
    .with_light(Light::point(Vec3::new(3.0, 3.0, 3.0)));

// Or build from scratch:
let rig = LightRig::ambient_only(Vec3::splat(0.1))
    .with_light(Light::directional(Vec3::new(-0.4, -1.0, -0.6)));
```

Lighting is Lambert diffuse (`color · n·l · attenuation · intensity`) summed over
the lights plus ambient, modulated by the material. A `Material` carries
`base_color`, `kd`/`ks`, `emissive`, optional `cel_levels` (toon quantization),
and an optional texture handle:

```rust,ignore
let mat = Material::colored(Vec3::new(0.7, 0.8, 0.9)).cel(4);  // 4-band cel shading
```

Set `rig.fog = Some((color, near, far))` to fade samples toward a color with
depth — a cheap atmospheric cue.

## Presenting the result

Wrap the filled buffer in a `Viewport3D` widget and render it into a `Frame`; the
widget runs the cell shader and composites the glyphs, leaving empty cells
transparent so the viewport floats over whatever is underneath:

```rust,ignore
let shader = LuminanceRamp::default();
let mut frame = Frame::root(&mut buf);
Viewport3D::new(&samples, &shader).render(area, &mut frame);
```

See [Cell shaders](cell-shaders.md) for the shader options, and the
`spinning-cube` example (`cargo run -p xre-tui --example spinning-cube`) for the
whole pipeline in one file.

## Parallelism and determinism

The renderer is **row-parallel** behind the `parallel` feature, which is on by
default. Above an internal size threshold, `draw_mesh` splits the sample buffer
into disjoint horizontal **row bands**, bins each triangle into the bands it
touches, and fills the bands concurrently with rayon; the cell-shade stage
(`resolve_cells`, used by `Viewport3D`) is parallelized the same way, one cell
row per task.

Crucially, this **does not change a single pixel**. Each row band replays the full
triangle list in the same order and runs the identical per-pixel arithmetic and
depth test as the serial path, so the output is *bit-identical* regardless of core
count — the determinism guarantee (and the golden-frame snapshot tests) hold either
way. A property test renders every frame through both paths and asserts they match
bit-for-bit.

Build the single-core path with `--no-default-features` on `xre-render` if you
want it; the API is unchanged. Measure both on your machine with
[`xre bench`](cli.md).
