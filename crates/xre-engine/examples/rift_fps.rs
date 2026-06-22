//! `rift-fps` — the Phase 5 / Gate G5 demo game.
//!
//! A tiny dungeon crawler on the grid-raycaster backend: WASD move with swept
//! collision (no wall-clipping), brick-textured walls lit by a lamp at the top
//! of the map, a TUI HUD (minimap, health gauge, log), pickups to collect and an
//! exit tile that wins. Exercises the raycaster, input map, collision-lite and
//! the TUI layer in one artifact.
//!
//! Run with `cargo run -p xre-engine --example rift-fps --features grid-raycaster`.
//! Move: W/A/S/D (hold two for diagonal) · look: move the mouse to turn the view
//! (FPS-style); hold the pointer at a viewport edge to keep spinning, or ← / → to
//! turn · fire: left-click (at the cursor) or <kbd>Space</kbd> (straight ahead) ·
//! menu: h (frees the mouse) · color: c (toggle 256-palette to cut present
//! bytes when the terminal is the bottleneck) · quit: q.
//!
//! Firing casts a ray into the level and bursts a muzzle flash plus sparks on the
//! wall it strikes — a click lands them where the cursor is, Space down the centre.
//! A reticle marks where a click will land; it hides when you use the arrow keys or
//! <kbd>Space</kbd> and returns when you move the mouse. While a menu is open the
//! pointer is freed to click it and the view stops tracking the mouse.
//!
//! Each pickup pops a message with a **Confirm** button; reaching the exit pops a
//! win screen with **Exit game** / **Restart**. The pause menu (`h`) has an
//! **Options** sub-menu to pick the shader and tune mouse sensitivity and movement
//! speed. While any menu is open the mouse is freed for clicking and the view
//! stops tracking it.
//!
//! On a terminal with the kitty keyboard protocol (Kitty, Ghostty, WezTerm, …)
//! movement is hold-to-move with real key releases. On terminals that cannot
//! report releases the HUD shows `LATCH`: **tap** a direction to move and **tap it
//! again** (or <kbd>Backspace</kbd>) to stop — so W+D still stack into a diagonal.
//! Press `l` to toggle latch mode; `h` pauses and opens the (clickable) menu.
#![allow(clippy::suboptimal_flops)] // determinism: plain float math, no FMA

use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

use xre_core::math::{UVec2, Vec2, Vec3};
use xre_core::{Cell, CellBuffer, Color, ColorDepth, Rect, Style};
use xre_engine::collide::move_and_slide;
use xre_engine::raycaster::{PointLight2D, Raycaster, TileMap};
use xre_engine::{Binding, FixedTimestep, FramePacer, InputMap, LatchAxis};
use xre_render::{
    builtin_cell_shaders, resolve_cells, CellShader, Sample, SampleBuffer, TextureSampler,
};
use xre_term::{
    Capabilities, Event, EventQueue, KeyCode, KeyState, MouseButton, MouseKind, Presenter,
    TerminalGuard,
};
use xre_tui::{BorderSet, Constraint, Frame, Gauge, Layout, Log, Panel, Text, Widget};

/// Brick wall texture, resolved relative to the crate so it works from any CWD.
const BRICK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/brick_wall.jpg");

const MAP: &str = "\
###############
#......#......#
#.####.#.####.#
#.#..........#.
#.#.####.###.#.
#...#......#..E#
#.#.#.####.#.#.#
#.#......P...#.#
#.############.#
#.....P.......#.#
###############";

struct Player {
    pos: Vec2,
    yaw: f32,
    /// Look pitch (radians-ish): shifts the horizon only, so the player stays on
    /// the floor plane. Clamped to [`PITCH_LIMIT`].
    pitch: f32,
    health: f32,
    score: u32,
}

/// A fresh player at the spawn tile (used at startup and on restart).
const fn new_player() -> Player {
    Player {
        pos: Vec2::new(1.5, 1.5),
        yaw: 0.0,
        pitch: 0.0,
        health: 1.0,
        score: 0,
    }
}

/// Which modal overlay (if any) is up. While anything other than [`Overlay::None`]
/// is showing, the game is frozen and the mouse is freed for clicking instead of
/// turning the view.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    /// Playing; mouse turns the view.
    None,
    /// The `h` pause menu (Resume / Options / Restart / Quit).
    Pause,
    /// The Options sub-menu (shader, mouse sensitivity, move speed).
    Options,
    /// A pickup acknowledgement with a single Confirm button.
    Pickup,
    /// The win screen with Exit game / Restart buttons.
    Exit,
}

/// FPS mouselook: yaw turned per horizontal cell of mouse *motion* (scaled by
/// `mouse_sens`). Relative — moving the mouse turns the view; the crosshair stays
/// pinned to the centre (the cursor never drives an absolute screen position).
const BASE_LOOK_X: f32 = 0.03;
/// FPS mouselook: pitch tilted per vertical cell of mouse motion (× `mouse_sens`).
const BASE_LOOK_Y: f32 = 0.02;
/// How far the view may tilt up or down (a fraction of the horizon shift).
const PITCH_LIMIT: f32 = 0.3;

/// Edge-pan: the fraction of the viewport (each side) that counts as the edge
/// band. With the pointer in this band the view keeps turning that way, so you can
/// spin past the terminal edge without lifting the mouse.
const EDGE_PAN_BAND: f32 = 0.14;
/// Edge-pan turn rate at the very edge, in radians/sec (scaled by `mouse_sens`).
const EDGE_PAN_SPEED: f32 = 2.5;

/// Edge-pan strength `(x, y)` in `-1.0..=1.0` for the pointer over the viewport:
/// the deeper into an edge band the pointer sits, the harder it pushes (negative =
/// left/up, positive = right/down, `0` = inside the dead zone). The cursor is
/// clamped into the viewport first, so a pointer parked on the HUD pins at full
/// push. Returns `(0, 0)` when there is no cursor.
fn edge_pan(cursor: Option<(u32, u32)>, view: Rect) -> (f32, f32) {
    let Some((c, r)) = cursor else {
        return (0.0, 0.0);
    };
    let (w, h) = (view.width(), view.height());
    if w == 0 || h == 0 {
        return (0.0, 0.0);
    }
    let col = c.clamp(view.left(), view.left() + w - 1);
    let row = r.clamp(view.top(), view.top() + h - 1);
    let nx = (col - view.left()) as f32 / (w - 1).max(1) as f32; // 0 (left) .. 1 (right)
    let ny = (row - view.top()) as f32 / (h - 1).max(1) as f32; // 0 (top) .. 1 (bottom)
    let band = |t: f32| {
        if t < EDGE_PAN_BAND {
            -(EDGE_PAN_BAND - t) / EDGE_PAN_BAND
        } else if t > 1.0 - EDGE_PAN_BAND {
            (t - (1.0 - EDGE_PAN_BAND)) / EDGE_PAN_BAND
        } else {
            0.0
        }
    };
    (band(nx), band(ny))
}

/// Mouse-sensitivity multiplier range and step (tuned in the Options menu).
const SENS_MIN: f32 = 0.2;
const SENS_MAX: f32 = 3.0;
const SENS_STEP: f32 = 0.2;
/// Movement-speed (cells/sec) range and step (tuned in the Options menu).
const SPEED_MIN: f32 = 0.4;
const SPEED_MAX: f32 = 3.0;
const SPEED_STEP: f32 = 0.2;
/// Latch re-tap debounce: ignore further toggles of a direction for this long,
/// so a held key's auto-repeat burst can't flicker it (tap again after this to
/// stop). See the latch toggle in `main`.
const RETAP: f32 = 0.22;

/// Render frame-rate target (paced) and the fixed physics update rate. Rendering
/// runs at up to `TARGET_FPS`; physics steps at `PHYSICS_HZ` via an accumulator,
/// with the camera interpolated between ticks so motion stays smooth on a
/// high-refresh terminal even when several render frames fall between ticks.
const TARGET_FPS: f32 = 120.0;
const PHYSICS_HZ: f32 = 60.0;

/// Pause-menu rows, in order.
const MENU_ITEMS: [&str; 4] = ["Resume", "Options", "Restart", "Quit"];

/// Options sub-menu rows, in order.
const OPTION_ITEMS: [&str; 6] = [
    "Shader",
    "Mouse sens",
    "Move speed",
    "Detail",
    "Textures",
    "Back",
];

/// Exit-overlay buttons, in order.
const EXIT_LABELS: [&str; 2] = ["Exit game", "Restart"];

/// What activating a menu row should do — returned by [`activate_menu`] so the
/// keyboard (Enter) and mouse (click) paths share one decision.
enum MenuAction {
    Resume,
    OpenOptions,
    Restart,
    Quit,
}

