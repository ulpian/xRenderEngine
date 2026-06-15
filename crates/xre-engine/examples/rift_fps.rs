//! `rift-fps` — the Phase 5 / Gate G5 demo game.
//!
//! A tiny dungeon crawler on the grid-raycaster backend: WASD move with swept
//! collision (no wall-clipping), a TUI HUD (minimap, health gauge, log), pickups
//! to collect and an exit tile that wins. Exercises the raycaster, input map,
//! collision-lite and the TUI layer in one artifact.
//!
//! Run with `cargo run -p xre-engine --example rift-fps --features grid-raycaster`.
//! Move: W/A/S/D · turn: ← / → · menu: h · quit: q.
//!
//! Press `h` to pause and open the menu (resume, switch shader, restart, quit).
#![allow(clippy::suboptimal_flops)] // determinism: plain float math, no FMA

use std::time::{Duration, Instant};

use xre_core::math::{UVec2, Vec2, Vec3};
use xre_core::{CellBuffer, Color, Style};
use xre_engine::collide::move_and_slide;
use xre_engine::raycaster::{Raycaster, TileMap};
use xre_engine::{Binding, InputMap};
use xre_render::{builtin_cell_shaders, CellShader, SampleBuffer};
use xre_term::{Capabilities, Event, EventQueue, KeyCode, Presenter, TerminalGuard};
use xre_tui::{BorderSet, Constraint, Frame, Gauge, Layout, Log, Panel, Text, Widget};

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
    health: f32,
    score: u32,
    won: bool,
}

/// A fresh player at the spawn tile (used at startup and on restart).
const fn new_player() -> Player {
    Player {
        pos: Vec2::new(1.5, 1.5),
        yaw: 0.0,
        health: 1.0,
        score: 0,
        won: false,
    }
}

/// Pause-menu rows, in order.
const MENU_ITEMS: [&str; 4] = ["Resume", "Shader", "Restart", "Quit"];

#[allow(clippy::too_many_lines)]
fn main() -> std::io::Result<()> {
    let _guard = TerminalGuard::enter().map_err(std::io::Error::other)?;
    let caps = Capabilities::probe();
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

    let raycaster = Raycaster::default();
    // Shaders cycled from the pause menu; default to "blocks" (the retro look).
    let shaders = builtin_cell_shaders();
    let mut shader_idx = shaders
        .iter()
        .position(|(name, _)| *name == "blocks")
        .unwrap_or(0);
    let mut samples = SampleBuffer::new(UVec2::new(1, 1), 2, 4);
    let mut log = Log::new(64);
    log.push("find the exit (E). collect P for points.");

    let mut player = new_player();
    // Pause/menu state: `h` toggles `paused`; `menu_idx` is the cursor row.
    let mut paused = false;
    let mut menu_idx = 0usize;
    let mut last = Instant::now();
    let mut running = true;

    while running {
        events
            .pump(Duration::from_millis(16))
            .map_err(std::io::Error::other)?;
        input.begin_frame();
        for ev in events.drain() {
            match ev {
                Event::Resize(size) => {
                    buf.resize(size);
                    presenter.resize(size);
                }
                // While paused, keys drive the menu instead of the player.
                Event::Key(k) if paused => match k.code {
                    KeyCode::Up => {
                        menu_idx = (menu_idx + MENU_ITEMS.len() - 1) % MENU_ITEMS.len();
                    }
                    KeyCode::Down => menu_idx = (menu_idx + 1) % MENU_ITEMS.len(),
                    KeyCode::Left => {
                        shader_idx = (shader_idx + shaders.len() - 1) % shaders.len();
                    }
                    KeyCode::Right => shader_idx = (shader_idx + 1) % shaders.len(),
                    KeyCode::Enter => match menu_idx {
                        0 => paused = false,                                // Resume
                        1 => shader_idx = (shader_idx + 1) % shaders.len(), // Shader
                        2 => {
                            // Restart: reset the world to its initial state.
                            player = new_player();
                            pickups = pickup_positions(MAP);
                            log = Log::new(64);
                            log.push("find the exit (E). collect P for points.");
                            paused = false;
                        }
                        _ => running = false, // Quit
                    },
                    KeyCode::Char('h') | KeyCode::Esc => paused = false,
                    KeyCode::Char('q') => running = false,
                    _ => {}
                },
                Event::Key(k) if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) => {
                    running = false;
                }
                Event::Key(k) if matches!(k.code, KeyCode::Char('h')) => {
                    paused = true;
                    menu_idx = 0;
                }
                ref e => input.feed(e),
            }
        }

        let dt = last.elapsed().as_secs_f32();
        last = Instant::now();

        if !paused && !player.won {
            step_player(&mut player, &input, &statics, dt);
            collect_pickups(&mut player, &mut pickups, &mut log);
            if dist(player.pos, exit) < 0.6 {
                player.won = true;
                log.push("you reached the exit — you win!");
            }
        }

        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        buf.fill(Style::DEFAULT.with_bg(Color::Rgb(6, 6, 10)).cell(' '));
        let area = buf.area();
        let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Len(22)]).split(area);

        let view = Panel::new()
            .border(Some(BorderSet::ROUNDED))
            .title("rift-fps");
        let inner = view.inner(cols[0]);
        samples.resize(
            UVec2::new(inner.width().max(1), inner.height().max(1)),
            2,
            4,
        );
        samples.clear([10, 10, 16]);
        raycaster.render(&mut samples, &map, player.pos, player.yaw, 0.0);

        {
            let mut frame = Frame::root(&mut buf);
            view.render(cols[0], &mut frame);
            render_viewport(&samples, shaders[shader_idx].1.as_ref(), inner, &mut frame);
            draw_hud(&mut frame, cols[1], &player, &map, &pickups, &log);
            if paused {
                draw_menu(&mut frame, cols[0], menu_idx, shaders[shader_idx].0);
            }
        }
        presenter.present(&buf).map_err(std::io::Error::other)?;
    }
    Ok(())
}

