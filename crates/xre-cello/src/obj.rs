//! A from-scratch OBJ/MTL loader (Stage 3.1).
//!
//! No `tobj` dependency — loader robustness is a core competency for this engine
//! and we want zero-copy parsing and fuzzing control
//! (`RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.1). Handles the index forms
//! `v`, `v/vt`, `v//vn`, `v/vt/vn`, **negative (relative) indices**, n-gon faces
//! (fan for convex, [`crate::triangulate`] ear-clipping for concave), `o`/`g`
//! groups, `mtllib`/`usemtl`, line continuations, CRLF and comments. The policy
//! is **warn-and-continue**: a malformed line is recorded and skipped, never a
//! panic — so the same entry point is a `cargo fuzz` target.

use std::collections::HashMap;

use xre_core::math::{Vec2, Vec3};
use xre_render::{Material, Mesh};

use crate::triangulate::triangulate;

/// A fatal loader error (only I/O or genuinely unrecoverable structure). Most
/// problems are warnings instead; see [`ObjModel::warnings`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ObjError {
    /// A referenced file could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        /// The path that failed.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// One named sub-object of a loaded OBJ (split on `o`/`g` and `usemtl`).
#[derive(Clone, Debug)]
pub struct ObjObject {
    /// The object/group name (`o`/`g`), or `"default"`.
    pub name: String,
    /// The active material name, if any.
    pub material: Option<String>,
    /// The triangulated, index-deduplicated mesh.
    pub mesh: Mesh,
}

/// A parsed OBJ model: its sub-objects, referenced materials, and any warnings.
#[derive(Clone, Debug, Default)]
pub struct ObjModel {
    /// The sub-objects.
    pub objects: Vec<ObjObject>,
    /// Materials parsed from referenced `mtllib`s (empty unless resolved).
    pub materials: HashMap<String, Material>,
    /// `mtllib` filenames referenced by the OBJ (for the caller to resolve).
    pub material_libs: Vec<String>,
    /// Non-fatal problems encountered, with `line N:` prefixes.
    pub warnings: Vec<String>,
}

impl ObjModel {
    /// Merge every object's mesh into one combined [`Mesh`] (for simple viewers).
    #[must_use]
    pub fn combined_mesh(&self) -> Mesh {
        let mut out = Mesh::default();
        for obj in &self.objects {
            let base = out.positions.len() as u32;
            out.positions.extend_from_slice(&obj.mesh.positions);
            out.normals.extend_from_slice(&obj.mesh.normals);
            out.uvs.extend_from_slice(&obj.mesh.uvs);
            out.indices.extend(
                obj.mesh
                    .indices
                    .iter()
                    .map(|&[a, b, c]| [a + base, b + base, c + base]),
            );
        }
        out
    }

    /// Total triangle count across all objects.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.objects.iter().map(|o| o.mesh.triangle_count()).sum()
    }
}

/// Deduplicates `(v, vt, vn)` tuples into a single vertex buffer.
#[derive(Default)]
struct MeshBuilder {
    mesh: Mesh,
    map: HashMap<(i64, i64, i64), u32>,
    any_missing_normal: bool,
}

impl MeshBuilder {
    fn vertex(&mut self, key: (i64, i64, i64), pos: Vec3, uv: Vec2, normal: Option<Vec3>) -> u32 {
        if let Some(&i) = self.map.get(&key) {
            return i;
        }
        let idx = self.mesh.positions.len() as u32;
        self.mesh.positions.push(pos);
        self.mesh.uvs.push(uv);
        if let Some(n) = normal {
            self.mesh.normals.push(n);
        } else {
            self.mesh.normals.push(Vec3::ZERO);
            self.any_missing_normal = true;
        }
        self.map.insert(key, idx);
        idx
    }

    fn finish(mut self) -> Mesh {
        if self.any_missing_normal {
            self.mesh.recompute_smooth_normals();
        }
        self.mesh
    }

    const fn is_empty(&self) -> bool {
        self.mesh.indices.is_empty()
    }
}

