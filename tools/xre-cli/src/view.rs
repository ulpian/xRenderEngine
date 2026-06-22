//! `xre view <file.obj>` — the engine's first real product (Stage 3.5).
//!
//! Loads an OBJ, fits it to the unit sphere, and either renders one headless
//! snapshot to a text file (`--snapshot out.txt`, the CI-friendly QA path) or
//! opens an interactive orbit viewer with a stats panel. The viewer doubles as
//! the manual QA harness for every later renderer change
//! (`RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.5).

use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use xre::core::math::{UVec2, Vec3};
use xre::core::{CellBuffer, Color, Style, Transform};
use xre::prelude::*;
use xre::term::{
    Capabilities, Event, EventQueue, KeyCode, KeyState, MouseKind, Presenter, TerminalGuard,
};

/// Image extensions `xre view` renders as a textured quad (the rest are OBJ).
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "ppm", "pgm"];

/// What the viewer is showing: geometry plus how to shade it. An OBJ is a lit,
/// back-culled untextured mesh; an image is an unlit, double-sided textured quad.
struct Subject {
    mesh: Mesh,
    material: Material,
    texture: Option<Texture>,
    cull: Cull,
    /// Stats line describing the subject (e.g. `tris   12` or `image  640x480`).
    info: String,
}

impl Subject {
    /// The texture as a sampler the rasterizer can use, if this is an image.
    fn sampler(&self) -> Option<&dyn TextureSampler> {
        self.texture.as_ref().map(|t| t as &dyn TextureSampler)
    }

    /// The initial camera pose: angled for meshes, face-on for images.
    fn start_orbit(&self) -> OrbitController {
        if self.texture.is_some() {
            face_on_orbit()
        } else {
            default_orbit()
        }
    }
}

/// Whether `path`'s extension names a supported raster image.
fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| IMAGE_EXTS.contains(&e.as_str()))
}

/// A freshly loaded [`Subject`] plus any non-fatal parse warnings. Used by the
/// interactive path, which loads off-thread and must defer printing warnings
/// until the terminal guard has been dropped (a raw-mode `eprintln!` would
/// corrupt the alternate screen).
struct LoadOutcome {
    subject: Subject,
    warnings: Vec<String>,
}

/// Build a lit, back-culled mesh subject from an already-parsed OBJ model.
fn build_obj_subject(model: &ObjModel) -> Subject {
    let mesh = fit_unit(model.combined_mesh());
    let info = format!("tris   {}", mesh.triangle_count());
    Subject {
        mesh,
        material: Material::default(),
        texture: None,
        cull: Cull::Back,
        info,
    }
}

/// Load an OBJ as a subject, returning warnings rather than printing them.
fn obj_outcome(path: &Path) -> Result<LoadOutcome, String> {
    let model = xre::scene::load_obj_file(path).map_err(|e| e.to_string())?;
    let subject = build_obj_subject(&model);
    Ok(LoadOutcome {
        subject,
        warnings: model.warnings,
    })
}

/// Load an image as a subject (images carry no parse warnings).
fn image_outcome(path: &Path) -> Result<LoadOutcome, String> {
    Ok(LoadOutcome {
        subject: load_image_subject(path)?,
        warnings: Vec::new(),
    })
}

/// Load an OBJ as a lit, back-culled mesh subject, printing warnings to stderr
/// (the synchronous snapshot path, where no TUI is on screen).
fn load_obj_subject(path: &Path) -> Result<Subject, String> {
    let outcome = obj_outcome(path)?;
    if !outcome.warnings.is_empty() {
        eprintln!(
            "xre view: {} warning(s) while loading",
            outcome.warnings.len()
        );
        for w in outcome.warnings.iter().take(5) {
            eprintln!("  {w}");
        }
    }
    Ok(outcome.subject)
}

/// Load a raster image as an unlit, double-sided textured-quad subject.
fn load_image_subject(path: &Path) -> Result<Subject, String> {
    let texture = xre::scene::load_image_file(path).map_err(|e| e.to_string())?;
    let (w, h) = (texture.width(), texture.height());
    let aspect = w as f32 / h.max(1) as f32;
    let mesh = fit_unit(Mesh::image_quad(aspect));
    let info = format!("image  {w}x{h}");
    Ok(Subject {
        mesh,
        // White + unlit ⇒ the picture's colors pass through faithfully, unaffected
        // by light position. Cull::None keeps it visible when orbited past edge-on.
        material: Material::colored(Vec3::ONE).unlit(),
        texture: Some(texture),
        cull: Cull::None,
        info,
    })
}

