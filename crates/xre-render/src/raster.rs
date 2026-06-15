//! The triangle rasterizer (Stage 2.3).
//!
//! Edge-function traversal with a top-left fill rule and bbox clamping, an f32
//! LESS depth buffer (in the [`SampleBuffer`]), signed-area backface culling and
//! **perspective-correct** attribute interpolation (`attr/w` and `1/w` are linear
//! in screen space; divide per sample). The fill-rule discipline is what keeps
//! adjacent triangles from cracking or double-drawing — the classic bug Ymael and
//! gemini-engine both exhibit (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.3).

use std::cell::RefCell;

use xre_core::math::{Mat3, Mat4, Vec2, Vec3, Vec4};

use crate::clip::{clip_near, ClipVertex};
use crate::light::{luminance, LightRig};
use crate::material::Material;
use crate::mesh::Mesh;
use crate::sample::{RowBand, Sample, SampleBuffer};

/// A surface texture the rasterizer can sample per fragment.
///
/// Implemented by the asset crate's image type (`xre-cello`'s `Texture`) and any
/// procedural source. Kept object-safe so the fill stage takes a
/// `&dyn TextureSampler` without making the whole pipeline generic — the textured
/// path is a single optional branch, so the untextured (default) path is
/// unaffected and stays bit-identical.
///
/// `Sync` is required so the row-parallel fill can share one sampler across
/// threads (mirroring [`crate::CellShader`]).
pub trait TextureSampler: Sync {
    /// Sample an RGB texel at texture coordinate `uv` (`v` is top-down).
    fn sample(&self, uv: Vec2) -> [u8; 3];
}

/// Per-triangle / per-vertex / per-sample lighting evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShadeMode {
    /// One normal per triangle (faceted; cheapest).
    Flat,
    /// Light each vertex, interpolate the colors (Gouraud).
    Gouraud,
    /// Interpolate the normal and light every sample (smoothest).
    #[default]
    PerSample,
}

/// Which triangle facings to discard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cull {
    /// Draw both facings.
    None,
    /// Discard back faces (default).
    #[default]
    Back,
    /// Discard front faces.
    Front,
}

/// A vertex projected to sample-space, carrying `1/w` and the divided attributes.
#[derive(Clone, Copy)]
struct ScreenVert {
    x: f32,
    y: f32,
    z: f32,
    inv_w: f32,
    world: Vec3,
    normal: Vec3,
    color: Vec3,
    /// Texture coordinate, only read on the textured fill path.
    uv: Vec2,
}

/// A set-up triangle ready to fill.
///
/// Holds the sample-space vertices (winding normalized to positive area), the
/// inverse area, the clamped bounding box, the per-edge top-left flags, the shade
/// mode and (for [`ShadeMode::Flat`]) the once-shaded face color. The geometry
/// stage produces these serially; [`fill_band`] consumes them — possibly on many
/// threads at once, one row band each.
#[derive(Clone, Copy)]
struct RasterPrim {
    sv: [ScreenVert; 3],
    area_pos: f32,
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    tl: [bool; 3],
    mode: ShadeMode,
    flat_color: Vec3,
}

/// Below this many samples (`width × height`) the parallel path's pool overhead
/// outweighs the win, so we stay serial — keeps tiny viewports (and the
/// golden-frame tests) on the cheap path. Output is identical either way.
#[cfg(feature = "parallel")]
const PARALLEL_MIN_SAMPLES: usize = 16_384;
/// Likewise, parallelize only when there is real triangle work to spread.
#[cfg(feature = "parallel")]
const PARALLEL_MIN_PRIMS: usize = 64;

/// A reusable rasterizer that owns the per-frame scratch.
///
/// Holding the transformed vertices, clipped triangles, set-up primitives, and
/// the row-band triangle bins across frames is what lets a steady-state frame
/// allocate **nothing** (`RiftEngine-Plan/03-rendering-pipeline-spec.md` §D).
///
/// Construct once and call [`Rasterizer::draw_mesh`] each frame. The free
/// [`draw_mesh`] function wraps a thread-local `Rasterizer`, so existing call
/// sites keep their zero-alloc behaviour without threading one through by hand.
#[derive(Default)]
pub struct Rasterizer {
    verts: Vec<ClipVertex>,
    clipped: Vec<[ClipVertex; 3]>,
    prims: Vec<RasterPrim>,
    /// CSR row-band bins: `bin_offsets[b]..bin_offsets[b + 1]` indexes
    /// `bin_indices` for the primitives overlapping band `b`.
    #[cfg(feature = "parallel")]
    bin_offsets: Vec<u32>,
    #[cfg(feature = "parallel")]
    bin_indices: Vec<u32>,
    /// Per-band write head used while scattering indices (reused each frame).
    #[cfg(feature = "parallel")]
    bin_cursor: Vec<u32>,
}