/// Parse OBJ source text into a model (materials unresolved; see
/// [`ObjModel::material_libs`] and [`parse_mtl`]).
///
/// Never panics: malformed lines become warnings. This is the fuzz entry point.
#[must_use]
pub fn parse_obj(text: &str) -> ObjModel {
    let mut positions: Vec<Vec3> = Vec::new();
    let mut texcoords: Vec<Vec2> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut model = ObjModel::default();

    let mut name = String::from("default");
    let mut material: Option<String> = None;
    let mut builder = MeshBuilder::default();

    let flush =
        |model: &mut ObjModel, builder: &mut MeshBuilder, name: &str, mat: &Option<String>| {
            if !builder.is_empty() {
                let done = core::mem::take(builder);
                model.objects.push(ObjObject {
                    name: name.to_string(),
                    material: mat.clone(),
                    mesh: done.finish(),
                });
            }
        };

    for (line_no, raw) in logical_lines(text).into_iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(directive) = tokens.next() else {
            continue;
        };
        let rest: Vec<&str> = tokens.collect();
        let warn = |model: &mut ObjModel, msg: &str| {
            model.warnings.push(format!("line {}: {msg}", line_no + 1));
        };
        match directive {
            "v" => match parse_vec3(&rest) {
                Some(v) => positions.push(v),
                None => warn(&mut model, "malformed vertex"),
            },
            "vt" => texcoords.push(parse_vec2(&rest).unwrap_or(Vec2::ZERO)),
            "vn" => match parse_vec3(&rest) {
                Some(n) => normals.push(n),
                None => warn(&mut model, "malformed normal"),
            },
            "f" => {
                if let Err(msg) = parse_face(&rest, &positions, &texcoords, &normals, &mut builder)
                {
                    warn(&mut model, &msg);
                }
            }
            "o" | "g" => {
                flush(&mut model, &mut builder, &name, &material);
                name = rest.join(" ");
                if name.is_empty() {
                    name = "default".into();
                }
            }
            "usemtl" => {
                flush(&mut model, &mut builder, &name, &material);
                material = rest.first().map(|s| (*s).to_string());
            }
            "mtllib" => {
                // The rest of the line is a single filename, which may contain
                // spaces (e.g. `mtllib 2018 BMW M8 GTE.mtl`).
                let lib = rest.join(" ");
                if !lib.is_empty() {
                    model.material_libs.push(lib);
                }
            }
            "s" | "vp" | "l" | "p" => {} // smoothing groups / params / lines: ignored
            other => warn(&mut model, &format!("ignored directive `{other}`")),
        }
    }
    flush(&mut model, &mut builder, &name, &material);
    model
}

/// Parse a face directive into triangles, appending to `builder`.
fn parse_face(
    tokens: &[&str],
    positions: &[Vec3],
    texcoords: &[Vec2],
    normals: &[Vec3],
    builder: &mut MeshBuilder,
) -> Result<(), String> {
    if tokens.len() < 3 {
        return Err("face with fewer than 3 vertices".into());
    }
    // Resolve each face vertex to (v_idx, vt_idx, vn_idx) and its position.
    let mut loop_keys: Vec<(i64, i64, i64)> = Vec::with_capacity(tokens.len());
    let mut loop_pos: Vec<Vec3> = Vec::with_capacity(tokens.len());
    let mut loop_uv: Vec<Vec2> = Vec::with_capacity(tokens.len());
    let mut loop_n: Vec<Option<Vec3>> = Vec::with_capacity(tokens.len());
    for tok in tokens {
        let (vi, vti, vni) = parse_face_vertex(tok)?;
        let v = resolve(vi, positions.len()).ok_or("vertex index out of range")?;
        let pos = positions[v];
        let uv = vti
            .and_then(|i| resolve(i, texcoords.len()))
            .map_or(Vec2::ZERO, |i| texcoords[i]);
        let normal = vni
            .and_then(|i| resolve(i, normals.len()))
            .map(|i| normals[i]);
        loop_keys.push((v as i64, vti.unwrap_or(0), vni.unwrap_or(0)));
        loop_pos.push(pos);
        loop_uv.push(uv);
        loop_n.push(normal);
    }

    let loop_idx: Vec<u32> = (0..loop_pos.len() as u32).collect();
    for tri in triangulate(&loop_pos, &loop_idx) {
        let mut out = [0u32; 3];
        for (slot, &local) in out.iter_mut().zip(tri.iter()) {
            *slot = builder.vertex(
                loop_keys[local],
                loop_pos[local],
                loop_uv[local],
                loop_n[local],
            );
        }
        builder.mesh.indices.push(out);
    }
    Ok(())
}

