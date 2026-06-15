# The game loop

This chapter covers `xre-engine`, the application layer that turns the renderer and
TUI into a *game* engine: a deterministic fixed-timestep loop, a thin ECS over
`hecs`, keyframe animation, action-based input, swept-AABB collision, and an
optional grid raycaster. The loop entry points (`run`, `Game`, `Time`,
`FixedTimestep`, `FramePacer`, `InputMap`, `World`, `Schedule`) are in the
`xre::prelude`; the animation and binding types live under `xre::engine`.

## The fixed-timestep loop

Simulation runs at a fixed `dt` decoupled from the render rate. Two pieces drive
this: `FixedTimestep`, an accumulator that decides *how many* fixed updates to run
this frame, and `FramePacer`, which sleeps to hold a target frame rate.

You construct a timestep with an update rate in Hz (clamped to `1.0..=1000.0`):

```rust,ignore
use xre::prelude::*; // FixedTimestep, FramePacer, Time

let mut timestep = FixedTimestep::new(60.0); // 60 Hz → 16.6 ms step
```

Each frame you feed it the wall-clock delta and it returns the number of whole
fixed steps to run:

```rust,ignore
let frame_dt = last.elapsed().as_secs_f32();
let steps = timestep.advance(frame_dt);
for _ in 0..steps {
    let time = timestep.time();
    // ... advance the simulation by exactly time.dt ...
}
```

`advance` adds `frame_dt` to an internal accumulator, then drains it one `step` at
a time. Two guards matter:

- **Spiral-of-death clamp.** The accumulator is capped at `max_steps * step`
  (`max_steps` is 8). A 10-second stall — a debug breakpoint, a GC pause — runs at
  most 8 steps, never an unbounded catch-up burst.
- **Defensive deltas.** Negative or non-finite `frame_dt` is treated as `0.0`, so a
  clock glitch can never run a step backwards.

`FramePacer::new(fps)` targets a frame budget. `remaining(frame_start)` returns the
leftover budget (zero on overrun), which doubles as an input-poll timeout;
`pace(frame_start)` sleeps until the deadline, using a coarse `thread::sleep` then
a short busy-spin for the final millisecond to land precisely.

### Determinism

A fixed `dt` plus a recorded input stream yields a **bit-identical state hash**
across runs and platforms — the property that makes replays and `insta`
golden-frame tests possible. The crate forbids FMA in result paths (plain
`a*b + c`), so float results don't vary by intrinsic. The default `parallel`
rendering feature preserves this: rows are independent, so parallel shading
produces the same buffer as serial. If you render through `draw_mesh` /
`Viewport3D`, you get the speedup transparently with no determinism cost.

## The `Game` trait and `run`

The entry point is the `Game` trait plus the `run` driver. `Game` has three
methods:

```rust,ignore
pub trait Game {
    /// Advance the simulation by one fixed `time.dt`, consuming this step's input.
    fn update(&mut self, time: &Time, events: &[Event]);
    /// Draw the current state into `buf` (already sized to the terminal).
    fn render(&mut self, buf: &mut CellBuffer, time: &Time);
    /// Return `false` to exit the loop.
    fn running(&self) -> bool;
}
```

`run` wires up the terminal and drives your game:

```rust,ignore
pub fn run(game: impl Game, config: EngineConfig) -> Result<(), EngineError>;
```

`EngineConfig` is two rates, both defaulting to 60.0:

```rust,ignore
pub struct EngineConfig {
    pub update_hz: f32,  // fixed update rate
    pub target_fps: f32, // render frame cap
}
```

Internally `run` installs a `TerminalGuard` (RAII raw-mode restore *and* a panic
hook), probes `Capabilities`, builds a diffed `Presenter`, and loops: pump input
within the pacer's remaining budget, handle `Resize`, call `advance`, run `update`
for each fixed step (events belong to the *first* update of the frame — frame
coherent), then `render` and `present`, and finally `pace`. The loop exits when
`running()` returns `false`.

A minimal game:

```rust,ignore
use xre::prelude::*;

struct Demo { frames: u64, alive: bool }

impl Game for Demo {
    fn update(&mut self, _time: &Time, events: &[Event]) {
        self.frames += 1;
        for ev in events {
            if let Event::Key(k) = ev {
                if matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    self.alive = false;
                }
            }
        }
    }
    fn render(&mut self, buf: &mut CellBuffer, _time: &Time) {
        buf.fill(Cell::new(' '));
        // ... draw into buf via Frame::root(buf) ...
    }
    fn running(&self) -> bool { self.alive }
}

fn main() -> Result<(), xre::engine::EngineError> {
    run(Demo { frames: 0, alive: true }, EngineConfig::default())
}
```

> The `rift-fps` example does *not* use `run` — it hand-rolls the loop to integrate
> the TUI `Frame` and a `SampleBuffer` directly. That is a valid pattern: `run` is
> the convenience driver, but the building blocks (`FixedTimestep`, `FramePacer`,
> `EventQueue`, `Presenter`) compose freely.

## `Time`

`Time` is the per-frame timing resource handed to `update` and `render`:

```rust,ignore
pub struct Time {
    pub dt: f32,      // the fixed update timestep, seconds
    pub elapsed: f32, // total simulated time, seconds
    pub frame: u64,   // update tick counter
    pub alpha: f32,   // 0.0..=1.0 interpolation between the last two fixed states
}
```

In `update`, always integrate by `time.dt` (never wall-clock) — that is what keeps
the simulation deterministic. In `render`, use `time.alpha` to interpolate between
the previous and current fixed state so motion looks smooth even when the render
rate exceeds the update rate.

## ECS

The ECS is a deliberately thin facade over `hecs`: `World` and `Entity` are
re-exported as-is (semver insulation), and the crate adds a handful of provided
components plus an ordered system `Schedule`.

Provided components (in `xre::engine::ecs`): `Velocity(Vec3)`, `Spin { axis, speed
}`, `AabbCollider { half }`, `Lifetime { remaining }`, and `MeshInstance { mesh:
Arc<Mesh>, material }`. They pair with `xre_core::Transform`.

```rust,ignore
use xre::prelude::*; // World, Schedule
use xre::engine::ecs::{Velocity, Spin};

let mut world = World::new();
let e = world.spawn((
    Transform::from_translation(Vec3::ZERO),
    Velocity(Vec3::new(1.0, 0.0, 0.0)),
    Spin { axis: Vec3::Y, speed: 1.0 },
));
```

A `System` is `Box<dyn FnMut(&mut World, &Time)>`. `Schedule` is a plain ordered
list — no async, no dependency graph. Build one with `with`, run it once per fixed
update:

```rust,ignore
let mut schedule = Schedule::new().with(|world, time| {
    for (_, (t, v)) in world.query_mut::<(&mut Transform, &Velocity)>() {
        t.translation += v.0 * time.dt;
    }
});

schedule.run(&mut world, &time); // call inside Game::update
```

`Schedule::standard()` bundles the built-in systems: integrate velocity, apply
spin, update animators, then tick lifetimes (despawning expired entities). To feed
the renderer, `draw_items(&world)` hoists `(Mat4, Arc<Mesh>, Material)` tuples out
once per frame — queries stay out of the pixel loop.

## Animation

Animation is keyframe-driven. A `Track<T>` is a sorted list of keys interpolated
with per-segment `Easing`; `T` may be `f32`, `Vec3`, or `Quat` (the last via
slerp). Easings are `Linear`, `Smooth`, `EaseIn`, `EaseOut`, `Step`.

```rust,ignore
use xre::prelude::*; // Vec3
use xre::engine::{Track, Easing};

let path = Track::<Vec3>::new()
    .key(0.0, Vec3::ZERO, Easing::Linear)
    .key(1.0, Vec3::X,    Easing::Smooth);
```

`key(time, value, easing)` keeps the track sorted (the easing applies to the
segment *leading into* that key). `sample(t)` clamps to the endpoints.

A `Clip` bundles `translation`, `rotation`, and `scale` tracks into a `Transform`
animation; unset channels fall back to a base transform. An `Animator` plays a
clip:

```rust,ignore
use xre::engine::{Animator, Clip, PlayMode};

let clip = Clip { translation: path, ..Clip::default() };
let mut animator = Animator::new(clip)
    .mode(PlayMode::Loop)              // Once | Loop | PingPong
    .speed(1.0);

animator.update(time.dt);                                 // advance the playhead
let transform = animator.sample(Transform::IDENTITY);     // sampled pose
```

