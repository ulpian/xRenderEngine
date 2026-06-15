# Scenes & assets

This chapter covers `xre-cello`: the OBJ/MTL loader, image-texture readers, the
arena scene graph, and the camera controllers. The loader entry points and the
controllers are re-exported through the facade — `use xre::prelude::*;` brings in
`load_obj_file`, `parse_obj`, `ObjModel`, `Scene`, `Texture`, `OrbitController`,
and `FpsController`; the rest live under `xre::scene`.

## The OBJ/MTL loader

The loader is written from scratch (no `tobj`) so the engine controls robustness
and fuzzing. Two entry points:

```rust,ignore
use xre::scene::{load_obj_file, parse_obj, ObjModel};

// From a file (resolves sibling `mtllib`s relative to the OBJ):
let model: ObjModel = load_obj_file(std::path::Path::new("assets/cube.obj"))?;

// From an in-memory string (materials left unresolved):
let model = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
```

`parse_obj` returns an `ObjModel` and **never panics** — it is the `cargo fuzz`
target. An `ObjModel` holds:

- `objects: Vec<ObjObject>` — one entry per `o`/`g` group *and* per `usemtl`
  switch. Each carries a `name`, an optional `material` name, and a triangulated,
  index-deduplicated `mesh: Mesh`.
- `materials: HashMap<String, Material>` — populated only when `mtllib`s are
  resolved (i.e. by `load_obj_file`, not bare `parse_obj`).
- `material_libs: Vec<String>` — the `mtllib` filenames the OBJ referenced.
- `warnings: Vec<String>` — non-fatal problems, each prefixed `line N:`.

The only hard error is `ObjError::Io` (`#[non_exhaustive]`), returned by
`load_obj_file` when the OBJ file can't be read. A *missing* MTL file is recorded
as a warning, not an error.

The parser handles all four index forms (`v`, `v/vt`, `v//vn`, `v/vt/vn`), negative
(relative) indices, n-gon faces, `o`/`g` groups, `usemtl`/`mtllib`, backslash line
continuations, CRLF, and `#` comments. Its policy is **warn-and-continue**: a
malformed vertex, an out-of-range face index, or an unknown directive becomes a
warning and is skipped. During building, `(v, vt, vn)` tuples are deduplicated into
a single vertex buffer, and if any face vertex lacked a normal the loader calls
`mesh.recompute_smooth_normals()` so lighting still works.

Two `ObjModel` helpers are handy for simple viewers:

```rust,ignore
let mesh = model.combined_mesh();   // merge every object into one Mesh
let tris = model.triangle_count();  // total triangles across all objects
```

### The ear-clipping triangulator

Faces with more than three vertices are triangulated by the vendored `triangulate`
function (`xre::scene::triangulate`). It takes a polygon as a slice of `Vec3`
positions plus a loop of `u32` indices and returns `Vec<[usize; 3]>` triples —
*local* indices into the loop, which the loader remaps:

```rust,ignore
use xre::scene::triangulate;
let tris = triangulate(&positions, &loop_indices); // Vec<[usize; 3]>
```

3-D faces are projected onto their best-fit plane (a Newell normal, dominant axis
dropped, winding preserved) and ear-clipped in 2-D. Convex polygons fan-triangulate;
the ear loop has a bounded iteration guard and falls back to a simple fan on
degenerate input, so it never loops forever and never panics. Property tests confirm
a convex n-gon always yields exactly `n - 2` triangles with the polygon's area
preserved.

## Materials & textures

`parse_mtl(text)` parses a `.mtl` into a name→`Material` map plus warnings, mapping
the MTL subset directly onto the renderer's `Material`: `Kd` → `base_color`, `Ke` →
`emissive`, `Ns` → `ks` (rescaled). `map_Kd` texture references are deferred to the
asset layer.

Image textures are decoded by `Texture`, which sniffs magic bytes:

```rust,ignore
use xre::scene::Texture;
let tex  = Texture::decode(bytes)?;   // PGM/PPM (P2/P3/P5/P6) or 24-bit BMP
let rgb  = tex.sample(uv);            // bilinear, wrapping -> [u8; 3]
let luma = tex.sample_luma(uv);       // Rec. 709 luma in 0.0..=1.0
```

Supported formats are Netpbm PGM/PPM (ASCII `P2`/`P3` and binary `P5`/`P6`) and
24-bit uncompressed BMP; an unrecognized magic yields `TextureError::Unknown`. You
can also build one directly with `Texture::from_rgb(width, height, pixels)` or
generate a `Texture::checkerboard(size, a, b)` — the checkerboard is a
UV-correctness fixture that warps visibly if perspective interpolation regresses.

## The scene graph

`Scene` is an arena: nodes live in a `Vec` keyed by `NodeId(usize)`, each holding a
local `Transform`, a parent link, children, and a `NodeKind` — one of `Empty` (a
pure pivot), `Mesh { mesh: Arc<Mesh>, material }`, `Light(Light)`, or
`Camera(Camera)`. Meshes are `Arc`-shared, so many nodes **instance** one geometry.