/// Parse one `v/vt/vn` face-vertex token. `vt`/`vn` are optional.
fn parse_face_vertex(tok: &str) -> Result<(i64, Option<i64>, Option<i64>), String> {
    let mut parts = tok.split('/');
    let v: i64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("missing vertex index")?;
    let vt = parts
        .next()
        .and_then(|s| if s.is_empty() { None } else { s.parse().ok() });
    let vn = parts
        .next()
        .and_then(|s| if s.is_empty() { None } else { s.parse().ok() });
    Ok((v, vt, vn))
}

/// Resolve a 1-based / negative OBJ index against a list of `len` items.
fn resolve(idx: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let resolved = match idx.cmp(&0) {
        core::cmp::Ordering::Greater => idx - 1,
        core::cmp::Ordering::Less => len + idx,
        core::cmp::Ordering::Equal => return None, // 0 is invalid in OBJ
    };
    (0..len).contains(&resolved).then_some(resolved as usize)
}

/// Join physical lines ending in a backslash continuation, trimming CR.
fn logical_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(prefix) = line.strip_suffix('\\') {
            acc.push_str(prefix);
            acc.push(' ');
        } else {
            acc.push_str(line);
            out.push(core::mem::take(&mut acc));
        }
    }
    if !acc.is_empty() {
        out.push(acc);
    }
    out
}

fn parse_vec3(tokens: &[&str]) -> Option<Vec3> {
    let x = tokens.first()?.parse().ok()?;
    let y = tokens.get(1)?.parse().ok()?;
    let z = tokens.get(2)?.parse().ok()?;
    Some(Vec3::new(x, y, z))
}

fn parse_vec2(tokens: &[&str]) -> Option<Vec2> {
    let x = tokens.first()?.parse().ok()?;
    let y = tokens.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    Some(Vec2::new(x, y))
}

/// Parse MTL source text into a name→[`Material`] map.
///
/// Supports the subset `Kd`, `Ka`, `Ke`, `Ns`. `map_Kd` textures are resolved
/// later by the asset layer; unknown properties are ignored.
#[must_use]
pub fn parse_mtl(text: &str) -> (HashMap<String, Material>, Vec<String>) {
    let mut materials = HashMap::new();
    let mut warnings = Vec::new();
    let mut current: Option<(String, Material)> = None;
    for (line_no, raw) in logical_lines(text).into_iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let Some(directive) = tokens.next() else {
            continue;
        };
        let rest: Vec<&str> = tokens.collect();
        match directive {
            "newmtl" => {
                if let Some((name, mat)) = current.take() {
                    materials.insert(name, mat);
                }
                current = Some((rest.join(" "), Material::default()));
            }
            "Kd" => {
                if let (Some((_, mat)), Some(c)) = (current.as_mut(), parse_vec3(&rest)) {
                    mat.base_color = c;
                }
            }
            "Ka" => {
                if let (Some((_, mat)), Some(c)) = (current.as_mut(), parse_vec3(&rest)) {
                    mat.emissive = c * 0.0; // ambient is the rig's job; keep as 0
                    let _ = mat;
                }
            }
            "Ke" => {
                if let (Some((_, mat)), Some(c)) = (current.as_mut(), parse_vec3(&rest)) {
                    mat.emissive = c;
                }
            }
            "Ns" => {
                if let (Some((_, mat)), Some(v)) = (
                    current.as_mut(),
                    rest.first().and_then(|s| s.parse::<f32>().ok()),
                ) {
                    mat.ks = (v / 1000.0).clamp(0.0, 1.0);
                }
            }
            "newmtl_ignore" => {}
            _ => {
                if current.is_none() {
                    warnings.push(format!(
                        "line {}: material property before newmtl",
                        line_no + 1
                    ));
                }
            }
        }
    }
    if let Some((name, mat)) = current.take() {
        materials.insert(name, mat);
    }
    (materials, warnings)
}