Attach an `Animator` to an entity and the `update_animators` system (in
`Schedule::standard()`) advances it and writes the sampled `Transform`
automatically. For procedural one-shots — UI slides, camera moves — use a
`Tween<T>`:

```rust,ignore
use xre::engine::{Tween, Easing};

let mut tween = Tween::new(0.0f32, 100.0).over(0.5).ease(Easing::EaseOut);
let v = tween.update(time.dt);   // current value
let done = tween.finished();
```

## Input

`InputMap` maps raw events to named actions, organised into **contexts** (e.g.
`"menu"` vs `"game"`) stacked so you can layer a pause menu over gameplay. Bind
actions, push a context, then query:

```rust,ignore
use xre::prelude::*;       // InputMap, KeyCode
use xre::engine::Binding;

let mut input = InputMap::new();
input.bind("game", "fwd",  Binding::key(KeyCode::Char('w')));
input.bind("game", "back", Binding::key(KeyCode::Char('s')));
input.bind("game", "turn_l", Binding::key(KeyCode::Left));
input.bind("game", "turn_r", Binding::key(KeyCode::Right));
input.push_context("game");
```

Each frame: call `begin_frame()` (last frame's actions become the baseline),
`feed(&event)` every drained event, then query. `held(action)` is level (active
this frame), `pressed` is the rising edge, `released` the falling edge.
`axis(neg, pos)` returns `-1.0 / 0.0 / +1.0` from a pair of held actions:

```rust,ignore
input.begin_frame();
for ev in events.drain() { input.feed(&ev); }

let turn = input.axis("turn_l", "turn_r");
let fwd  = input.axis("back", "fwd");
```

A `Binding` is `Binding::Key(code, mods)` or `Binding::Mouse(button)`;
`Binding::key(code)` is the no-modifier shorthand. Terminals without the kitty
protocol can't report key *release*, so `held` is synthesised from press/repeat and
released the frame after input stops. `rebind` replaces all bindings for an action
— the user-rebinding API.

## Collision

Movement uses **swept-AABB** resolution: it is tunnel-proof at any speed or `dt`.
`move_and_slide` moves a box against a slice of static `Aabb`s, stopping at contacts
and sliding along surfaces:

```rust,ignore
use xre::engine::collide::move_and_slide;
use xre::prelude::*; // Vec3

let start = Vec3::new(player.x, 0.0, player.z);
let half  = Vec3::new(0.2, 0.5, 0.2);
let vel   = Vec3::new(move2d.x, 0.0, move2d.y);
let end = move_and_slide(start, half, vel, &statics, 4); // 4 slide passes
```

Each pass sweeps the box against every static, takes the earliest contact, advances
to just before it, removes the velocity component into the surface, and retries. A
property test confirms no tunnelling for speeds up to 1000 units. For broadphase,
`UniformGrid::new(cell)` buckets boxes by cell and `query(aabb)` returns
deduplicated candidate indices.

## Grid raycaster

Behind the `grid-raycaster` feature, a per-column DDA renders a 2-D `TileMap` from
a first-person camera into the standard `SampleBuffer` — so **every cell shader
works on it for free**, and the same map yields static colliders for the swept
resolver.

```rust,ignore
use xre::engine::raycaster::{Raycaster, TileMap};

let map = TileMap::parse(MAP);          // '#'/'1'..'9' = walls, ' '/'.' = empty
let statics = map.colliders();          // one unit AABB per solid tile
let raycaster = Raycaster::default();   // fov ≈ π/3, far = 32

raycaster.render(&mut samples, &map, player.pos, player.yaw, /*pitch*/ 0.0);
```

`rift-fps` ties it all together: parse the map, derive colliders, drive
`player.pos`/`yaw` from the `InputMap` axes, resolve motion with `move_and_slide`,
raycast into a `SampleBuffer`, shade it with `BlockShades`, and composite alongside
a TUI HUD (minimap, health `Gauge`, `Log`).

## Running rift-fps

The example needs its feature flag:

```sh
cargo run -p xre-engine --example rift-fps --features grid-raycaster
```

Move with `W`/`A`/`S`/`D`, turn with `←`/`→`, quit with `q`. Its source is the
canonical reference for stitching the loop, input, collision, raycaster, and TUI
into one artifact.