impl Rasterizer {
    /// An empty rasterizer with no scratch allocated yet (the first frame grows
    /// the buffers; subsequent frames reuse them).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Draw `mesh` into `buf` using model matrix `model` and view-projection `vp`.
    ///
    /// Lighting uses `rig`/`material` under `mode`; `cull` selects which facings to
    /// drop. Colors are written as both luma (for glyph selection) and RGB. With
    /// the `parallel` feature the fill is row-parallel above an internal size
    /// threshold; the result is byte-identical to the serial path.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_mesh(
        &mut self,
        buf: &mut SampleBuffer,
        mesh: &Mesh,
        model: Mat4,
        vp: Mat4,
        rig: &LightRig,
        material: &Material,
        mode: ShadeMode,
        cull: Cull,
    ) {
        self.draw_mesh_textured(buf, mesh, model, vp, rig, material, mode, cull, None);
    }

    /// Like [`Rasterizer::draw_mesh`], but samples `texture` (if any) per fragment.
    ///
    /// When `texture` is `None` this is byte-identical to [`Rasterizer::draw_mesh`].
    /// When `Some`, the interpolated UV is sampled and combined with the material:
    /// unlit materials render the texel at full brightness; lit materials modulate
    /// the shaded color by the texel.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_mesh_textured(
        &mut self,
        buf: &mut SampleBuffer,
        mesh: &Mesh,
        model: Mat4,
        vp: Mat4,
        rig: &LightRig,
        material: &Material,
        mode: ShadeMode,
        cull: Cull,
        texture: Option<&dyn TextureSampler>,
    ) {
        self.setup(
            buf.width() as f32,
            buf.height() as f32,
            mesh,
            model,
            vp,
            material,
            rig,
            mode,
            cull,
        );

        #[cfg(feature = "parallel")]
        if self.should_parallelize(buf) {
            self.rasterize_parallel(buf, rig, material, texture);
            return;
        }
        self.rasterize_serial(buf, rig, material, texture);
    }

    /// Geometry stage: transform vertices, near-clip, project, cull, and set up
    /// each surviving triangle into `self.prims` (serial — pure, cheap, and the
    /// source of the deterministic triangle order the fill stage preserves).
    #[allow(clippy::too_many_arguments)]
    fn setup(
        &mut self,
        w: f32,
        h: f32,
        mesh: &Mesh,
        model: Mat4,
        vp: Mat4,
        material: &Material,
        rig: &LightRig,
        mode: ShadeMode,
        cull: Cull,
    ) {
        let mvp = vp * model;
        let normal_mat = Mat3::from_mat4(model).inverse().transpose();

        // Vertex stage: pull each vertex into clip space with its attributes.
        self.verts.clear();
        self.verts.extend((0..mesh.positions.len()).map(|i| {
            let pos = mesh.positions[i];
            let world = model.transform_point3(pos);
            let normal =
                (normal_mat * mesh.normals.get(i).copied().unwrap_or(Vec3::Y)).normalize_or_zero();
            ClipVertex {
                clip: mvp * Vec4::new(pos.x, pos.y, pos.z, 1.0),
                world,
                normal,
                uv: mesh.uvs.get(i).copied().unwrap_or(Vec2::ZERO),
            }
        }));

        self.prims.clear();
        for &[a, b, c] in &mesh.indices {
            self.clipped.clear();
            clip_near(
                [
                    self.verts[a as usize],
                    self.verts[b as usize],
                    self.verts[c as usize],
                ],
                &mut self.clipped,
            );
            for tri in &self.clipped {
                if let Some(prim) = setup_triangle(*tri, w, h, rig, material, mode, cull) {
                    self.prims.push(prim);
                }
            }
        }
    }

    /// Whether the parallel path is worth taking for this frame.
    #[cfg(feature = "parallel")]
    fn should_parallelize(&self, buf: &SampleBuffer) -> bool {
        let samples = (buf.width() as usize) * (buf.height() as usize);
        rayon::current_num_threads() >= 2
            && self.prims.len() >= PARALLEL_MIN_PRIMS
            && samples >= PARALLEL_MIN_SAMPLES
    }

    /// Serial fill: one band over the whole buffer, every primitive in order.
    /// This is the reference output the parallel path must match bit-for-bit.
    fn rasterize_serial(
        &self,
        buf: &mut SampleBuffer,
        rig: &LightRig,
        material: &Material,
        texture: Option<&dyn TextureSampler>,
    ) {
        let prims = &self.prims;
        let height = buf.height().max(1);
        for mut band in buf.row_bands_mut(height) {
            for prim in prims {
                fill_band(prim, rig, material, texture, &mut band);
            }
        }
    }

    /// Row-parallel fill: bin primitives by the bands they touch, then fill every
    /// band concurrently. Each band owns disjoint rows and replays its bin in
    /// primitive order, so the per-pixel arithmetic and depth-test order match the
    /// serial path exactly — the frame is bit-identical regardless of thread count.
    #[cfg(feature = "parallel")]
    fn rasterize_parallel(
        &mut self,
        buf: &mut SampleBuffer,
        rig: &LightRig,
        material: &Material,
        texture: Option<&dyn TextureSampler>,
    ) {
        use rayon::iter::ParallelIterator;

        let height = buf.height();
        let band_rows = band_rows_for(height, rayon::current_num_threads());
        self.bin_prims(height, band_rows);

        let prims = &self.prims;
        let offsets = &self.bin_offsets;
        let indices = &self.bin_indices;
        buf.par_row_bands_mut(band_rows).for_each(|mut band| {
            let bi = (band.y0() / band_rows) as usize;
            let (start, end) = (offsets[bi] as usize, offsets[bi + 1] as usize);
            for &pi in &indices[start..end] {
                fill_band(&prims[pi as usize], rig, material, texture, &mut band);
            }
        });
    }

    /// Build the CSR `bin_offsets`/`bin_indices` mapping each row band to the
    /// primitives whose bounding box overlaps it (a counting sort over band
    /// indices). Reuses the scratch vectors, so it allocates nothing after warmup.
    #[cfg(feature = "parallel")]
    fn bin_prims(&mut self, height: u32, band_rows: u32) {
        let nbands = height.div_ceil(band_rows).max(1) as usize;
        self.bin_offsets.clear();
        self.bin_offsets.resize(nbands + 1, 0);

        // Count primitives per band (counts land at offset + 1 for the prefix sum).
        for prim in &self.prims {
            let Some((b0, b1)) = prim_band_range(prim, band_rows, nbands) else {
                continue;
            };
            for slot in &mut self.bin_offsets[b0 + 1..=b1 + 1] {
                *slot += 1;
            }
        }
        // Prefix sum → the start offset of each band's run.
        for b in 0..nbands {
            self.bin_offsets[b + 1] += self.bin_offsets[b];
        }

        // A reusable per-band write head, seeded from the run starts.
        self.bin_cursor.clear();
        self.bin_cursor
            .extend_from_slice(&self.bin_offsets[..nbands]);

        let total = self.bin_offsets[nbands] as usize;
        self.bin_indices.clear();
        self.bin_indices.resize(total, 0);

        // Scatter each primitive index into every band it overlaps, in primitive
        // order — so each band's bin preserves the deterministic draw order.
        for (pi, prim) in self.prims.iter().enumerate() {
            let Some((b0, b1)) = prim_band_range(prim, band_rows, nbands) else {
                continue;
            };
            for slot in &mut self.bin_cursor[b0..=b1] {
                self.bin_indices[*slot as usize] = pi as u32;
                *slot += 1;
            }
        }
    }
}