/// Load an OBJ from a file, resolving any sibling `mtllib` files.
///
/// # Errors
/// Returns [`ObjError::Io`] if the OBJ file cannot be read. A referenced MTL
/// file that simply does not exist is silently skipped (the model renders with
/// the default material); other read failures and MTL parse problems are
/// recorded as warnings, not errors.
pub fn load_obj_file(path: &std::path::Path) -> Result<ObjModel, ObjError> {
    let text = std::fs::read_to_string(path).map_err(|source| ObjError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut model = parse_obj(&text);
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    for lib in model.material_libs.clone() {
        let lib_path = dir.join(&lib);
        match std::fs::read_to_string(&lib_path) {
            Ok(mtl_text) => {
                let (mats, mut warns) = parse_mtl(&mtl_text);
                model.materials.extend(mats);
                model.warnings.append(&mut warns);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A missing material library is benign: the OBJ still renders
                // with the default material, so don't surface it as a warning.
            }
            Err(e) => model.warnings.push(format!("mtllib {lib}: {e}")),
        }
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtllib_filename_with_spaces_is_one_lib() {
        // A filename with spaces must stay a single library, not split per word.
        let model = parse_obj("mtllib 2018 BMW M8 GTE.mtl\n");
        assert_eq!(model.material_libs, vec!["2018 BMW M8 GTE.mtl".to_string()]);
    }

    #[test]
    fn missing_mtllib_is_not_a_warning() {
        // Write a tiny OBJ referencing a non-existent .mtl and confirm the
        // loader skips it silently rather than warning.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("xre_obj_test_{}.obj", std::process::id()));
        std::fs::write(
            &path,
            "mtllib does_not_exist.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
        )
        .expect("write temp obj");
        let model = load_obj_file(&path).expect("load temp obj");
        let _ = std::fs::remove_file(&path);
        assert!(model.warnings.is_empty(), "warnings: {:?}", model.warnings);
    }

    #[test]
    fn parses_a_simple_quad() {
        let obj = "\
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3 4
";
        let model = parse_obj(obj);
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.triangle_count(), 2); // quad → 2 tris
        assert!(model.warnings.is_empty(), "warnings: {:?}", model.warnings);
    }

    #[test]
    fn handles_all_index_forms_and_negative() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vn 0 0 1
f 1/1/1 2/1/1 3/1/1
f -3 -2 -1
f 1//1 2//1 3//1
";
        let model = parse_obj(obj);
        assert_eq!(model.triangle_count(), 3);
        assert!(model.warnings.is_empty(), "warnings: {:?}", model.warnings);
    }

    #[test]
    fn generates_missing_normals() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let model = parse_obj(obj);
        let n = &model.objects[0].mesh.normals[0];
        assert!(n.length() > 0.5, "a normal should have been generated");
    }

    #[test]
    fn splits_on_object_and_material() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
usemtl red
f 1 2 3
o second
usemtl blue
f 1 2 3
";
        let model = parse_obj(obj);
        assert_eq!(model.objects.len(), 2);
        assert_eq!(model.objects[0].material.as_deref(), Some("red"));
        assert_eq!(model.objects[1].material.as_deref(), Some("blue"));
    }

    #[test]
    fn line_continuation_and_crlf() {
        let obj = "v 0 \\\r\n0 0\r\nv 1 0 0\r\nv 0 1 0\r\nf 1 2 3\r\n";
        let model = parse_obj(obj);
        assert_eq!(model.triangle_count(), 1);
        assert!(model.warnings.is_empty(), "warnings: {:?}", model.warnings);
    }

    #[test]
    fn junk_lines_warn_but_do_not_panic() {
        let obj = "v 0 0 0\nbanana 1 2 3\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let model = parse_obj(obj);
        assert_eq!(model.triangle_count(), 1);
        assert_eq!(model.warnings.len(), 1);
    }

    #[test]
    fn out_of_range_face_is_skipped_not_panicked() {
        let obj = "v 0 0 0\nf 1 99 100\n";
        let model = parse_obj(obj);
        assert_eq!(model.triangle_count(), 0);
        assert!(!model.warnings.is_empty());
    }

    #[test]
    fn mtl_subset_parses() {
        let mtl = "newmtl red\nKd 0.8 0.1 0.1\nKe 0.0 0.0 0.0\n";
        let (mats, warns) = parse_mtl(mtl);
        assert!(warns.is_empty());
        assert_eq!(mats["red"].base_color, Vec3::new(0.8, 0.1, 0.1));
    }

    #[test]
    fn garbage_never_panics() {
        // A fuzz-flavored smoke test of adversarial input.
        for junk in [
            "",
            "f",
            "f 1",
            "v\n\nf //// ",
            "f 1/2/3/4/5",
            "vn nan inf -inf",
        ] {
            let _ = parse_obj(junk);
        }
    }
}