```rust,ignore
use std::sync::Arc;
use xre::prelude::*;            // Transform, Vec3, Mesh, Material
use xre::scene::Scene;

let mut scene = Scene::new();   // contains only an identity root
let mesh = Arc::new(Mesh::cube());

// Instance one mesh under the root 1000 times:
for i in 0..1000 {
    scene.add_mesh(
        Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
        Arc::clone(&mesh),
        Material::default(),
    );
}
```

Use `scene.add(parent, transform, kind)` for arbitrary parenting, `add_mesh(...)`
as a shortcut under the root, and `scene.name(id, "...")` / `scene.find("...")` for
named lookups.

**World matrices use a dirty flag.** Mutating a transform (`set_transform`,
`transform_mut`) or adding a node sets a dirty bit; `update_world_matrices()` is a
cheap no-op unless something changed, otherwise it does a breadth-first
parent-before-child pass so each node's `world = parent_world * local`.
`reparent(id, new_parent)` preserves a node's *world* transform.

**Draw-list extraction hoists scene queries out of the pixel loop.**
`draw_list(view_proj: Option<Mat4>)` recomputes world matrices, then collects every
visible `Mesh` node into a flat `Vec<DrawItem>` where each `DrawItem { world, mesh,
material }` is ready for the renderer:

```rust,ignore
let vp = camera.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
for item in scene.draw_list(Some(vp)) {
    draw_mesh(&mut samples, &item.mesh, item.world, vp, &rig,
              &item.material, ShadeMode::PerSample, Cull::Back);
}
```

Pass `Some(view_proj)` to **frustum-cull**: each mesh's world-space AABB has its 8
corners tested against the six clip planes, and a node is dropped only if every
corner falls outside one plane (conservative). Pass `None` to skip culling.
`scene.lights()` collects `Light`s resolved into world space.

## Camera controllers

Controllers are fed *deltas*, not raw keys, so they work under any input mapping.
`OrbitController` orbits a target; `FpsController` is a free-fly camera with true
pitch. Both expose `apply(&mut Camera)`, which writes a look-at `Transform`:

```rust,ignore
use xre::prelude::*;            // OrbitController, Camera, Vec3
let mut orbit = OrbitController::new(Vec3::ZERO, 3.0);
orbit.rotate(0.6, 0.3);        // yaw / pitch deltas in radians (pitch clamped)
orbit.zoom(0.9);               // multiplicative
orbit.update(dt);              // frame-rate-independent exponential damping
let mut camera = Camera::default();
orbit.apply(&mut camera);      // writes a look-at transform from the smoothed eye()
```

`FpsController::new(position)` exposes `look(dyaw, dpitch)` (pitch clamped),
`forward()`, and `move_local(right, up, forward, dt)`; call `apply(&mut camera)`
afterward.

## Worked example: load and render a frame

This mirrors the snapshot path in the `xre view` CLI: load an OBJ, fit it to the
unit sphere, rasterize into a `SampleBuffer`, then resolve glyphs with a
`CellShader`. It is pure and terminal-free — which is exactly why the snapshot path
is unit-tested.

```rust,ignore
use xre::core::math::{UVec2, Vec3};
use xre::core::Transform;
use xre::prelude::*;   // Mesh, Camera, Projection, Light, LightRig, Material,
                       // SampleBuffer, draw_mesh, ShadeMode, Cull, LuminanceRamp, CellShader

let model = xre::scene::load_obj_file(std::path::Path::new("assets/cube.obj"))?;
let mut mesh = model.combined_mesh();

// Fit to the unit sphere (what the viewer does):
let aabb = mesh.aabb();
let (center, radius) = (aabb.center(), aabb.bounding_radius().max(1e-3));
for p in &mut mesh.positions { *p = (*p - center) / radius; }

let (cols, rows) = (80u32, 40u32);
let mut samples = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
samples.clear([0, 0, 0]);

let cam = Camera::look_at(Vec3::new(2.5, 2.0, 3.5), Vec3::ZERO);
let vp = cam.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
let rig = LightRig::default().with_light(Light::directional(Vec3::new(-0.5, -0.7, -0.5)));

draw_mesh(&mut samples, &mesh, Transform::IDENTITY.to_mat4(), vp,
          &rig, &Material::default(), ShadeMode::PerSample, Cull::Back);

let shader = LuminanceRamp::default();
let mut out = String::new();
for cy in 0..rows {
    for cx in 0..cols {
        out.push(shader.shade(&samples, cx, cy).map_or(' ', |c| c.glyph));
    }
    out.push('\n');
}
print!("{out}");
```

## Trying it

The repo ships a sample model at `assets/cube.obj`. Render a headless snapshot to a
text file:

```sh
xre view assets/cube.obj --snapshot out.txt
xre view assets/cube.obj --snapshot out.txt --ascii --size 120x60
```

Run `xre view <file.obj>` with no `--snapshot` to open the interactive orbit viewer
(orbit, scroll to zoom, `m` cycles lighting, `c` cycles shaders, `q` quits). See the
[CLI reference](cli.md) for all the flags.