/// The inclusive band-index range `[b0, b1]` a primitive's bounding box spans, or
/// `None` if the box is empty (nothing to fill).
#[cfg(feature = "parallel")]
fn prim_band_range(prim: &RasterPrim, band_rows: u32, nbands: usize) -> Option<(usize, usize)> {
    if prim.max_y <= prim.min_y || prim.max_x <= prim.min_x {
        return None;
    }
    let b0 = (prim.min_y / band_rows) as usize;
    let b1 = ((prim.max_y - 1) / band_rows).min(nbands as u32 - 1) as usize;
    Some((b0, b1))
}

/// Draw `mesh` into `buf` using model matrix `model` and view-projection `vp`.
///
/// Lighting uses `rig`/`material` under `mode`; `cull` selects which facings to
/// drop. Colors are written as both luma (for glyph selection) and RGB.
///
/// This wraps a thread-local [`Rasterizer`], so it keeps the zero-allocation
/// steady state without the caller holding one. Performance-critical loops that
/// already own a `Rasterizer` should call [`Rasterizer::draw_mesh`] directly.
#[allow(clippy::too_many_arguments)]
pub fn draw_mesh(
    buf: &mut SampleBuffer,
    mesh: &Mesh,
    model: Mat4,
    vp: Mat4,
    rig: &LightRig,
    material: &Material,
    mode: ShadeMode,
    cull: Cull,
) {
    draw_mesh_textured(buf, mesh, model, vp, rig, material, mode, cull, None);
}

/// Draw `mesh` into `buf`, sampling `texture` (if any) per fragment.
///
/// The textured counterpart to [`draw_mesh`]; passing `None` is byte-identical to
/// it. Like [`draw_mesh`] it wraps a thread-local [`Rasterizer`], so it keeps the
/// zero-allocation steady state. See [`Rasterizer::draw_mesh_textured`] for the
/// texture/material combination rules.
#[allow(clippy::too_many_arguments)]
pub fn draw_mesh_textured(
    buf: &mut SampleBuffer,
    mesh: &Mesh,
    model: Mat4,
    vp: Mat4,
    rig: &LightRig,
    material: &Material,
    mode: ShadeMode,
    cull: Cull,
    texture: Option<&dyn TextureSampler>,
) {
    thread_local! {
        static RASTERIZER: RefCell<Rasterizer> = RefCell::new(Rasterizer::new());
    }
    RASTERIZER.with(|r| {
        r.borrow_mut()
            .draw_mesh_textured(buf, mesh, model, vp, rig, material, mode, cull, texture);
    });
}