fn step_player(player: &mut Player, input: &InputMap, statics: &[xre_render::Aabb], dt: f32) {
    let turn = input.axis("turn_l", "turn_r");
    player.yaw += turn * 2.0 * dt;
    let fwd = input.axis("back", "fwd");
    let strafe = input.axis("left", "right");
    let (sy, cy) = player.yaw.sin_cos();
    let forward = Vec2::new(cy, sy);
    let rightv = Vec2::new(-sy, cy);
    let speed = 2.5;
    let move2d = (forward * fwd + rightv * strafe) * (speed * dt);
    // Resolve in 3D against the tile colliders (y is flat).
    let start = Vec3::new(player.pos.x, 0.0, player.pos.y);
    let vel = Vec3::new(move2d.x, 0.0, move2d.y);
    let end = move_and_slide(start, Vec3::new(0.2, 0.5, 0.2), vel, statics, 4);
    player.pos = Vec2::new(end.x, end.z);
}

fn collect_pickups(player: &mut Player, pickups: &mut Vec<(f32, f32)>, log: &mut Log) {
    let before = pickups.len();
    pickups.retain(|&(x, y)| dist(player.pos, Vec2::new(x, y)) > 0.5);
    let got = before - pickups.len();
    if got > 0 {
        player.score += got as u32 * 10;
        log.push(format!(
            "picked up {got} (+{}) — score {}",
            got * 10,
            player.score
        ));
    }
}

fn render_viewport(
    samples: &SampleBuffer,
    shader: &dyn CellShader,
    area: xre_core::Rect,
    frame: &mut Frame,
) {
    let cells = samples.cells();
    for cy in 0..area.height().min(cells.y) {
        for cx in 0..area.width().min(cells.x) {
            if let Some(cell) = shader.shade(samples, cx, cy) {
                frame.set(area.left() + cx, area.top() + cy, cell);
            }
        }
    }
}

/// Draw the centred pause menu over `area`, highlighting `menu_idx` and showing
/// the active `shader_name` on the Shader row.
fn draw_menu(frame: &mut Frame, area: xre_core::Rect, menu_idx: usize, shader_name: &str) {
    let w = 24.min(area.width());
    let h = (MENU_ITEMS.len() as u32 + 2).min(area.height());
    let x = area.left() + area.width().saturating_sub(w) / 2;
    let y = area.top() + area.height().saturating_sub(h) / 2;
    let panel = Panel::new()
        .border(Some(BorderSet::ROUNDED))
        .fill(Style::DEFAULT.with_bg(Color::Rgb(16, 16, 24)).cell(' '))
        .title("PAUSED");
    let pi = panel.render(xre_core::Rect::new(x, y, w, h), frame);
    for (i, item) in MENU_ITEMS.iter().enumerate() {
        let cursor = if i == menu_idx { '>' } else { ' ' };
        let label = if *item == "Shader" {
            format!("{cursor} Shader: {shader_name}")
        } else {
            format!("{cursor} {item}")
        };
        let style = if i == menu_idx {
            Style::fg(Color::Rgb(120, 240, 140))
        } else {
            Style::fg(Color::Rgb(200, 200, 210))
        };
        Text::styled(label, style).render_into(
            xre_core::Rect::new(pi.left(), pi.top() + i as u32, pi.width(), 1),
            frame,
        );
    }
}

fn draw_hud(
    frame: &mut Frame,
    area: xre_core::Rect,
    player: &Player,
    map: &TileMap,
    pickups: &[(f32, f32)],
    log: &Log,
) {
    let rows = Layout::vertical([
        Constraint::Len(map.height() + 2),
        Constraint::Len(5),
        Constraint::Fill(1),
    ])
    .split(area);

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