/// Map a pause-menu row index to its action.
const fn activate_menu(idx: usize) -> Option<MenuAction> {
    match idx {
        0 => Some(MenuAction::Resume),
        1 => Some(MenuAction::OpenOptions),
        2 => Some(MenuAction::Restart),
        3 => Some(MenuAction::Quit),
        _ => None,
    }
}

/// Toggle a latch axis: pressing a direction that is already set that way clears
/// it (so a tap moves and a second tap stops), otherwise it (re-)sets it.
fn latch_toggle(ax: &mut LatchAxis, positive: bool) {
    let already = if positive {
        ax.value() > 0.0
    } else {
        ax.value() < 0.0
    };
    if already {
        ax.clear();
    } else if positive {
        ax.set_positive();
    } else {
        ax.set_negative();
    }
}

/// Map an exit-overlay button index to its action ("Exit game" / "Restart"),
/// reusing [`MenuAction`] so the apply site stays single.
const fn exit_action(idx: usize) -> Option<MenuAction> {
    match idx {
        0 => Some(MenuAction::Quit),
        1 => Some(MenuAction::Restart),
        _ => None,
    }
}

/// Where a fired shot is aimed, captured at input time and resolved once the
/// viewport rect and sample buffer are current (see the render block in `main`).
enum ShotAim {
    /// A left-click at a terminal cell: aim at — and spark at — that screen point.
    At { col: u32, row: u32 },
    /// <kbd>Space</kbd>: aim straight down the centre of the view.
    Forward,
}

/// A tiny deterministic PRNG (xorshift32) for spark scatter — keeps the demo
/// reproducible without pulling in `rand` (the repo's determinism rule).
struct Rng(u32);

impl Rng {
    const fn new(seed: u32) -> Self {
        Self(seed | 1) // a zero state would stay zero forever
    }

    const fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A float in `0.0..1.0`.
    const fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Number of sparks emitted per shot.
const SPARK_COUNT: usize = 16;
/// Base spark lifetime, seconds (jittered per spark).
const SPARK_LIFE: f32 = 0.35;
/// Muzzle-flash duration, seconds.
const FLASH_TIME: f32 = 0.07;
/// Downward pull on sparks, in samples/sec² (so the shower falls). Scaled with the
/// spread below so the enlarged burst keeps the same arc shape (2× base, then +30%).
const SPARK_GRAVITY: f32 = 98.8;

/// One impact spark, positioned in sample space (the viewport's sub-cell grid).
struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: f32,
    life: f32,
}

/// Transient muzzle-flash + impact-spark effects for the FPS demo. Sparks are
/// composited straight into the [`SampleBuffer`] after the raycast, so the cell
/// shader renders them as glowing glyphs sitting on the wall that was hit.
#[derive(Default)]
struct Shots {
    sparks: Vec<Spark>,
    /// Seconds of muzzle flash remaining.
    flash: f32,
}

impl Shots {
    /// Burst a flash plus a fan of sparks at sample position `(x, y)`.
    fn spawn(&mut self, x: f32, y: f32, rng: &mut Rng) {
        self.flash = FLASH_TIME;
        for _ in 0..SPARK_COUNT {
            let ang = rng.f32() * core::f32::consts::TAU;
            let spd = 15.6 + rng.f32() * 57.2; // samples/sec (2× base spread, +30%)
            self.sparks.push(Spark {
                x,
                y,
                vx: ang.cos() * spd,
                vy: ang.sin() * spd - 10.4, // slight upward bias before gravity
                age: 0.0,
                life: SPARK_LIFE * (0.6 + rng.f32() * 0.4),
            });
        }
    }

    /// Advance the flash timer and every live spark by `dt`, dropping the dead.
    fn update(&mut self, dt: f32) {
        self.flash = (self.flash - dt).max(0.0);
        for s in &mut self.sparks {
            s.age += dt;
            s.x += s.vx * dt;
            s.y += s.vy * dt;
            s.vy += SPARK_GRAVITY * dt;
        }
        self.sparks.retain(|s| s.age < s.life);
    }

    /// Composite the muzzle flash, the impact sparks and an aiming crosshair into
    /// the freshly raycast viewport buffer. Foreground effects use the
    /// unconditional [`SampleBuffer::put`] (depth `0`) so they always read over the
    /// wall.
    fn draw(&self, buf: &mut SampleBuffer) {
        let (w, h) = (buf.width(), buf.height());
        if w == 0 || h == 0 {
            return;
        }

        // A faint center crosshair so the line-of-sight (Space) shot is aimable.
        let (cx, cy) = (w / 2, h / 2);
        let cross = Sample::new(0.55, [200, 210, 230], 0.0);
        for d in 0..=1 {
            buf.put(cx.saturating_sub(d), cy, cross);
            buf.put((cx + d).min(w - 1), cy, cross);
            buf.put(cx, cy.saturating_sub(d), cross);
            buf.put(cx, (cy + d).min(h - 1), cross);
        }

        // Impact sparks: bright, cooling from white through to red as they age.
        // Each is a 2×2 sample block so it reads at twice the size.
        for s in &self.sparks {
            if s.x < 0.0 || s.y < 0.0 {
                continue;
            }
            let t = (s.age / s.life).clamp(0.0, 1.0);
            let luma = (1.0 - t).clamp(0.0, 1.0);
            let color = spark_color(t);
            let (bx, by) = (s.x as u32, s.y as u32);
            for oy in 0..2 {
                for ox in 0..2 {
                    let (px, py) = (bx + ox, by + oy);
                    if px < w && py < h {
                        buf.put(px, py, Sample::new(luma, color, 0.0));
                    }
                }
            }
        }

        // Muzzle flash: a radial bloom at the gun muzzle (bottom-center).
        if self.flash > 0.0 {
            let t = self.flash / FLASH_TIME; // 1 → 0 over the flash
            let (fx, fy) = (w as f32 * 0.5, h as f32 - 1.0);
            let radius = (2.0 + 4.0 * t) * 2.0; // 2× the base bloom
            let r = radius as i32;
            for dy in -r..=r {
                for dx in -r..=r {
                    let px = fx + dx as f32;
                    let py = fy + dy as f32;
                    if px < 0.0 || py < 0.0 {
                        continue;
                    }
                    let dist = ((dx * dx + dy * dy) as f32).sqrt();
                    if dist > radius {
                        continue;
                    }
                    let falloff = (1.0 - dist / radius).clamp(0.0, 1.0);
                    let luma = (t * falloff).clamp(0.0, 1.0);
                    buf.put(
                        px as u32,
                        py as u32,
                        Sample::new(luma, [255, 230, 150], 0.0),
                    );
                }
            }
        }
    }
}

/// Spark color by normalized age `t` (`0` = fresh): white → yellow → orange → red.
const fn spark_color(t: f32) -> [u8; 3] {
    if t < 0.33 {
        [255, 255, 230]
    } else if t < 0.66 {
        [255, 200, 90]
    } else {
        [220, 90, 40]
    }
}

