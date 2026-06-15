//! The ECS layer: components, a thin [`hecs`] world facade, and built-in systems
//! (Stage 5.2).
//!
//! Per the semver-insulation decision (Q5), [`hecs`] types are re-exported rather
//! than re-implemented; this module adds the provided components, a system
//! [`Schedule`] (a plain ordered list — no async, no graph solver), and the
//! ECS→scene bridge so rendering stays decoupled from gameplay state, with
//! queries hoisted out of the pixel loop (the Command_Line_3D EnTT lesson,
//! `RiftEngine-Plan/10-phase-5-game-engine.md` §5.2).

use std::sync::Arc;

use xre_core::math::{Quat, Vec3};
use xre_core::Transform;
use xre_render::{Material, Mesh};

pub use hecs::{Entity, World};

use crate::anim::Animator;
use crate::time::Time;

/// A drawable mesh + material attached to an entity.
#[derive(Clone)]
pub struct MeshInstance {
    /// Shared geometry.
    pub mesh: Arc<Mesh>,
    /// Material.
    pub material: Material,
}

/// Linear velocity (units/second).
#[derive(Clone, Copy, Debug, Default)]
pub struct Velocity(pub Vec3);

/// Constant angular velocity about an axis (radians/second).
#[derive(Clone, Copy, Debug)]
pub struct Spin {
    /// Rotation axis (need not be unit).
    pub axis: Vec3,
    /// Angular speed, radians/second.
    pub speed: f32,
}

/// An AABB collider attached to an entity (half-extents).
#[derive(Clone, Copy, Debug)]
pub struct AabbCollider {
    /// Half-extents.
    pub half: Vec3,
}

/// A countdown after which the entity is despawned.
#[derive(Clone, Copy, Debug)]
pub struct Lifetime {
    /// Seconds remaining.
    pub remaining: f32,
}

/// A system: mutates the world given the frame [`Time`].
pub type System = Box<dyn FnMut(&mut World, &Time)>;

/// An ordered list of systems run each fixed update.
#[derive(Default)]
pub struct Schedule {
    systems: Vec<System>,
}

impl Schedule {
    /// An empty schedule.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a system.
    #[must_use]
    pub fn with(mut self, system: impl FnMut(&mut World, &Time) + 'static) -> Self {
        self.systems.push(Box::new(system));
        self
    }

    /// Run every system in order.
    pub fn run(&mut self, world: &mut World, time: &Time) {
        for system in &mut self.systems {
            system(world, time);
        }
    }

    /// A schedule with the built-in motion/lifetime/animation systems.
    #[must_use]
    pub fn standard() -> Self {
        Self::new()
            .with(integrate_velocity)
            .with(apply_spin)
            .with(update_animators)
            .with(tick_lifetimes)
    }
}

/// Move entities by their [`Velocity`].
pub fn integrate_velocity(world: &mut World, time: &Time) {
    for (_, (transform, vel)) in world.query_mut::<(&mut Transform, &Velocity)>() {
        transform.translation += vel.0 * time.dt;
    }
}

/// Rotate entities by their [`Spin`].
pub fn apply_spin(world: &mut World, time: &Time) {
    for (_, (transform, spin)) in world.query_mut::<(&mut Transform, &Spin)>() {
        let axis = spin.axis.normalize_or_zero();
        if axis.length_squared() > 0.0 {
            let delta = Quat::from_axis_angle(axis, spin.speed * time.dt);
            transform.rotation = (delta * transform.rotation).normalize();
        }
    }
}

/// Advance [`Animator`]s and write their sampled transform.
pub fn update_animators(world: &mut World, time: &Time) {
    for (_, (transform, animator)) in world.query_mut::<(&mut Transform, &mut Animator)>() {
        animator.update(time.dt);
        *transform = animator.sample(*transform);
    }
}

/// Decrement [`Lifetime`]s and despawn expired entities.
pub fn tick_lifetimes(world: &mut World, time: &Time) {
    let mut dead = Vec::new();
    for (entity, life) in world.query_mut::<&mut Lifetime>() {
        life.remaining -= time.dt;
        if life.remaining <= 0.0 {
            dead.push(entity);
        }
    }
    for entity in dead {
        let _ = world.despawn(entity);
    }
}

/// Extract drawable instances `(world_matrix, mesh, material)` from the ECS for
/// the renderer. Queries are hoisted here, once per frame, not per pixel.
#[must_use]
#[allow(clippy::explicit_iter_loop)]
pub fn draw_items(world: &World) -> Vec<(xre_core::math::Mat4, Arc<Mesh>, Material)> {
    let mut out = Vec::new();
    for (_, (transform, instance)) in world.query::<(&Transform, &MeshInstance)>().iter() {
        out.push((
            transform.to_mat4(),
            Arc::clone(&instance.mesh),
            instance.material,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn time() -> Time {
        Time::new(0.1) // 100 ms steps for easy arithmetic
    }

    #[test]
    fn velocity_moves_entities() {
        let mut world = World::new();
        let e = world.spawn((
            Transform::from_translation(Vec3::ZERO),
            Velocity(Vec3::new(1.0, 0.0, 0.0)),
        ));
        let mut sched = Schedule::new().with(integrate_velocity);
        sched.run(&mut world, &time());
        let t = world.get::<&Transform>(e).unwrap();
        assert!((t.translation.x - 0.1).abs() < 1e-5);
    }

    #[test]
    fn lifetimes_despawn() {
        let mut world = World::new();
        world.spawn((Lifetime { remaining: 0.05 },));
        world.spawn((Lifetime { remaining: 1.0 },));
        let mut sched = Schedule::new().with(tick_lifetimes);
        sched.run(&mut world, &time());
        assert_eq!(world.len(), 1); // the 0.05s entity expired
    }

    #[test]
    fn spin_rotates() {
        let mut world = World::new();
        let e = world.spawn((
            Transform::IDENTITY,
            Spin {
                axis: Vec3::Y,
                speed: 1.0,
            },
        ));
        let mut sched = Schedule::new().with(apply_spin);
        sched.run(&mut world, &time());
        let t = world.get::<&Transform>(e).unwrap();
        assert!(t.rotation.is_normalized());
        assert_ne!(t.rotation, Quat::IDENTITY);
    }

    #[test]
    fn ten_thousand_entities_step() {
        let mut world = World::new();
        let mesh = Arc::new(Mesh::cube());
        for i in 0..10_000 {
            world.spawn((
                Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0)),
                Velocity(Vec3::Y),
                MeshInstance {
                    mesh: Arc::clone(&mesh),
                    material: Material::default(),
                },
            ));
        }
        let mut sched = Schedule::standard();
        sched.run(&mut world, &time());
        assert_eq!(draw_items(&world).len(), 10_000);
    }
}