/// Auto-rotate yaw speed (rad/s) for the `z` toggle. Negative sign matches the
/// Left-arrow direction (orbits left/clockwise); flip to reverse.
const AUTO_YAW_RATE: f32 = 0.9;

/// Per-press orbit step (radians) for the arrow keys when viewing a mesh.
const ORBIT_KEY_STEP: f32 = 0.1;

/// Per-press image pan, as a fraction of the camera distance. Scaling by
/// distance keeps the on-screen nudge roughly constant across zoom levels.
const PAN_KEY_FRACTION: f32 = 0.1;

/// Apply one arrow-key press. `right`/`up` are unit screen directions
/// (−1, 0, or 1): an image pans across the picture in 2D, while a mesh orbits
/// (yaw / pitch). No-op until the asset has loaded (`orbit` is `None`).
fn nudge(orbit: Option<&mut OrbitController>, is_image: bool, right: f32, up: f32) {
    let Some(o) = orbit else { return };
    if is_image {
        let step = PAN_KEY_FRACTION * o.distance;
        o.pan(right * step, up * step);
    } else {
        o.rotate(right * ORBIT_KEY_STEP, up * ORBIT_KEY_STEP);
    }
}

/// Distance of the cardinal viewer lights from the origin. The model is fit to
/// the unit sphere, so 3.0 places the light clearly outside it.
const LIGHT_DISTANCE: f32 = 3.0;

/// World-space light point + label for a `t`/`b`/`l`/`r` key, else `None`.
///
/// The model sits at `Transform::IDENTITY` and only the camera orbits, so the
/// returned point stays fixed relative to the model as it is rotated — the light
/// remains static while the model moves around. Pure, so it is unit-tested
/// without a terminal.
fn light_for(key: char) -> Option<(Vec3, &'static str)> {
    let d = LIGHT_DISTANCE;
    match key {
        't' => Some((Vec3::new(0.0, d, 0.0), "top")),
        'b' => Some((Vec3::new(0.0, -d, 0.0), "bottom")),
        'l' => Some((Vec3::new(-d, 0.0, 0.0), "left")),
        'r' => Some((Vec3::new(d, 0.0, 0.0), "right")),
        _ => None,
    }
}

/// Parsed `view` arguments.
struct ViewArgs {
    path: String,
    snapshot: Option<String>,
    ascii: bool,
    width: u32,
    height: u32,
}