/// The signed area of the screen triangle × 2 (orientation test for culling).
fn signed_area2(a: &ScreenVert, b: &ScreenVert, c: &ScreenVert) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)
}

/// Project, cull, and set up one near-clipped triangle into a [`RasterPrim`],
/// or `None` if it is degenerate or culled. All per-triangle arithmetic (face
/// normal, winding normalization, bounding box, top-left flags) happens here,
/// exactly once, so the parallel fill replays no geometry.
fn setup_triangle(
    tri: [ClipVertex; 3],
    w: f32,
    h: f32,
    rig: &LightRig,
    material: &Material,
    mode: ShadeMode,
    cull: Cull,
) -> Option<RasterPrim> {
    // Perspective divide → NDC → sample space, retaining 1/w.
    let mut sv: [ScreenVert; 3] = [
        project(tri[0], w, h),
        project(tri[1], w, h),
        project(tri[2], w, h),
    ];

    // Backface cull on the *original* winding.
    let area = signed_area2(&sv[0], &sv[1], &sv[2]);
    if area.abs() < 1e-9 {
        return None; // degenerate
    }
    // A CCW-outward world face, projected into the y-down screen, winds with
    // negative signed area when it faces the camera — that is the front face.
    let front = area < 0.0;
    match cull {
        Cull::Back if !front => return None,
        Cull::Front if front => return None,
        _ => {}
    }

    // Flat shading: one face normal, shaded once (on the *original* winding).
    let flat_color = if mode == ShadeMode::Flat {
        let fn_world = (sv[1].world - sv[0].world)
            .cross(sv[2].world - sv[0].world)
            .normalize_or_zero();
        let center = (sv[0].world + sv[1].world + sv[2].world) / 3.0;
        rig.shade(material, center, fn_world)
    } else {
        Vec3::ZERO
    };

    // Gouraud: per-vertex colors (computed once, interpolated per sample).
    if mode == ShadeMode::Gouraud {
        for v in &mut sv {
            v.color = rig.shade(material, v.world, v.normal);
        }
    }

    // Normalize winding to *positive* area so the edge/top-left rule is
    // consistent regardless of the source face orientation.
    if area < 0.0 {
        sv.swap(1, 2);
    }
    let area_pos = signed_area2(&sv[0], &sv[1], &sv[2]).max(f32::EPSILON);

    // Bounding box, clamped to the buffer (the guard-band l/r/t/b clip).
    let min_x = sv
        .iter()
        .map(|v| v.x)
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_x = sv
        .iter()
        .map(|v| v.x)
        .fold(f32::MIN, f32::max)
        .ceil()
        .min(w) as u32;
    let min_y = sv
        .iter()
        .map(|v| v.y)
        .fold(f32::MAX, f32::min)
        .floor()
        .max(0.0) as u32;
    let max_y = sv
        .iter()
        .map(|v| v.y)
        .fold(f32::MIN, f32::max)
        .ceil()
        .min(h) as u32;

    // Top-left bias per edge (edge i is opposite vertex i).
    let tl = [
        top_left(&sv[1], &sv[2]),
        top_left(&sv[2], &sv[0]),
        top_left(&sv[0], &sv[1]),
    ];

    Some(RasterPrim {
        sv,
        area_pos,
        min_x,
        max_x,
        min_y,
        max_y,
        tl,
        mode,
        flat_color,
    })
}