/// Draw the mouse cursor reticle — a small bright `X` (so it reads apart from the
/// fixed `+` aim crosshair) — centered at sample `(cx, cy)` in the viewport buffer.
fn draw_cursor(buf: &mut SampleBuffer, cx: u32, cy: u32) {
    let (w, h) = (buf.width(), buf.height());
    if w == 0 || h == 0 {
        return;
    }
    let mark = Sample::new(0.95, [255, 255, 255], 0.0);
    buf.put(cx, cy, mark);
    for d in 1..=2 {
        buf.put(cx.saturating_sub(d), cy.saturating_sub(d), mark);
        buf.put((cx + d).min(w - 1), cy.saturating_sub(d), mark);
        buf.put(cx.saturating_sub(d), (cy + d).min(h - 1), mark);
        buf.put((cx + d).min(w - 1), (cy + d).min(h - 1), mark);
    }
}

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
    let guard = TerminalGuard::enter().map_err(std::io::Error::other)?;
    let caps = Capabilities::probe();
    // When the terminal speaks the kitty protocol we get real key releases, so
    // held keys (and W+D diagonals) are exact; otherwise fall back to latch mode.
    let release_reporting = guard.keyboard_enhanced();
    let mut presenter = Presenter::stdout(&caps);
    let mut buf = CellBuffer::new(caps.size);
    let mut events = EventQueue::new();

    let map = TileMap::parse(MAP);
    let statics = map.colliders();
    let mut pickups: Vec<(f32, f32)> = pickup_positions(MAP);
    let exit = exit_position(MAP);

    let mut input = InputMap::new();
    for (action, key) in [("fwd", 'w'), ("back", 's'), ("left", 'a'), ("right", 'd')] {
        input.bind("game", action, Binding::key(KeyCode::Char(key)));
    }
    input.bind("game", "turn_l", Binding::key(KeyCode::Left));
    input.bind("game", "turn_r", Binding::key(KeyCode::Right));
    input.push_context("game");
    input.set_release_reporting(release_reporting);

    let raycaster = Raycaster::default();
    // Shaders cycled from the Options menu; default to "half-block" (crisp).
    let shaders = builtin_cell_shaders();
    let mut shader_idx = shaders
        .iter()
        .position(|(name, _)| *name == "half-block")
        .unwrap_or(0);
    let mut samples = SampleBuffer::new(UVec2::new(1, 1), 2, 4);
    let mut log = Log::new(64);
    log.push("find the exit (E). collect P for points.");

    // Brick texture for the walls (falls back to the flat palette if missing),
    // and a lamp near the top-center of the map that brightens nearby surfaces.
    let brick = match xre_cello::load_image_file(Path::new(BRICK_PATH)) {
        Ok(tex) => Some(tex),
        Err(e) => {
            log.push(format!("walls: brick texture unavailable ({e})"));
            None
        }
    };
    let wall_tex: Option<&dyn TextureSampler> = brick.as_ref().map(|t| t as &dyn TextureSampler);
    let light = PointLight2D {
        pos: Vec2::new(map.width() as f32 / 2.0, 1.5),
        intensity: 1.4,
        radius: 5.0,
    };

    // Tunable settings (adjusted in the Options menu). `mouse_sens` scales how fast
    // mouse motion turns the view (relative FPS mouselook).
    let mut mouse_sens = 0.8f32;
    let mut move_speed = 0.4f32;
    // Vertical supersampling for the viewport: 2 (the default — half the raycast
    // and shade samples) or 4 (crisper). The cheapest render-side quality knob,
    // toggled in Options.
    let mut super_sy = 2u32;
    // Whether walls use the brick texture; off = flat palette (skips the per-sample
    // texture fetch, cheaper). Toggled in Options.
    let mut textures_on = true;
    // Cap colors to the 256-palette at present time. Truecolor + a moving 3D view
    // repaints the whole viewport every frame; `38;5;N` is shorter than
    // `38;2;R;G;B` and near-identical colors coalesce, so this slashes the bytes a
    // terminal-I/O-bound present must push. Toggle with `c`. Off = full truecolor.
    let mut color_256 = false;

    let mut player = new_player();
    // Overlay state: `h` toggles the pause menu; pickups and the exit raise their
    // own modal overlays. `menu_idx` is the pause cursor; `opt_idx` the Options
    // cursor; `exit_idx` the exit button focus; `overlay_msg` the pickup/exit text.
    let mut overlay = Overlay::None;
    let mut menu_idx = 0usize;
    let mut opt_idx = 0usize;
    let mut exit_idx = 0usize;
    let mut overlay_msg = String::new();
    // FPS mouselook: the previous cursor cell, used to turn the view by the motion
    // *delta* each event; `None` re-baselines after any overlay so the view never
    // jumps when control returns to the game.
    let mut last_mouse: Option<(u32, u32)> = None;
    // Shooting: a click (aimed at the clicked cell) or Space (down the centre)
    // captured this frame, resolved in the render block once the viewport rect and
    // sample buffer are current.
    let mut pending_shot: Option<ShotAim> = None;
    let mut shots = Shots::default();
    let mut rng = Rng::new(0x5eed_1234);
    // The mouse cursor reticle is shown while the mouse is the active input and
    // hidden once the player uses the arrow keys or Space, until the mouse moves
    // again. (The OS pointer can't be hidden portably, so this is the in-view one.)
    let mut cursor_visible = true;
    // Latch movement (sticky directions) for terminals that can't report held
    // keys; default on when releases aren't reported. Each direction toggles on a
    // tap (debounced by `tap_cd` so a held key's auto-repeat can't flicker it).
    let mut latch = !release_reporting;
    let mut move_latch = LatchAxis::default();
    let mut strafe_latch = LatchAxis::default();
    let mut tap_cd = [0.0f32; 4]; // fwd, back, right, left
    if latch {
        log.push("latch mode: tap a direction to move, tap it again (or Space) to stop.");
    }
    let mut last = Instant::now();
    let mut running = true;
    // Decoupled loop: render is paced to `TARGET_FPS`, physics advances at a fixed
    // `PHYSICS_HZ` through an accumulator, and the rendered camera position is
    // interpolated between the last two physics ticks so motion stays smooth.
    let pacer = FramePacer::new(TARGET_FPS);
    let mut timestep = FixedTimestep::new(PHYSICS_HZ);
    let mut pos_prev = player.pos;
    let mut pos_curr = player.pos;
    // EMA-smoothed readout for the HUD: FPS, plus render vs present time so the
    // bottleneck is visible (present blocks on the terminal — the usual ceiling).
    let mut fps_ema = TARGET_FPS;
    let mut render_ms_ema = 0.0f32;
    let mut present_ms_ema = 0.0f32;
    // Cache the viewport|HUD column split; it only changes on resize, so recompute
    // it then rather than allocating a `Vec<Rect>` every frame.
    let mut layout_size = UVec2::new(u32::MAX, u32::MAX);
    let mut layout_cols = [Rect::new(0, 0, 0, 0); 2];
    // Reused scratch for parallel cell shading (grows once, then reused).
    let mut shade_scratch: Vec<Option<Cell>> = Vec::new();

    while running {
        let frame_start = Instant::now();
        // Drain input non-blocking and let `pacer.pace` do all the sleeping below,
        // so every frame lands on the target period regardless of input timing
        // (input latency is at most one frame).
        events.pump(Duration::ZERO).map_err(std::io::Error::other)?;

        // Advance time and the input frame up front so held-key grace decay uses
        // this frame's wall-clock delta.
        let frame_dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        input.begin_frame(frame_dt);

        // The overlay layout for hit-testing, from the size on screen now (the
        // render path refreshes the same cache after any resize below).
        ensure_cols(presenter.size(), &mut layout_size, &mut layout_cols);
        let overlay_area = layout_cols[0];

        let mut pending_menu: Option<MenuAction> = None;
        for ev in events.drain() {
            match ev {
                Event::Resize(size) => {
                    buf.resize(size);
                    presenter.resize(size);
                }
                Event::Key(k) => {
                    // Discrete actions fire on the press edge only — repeats and
                    // releases (carried once the kitty protocol is active) must
                    // not double-trigger menu nav, quit or toggles.
                    if k.state == KeyState::Press {
                        match overlay {
                            Overlay::Pause => match k.code {
                                KeyCode::Up => {
                                    menu_idx = (menu_idx + MENU_ITEMS.len() - 1) % MENU_ITEMS.len();
                                }
                                KeyCode::Down => menu_idx = (menu_idx + 1) % MENU_ITEMS.len(),
                                KeyCode::Enter => pending_menu = activate_menu(menu_idx),
                                KeyCode::Char('h') | KeyCode::Esc => {
                                    pending_menu = Some(MenuAction::Resume);
                                }
                                KeyCode::Char('q') => running = false,
                                _ => {}
                            },
                            // Options sub-menu: ↑/↓ pick a row, ←/→ adjust it,
                            // Enter on "Back" (or Esc) returns to the pause menu.
                            Overlay::Options => match k.code {
                                KeyCode::Up => {
                                    opt_idx =
                                        (opt_idx + OPTION_ITEMS.len() - 1) % OPTION_ITEMS.len();
                                }
                                KeyCode::Down => opt_idx = (opt_idx + 1) % OPTION_ITEMS.len(),
                                KeyCode::Left => adjust_option(
                                    opt_idx,
                                    -1,
                                    &mut shader_idx,
                                    shaders.len(),
                                    &mut mouse_sens,
                                    &mut move_speed,
                                    &mut super_sy,
                                    &mut textures_on,
                                ),
                                KeyCode::Right => adjust_option(
                                    opt_idx,
                                    1,
                                    &mut shader_idx,
                                    shaders.len(),
                                    &mut mouse_sens,
                                    &mut move_speed,
                                    &mut super_sy,
                                    &mut textures_on,
                                ),
                                KeyCode::Enter => {
                                    if opt_idx == OPTION_ITEMS.len() - 1 {
                                        overlay = Overlay::Pause;
                                    }
                                }
                                KeyCode::Esc => overlay = Overlay::Pause,
                                KeyCode::Char('q') => running = false,
                                _ => {}
                            },
                            // A pickup just acknowledges and resumes.
                            Overlay::Pickup => match k.code {
                                KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ') => {
                                    pending_menu = Some(MenuAction::Resume);
                                }
                                KeyCode::Char('q') => running = false,
                                _ => {}
                            },
                            // The win screen: cycle the two buttons, Enter activates.
                            Overlay::Exit => match k.code {
                                KeyCode::Left | KeyCode::Up | KeyCode::Right | KeyCode::Down => {
                                    exit_idx = (exit_idx + 1) % EXIT_LABELS.len();
                                }
                                KeyCode::Enter => pending_menu = exit_action(exit_idx),
                                KeyCode::Char('q') => running = false,
                                _ => {}
                            },
                            Overlay::None => match k.code {
                                KeyCode::Char('q') | KeyCode::Esc => running = false,
                                KeyCode::Char('h') => {
                                    overlay = Overlay::Pause;
                                    menu_idx = 0;
                                    last_mouse = None;
                                }
                                // Toggle latch (sticky-direction) movement.
                                KeyCode::Char('l') => {
                                    latch = !latch;
                                    move_latch.clear();
                                    strafe_latch.clear();
                                }
                                // Toggle 256-color quantization (cuts present bytes
                                // on a terminal-I/O-bound view; see `color_256`).
                                KeyCode::Char('c') => {
                                    color_256 = !color_256;
                                    log.push(if color_256 {
                                        "color: 256-palette (fewer present bytes)"
                                    } else {
                                        "color: truecolor"
                                    });
                                }
                                // Fire down the centre of the view, and hide the
                                // mouse cursor (keyboard aiming now).
                                KeyCode::Char(' ') => {
                                    pending_shot = Some(ShotAim::Forward);
                                    cursor_visible = false;
                                }
                                // Arrow-key look/turn is keyboard aiming: hide the
                                // mouse cursor until the mouse is used again.
                                KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                                    cursor_visible = false;
                                }
                                // Stop: clear any latched directions.
                                KeyCode::Backspace => {
                                    move_latch.clear();
                                    strafe_latch.clear();
                                }
                                _ => {}
                            },
                        }
                    }
                    // Held-key tracking sees every state, but only while playing
                    // (a frozen player must not accumulate held directions).
                    if overlay == Overlay::None {
                        input.feed(&Event::Key(k));
                    }
                }
                Event::Mouse(m) => match overlay {
                    // While any menu is up the mouse is free: clicks drive the
                    // buttons and "fire" is never armed.
                    Overlay::Pause => {
                        if m.kind == MouseKind::Down(MouseButton::Left) {
                            if let Some(i) = menu_hit(overlay_area, m.col, m.row) {
                                menu_idx = i;
                                pending_menu = activate_menu(i);
                            }
                        }
                    }
                    // Click a row to select it; click its `<`/`>` to adjust, the
                    // Back row returns to the menu.
                    Overlay::Options => {
                        if m.kind == MouseKind::Down(MouseButton::Left) {
                            if let Some(i) = options_hit(overlay_area, m.col, m.row) {
                                opt_idx = i;
                                if i == OPTION_ITEMS.len() - 1 {
                                    overlay = Overlay::Pause;
                                } else {
                                    let dir = option_click_dir(overlay_area, m.col);
                                    if dir != 0 {
                                        adjust_option(
                                            i,
                                            dir,
                                            &mut shader_idx,
                                            shaders.len(),
                                            &mut mouse_sens,
                                            &mut move_speed,
                                            &mut super_sy,
                                            &mut textures_on,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Overlay::Pickup => {
                        if m.kind == MouseKind::Down(MouseButton::Left)
                            && confirm_hit(overlay_area, m.col, m.row)
                        {
                            pending_menu = Some(MenuAction::Resume);
                        }
                    }
                    Overlay::Exit => {
                        if m.kind == MouseKind::Down(MouseButton::Left) {
                            if let Some(i) = exit_hit(overlay_area, m.col, m.row) {
                                exit_idx = i;
                                pending_menu = exit_action(i);
                            }
                        }
                    }
                    Overlay::None => {
                        // FPS mouselook: motion *deltas* turn (yaw) and tilt (pitch)
                        // the view; a left-click fires at — and sparks at — the
                        // clicked cell. Any mouse use re-shows the reticle.
                        cursor_visible = true;
                        match m.kind {
                            MouseKind::Down(button) => {
                                if button == MouseButton::Left {
                                    pending_shot = Some(ShotAim::At {
                                        col: m.col,
                                        row: m.row,
                                    });
                                }
                                last_mouse = Some((m.col, m.row));
                            }
                            MouseKind::Up(_) => last_mouse = Some((m.col, m.row)),
                            MouseKind::Moved | MouseKind::Drag(_) => {
                                if let Some((px, py)) = last_mouse {
                                    player.yaw +=
                                        (m.col as f32 - px as f32) * BASE_LOOK_X * mouse_sens;
                                    player.pitch = (player.pitch
                                        - (m.row as f32 - py as f32) * BASE_LOOK_Y * mouse_sens)
                                        .clamp(-PITCH_LIMIT, PITCH_LIMIT);
                                }
                                last_mouse = Some((m.col, m.row));
                            }
                            _ => {}
                        }
                    }
                },
                _ => {}
            }
        }

        // Apply a chosen action once — shared by keyboard (Enter) and click,
        // across the pause menu, the pickup Confirm and the exit buttons.
        if let Some(action) = pending_menu {
            match action {
                MenuAction::Resume => overlay = Overlay::None,
                MenuAction::OpenOptions => {
                    overlay = Overlay::Options;
                    opt_idx = 0;
                }
                MenuAction::Restart => {
                    player = new_player();
                    pos_prev = player.pos;
                    pos_curr = player.pos;
                    pickups = pickup_positions(MAP);
                    log = Log::new(64);
                    log.push("find the exit (E). collect P for points.");
                    overlay = Overlay::None;
                }
                MenuAction::Quit => running = false,
            }
            // Re-baseline mouse-look whenever control returns to the game.
            last_mouse = None;
        }

        // Resolve movement: latched (sticky directions) or direct axes (true held
        // keys). In latch mode each direction *toggles* on a fresh tap — tap to
        // move, tap again to stop — and W+D stay set together for a diagonal.
        // `tap_cd` debounces a held key's auto-repeat burst so it can't flicker
        // the latch; `Space` is an instant stop.
        let turn = input.axis("turn_l", "turn_r");
        if latch {
            for c in &mut tap_cd {
                *c = (*c - frame_dt).max(0.0);
            }
            if input.pressed("fwd") && tap_cd[0] <= 0.0 {
                latch_toggle(&mut move_latch, true);
                tap_cd[0] = RETAP;
            }
            if input.pressed("back") && tap_cd[1] <= 0.0 {
                latch_toggle(&mut move_latch, false);
                tap_cd[1] = RETAP;
            }
            if input.pressed("right") && tap_cd[2] <= 0.0 {
                latch_toggle(&mut strafe_latch, true);
                tap_cd[2] = RETAP;
            }
            if input.pressed("left") && tap_cd[3] <= 0.0 {
                latch_toggle(&mut strafe_latch, false);
                tap_cd[3] = RETAP;
            }
        }
        let (fwd, strafe) = if latch {
            (move_latch.value(), strafe_latch.value())
        } else {
            (input.axis("back", "fwd"), input.axis("left", "right"))
        };

        if overlay == Overlay::None {
            // Run physics at the fixed step (0..N times this render frame). Only
            // while playing, so a paused game never banks time to burst on resume.
            let steps = timestep.advance(frame_dt);
            for _ in 0..steps {
                pos_prev = pos_curr;
                step_player(
                    &mut player,
                    fwd,
                    strafe,
                    turn,
                    move_speed,
                    &statics,
                    timestep.step(),
                );
                pos_curr = player.pos;
            }
            if let Some(msg) = collect_pickups(&mut player, &mut pickups) {
                log.push(msg.clone());
                overlay_msg = msg;
                overlay = Overlay::Pickup;
                last_mouse = None;
            }
            if dist(player.pos, exit) < 0.6 {
                log.push("you reached the exit — you win!");
                overlay_msg = format!("You reached the exit!  Final score: {}", player.score);
                overlay = Overlay::Exit;
                exit_idx = 0;
                last_mouse = None;
            }
        }

        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        buf.fill(Style::DEFAULT.with_bg(Color::Rgb(6, 6, 10)).cell(' '));
        ensure_cols(buf.size(), &mut layout_size, &mut layout_cols);
        let cols = layout_cols;

        let view = Panel::new()
            .border(Some(BorderSet::ROUNDED))
            .title("rift-fps");
        let inner = view.inner(cols[0]);
        samples.resize(
            UVec2::new(inner.width().max(1), inner.height().max(1)),
            2,
            super_sy,
        );
        // No `samples.clear()` here: the raycaster paints every sample of the
        // viewport buffer (ceiling/wall/floor for every column and row), so a
        // pre-clear would be redundant work.
        //
        // Interpolate the camera between the last two physics ticks (translation
        // only — yaw/pitch are applied per render frame from the mouse, so they're
        // already smooth and must not lag behind it).
        let render_pos = pos_prev.lerp(pos_curr, timestep.alpha());
        // Edge-pan assist: while the pointer rests in a viewport edge band, keep
        // turning (and tilting) that way so you can spin continuously without
        // lifting the mouse past the terminal edge. Only while the mouse is the
        // active input; a centred pointer adds nothing.
        if overlay == Overlay::None && cursor_visible {
            let (ex, ey) = edge_pan(last_mouse, inner);
            player.yaw += ex * EDGE_PAN_SPEED * mouse_sens * frame_dt;
            player.pitch = (player.pitch - ey * EDGE_PAN_SPEED * mouse_sens * frame_dt)
                .clamp(-PITCH_LIMIT, PITCH_LIMIT);
        }
        raycaster.render_textured(
            &mut samples,
            &map,
            render_pos,
            player.yaw,
            player.pitch,
            if textures_on { wall_tex } else { None },
            Some(light),
        );

        // Shot effects: advance live sparks, resolve any shot captured this frame
        // (a click sparks at the clicked cell; Space sparks at the centre), then
        // composite the flash/sparks/crosshair onto the freshly raycast viewport
        // (before it is shaded into cells).
        shots.update(frame_dt);
        if let Some(aim) = pending_shot.take() {
            let (sx, sy) = samples.samples_per_cell();
            let (w, h) = (samples.width(), samples.height());
            // Sample position to burst at, plus the normalized horizontal screen
            // coordinate of the ray — computed like the raycaster's columns
            // (`t = sample_x / (w - 1)`), so the pick agrees with what is drawn.
            let aimed = match aim {
                ShotAim::At { col, row } if inner.contains(UVec2::new(col, row)) => {
                    let cx = (col - inner.left()).min(inner.width() - 1);
                    let cy = (row - inner.top()).min(inner.height() - 1);
                    let s_x = cx * sx + sx / 2;
                    let s_y = cy * sy + sy / 2;
                    Some((s_x as f32, s_y as f32, s_x as f32 / (w.max(2) - 1) as f32))
                }
                // Click landed outside the 3D viewport (e.g. on the HUD): ignore.
                ShotAim::At { .. } => None,
                ShotAim::Forward => {
                    let horizon = (h as f32 * 0.5) + player.pitch * h as f32;
                    Some((w as f32 * 0.5, horizon.clamp(0.0, (h - 1) as f32), 0.5))
                }
            };
            if let Some((px, py, screen_x)) = aimed {
                let dir = raycaster.ray_dir(player.yaw, screen_x);
                if let Some(hit) = raycaster.raycast(&map, render_pos, dir) {
                    shots.spawn(px, py, &mut rng);
                    log.push(format!("hit wall #{} @ {:.1}m", hit.wall_id, hit.distance));
                }
            }
        }
        shots.draw(&mut samples);

        // Cursor reticle: marks where a click will land (and spark). Shown while the
        // mouse is the active input and the pointer is over the viewport; hidden once
        // the player uses the arrow keys or Space (see `cursor_visible`).
        if overlay == Overlay::None && cursor_visible {
            if let Some((mc, mr)) = last_mouse {
                if inner.contains(UVec2::new(mc, mr)) {
                    let (sx, sy) = samples.samples_per_cell();
                    let px = (mc - inner.left()).min(inner.width() - 1) * sx + sx / 2;
                    let py = (mr - inner.top()).min(inner.height() - 1) * sy + sy / 2;
                    draw_cursor(&mut samples, px, py);
                }
            }
        }

        // HUD held-key cluster: [W, A, S, D]. In latch mode reflect the latched
        // directions; otherwise the real held state.
        let dirs = if latch {
            [
                move_latch.value() > 0.0,
                strafe_latch.value() < 0.0,
                move_latch.value() < 0.0,
                strafe_latch.value() > 0.0,
            ]
        } else {
            [
                input.held("fwd"),
                input.held("left"),
                input.held("back"),
                input.held("right"),
            ]
        };
        let mode = if latch {
            "LATCH"
        } else if release_reporting {
            "KBD"
        } else {
            "GRACE"
        };

        {
            let mut frame = Frame::root(&mut buf);
            view.render(cols[0], &mut frame);
            render_viewport(
                &samples,
                shaders[shader_idx].1.as_ref(),
                inner,
                &mut frame,
                &mut shade_scratch,
            );
            draw_hud(
                &mut frame,
                cols[1],
                &player,
                &map,
                &pickups,
                &log,
                dirs,
                mode,
                fps_ema,
                render_ms_ema,
                present_ms_ema,
            );
            match overlay {
                Overlay::None => {}
                Overlay::Pause => draw_menu(&mut frame, cols[0], menu_idx),
                Overlay::Options => draw_options(
                    &mut frame,
                    cols[0],
                    opt_idx,
                    shaders[shader_idx].0,
                    mouse_sens,
                    move_speed,
                    super_sy,
                    textures_on,
                ),
                Overlay::Pickup => draw_pickup(&mut frame, cols[0], &overlay_msg),
                Overlay::Exit => draw_exit(&mut frame, cols[0], &overlay_msg, exit_idx),
            }
        }
        // Apply the color-depth cap only when it changes (a change forces a full
        // redraw, so we must not call it every frame).
        let want_depth = if color_256 {
            ColorDepth::Ansi256
        } else {
            caps.color
        };
        if presenter.color_depth() != want_depth {
            presenter.set_color_depth(want_depth);
        }
        // Measure render (raycast + shade + draw) and present (diff + flush, which
        // blocks on the terminal) separately, so the HUD shows where time goes.
        let render_s = frame_start.elapsed().as_secs_f32();
        presenter.present(&buf).map_err(std::io::Error::other)?;
        let present_s = frame_start.elapsed().as_secs_f32() - render_s;

        render_ms_ema = render_ms_ema * 0.9 + render_s * 1000.0 * 0.1;
        present_ms_ema = present_ms_ema * 0.9 + present_s * 1000.0 * 0.1;
        if frame_dt > 0.0 {
            fps_ema = fps_ema * 0.9 + (1.0 / frame_dt) * 0.1;
        }
        // Sleep to hold the target frame rate (coarse sleep + a short spin tail).
        pacer.pace(frame_start);
    }
    Ok(())
}

fn step_player(
    player: &mut Player,
    fwd: f32,
    strafe: f32,
    turn: f32,
    speed: f32,
    statics: &[xre_render::Aabb],
    dt: f32,
) {
    // Arrow-key turning of the view heading (mouse motion turns it directly in the
    // event loop); movement then follows `player.yaw`.
    player.yaw += turn * 2.0 * dt;
    let (sy, cy) = player.yaw.sin_cos();
    let forward = Vec2::new(cy, sy);
    let rightv = Vec2::new(-sy, cy);
    // `speed` is cells/sec (tuned in the Options menu).
    // Normalize diagonal input so W+D isn't ~1.4x faster than a single key
    // (which also kept the mover from overshooting into wall corners).
    let dir = Vec2::new(strafe, fwd);
    let dir = if dir.length_squared() > 1.0 {
        dir.normalize()
    } else {
        dir
    };
    let move2d = (forward * dir.y + rightv * dir.x) * (speed * dt);
    // Resolve in 3D against the tile colliders (y is flat).
    let start = Vec3::new(player.pos.x, 0.0, player.pos.y);
    let vel = Vec3::new(move2d.x, 0.0, move2d.y);
    let end = move_and_slide(start, Vec3::new(0.2, 0.5, 0.2), vel, statics, 4);
    player.pos = Vec2::new(end.x, end.z);
}

/// Collect any pickups within range, scoring them. Returns a message describing
/// what was grabbed (for the log and the Confirm overlay), or `None` if nothing
/// was in range this frame.
fn collect_pickups(player: &mut Player, pickups: &mut Vec<(f32, f32)>) -> Option<String> {
    let before = pickups.len();
    pickups.retain(|&(x, y)| dist(player.pos, Vec2::new(x, y)) > 0.5);
    let got = (before - pickups.len()) as u32;
    if got == 0 {
        return None;
    }
    let gained = got * 10;
    player.score += gained;
    let what = if got == 1 { "an item" } else { "items" };
    Some(format!(
        "Picked up {what} (+{gained}) — score {}",
        player.score
    ))
}

/// Refresh the cached viewport|HUD column split, recomputing the `Layout::split`
/// (which allocates a `Vec<Rect>`) only when the screen size actually changed.
fn ensure_cols(size: UVec2, cache_size: &mut UVec2, cache: &mut [Rect; 2]) {
    if *cache_size != size {
        let v = Layout::horizontal([Constraint::Fill(1), Constraint::Len(22)])
            .split(Rect::new(0, 0, size.x, size.y));
        *cache = [v[0], v[1]];
        *cache_size = size;
    }
}

fn render_viewport(
    samples: &SampleBuffer,
    shader: &dyn CellShader,
    area: xre_core::Rect,
    frame: &mut Frame,
    scratch: &mut Vec<Option<Cell>>,
) {
    let cells = samples.cells();
    let count = (cells.x * cells.y) as usize;
    if scratch.len() < count {
        scratch.resize(count, None);
    }
    // Shade every cell in parallel (row-parallel under the `parallel` feature),
    // then blit the visible region — the per-cell shade is no longer serial.
    resolve_cells(samples, shader, cells.x, cells.y, &mut scratch[..count]);
    for cy in 0..area.height().min(cells.y) {
        for cx in 0..area.width().min(cells.x) {
            if let Some(cell) = scratch[(cy * cells.x + cx) as usize] {
                frame.set(area.left() + cx, area.top() + cy, cell);
            }
        }
    }
}

/// The centred pause-menu panel rect over `area` — the single source of truth
/// shared by [`draw_menu`] (render) and [`menu_hit`] (click hit-testing).
fn menu_panel_rect(area: Rect) -> Rect {
    let w = 24.min(area.width());
    let h = (MENU_ITEMS.len() as u32 + 2).min(area.height());
    let x = area.left() + area.width().saturating_sub(w) / 2;
    let y = area.top() + area.height().saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}

/// The panel's inner content rect (inside the border), where the rows render.
fn menu_inner(area: Rect) -> Rect {
    Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .inner(menu_panel_rect(area))
}

/// Which menu row, if any, is under `(col, row)`. `None` over the border, title
/// or outside the panel.
fn menu_hit(area: Rect, col: u32, row: u32) -> Option<usize> {
    let inner = menu_inner(area);
    if col < inner.left() || col >= inner.right() {
        return None;
    }
    let i = usize::try_from(row.checked_sub(inner.top())?).unwrap_or(usize::MAX);
    (i < MENU_ITEMS.len()).then_some(i)
}

/// Draw the centred pause menu over `area`, highlighting `menu_idx`.
fn draw_menu(frame: &mut Frame, area: Rect, menu_idx: usize) {
    let panel = Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .fill(Style::DEFAULT.with_bg(Color::Rgb(16, 16, 24)).cell(' '))
        .title("PAUSED");
    let pi = panel.render(menu_panel_rect(area), frame);
    debug_assert_eq!(pi, menu_inner(area), "menu render/hit-test layout drift");
    for (i, item) in MENU_ITEMS.iter().enumerate() {
        let cursor = if i == menu_idx { '>' } else { ' ' };
        let style = if i == menu_idx {
            Style::fg(Color::Rgb(120, 240, 140))
        } else {
            Style::fg(Color::Rgb(200, 200, 210))
        };
        Text::styled(format!("{cursor} {item}"), style).render_into(
            xre_core::Rect::new(pi.left(), pi.top() + i as u32, pi.width(), 1),
            frame,
        );
    }
}

// --- Options sub-menu (shader, mouse sensitivity, move speed) ---

/// Adjust the option at `idx` by `dir` (`-1`/`+1`): cycle the shader, or step the
/// mouse sensitivity / move speed within their ranges. The "Back" row is a no-op.
#[allow(clippy::too_many_arguments)]
fn adjust_option(
    idx: usize,
    dir: i32,
    shader_idx: &mut usize,
    shaders_len: usize,
    mouse_sens: &mut f32,
    move_speed: &mut f32,
    super_sy: &mut u32,
    textures_on: &mut bool,
) {
    match idx {
        0 => {
            // Cycle the shader either way without underflowing.
            *shader_idx = if dir > 0 {
                (*shader_idx + 1) % shaders_len
            } else {
                (*shader_idx + shaders_len - 1) % shaders_len
            };
        }
        1 => *mouse_sens = (*mouse_sens + dir as f32 * SENS_STEP).clamp(SENS_MIN, SENS_MAX),
        2 => *move_speed = (*move_speed + dir as f32 * SPEED_STEP).clamp(SPEED_MIN, SPEED_MAX),
        // Detail toggles vertical supersampling 4 <-> 2 (either direction).
        3 => *super_sy = if *super_sy >= 4 { 2 } else { 4 },
        // Textures toggles brick texture vs flat palette.
        4 => *textures_on = !*textures_on,
        _ => {}
    }
}

/// Fixed row layout (columns relative to the panel's inner left edge) so the
/// clickable `<`/`>` glyphs render and hit-test at the same place on every
/// adjustable row: `{cursor} {item:<ITEM_W} < {value:<VAL_W} >`.
const OPT_ITEM_W: usize = 10;
const OPT_VAL_W: usize = 12;
// The row format string below hard-codes these widths (inlined for clippy); keep
// them in lock-step so the `<`/`>` columns match what is drawn.
const _: () = assert!(OPT_ITEM_W == 10 && OPT_VAL_W == 12);
/// Column of the `<` (decrement) glyph: cursor + space + item + space.
const OPT_DEC_COL: u32 = 1 + 1 + OPT_ITEM_W as u32 + 1;
/// Column of the `>` (increment) glyph: `<` + space + value + trailing space.
const OPT_INC_COL: u32 = OPT_DEC_COL + 1 + 1 + OPT_VAL_W as u32 + 1;

/// The centred Options panel rect (one row taller than the pause menu for the
/// keyboard hint), shared by [`draw_options`] and [`options_hit`].
fn options_panel_rect(area: Rect) -> Rect {
    let w = 36.min(area.width());
    let h = (OPTION_ITEMS.len() as u32 + 3).min(area.height());
    let x = area.left() + area.width().saturating_sub(w) / 2;
    let y = area.top() + area.height().saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}
fn options_inner(area: Rect) -> Rect {
    Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .inner(options_panel_rect(area))
}

/// Which option row, if any, is under `(col, row)`.
fn options_hit(area: Rect, col: u32, row: u32) -> Option<usize> {
    let inner = options_inner(area);
    if col < inner.left() || col >= inner.right() {
        return None;
    }
    let i = usize::try_from(row.checked_sub(inner.top())?).unwrap_or(usize::MAX);
    (i < OPTION_ITEMS.len()).then_some(i)
}

/// Which way a click at `col` adjusts an adjustable row: `-1` on the `<` glyph,
/// `+1` on the `>` glyph, `0` elsewhere (each a 2-cell zone for easy clicking).
fn option_click_dir(area: Rect, col: u32) -> i32 {
    let left = options_inner(area).left();
    let dec = left + OPT_DEC_COL;
    let inc = left + OPT_INC_COL;
    if col == dec || col == dec + 1 {
        -1
    } else {
        i32::from(col == inc || col + 1 == inc)
    }
}

/// The value text shown for each adjustable option row (empty for "Back").
fn option_value(
    idx: usize,
    shader_name: &str,
    mouse_sens: f32,
    move_speed: f32,
    super_sy: u32,
    textures_on: bool,
) -> String {
    match idx {
        0 => shader_name.to_string(),
        1 => format!("{mouse_sens:.1}"),
        2 => format!("{move_speed:.1}"),
        3 => format!("2x{super_sy}"),
        4 => if textures_on { "on" } else { "off" }.to_string(),
        _ => String::new(),
    }
}

/// Draw the centred Options sub-menu, highlighting `opt_idx`.
#[allow(clippy::too_many_arguments)]
fn draw_options(
    frame: &mut Frame,
    area: Rect,
    opt_idx: usize,
    shader_name: &str,
    mouse_sens: f32,
    move_speed: f32,
    super_sy: u32,
    textures_on: bool,
) {
    let panel = Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .fill(Style::DEFAULT.with_bg(Color::Rgb(16, 16, 24)).cell(' '))
        .title("OPTIONS");
    let pi = panel.render(options_panel_rect(area), frame);
    debug_assert_eq!(
        pi,
        options_inner(area),
        "options render/hit-test layout drift"
    );
    for (i, item) in OPTION_ITEMS.iter().enumerate() {
        let cursor = if i == opt_idx { '>' } else { ' ' };
        let style = if i == opt_idx {
            Style::fg(Color::Rgb(120, 240, 140))
        } else {
            Style::fg(Color::Rgb(200, 200, 210))
        };
        let value = option_value(
            i,
            shader_name,
            mouse_sens,
            move_speed,
            super_sy,
            textures_on,
        );
        // Adjustable rows lay the value between clickable `<`/`>` at fixed
        // columns; "Back" (empty value) is a plain row.
        let label = if value.is_empty() {
            format!("{cursor} {item}")
        } else {
            format!("{cursor} {item:<10} < {value:<12} >")
        };
        Text::styled(label, style).render_into(
            Rect::new(pi.left(), pi.top() + i as u32, pi.width(), 1),
            frame,
        );
    }
    // Keyboard hint on the last inner row.
    Text::styled(
        "↑↓ select · ←→ or click < > · esc back",
        Style::fg(Color::Rgb(130, 140, 160)),
    )
    .render_into(
        Rect::new(
            pi.left(),
            pi.top() + pi.height().saturating_sub(1),
            pi.width(),
            1,
        ),
        frame,
    );
}

// --- Modal overlays: pickup "Confirm" and the exit "Exit game / Restart" ---

/// Shared width of the pickup/exit overlay panels.
const OVERLAY_W: u32 = 46;
/// Shared height (border + a message row, a spacer and the button row).
const OVERLAY_H: u32 = 5;

/// A centred rect of size `(w, h)`, clamped to `area`.
fn centered(area: Rect, w: u32, h: u32) -> Rect {
    let w = w.min(area.width());
    let h = h.min(area.height());
    let x = area.left() + area.width().saturating_sub(w) / 2;
    let y = area.top() + area.height().saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}

/// Cell width of the rendered button `[ label ]`.
fn button_width(label: &str) -> u32 {
    label.chars().count() as u32 + 4
}

/// Draw a `[ label ]` button filling `rect`, highlighted when `focused`.
fn draw_button(frame: &mut Frame, rect: Rect, label: &str, focused: bool) {
    let style = if focused {
        Style::DEFAULT
            .with_fg(Color::Rgb(16, 16, 24))
            .with_bg(Color::Rgb(120, 240, 140))
    } else {
        Style::DEFAULT
            .with_fg(Color::Rgb(210, 210, 220))
            .with_bg(Color::Rgb(48, 48, 64))
    };
    Text::styled(format!("[ {label} ]"), style).render_into(rect, frame);
}

/// Whether `(col, row)` falls inside `rect`.
const fn button_hit(rect: Rect, col: u32, row: u32) -> bool {
    col >= rect.left() && col < rect.right() && row >= rect.top() && row < rect.bottom()
}

/// The pickup overlay panel and its inner content rect (shared by render + hit-test).
fn pickup_panel(area: Rect) -> Rect {
    centered(area, OVERLAY_W, OVERLAY_H)
}
fn pickup_inner(area: Rect) -> Rect {
    Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .inner(pickup_panel(area))
}

/// The "Confirm" button rect, centred on the panel's last inner row.
fn confirm_button_rect(area: Rect) -> Rect {
    let inner = pickup_inner(area);
    let w = button_width("Confirm").min(inner.width());
    let x = inner.left() + inner.width().saturating_sub(w) / 2;
    let y = inner.top() + inner.height().saturating_sub(1);
    Rect::new(x, y, w, 1)
}

/// Whether `(col, row)` hits the Confirm button.
fn confirm_hit(area: Rect, col: u32, row: u32) -> bool {
    button_hit(confirm_button_rect(area), col, row)
}

/// Draw the centred pickup overlay (`msg` + a focused Confirm button).
fn draw_pickup(frame: &mut Frame, area: Rect, msg: &str) {
    let panel = Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .fill(Style::DEFAULT.with_bg(Color::Rgb(16, 16, 24)).cell(' '))
        .title("ITEM");
    let pi = panel.render(pickup_panel(area), frame);
    debug_assert_eq!(
        pi,
        pickup_inner(area),
        "pickup render/hit-test layout drift"
    );
    Text::styled(msg, Style::fg(Color::Rgb(225, 225, 235)))
        .render_into(Rect::new(pi.left(), pi.top(), pi.width(), 1), frame);
    draw_button(frame, confirm_button_rect(area), "Confirm", true);
}

/// The exit overlay panel and its inner content rect (shared by render + hit-test).
fn exit_panel(area: Rect) -> Rect {
    centered(area, OVERLAY_W, OVERLAY_H)
}
fn exit_inner(area: Rect) -> Rect {
    Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .inner(exit_panel(area))
}

/// The two exit-button rects ("Exit game", "Restart"), centred side by side on
/// the panel's last inner row.
fn exit_button_rects(area: Rect) -> [Rect; 2] {
    let inner = exit_inner(area);
    let gap = 3u32;
    let w0 = button_width(EXIT_LABELS[0]);
    let w1 = button_width(EXIT_LABELS[1]);
    let total = w0 + gap + w1;
    let x0 = inner.left() + inner.width().saturating_sub(total) / 2;
    let y = inner.top() + inner.height().saturating_sub(1);
    [Rect::new(x0, y, w0, 1), Rect::new(x0 + w0 + gap, y, w1, 1)]
}

/// Which exit button, if any, is under `(col, row)`.
fn exit_hit(area: Rect, col: u32, row: u32) -> Option<usize> {
    exit_button_rects(area)
        .iter()
        .position(|r| button_hit(*r, col, row))
}

/// Draw the centred exit overlay (`msg` + two buttons, `focus` highlighted).
fn draw_exit(frame: &mut Frame, area: Rect, msg: &str, focus: usize) {
    let panel = Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .fill(Style::DEFAULT.with_bg(Color::Rgb(16, 16, 24)).cell(' '))
        .title("EXIT");
    let pi = panel.render(exit_panel(area), frame);
    debug_assert_eq!(pi, exit_inner(area), "exit render/hit-test layout drift");
    Text::styled(msg, Style::fg(Color::Rgb(225, 225, 235)))
        .render_into(Rect::new(pi.left(), pi.top(), pi.width(), 1), frame);
    for (i, (rect, label)) in exit_button_rects(area).iter().zip(EXIT_LABELS).enumerate() {
        draw_button(frame, *rect, label, i == focus);
    }
}

/// The HUD's row split (minimap / status / log), memoized on `(area, map_height)`.
///
/// `Layout::split` allocates a `Vec<Rect>`, and the HUD is laid out every frame
/// though its rects only change on resize — so cache them in a thread-local,
/// mirroring `ensure_cols` for the top-level columns. Keeps the steady-state
/// frame from allocating here.
fn hud_rows(area: Rect, map_h: u32) -> [Rect; 3] {
    // `(layout key, cached rows)`: the rows are valid while the area and map height
    // are unchanged.
    type HudRowCache = Option<((Rect, u32), [Rect; 3])>;
    thread_local! {
        static CACHE: RefCell<HudRowCache> = const { RefCell::new(None) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((key, rows)) = *c {
            if key == (area, map_h) {
                return rows;
            }
        }
        let v = Layout::vertical([
            Constraint::Len(map_h + 2),
            Constraint::Len(7),
            Constraint::Fill(1),
        ])
        .split(area);
        let rows = [v[0], v[1], v[2]];
        *c = Some(((area, map_h), rows));
        rows
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_hud(
    frame: &mut Frame,
    area: xre_core::Rect,
    player: &Player,
    map: &TileMap,
    pickups: &[(f32, f32)],
    log: &Log,
    dirs: [bool; 4],
    mode: &str,
    fps: f32,
    render_ms: f32,
    present_ms: f32,
) {
    let rows = hud_rows(area, map.height());

    // Minimap.
    let mp = Panel::new().border(Some(BorderSet::ASCII)).title("map");
    let mi = mp.render(rows[0], frame);
    for y in 0..map.height() {
        for x in 0..map.width() {
            let solid = map.is_solid(x as i32, y as i32);
            let glyph = if solid { '#' } else { ' ' };
            frame.set(
                mi.left() + x,
                mi.top() + y,
                Style::fg(Color::Rgb(80, 90, 110)).cell(glyph),
            );
        }
    }
    for &(px, py) in pickups {
        frame.set(
            mi.left() + px as u32,
            mi.top() + py as u32,
            Style::fg(Color::Rgb(240, 220, 80)).cell('P'),
        );
    }
    let (plx, ply) = (player.pos.x as u32, player.pos.y as u32);
    frame.set(
        mi.left() + plx,
        mi.top() + ply,
        Style::fg(Color::Rgb(120, 240, 140)).cell('@'),
    );

    // Stats: health gauge + score.
    let sp = Panel::new().border(Some(BorderSet::ASCII)).title("status");
    let si = sp.render(rows[1], frame);
    Gauge::new(player.health)
        .label(format!("HP {:.0}%", player.health * 100.0))
        .render(
            xre_core::Rect::new(si.left(), si.top(), si.width(), 1),
            frame,
        );
    Text::raw(format!("score {}", player.score)).render_into(
        xre_core::Rect::new(si.left(), si.top() + 1, si.width(), 1),
        frame,
    );
    Text::styled("h: menu", Style::fg(Color::Rgb(140, 150, 170))).render_into(
        xre_core::Rect::new(si.left(), si.top() + 2, si.width(), 1),
        frame,
    );
    // Held-key cluster: W/A/S/D light up while held, plus the input mode tag.
    let keys = [
        ('W', dirs[0]),
        ('A', dirs[1]),
        ('S', dirs[2]),
        ('D', dirs[3]),
    ];
    let ky = si.top() + 3;
    for (i, (ch, on)) in keys.iter().enumerate() {
        let style = if *on {
            Style::fg(Color::Rgb(120, 240, 140))
        } else {
            Style::fg(Color::Rgb(80, 85, 100))
        };
        Text::styled(ch.to_string(), style)
            .render_into(Rect::new(si.left() + i as u32 * 2, ky, 1, 1), frame);
    }
    Text::styled(mode, Style::fg(Color::Rgb(150, 160, 180))).render_into(
        Rect::new(si.left() + 9, ky, si.width().saturating_sub(9), 1),
        frame,
    );
    // Frame-time readout: r = render (CPU), p = present (terminal). A high `p`
    // with low `r` means the terminal's redraw rate is the ceiling, not the engine.
    Text::styled(
        format!("{fps:.0}fps r{render_ms:.1} p{present_ms:.1}ms"),
        Style::fg(Color::Rgb(150, 160, 180)),
    )
    .render_into(Rect::new(si.left(), ky + 1, si.width(), 1), frame);

    // Log.
    let lp = Panel::new().border(Some(BorderSet::ASCII)).title("log");
    let li = lp.render(rows[2], frame);
    log.render(li, frame);
}

fn dist(a: Vec2, b: Vec2) -> f32 {
    a.distance(b)
}

fn pickup_positions(map: &str) -> Vec<(f32, f32)> {
    char_positions(map, 'P')
}

fn exit_position(map: &str) -> Vec2 {
    char_positions(map, 'E')
        .first()
        .map_or(Vec2::new(1.5, 1.5), |&(x, y)| Vec2::new(x, y))
}

fn char_positions(map: &str, target: char) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for (y, line) in map.lines().filter(|l| !l.is_empty()).enumerate() {
        for (x, ch) in line.chars().enumerate() {
            if ch == target {
                out.push((x as f32 + 0.5, y as f32 + 0.5));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_hit_maps_rows_and_misses() {
        let area = Rect::new(0, 0, 80, 24);
        let inner = menu_inner(area);
        // Each item row hit-tests to its index, at both inner edges.
        for i in 0..MENU_ITEMS.len() as u32 {
            let row = inner.top() + i;
            assert_eq!(menu_hit(area, inner.left(), row), Some(i as usize));
            assert_eq!(menu_hit(area, inner.right() - 1, row), Some(i as usize));
        }
        // Border row above the first item: miss.
        assert_eq!(menu_hit(area, inner.left(), inner.top() - 1), None);
        // Column just left of the inner area: miss.
        assert_eq!(menu_hit(area, inner.left() - 1, inner.top()), None);
        // Row past the last item: miss.
        assert_eq!(
            menu_hit(area, inner.left(), inner.top() + MENU_ITEMS.len() as u32),
            None
        );
        // Far outside the panel: miss.
        assert_eq!(menu_hit(area, 0, 0), None);
    }

    #[test]
    fn activate_menu_covers_every_row() {
        assert!(matches!(activate_menu(0), Some(MenuAction::Resume)));
        assert!(matches!(activate_menu(1), Some(MenuAction::OpenOptions)));
        assert!(matches!(activate_menu(2), Some(MenuAction::Restart)));
        assert!(matches!(activate_menu(3), Some(MenuAction::Quit)));
        assert!(activate_menu(MENU_ITEMS.len()).is_none());
    }

    #[test]
    fn latch_toggle_starts_stops_and_reverses() {
        let mut ax = LatchAxis::default();
        latch_toggle(&mut ax, true); // tap forward → +1
        assert_eq!(ax.value(), 1.0);
        latch_toggle(&mut ax, true); // tap forward again → stop
        assert_eq!(ax.value(), 0.0);
        latch_toggle(&mut ax, true); // forward again → +1
        latch_toggle(&mut ax, false); // opposite → reverses to -1
        assert_eq!(ax.value(), -1.0);
    }

    #[test]
    fn adjust_option_cycles_shader_and_clamps_sliders() {
        let mut shader = 0usize;
        let mut sens = SENS_MIN;
        let mut speed = SPEED_MAX;
        let mut ss = 4u32;
        let mut tex = true;
        // Shader wraps both directions.
        adjust_option(
            0,
            -1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert_eq!(shader, 3);
        adjust_option(
            0,
            1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert_eq!(shader, 0);
        // Sliders clamp at their bounds.
        adjust_option(
            1,
            -1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert!((sens - SENS_MIN).abs() < f32::EPSILON, "sens floors at min");
        adjust_option(
            2,
            1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert!(
            (speed - SPEED_MAX).abs() < f32::EPSILON,
            "speed caps at max"
        );
        // A step moves by exactly one increment.
        adjust_option(
            1,
            1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert!((sens - (SENS_MIN + SENS_STEP)).abs() < 1e-6);
        // Detail toggles supersampling 4 <-> 2 either direction.
        adjust_option(
            3,
            1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert_eq!(ss, 2);
        adjust_option(
            3,
            -1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert_eq!(ss, 4);
        // Textures toggles off then on.
        adjust_option(
            4,
            1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert!(!tex);
        adjust_option(
            4,
            -1,
            &mut shader,
            4,
            &mut sens,
            &mut speed,
            &mut ss,
            &mut tex,
        );
        assert!(tex);
    }

    #[test]
    fn options_hit_maps_rows_and_back_is_last() {
        let area = Rect::new(0, 0, 80, 24);
        let inner = options_inner(area);
        for i in 0..OPTION_ITEMS.len() as u32 {
            assert_eq!(
                options_hit(area, inner.left(), inner.top() + i),
                Some(i as usize)
            );
        }
        // The hint row past the last option is not a row.
        assert_eq!(
            options_hit(area, inner.left(), inner.top() + OPTION_ITEMS.len() as u32),
            None
        );
        assert_eq!(OPTION_ITEMS[OPTION_ITEMS.len() - 1], "Back");
    }

    #[test]
    fn option_click_dir_decrements_on_left_arrow() {
        let area = Rect::new(0, 0, 80, 24);
        let left = options_inner(area).left();
        // Clicking the `<` glyph (or the space after) decrements.
        assert_eq!(option_click_dir(area, left + OPT_DEC_COL), -1);
        assert_eq!(option_click_dir(area, left + OPT_DEC_COL + 1), -1);
        // Clicking the `>` glyph (or the space before) increments.
        assert_eq!(option_click_dir(area, left + OPT_INC_COL), 1);
        assert_eq!(option_click_dir(area, left + OPT_INC_COL - 1), 1);
        // The label and value columns are neither.
        assert_eq!(option_click_dir(area, left), 0);
        assert_eq!(option_click_dir(area, left + OPT_DEC_COL + 3), 0);
    }

    #[test]
    fn exit_action_maps_buttons() {
        // "Exit game" quits, "Restart" restarts; anything else is a miss.
        assert!(matches!(exit_action(0), Some(MenuAction::Quit)));
        assert!(matches!(exit_action(1), Some(MenuAction::Restart)));
        assert!(exit_action(EXIT_LABELS.len()).is_none());
    }

    #[test]
    fn confirm_button_hits_inside_and_misses_outside() {
        let area = Rect::new(0, 0, 80, 24);
        let r = confirm_button_rect(area);
        // Every cell of the button hit-tests true.
        for col in r.left()..r.right() {
            assert!(confirm_hit(area, col, r.top()));
        }
        // Just outside in each direction misses.
        assert!(!confirm_hit(area, r.left().saturating_sub(1), r.top()));
        assert!(!confirm_hit(area, r.right(), r.top()));
        assert!(!confirm_hit(area, r.left(), r.top().saturating_sub(1)));
        assert!(!confirm_hit(area, r.left(), r.bottom()));
    }

    #[test]
    fn exit_buttons_hit_distinctly() {
        let area = Rect::new(0, 0, 80, 24);
        let [r0, r1] = exit_button_rects(area);
        assert!(r0.right() <= r1.left(), "exit buttons must not overlap");
        assert_eq!(exit_hit(area, r0.left(), r0.top()), Some(0));
        assert_eq!(exit_hit(area, r1.left(), r1.top()), Some(1));
        // The gap between the two buttons is neither button.
        assert_eq!(exit_hit(area, r0.right(), r0.top()), None);
        // Far outside the panel misses.
        assert_eq!(exit_hit(area, 0, 0), None);
    }

    #[test]
    fn pickup_message_singular_and_plural() {
        let mut player = new_player();
        // One pickup right on the player → singular, +10.
        let mut one = vec![(player.pos.x, player.pos.y)];
        let msg = collect_pickups(&mut player, &mut one).expect("collected one");
        assert!(msg.contains("an item"), "got: {msg}");
        assert_eq!(player.score, 10);
        assert!(one.is_empty());
        // Two at once → plural, +20.
        let mut two = vec![(player.pos.x, player.pos.y), (player.pos.x, player.pos.y)];
        let msg = collect_pickups(&mut player, &mut two).expect("collected two");
        assert!(msg.contains("items"), "got: {msg}");
        assert_eq!(player.score, 30);
        // Nothing in range → None.
        let mut far = vec![(player.pos.x + 50.0, player.pos.y)];
        assert!(collect_pickups(&mut player, &mut far).is_none());
    }
}