/// Entry point for `xre view`. Returns a human-readable error string on failure.
pub fn run(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args)?;
    let path = Path::new(&parsed.path);

    // Snapshot path: load synchronously and print warnings as usual — no TUI is
    // on screen, so stderr is safe and determinism matters for golden frames.
    if let Some(out) = &parsed.snapshot {
        let subject = if is_image_path(path) {
            load_image_subject(path)?
        } else {
            load_obj_subject(path)?
        };
        let frame = render_snapshot(
            &subject,
            parsed.width,
            parsed.height,
            subject.start_orbit(),
            parsed.ascii,
        );
        std::fs::write(out, &frame).map_err(|e| format!("writing {out}: {e}"))?;
        eprintln!(
            "xre view: wrote {}×{} snapshot to {out}",
            parsed.width, parsed.height
        );
        return Ok(());
    }

    // Interactive path: bring the TUI up immediately and load the asset on a
    // background thread, showing a spinner until it arrives. The thread owns its
    // inputs so it satisfies `'static`; the channel carries the result back.
    let owned = path.to_path_buf();
    let is_image = is_image_path(path);
    let (tx, rx) = mpsc::channel::<Result<LoadOutcome, String>>();
    // Detached on purpose: if the user quits mid-load we want an instant exit,
    // not a wait for a large parse to finish. The thread only reads a file and
    // sends one message, so process teardown reaps it cleanly.
    std::thread::spawn(move || {
        let result = if is_image {
            image_outcome(&owned)
        } else {
            obj_outcome(&owned)
        };
        // The receiver is gone if the user quit during loading — that's fine.
        let _ = tx.send(result);
    });

    let warnings = run_interactive(&rx, parsed.ascii, is_image).map_err(|e| e.to_string())?;
    // The terminal guard has been restored by now, so stderr is clean again. Any
    // warnings exist only if the load actually completed before the viewer quit.
    if !warnings.is_empty() {
        eprintln!("xre view: {} warning(s) while loading", warnings.len());
        for w in warnings.iter().take(5) {
            eprintln!("  {w}");
        }
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<ViewArgs, String> {
    let mut path = None;
    let mut snapshot = None;
    let mut ascii = false;
    let mut width = 80u32;
    let mut height = 40u32;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--snapshot" => {
                snapshot = Some(it.next().cloned().ok_or("--snapshot needs a path")?);
            }
            "--ascii" => ascii = true,
            "--size" => {
                let s = it.next().ok_or("--size needs WxH")?;
                let (w, h) = s.split_once('x').ok_or("--size must be WxH")?;
                width = w.parse().map_err(|_| "bad --size width")?;
                height = h.parse().map_err(|_| "bad --size height")?;
            }
            other if !other.starts_with('-') => path = Some(other.to_string()),
            other => return Err(format!("unknown view flag: {other}")),
        }
    }
    Ok(ViewArgs {
        path: path.ok_or(
            "usage: xre view <file.obj|image.png|.jpg|.bmp> [--snapshot out.txt] [--ascii] [--size WxH]",
        )?,
        snapshot,
        ascii,
        width,
        height,
    })
}

/// Center a mesh at the origin and scale it to fit the unit sphere.
fn fit_unit(mut mesh: Mesh) -> Mesh {
    let aabb = mesh.aabb();
    let center = aabb.center();
    let radius = aabb.bounding_radius().max(1e-3);
    for p in &mut mesh.positions {
        *p = (*p - center) / radius;
    }
    mesh
}

fn default_orbit() -> OrbitController {
    let mut o = OrbitController::new(Vec3::ZERO, 3.0);
    o.rotate(0.6, 0.3);
    for _ in 0..120 {
        o.update(1.0 / 60.0); // settle the damping
    }
    o
}

/// A face-on orbit (yaw 0, pitch 0 ⇒ eye on `+Z`) so an image quad opens flat
/// before the user spins it.
fn face_on_orbit() -> OrbitController {
    let mut o = OrbitController::new(Vec3::ZERO, 3.0);
    o.yaw = 0.0;
    o.pitch = 0.0;
    for _ in 0..120 {
        o.update(1.0 / 60.0); // settle the damping to the face-on pose
    }
    o
}

/// Render one frame of `subject` to an ASCII string (the snapshot path; pure, so
/// it is unit-tested without a terminal).
#[must_use]
fn render_snapshot(
    subject: &Subject,
    cols: u32,
    rows: u32,
    orbit: OrbitController,
    _ascii: bool,
) -> String {
    let mut samples = SampleBuffer::new(UVec2::new(cols.max(1), rows.max(1)), 2, 4);
    samples.clear([0, 0, 0]);
    let mut camera = Camera::default();
    orbit.apply(&mut camera);
    let vp = camera.view_projection(cols.max(1), rows.max(1), Projection::DEFAULT_CELL_ASPECT);
    let rig = LightRig::default().with_light(Light::directional(Vec3::new(-0.5, -0.7, -0.5)));
    draw_mesh_textured(
        &mut samples,
        &subject.mesh,
        Transform::IDENTITY.to_mat4(),
        vp,
        &rig,
        &subject.material,
        ShadeMode::PerSample,
        subject.cull,
        subject.sampler(),
    );
    let shader = LuminanceRamp::default();
    let mut out = String::with_capacity(((cols + 1) * rows) as usize);
    for cy in 0..rows {
        for cx in 0..cols {
            out.push(shader.shade(&samples, cx, cy).map_or(' ', |c| c.glyph));
        }
        out.push('\n');
    }
    out
}

/// The "Loading …" label for the spinner. Pure, so it is unit-tested.
const fn loading_message(is_image: bool) -> &'static str {
    if is_image {
        "Loading image…"
    } else {
        "Loading model…"
    }
}