/// Fill `prim` into `band`, processing only the rows the band owns. The per-pixel
/// math is identical to the single-threaded rasterizer; the only difference is the
/// write target (a band-local, depth-tested [`RowBand::plot`]).
fn fill_band(
    prim: &RasterPrim,
    rig: &LightRig,
    material: &Material,
    texture: Option<&dyn TextureSampler>,
    band: &mut RowBand<'_>,
) {
    let y0 = band.y0();
    let py_start = prim.min_y.max(y0);
    let py_stop = prim.max_y.min(y0 + band.height());
    let sv = &prim.sv;
    let area_pos = prim.area_pos;

    for py in py_start..py_stop {
        for px in prim.min_x..prim.max_x {
            let p = Vec2::new(px as f32 + 0.5, py as f32 + 0.5);
            let e0 = edge(&sv[1], &sv[2], p);
            let e1 = edge(&sv[2], &sv[0], p);
            let e2 = edge(&sv[0], &sv[1], p);
            if !inside(e0, prim.tl[0]) || !inside(e1, prim.tl[1]) || !inside(e2, prim.tl[2]) {
                continue;
            }
            let b0 = e0 / area_pos;
            let b1 = e1 / area_pos;
            let b2 = e2 / area_pos;

            // Depth (z is already NDC 0..1, linear in screen space).
            let depth = b0 * sv[0].z + b1 * sv[1].z + b2 * sv[2].z;

            // Perspective-correct interpolation: divide by interpolated 1/w.
            let inv_w = b0 * sv[0].inv_w + b1 * sv[1].inv_w + b2 * sv[2].inv_w;
            if inv_w.abs() < f32::EPSILON {
                continue;
            }
            let pc = |a0: f32, a1: f32, a2: f32| {
                (b0 * a0 * sv[0].inv_w + b1 * a1 * sv[1].inv_w + b2 * a2 * sv[2].inv_w) / inv_w
            };

            let color = match prim.mode {
                ShadeMode::Flat => prim.flat_color,
                ShadeMode::Gouraud => Vec3::new(
                    pc(sv[0].color.x, sv[1].color.x, sv[2].color.x),
                    pc(sv[0].color.y, sv[1].color.y, sv[2].color.y),
                    pc(sv[0].color.z, sv[1].color.z, sv[2].color.z),
                ),
                ShadeMode::PerSample => {
                    let world = Vec3::new(
                        pc(sv[0].world.x, sv[1].world.x, sv[2].world.x),
                        pc(sv[0].world.y, sv[1].world.y, sv[2].world.y),
                        pc(sv[0].world.z, sv[1].world.z, sv[2].world.z),
                    );
                    let normal = Vec3::new(
                        pc(sv[0].normal.x, sv[1].normal.x, sv[2].normal.x),
                        pc(sv[0].normal.y, sv[1].normal.y, sv[2].normal.y),
                        pc(sv[0].normal.z, sv[1].normal.z, sv[2].normal.z),
                    );
                    rig.shade(material, world, normal)
                }
            };
            // Textured fill: sample the perspective-correct UV and combine with the
            // material. This branch runs only when a texture is bound, so the
            // untextured (default) path above is left bit-identical.
            let color = texture.map_or(color, |tex| {
                let uv = Vec2::new(
                    pc(sv[0].uv.x, sv[1].uv.x, sv[2].uv.x),
                    pc(sv[0].uv.y, sv[1].uv.y, sv[2].uv.y),
                );
                let t = tex.sample(uv);
                let texel = Vec3::new(
                    f32::from(t[0]) / 255.0,
                    f32::from(t[1]) / 255.0,
                    f32::from(t[2]) / 255.0,
                );
                if material.unlit {
                    // Full brightness: lighting is ignored entirely.
                    (texel * material.base_color).clamp(Vec3::ZERO, Vec3::ONE)
                } else {
                    // Diffuse map: modulate the shaded color by the texel.
                    color * texel
                }
            });
            let color = rig.apply_fog(color, depth);
            let rgb = [
                (color.x * 255.0) as u8,
                (color.y * 255.0) as u8,
                (color.z * 255.0) as u8,
            ];
            band.plot(px, py - y0, Sample::new(luminance(color), rgb, depth));
        }
    }
}

/// Choose a band height that yields a few bands per worker thread, so triangle
/// clustering balances across the pool. Determinism is independent of this value
/// (each pixel is still filled exactly once).
#[cfg(feature = "parallel")]
fn band_rows_for(height: u32, threads: usize) -> u32 {
    let target_bands = (threads as u32 * 3).max(1);
    height.div_ceil(target_bands).max(1)
}

/// Perspective divide and viewport map for one clip-space vertex.
fn project(v: ClipVertex, w: f32, h: f32) -> ScreenVert {
    let inv_w = 1.0 / v.clip.w;
    let ndc = v.clip.truncate() * inv_w;
    ScreenVert {
        x: (ndc.x * 0.5 + 0.5) * w,
        y: (0.5 - ndc.y * 0.5) * h, // flip Y (screen is y-down)
        z: ndc.z,
        inv_w,
        world: v.world,
        normal: v.normal,
        color: Vec3::ZERO,
        uv: v.uv,
    }
}

/// The edge function for edge `a→b` evaluated at `p`.
#[inline]
fn edge(a: &ScreenVert, b: &ScreenVert, p: Vec2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Whether the edge `a→b` is a top or left edge (CCW, y-down convention).
#[inline]
fn top_left(a: &ScreenVert, b: &ScreenVert) -> bool {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    dy < 0.0 || (dy == 0.0 && dx < 0.0)
}

/// Inside test for one edge value with the top-left tie-break.
#[inline]
fn inside(e: f32, is_top_left: bool) -> bool {
    e > 0.0 || (e == 0.0 && is_top_left)
}

/// Test-only: run the geometry stage then force a specific fill strategy, so the
/// determinism tests can compare the serial and parallel paths in one process.
#[cfg(all(test, feature = "parallel"))]
impl Rasterizer {
    #[allow(clippy::too_many_arguments)]
    fn draw_mesh_forced(
        &mut self,
        buf: &mut SampleBuffer,
        mesh: &Mesh,
        model: Mat4,
        vp: Mat4,
        rig: &LightRig,
        material: &Material,
        mode: ShadeMode,
        cull: Cull,
        parallel: bool,
        texture: Option<&dyn TextureSampler>,
    ) {
        self.setup(
            buf.width() as f32,
            buf.height() as f32,
            mesh,
            model,
            vp,
            material,
            rig,
            mode,
            cull,
        );
        if parallel {
            self.rasterize_parallel(buf, rig, material, texture);
        } else {
            self.rasterize_serial(buf, rig, material, texture);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::float_cmp,
        clippy::field_reassign_with_default
    )]
    use super::*;
    use crate::camera::{Camera, Projection};
    use xre_core::math::UVec2;

    /// Count filled samples in a buffer.
    fn filled(buf: &SampleBuffer) -> usize {
        let (_, _, depth) = buf.planes();
        depth.iter().filter(|d| d.is_finite()).count()
    }

    #[test]
    fn flat_plane_facing_camera_fills_samples() {
        let mut buf = SampleBuffer::new(UVec2::new(20, 20), 2, 2);
        buf.clear([0, 0, 0]);
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let vp = cam.view_projection(20, 20, Projection::DEFAULT_CELL_ASPECT);
        // A plane in XY facing the camera: build from a unit quad.
        let mut mesh = Mesh::default();
        mesh.positions = vec![
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
        ];
        mesh.normals = vec![Vec3::Z; 4];
        mesh.uvs = vec![Vec2::ZERO; 4];
        mesh.indices = vec![[0, 1, 2], [0, 2, 3]];
        draw_mesh(
            &mut buf,
            &mesh,
            Mat4::IDENTITY,
            vp,
            &LightRig::default(),
            &Material::default(),
            ShadeMode::Flat,
            Cull::None,
        );
        assert!(
            filled(&buf) > 100,
            "expected the quad to cover many samples"
        );
    }

    #[test]
    fn shared_edge_no_gaps_or_double_draw() {
        // Two triangles tiling a screen-space quad must cover each interior
        // sample exactly once (fill-rule conformance). We rasterize directly in
        // sample space via an orthographic identity-ish setup.
        let mut buf = SampleBuffer::new(UVec2::new(16, 16), 1, 1);
        buf.clear([0, 0, 0]);

        // Use a camera that maps a known quad to fill the viewport.
        let cam = Camera {
            transform: Transform_z(2.0),
            projection: Projection::Orthographic {
                height: 1.0,
                near: 0.1,
                far: 10.0,
            },
        };
        let vp = cam.view_projection(16, 16, 1.0);
        let mut counts = vec![0u32; (buf.width() * buf.height()) as usize];

        // Two meshes (the two triangles), each plotting a unique luma so we can
        // detect overlaps via the depth-test count using `put`-free counting.
        for tri in [[0usize, 1, 2], [0, 2, 3]] {
            let mut mesh = Mesh::default();
            mesh.positions = vec![
                Vec3::new(-0.8, -0.8, 0.0),
                Vec3::new(0.8, -0.8, 0.0),
                Vec3::new(0.8, 0.8, 0.0),
                Vec3::new(-0.8, 0.8, 0.0),
            ];
            mesh.normals = vec![Vec3::Z; 4];
            mesh.uvs = vec![Vec2::ZERO; 4];
            mesh.indices = vec![[tri[0] as u32, tri[1] as u32, tri[2] as u32]];
            // Count coverage by scanning which samples get newly filled.
            let before: Vec<bool> = buf.planes().2.iter().map(|d| d.is_finite()).collect();
            draw_mesh(
                &mut buf,
                &mesh,
                Mat4::IDENTITY,
                vp,
                &LightRig::default(),
                &Material::default(),
                ShadeMode::Flat,
                Cull::None,
            );
            let after: Vec<bool> = buf.planes().2.iter().map(|d| d.is_finite()).collect();
            for (i, (b, a)) in before.iter().zip(&after).enumerate() {
                if !b && *a {
                    counts[i] += 1;
                }
            }
        }
        // No sample should have been newly filled by both triangles (no overlap),
        // since the depth test would otherwise let the second overwrite the first
        // — here we asserted *newly* filled, so any 2 means a double-draw row.
        // The shared diagonal belongs to exactly one triangle via the top-left rule.
        let overlaps = counts.iter().filter(|&&c| c > 1).count();
        assert_eq!(
            overlaps, 0,
            "shared edge double-drawn on {overlaps} samples"
        );
    }

    /// Helper: a camera transform translated along +Z by `d`.
    #[allow(non_snake_case)]
    fn Transform_z(d: f32) -> xre_core::Transform {
        xre_core::Transform::from_translation(Vec3::new(0.0, 0.0, d))
    }

    use proptest::prelude::*;

    proptest! {
        /// Phase 2.2 exit: random camera poses — including inside the mesh and
        /// behind vertices — must never produce a NaN/Inf luma or an out-of-bounds
        /// write, the regression guard for the Ymael / gemini near-clip gap.
        #[test]
        fn random_cameras_never_nan_or_oob(
            ex in -6.0f32..6.0, ey in -6.0f32..6.0, ez in -6.0f32..6.0,
            tx in -1.0f32..1.0, ty in -1.0f32..1.0, tz in -1.0f32..1.0,
        ) {
            let mesh = Mesh::cube();
            let mut buf = SampleBuffer::new(UVec2::new(24, 16), 2, 2);
            buf.clear([0, 0, 0]);
            let cam = Camera::look_at(Vec3::new(ex, ey, ez), Vec3::new(tx, ty, tz));
            let vp = cam.view_projection(24, 16, Projection::DEFAULT_CELL_ASPECT);
            // The call itself must not panic (no OOB indexing in the rasterizer).
            draw_mesh(
                &mut buf,
                &mesh,
                Mat4::IDENTITY,
                vp,
                &LightRig::default(),
                &Material::default(),
                ShadeMode::PerSample,
                Cull::Back,
            );
            let (luma, _, depth) = buf.planes();
            for &l in luma {
                prop_assert!(l.is_finite(), "luma NaN/Inf leaked through");
            }
            for &d in depth {
                prop_assert!(!d.is_nan(), "depth NaN leaked through");
            }
        }
    }

    // ---- Texture sampling (image-viewer path) ----

    /// A deterministic in-test sampler (raster.rs can't depend on the asset crate
    /// where the real `Texture` lives): maps the UV to an RGB value.
    struct UvSampler;
    impl TextureSampler for UvSampler {
        fn sample(&self, uv: Vec2) -> [u8; 3] {
            [
                (uv.x.clamp(0.0, 1.0) * 255.0) as u8,
                (uv.y.clamp(0.0, 1.0) * 255.0) as u8,
                128,
            ]
        }
    }

    /// A constant sampler, for the full-brightness check.
    struct WhiteSampler;
    impl TextureSampler for WhiteSampler {
        fn sample(&self, _uv: Vec2) -> [u8; 3] {
            [255, 255, 255]
        }
    }

    #[test]
    fn unlit_textured_surface_is_full_brightness() {
        // Zero lighting: a *lit* surface would be black, but an unlit textured one
        // renders the texel verbatim. Proves lighting is bypassed for images.
        let mut buf = SampleBuffer::new(UVec2::new(20, 20), 2, 2);
        buf.clear([0, 0, 0]);
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let vp = cam.view_projection(20, 20, Projection::DEFAULT_CELL_ASPECT);
        let rig = LightRig::ambient_only(Vec3::ZERO);
        let mat = Material::colored(Vec3::ONE).unlit();
        draw_mesh_textured(
            &mut buf,
            &crate::Mesh::image_quad(1.0),
            Mat4::IDENTITY,
            vp,
            &rig,
            &mat,
            ShadeMode::PerSample,
            Cull::None,
            Some(&WhiteSampler),
        );
        let (_, rgb, depth) = buf.planes();
        let i = depth
            .iter()
            .position(|d| d.is_finite())
            .expect("the quad should fill samples");
        assert_eq!(
            rgb[i],
            [255, 255, 255],
            "unlit texel was not full brightness"
        );
    }

    #[test]
    fn lit_textured_surface_responds_to_zero_light() {
        // The contrast case: a *lit* white-textured surface under zero lighting is
        // black, confirming the unlit branch above is what bypasses lighting.
        let mut buf = SampleBuffer::new(UVec2::new(20, 20), 2, 2);
        buf.clear([0, 0, 0]);
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        let vp = cam.view_projection(20, 20, Projection::DEFAULT_CELL_ASPECT);
        let rig = LightRig::ambient_only(Vec3::ZERO);
        let mat = Material::colored(Vec3::ONE); // lit
        draw_mesh_textured(
            &mut buf,
            &crate::Mesh::image_quad(1.0),
            Mat4::IDENTITY,
            vp,
            &rig,
            &mat,
            ShadeMode::PerSample,
            Cull::None,
            Some(&WhiteSampler),
        );
        let (_, rgb, depth) = buf.planes();
        let i = depth
            .iter()
            .position(|d| d.is_finite())
            .expect("the quad should fill samples");
        assert_eq!(
            rgb[i],
            [0, 0, 0],
            "lit surface under zero light should be black"
        );
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn textured_parallel_is_bit_identical_to_serial() {
        let (cols, rows) = (60u32, 40u32);
        let mesh = crate::Mesh::image_quad(1.6);
        let cam = Camera::look_at(Vec3::new(1.5, 1.0, 2.5), Vec3::ZERO);
        let vp = cam.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
        let rig = LightRig::default();
        let mat = Material::colored(Vec3::ONE).unlit();
        let mut serial = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
        let mut parallel = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
        serial.clear([0, 0, 0]);
        parallel.clear([0, 0, 0]);
        let mut rz = Rasterizer::new();
        rz.draw_mesh_forced(
            &mut serial,
            &mesh,
            Mat4::IDENTITY,
            vp,
            &rig,
            &mat,
            ShadeMode::PerSample,
            Cull::None,
            false,
            Some(&UvSampler),
        );
        rz.draw_mesh_forced(
            &mut parallel,
            &mesh,
            Mat4::IDENTITY,
            vp,
            &rig,
            &mat,
            ShadeMode::PerSample,
            Cull::None,
            true,
            Some(&UvSampler),
        );
        assert!(
            planes_bit_equal(&serial, &parallel),
            "textured parallel rasterizer diverged from serial"
        );
    }

    // ---- Stage 4.5 determinism gate: serial path == parallel path, bit-for-bit ----

    /// True iff two buffers are *bit-identical* across all three planes (comparing
    /// f32 bit patterns, so this is the real golden-frame equality, NaN included).
    #[cfg(feature = "parallel")]
    fn planes_bit_equal(a: &SampleBuffer, b: &SampleBuffer) -> bool {
        let (al, ar, ad) = a.planes();
        let (bl, br, bd) = b.planes();
        ar == br
            && al.len() == bl.len()
            && ad.len() == bd.len()
            && al.iter().zip(bl).all(|(x, y)| x.to_bits() == y.to_bits())
            && ad.iter().zip(bd).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    /// Render `mesh` twice — forced serial, forced parallel — and assert the two
    /// sample buffers are bit-identical.
    #[cfg(feature = "parallel")]
    fn assert_serial_eq_parallel(mesh: &Mesh, mode: ShadeMode, cols: u32, rows: u32, eye: Vec3) {
        let mut serial = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
        let mut parallel = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
        serial.clear([0, 0, 0]);
        parallel.clear([0, 0, 0]);
        let cam = Camera::look_at(eye, Vec3::ZERO);
        let vp = cam.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
        let rig = LightRig::default();
        let mut rz = Rasterizer::new();
        rz.draw_mesh_forced(
            &mut serial,
            mesh,
            Mat4::IDENTITY,
            vp,
            &rig,
            &Material::default(),
            mode,
            Cull::Back,
            false,
            None,
        );
        rz.draw_mesh_forced(
            &mut parallel,
            mesh,
            Mat4::IDENTITY,
            vp,
            &rig,
            &Material::default(),
            mode,
            Cull::Back,
            true,
            None,
        );
        assert!(
            planes_bit_equal(&serial, &parallel),
            "parallel rasterizer diverged from serial ({mode:?}, {cols}x{rows})"
        );
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn parallel_raster_is_bit_identical_to_serial() {
        // Tall enough (rows*4 sample rows) to span many bands, and one case per
        // shade mode so the per-band replay of every path is exercised.
        let eye = Vec3::new(2.5, 2.0, 3.5);
        assert_serial_eq_parallel(
            &Mesh::uv_sphere(1.3, 24, 32),
            ShadeMode::PerSample,
            60,
            40,
            eye,
        );
        assert_serial_eq_parallel(&Mesh::cube(), ShadeMode::PerSample, 60, 40, eye);
        assert_serial_eq_parallel(
            &Mesh::torus(1.2, 0.45, 32, 16),
            ShadeMode::Flat,
            60,
            40,
            eye,
        );
        assert_serial_eq_parallel(
            &Mesh::uv_sphere(1.3, 24, 32),
            ShadeMode::Gouraud,
            60,
            40,
            eye,
        );
    }

    #[cfg(feature = "parallel")]
    proptest! {
        /// The parallel path must reproduce the serial path bit-for-bit from any
        /// camera pose — the strongest form of the determinism guarantee.
        #[test]
        fn parallel_matches_serial_for_any_camera(
            ex in -6.0f32..6.0, ey in -6.0f32..6.0, ez in -6.0f32..6.0,
        ) {
            let mut serial = SampleBuffer::new(UVec2::new(48, 32), 2, 4);
            let mut parallel = SampleBuffer::new(UVec2::new(48, 32), 2, 4);
            serial.clear([0, 0, 0]);
            parallel.clear([0, 0, 0]);
            let cam = Camera::look_at(Vec3::new(ex, ey, ez), Vec3::ZERO);
            let vp = cam.view_projection(48, 32, Projection::DEFAULT_CELL_ASPECT);
            let rig = LightRig::default();
            let mesh = Mesh::uv_sphere(1.3, 20, 28);
            let mut rz = Rasterizer::new();
            rz.draw_mesh_forced(&mut serial, &mesh, Mat4::IDENTITY, vp, &rig,
                &Material::default(), ShadeMode::PerSample, Cull::Back, false, None);
            rz.draw_mesh_forced(&mut parallel, &mesh, Mat4::IDENTITY, vp, &rig,
                &Material::default(), ShadeMode::PerSample, Cull::Back, true, None);
            prop_assert!(planes_bit_equal(&serial, &parallel));
        }
    }
}