/// A `0.0..=1.0` triangle wave over elapsed seconds `t`, one sweep every ~1.6 s.
/// Drives the indeterminate loader bar back and forth (there is no real progress
/// to report). Pure, so it is unit-tested without a terminal.
fn sweep_ratio(t: f32) -> f32 {
    const PERIOD: f32 = 1.6;
    let phase = (t / PERIOD).rem_euclid(1.0); // 0.0..1.0, robust for any t
    if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    }
}

/// The interactive orbit viewer.
///
/// The asset arrives over `rx` from a background loader: the TUI is drawn
/// immediately with a spinner, and the model/image replaces it once the load
/// completes. Returns the loader's parse warnings for the caller to print after
/// the terminal guard is dropped. A load error is surfaced as an `Err`.
#[allow(clippy::too_many_lines)]
fn run_interactive(
    rx: &Receiver<Result<LoadOutcome, String>>,
    ascii: bool,
    is_image: bool,
) -> std::io::Result<Vec<String>> {
    let _guard = TerminalGuard::enter().map_err(std::io::Error::other)?;
    let caps = Capabilities::probe();
    let mut presenter = Presenter::stdout(&caps);
    let mut buf = CellBuffer::new(caps.size);
    let mut events = EventQueue::new();
    let mut samples = SampleBuffer::new(UVec2::new(1, 1), 2, 4);
    // World-space light, fixed while the model is orbited. `t`/`b`/`l`/`r` snap
    // it to the cardinal positions; the startup diagonal is labelled "free".
    let mut rig = LightRig::default().with_light(Light::point(Vec3::new(3.0, 3.0, 3.0)));
    let mut light_label = "free";

    // Cell shaders cycled live with `c` (Stage 4.3): ramp, shape-vector, the
    // Unicode density modes. Shared with the demos so the set stays identical.
    let shaders = builtin_cell_shaders();
    let mut shader_idx = 0usize;

    // Asset-dependent state, filled once the background load lands over `rx`.
    let mut subject: Option<Subject> = None;
    let mut orbit: Option<OrbitController> = None;
    let mut warnings: Vec<String> = Vec::new();

    let modes = [ShadeMode::PerSample, ShadeMode::Gouraud, ShadeMode::Flat];
    let mut mode_idx = 0usize;
    let mut last = Instant::now();
    let mut running = true;
    let mut last_drag: Option<(u32, u32)> = None;
    let mut show_help = false;
    let mut auto_rotate = false;

    // Indeterminate-loader animation: `spinner_frame` advances on a wall-clock
    // accumulator (steady regardless of FPS); `spin_phase` drives the sweep bar.
    let mut spinner_frame = 0usize;
    let mut spin_accum = 0.0f32;
    let mut spin_phase = 0.0f32;

    while running {
        events
            .pump(Duration::from_millis(16))
            .map_err(std::io::Error::other)?;
        for ev in events.drain() {
            match ev {
                Event::Resize(size) => {
                    buf.resize(size);
                    presenter.resize(size);
                }
                // Press/repeat only; releases (kitty protocol) drive nothing here.
                Event::Key(k) if k.state != KeyState::Release => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => running = false,
                    // For an image the arrows pan across the picture in 2D; for a
                    // mesh they orbit (yaw/pitch). `nudge` applies whichever the
                    // subject calls for in the given screen direction.
                    KeyCode::Left => nudge(orbit.as_mut(), is_image, -1.0, 0.0),
                    KeyCode::Right => nudge(orbit.as_mut(), is_image, 1.0, 0.0),
                    KeyCode::Up => nudge(orbit.as_mut(), is_image, 0.0, 1.0),
                    KeyCode::Down => nudge(orbit.as_mut(), is_image, 0.0, -1.0),
                    KeyCode::Char('+' | '=') => {
                        if let Some(o) = orbit.as_mut() {
                            o.zoom(0.9);
                        }
                    }
                    // `_` is Shift+`-` on most layouts, so zoom-out works whether
                    // or not Shift is held (mirroring `+`/`=` for zoom-in).
                    KeyCode::Char('-' | '_') => {
                        if let Some(o) = orbit.as_mut() {
                            o.zoom(1.1);
                        }
                    }
                    KeyCode::Char('m') => mode_idx = (mode_idx + 1) % modes.len(),
                    KeyCode::Char('c') => shader_idx = (shader_idx + 1) % shaders.len(),
                    KeyCode::Char('z') => auto_rotate = !auto_rotate,
                    KeyCode::Char('h') => show_help = !show_help,
                    KeyCode::Char(c) if light_for(c).is_some() => {
                        if let Some((pos, label)) = light_for(c) {
                            rig = LightRig::default().with_light(Light::point(pos));
                            light_label = label;
                        }
                    }
                    _ => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseKind::Down(_) => last_drag = Some((m.col, m.row)),
                    MouseKind::Drag(_) => {
                        if let (Some((px, py)), Some(o)) = (last_drag, orbit.as_mut()) {
                            let dx = m.col as f32 - px as f32;
                            let dy = m.row as f32 - py as f32;
                            // Drag-to-orbit for a 3D mesh follows the grabbed
                            // surface: dragging right turns the model's right
                            // side toward you (inverted from the raw cursor
                            // delta). Images keep the plain mapping.
                            if is_image {
                                o.rotate(dx * 0.02, -dy * 0.02);
                            } else {
                                o.rotate(-dx * 0.02, dy * 0.02);
                            }
                        }
                        last_drag = Some((m.col, m.row));
                    }
                    MouseKind::ScrollUp => {
                        if let Some(o) = orbit.as_mut() {
                            o.zoom(0.92);
                        }
                    }
                    MouseKind::ScrollDown => {
                        if let Some(o) = orbit.as_mut() {
                            o.zoom(1.08);
                        }
                    }
                    _ => last_drag = None,
                },
                _ => {}
            }
        }

        // Poll the background loader (non-blocking) until the asset arrives.
        if subject.is_none() {
            match rx.try_recv() {
                Ok(Ok(outcome)) => {
                    // Build the orbit before moving the subject into its slot.
                    let s = outcome.subject;
                    orbit = Some(s.start_orbit());
                    subject = Some(s);
                    warnings = outcome.warnings;
                }
                Ok(Err(e)) => return Err(std::io::Error::other(e)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    return Err(std::io::Error::other(
                        "asset loader thread terminated unexpectedly",
                    ));
                }
            }
        }

        let dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        if let Some(o) = orbit.as_mut() {
            if auto_rotate {
                o.rotate(-AUTO_YAW_RATE * dt, 0.0); // continuous left/clockwise spin
            }
            o.update(dt);
        } else {
            // Still loading: advance the spinner on a steady wall-clock cadence.
            // One frame's `dt` (~16 ms) is well under a step, so a single advance
            // per tick keeps pace; a frame hitch simply advances once that tick.
            const SPIN_STEP: f32 = 0.09; // ~90 ms per throbber frame
            spin_accum += dt;
            spin_phase += dt;
            if spin_accum >= SPIN_STEP {
                spin_accum -= SPIN_STEP;
                spinner_frame = spinner_frame.wrapping_add(1);
            }
        }

        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        let bg = if ascii {
            Style::DEFAULT
        } else {
            Style::DEFAULT.with_bg(Color::Rgb(8, 10, 14))
        };
        buf.fill(bg.cell(' '));

        let area = buf.area();
        let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Len(24)]).split(area);
        let panel = Panel::new()
            .border(Some(if ascii {
                BorderSet::ASCII
            } else {
                BorderSet::ROUNDED
            }))
            .title("xre view");
        let inner = panel.inner(cols[0]);

        if let (Some(subject), Some(orbit)) = (subject.as_ref(), orbit.as_mut()) {
            let mut camera = Camera::default();
            orbit.apply(&mut camera);
            samples.resize(
                UVec2::new(inner.width().max(1), inner.height().max(1)),
                2,
                4,
            );
            samples.clear([8, 10, 14]);
            let vp = camera.view_projection(
                inner.width().max(1),
                inner.height().max(1),
                Projection::DEFAULT_CELL_ASPECT,
            );
            let shade_ms = {
                let t = Instant::now();
                draw_mesh_textured(
                    &mut samples,
                    &subject.mesh,
                    Transform::IDENTITY.to_mat4(),
                    vp,
                    &rig,
                    &subject.material,
                    modes[mode_idx],
                    subject.cull,
                    subject.sampler(),
                );
                t.elapsed().as_secs_f32() * 1000.0
            };

            let mut frame = Frame::root(&mut buf);
            panel.render(cols[0], &mut frame);
            Viewport3D::new(&samples, shaders[shader_idx].1.as_ref()).render(inner, &mut frame);

            let info = Panel::new()
                .border(Some(if ascii {
                    BorderSet::ASCII
                } else {
                    BorderSet::ROUNDED
                }))
                .title("stats");
            let ii = info.render(cols[1], &mut frame);
            let arrows_hint = if is_image {
                "arrows pan"
            } else {
                "arrows orbit"
            };
            let stats = [
                subject.info.clone(),
                format!("draw   {shade_ms:4.1}ms"),
                format!("fps    {:4.0}", 1.0 / dt.max(1e-3)),
                format!("mode   {:?}", modes[mode_idx]),
                format!("light  {light_label}"),
                format!("shader {}", shaders[shader_idx].0),
                String::new(),
                arrows_hint.into(),
                "drag   orbit".into(),
                "scroll zoom".into(),
                "m      shade mode".into(),
                "tblr   light pos".into(),
                "c      shader".into(),
                "z      spin".into(),
                "h      help".into(),
                "q      quit".into(),
            ];
            for (i, line) in stats.iter().enumerate() {
                Text::raw(line.clone()).render_into(
                    xre::core::Rect::new(ii.left(), ii.top() + i as u32, ii.width(), 1),
                    &mut frame,
                );
            }

            // Live help overlay (toggle with `h`), centred over the viewport.
            if show_help {
                let help_lines = [
                    if is_image {
                        "arrows          pan"
                    } else {
                        "arrows / drag   orbit"
                    },
                    "+ / - / scroll  zoom",
                    "m               shade mode",
                    "t/b/l/r         light pos",
                    "c               cycle shader",
                    "z               auto-rotate",
                    "h               help (toggle)",
                    "q / Esc         quit",
                ];
                let w = 32.min(cols[0].width());
                let h = (help_lines.len() as u32 + 2).min(cols[0].height());
                let x = cols[0].left() + cols[0].width().saturating_sub(w) / 2;
                let y = cols[0].top() + cols[0].height().saturating_sub(h) / 2;
                let help = Panel::new()
                    .border(Some(if ascii {
                        BorderSet::ASCII
                    } else {
                        BorderSet::ROUNDED
                    }))
                    .fill(if ascii {
                        Style::DEFAULT.cell(' ')
                    } else {
                        Style::DEFAULT.with_bg(Color::Rgb(16, 18, 24)).cell(' ')
                    })
                    .title("help");
                let hi = help.render(xre::core::Rect::new(x, y, w, h), &mut frame);
                for (i, line) in help_lines.iter().enumerate() {
                    Text::raw((*line).to_string()).render_into(
                        xre::core::Rect::new(hi.left(), hi.top() + i as u32, hi.width(), 1),
                        &mut frame,
                    );
                }
            }
        } else {
            // Still loading: draw the chrome instantly with a centred spinner,
            // an indeterminate sweep bar, and a "loading…" stats line.
            let mut frame = Frame::root(&mut buf);
            panel.render(cols[0], &mut frame);

            let msg = loading_message(is_image);
            // Spinner glyph + one space + label.
            let content_w = 2 + msg.chars().count() as u32;
            let sx = inner.left() + inner.width().saturating_sub(content_w) / 2;
            let sy = inner.top() + inner.height() / 2;
            Spinner::new(spinner_frame)
                .ascii(ascii)
                .label(msg)
                .render(xre::core::Rect::new(sx, sy, content_w, 1), &mut frame);

            // Indeterminate sweep bar one row below the spinner, if it fits.
            let bar_w = content_w.min(inner.width());
            if bar_w > 0 && sy + 2 < inner.bottom() {
                let bx = inner.left() + inner.width().saturating_sub(bar_w) / 2;
                Gauge::new(sweep_ratio(spin_phase))
                    .ascii(ascii)
                    .render(xre::core::Rect::new(bx, sy + 2, bar_w, 1), &mut frame);
            }

            let info = Panel::new()
                .border(Some(if ascii {
                    BorderSet::ASCII
                } else {
                    BorderSet::ROUNDED
                }))
                .title("stats");
            let ii = info.render(cols[1], &mut frame);
            Text::raw("loading…").render_into(
                xre::core::Rect::new(ii.left(), ii.top(), ii.width(), 1),
                &mut frame,
            );
        }
        presenter.present(&buf).map_err(std::io::Error::other)?;
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_unit_centers_and_scales() {
        let mut mesh = Mesh::cube();
        for p in &mut mesh.positions {
            *p = *p * 10.0 + Vec3::new(5.0, 0.0, 0.0);
        }
        let fitted = fit_unit(mesh);
        let aabb = fitted.aabb();
        assert!(
            aabb.center().length() < 1e-4,
            "not centered: {:?}",
            aabb.center()
        );
        assert!(
            aabb.bounding_radius() <= 1.001,
            "not unit: {}",
            aabb.bounding_radius()
        );
    }

    #[test]
    fn loading_message_picks_subject_word() {
        assert_eq!(loading_message(true), "Loading image…");
        assert_eq!(loading_message(false), "Loading model…");
    }

    #[test]
    fn sweep_ratio_is_a_bounded_triangle() {
        // Stays within the gauge's valid range for any time, including a full
        // period and beyond, and peaks at the half-period.
        for &t in &[0.0, 0.4, 0.8, 1.6, 3.21, 100.0] {
            let r = sweep_ratio(t);
            assert!((0.0..=1.0).contains(&r), "sweep {r} out of range at t={t}");
        }
        assert!(sweep_ratio(0.0) < 0.01, "sweep starts near 0");
        assert!(sweep_ratio(0.8) > 0.99, "sweep peaks at the half period");
    }

    #[test]
    fn light_for_maps_cardinal_keys() {
        let d = LIGHT_DISTANCE;
        assert_eq!(light_for('t'), Some((Vec3::new(0.0, d, 0.0), "top")));
        assert_eq!(light_for('b'), Some((Vec3::new(0.0, -d, 0.0), "bottom")));
        assert_eq!(light_for('l'), Some((Vec3::new(-d, 0.0, 0.0), "left")));
        assert_eq!(light_for('r'), Some((Vec3::new(d, 0.0, 0.0), "right")));
        assert_eq!(light_for('x'), None);
        assert_eq!(light_for('q'), None);
    }

    fn obj_subject(mesh: Mesh) -> Subject {
        Subject {
            mesh,
            material: Material::default(),
            texture: None,
            cull: Cull::Back,
            info: String::new(),
        }
    }

    #[test]
    fn snapshot_renders_nonempty_frame() {
        let subject = obj_subject(fit_unit(Mesh::uv_sphere(1.0, 16, 24)));
        let frame = render_snapshot(&subject, 40, 20, default_orbit(), false);
        let ink = frame.chars().filter(|c| !c.is_whitespace()).count();
        assert!(
            ink > 50,
            "snapshot should contain a rendered shape, got {ink} ink chars"
        );
        // 20 rows each terminated by '\n'.
        assert_eq!(frame.lines().count(), 20);
    }

    #[test]
    fn image_snapshot_renders_nonempty_frame() {
        let texture = Texture::checkerboard(16, [255, 255, 255], [0, 0, 0]);
        let subject = Subject {
            mesh: fit_unit(Mesh::image_quad(1.0)),
            material: Material::colored(Vec3::ONE).unlit(),
            texture: Some(texture),
            cull: Cull::None,
            info: String::new(),
        };
        let frame = render_snapshot(&subject, 40, 20, face_on_orbit(), false);
        let ink = frame.chars().filter(|c| !c.is_whitespace()).count();
        assert!(
            ink > 50,
            "image snapshot should contain rendered glyphs, got {ink} ink chars"
        );
        assert_eq!(frame.lines().count(), 20);
    }

    #[test]
    fn image_paths_are_detected() {
        assert!(is_image_path(Path::new("photo.PNG")));
        assert!(is_image_path(Path::new("a/b/c.jpg")));
        assert!(is_image_path(Path::new("scan.jpeg")));
        assert!(!is_image_path(Path::new("model.obj")));
        assert!(!is_image_path(Path::new("noext")));
    }
}
