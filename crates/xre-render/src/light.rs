//! Lights and the Lambert lighting model.
//!
//! Phase 2 ships Lambert diffuse + ambient with directional and point lights
//! (point lights attenuated per spec §A6), plus optional cel quantization and
//! depth fog. The lighting API shape converges with gemini-engine's `Light`
//! (acknowledged in [14](14-gemini-engine-analysis.md)) but adds attenuation and
//! color (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.4).

use xre_core::math::Vec3;

use crate::material::Material;

/// A single light source.
#[derive(Clone, Copy, Debug)]
pub enum Light {
    /// A directional light (sun): parallel rays from `-direction`.
    Directional {
        /// The direction the light travels (world space, will be normalized).
        direction: Vec3,
        /// Light color, `0.0..=1.0`.
        color: Vec3,
        /// Intensity multiplier.
        intensity: f32,
    },
    /// A point light with quadratic attenuation `1/(c + l·d + q·d²)`.
    Point {
        /// World position.
        position: Vec3,
        /// Light color, `0.0..=1.0`.
        color: Vec3,
        /// Intensity multiplier.
        intensity: f32,
        /// Attenuation coefficients `(constant, linear, quadratic)`.
        attenuation: (f32, f32, f32),
    },
}

impl Light {
    /// A white directional light from the given direction.
    #[must_use]
    pub const fn directional(direction: Vec3) -> Self {
        Self::Directional {
            direction,
            color: Vec3::ONE,
            intensity: 1.0,
        }
    }

    /// A white point light at `position` with sensible attenuation.
    #[must_use]
    pub const fn point(position: Vec3) -> Self {
        Self::Point {
            position,
            color: Vec3::ONE,
            intensity: 1.0,
            attenuation: (1.0, 0.09, 0.032),
        }
    }

    /// The diffuse contribution (color × NdotL × attenuation × intensity) this
    /// light adds at `world_pos` with surface `normal` (assumed unit length).
    #[must_use]
    pub fn contribution(&self, world_pos: Vec3, normal: Vec3) -> Vec3 {
        match *self {
            Self::Directional {
                direction,
                color,
                intensity,
            } => {
                let l = (-direction).normalize_or_zero();
                let ndotl = normal.dot(l).max(0.0);
                color * (ndotl * intensity)
            }
            Self::Point {
                position,
                color,
                intensity,
                attenuation,
            } => {
                let to_light = position - world_pos;
                let dist = to_light.length();
                let l = to_light.normalize_or_zero();
                let ndotl = normal.dot(l).max(0.0);
                let (c, lin, q) = attenuation;
                let atten = 1.0 / ((c + lin * dist + q * dist * dist).max(f32::EPSILON));
                color * (ndotl * intensity * atten)
            }
        }
    }
}

/// A collection of lights plus an ambient term and optional depth fog.
#[derive(Clone, Debug)]
pub struct LightRig {
    /// The lights.
    pub lights: Vec<Light>,
    /// Ambient color added unconditionally.
    pub ambient: Vec3,
    /// Optional depth fog: `(color, near, far)`. Samples fade toward `color`
    /// between `near` and `far` view depth.
    pub fog: Option<(Vec3, f32, f32)>,
}

impl Default for LightRig {
    fn default() -> Self {
        Self {
            lights: vec![Light::directional(Vec3::new(-0.4, -1.0, -0.6))],
            ambient: Vec3::splat(0.12),
            fog: None,
        }
    }
}

impl LightRig {
    /// An empty rig with the given ambient term.
    #[must_use]
    pub const fn ambient_only(ambient: Vec3) -> Self {
        Self {
            lights: Vec::new(),
            ambient,
            fog: None,
        }
    }

    /// Builder: add a light.
    #[must_use]
    pub fn with_light(mut self, light: Light) -> Self {
        self.lights.push(light);
        self
    }

    /// Shade a surface point: combine ambient + every light's diffuse term,
    /// modulate by the material, apply cel quantization and emission, and return
    /// the resulting linear color clamped to `0.0..=1.0`.
    #[must_use]
    pub fn shade(&self, material: &Material, world_pos: Vec3, normal: Vec3) -> Vec3 {
        let n = normal.normalize_or_zero();
        let mut diffuse = self.ambient;
        for light in &self.lights {
            diffuse += light.contribution(world_pos, n);
        }
        let mut lit = material.base_color * diffuse * material.kd;
        if let Some(levels) = material.cel_levels {
            lit = quantize(lit, levels);
        }
        lit += material.emissive;
        lit.clamp(Vec3::ZERO, Vec3::ONE)
    }

    /// Apply depth fog to a color at view-space `depth` (if fog is configured).
    #[must_use]
    pub fn apply_fog(&self, color: Vec3, depth: f32) -> Vec3 {
        if let Some((fog_color, near, far)) = self.fog {
            let t = ((depth - near) / (far - near).max(f32::EPSILON)).clamp(0.0, 1.0);
            color.lerp(fog_color, t)
        } else {
            color
        }
    }
}

/// Quantize each component to `levels` bands (cel shading).
fn quantize(color: Vec3, levels: u32) -> Vec3 {
    let n = levels.max(1) as f32;
    Vec3::new(
        (color.x * n).floor() / n,
        (color.y * n).floor() / n,
        (color.z * n).floor() / n,
    )
}

/// Convert a linear color to perceptual luminance (Rec. 709 weights).
#[must_use]
pub fn luminance(color: Vec3) -> f32 {
    0.2126 * color.x + 0.7152 * color.y + 0.0722 * color.z
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::float_cmp,
        clippy::field_reassign_with_default
    )]
    use super::*;

    #[test]
    fn facing_directional_light_is_bright() {
        let light = Light::directional(Vec3::NEG_Y);
        // Surface facing up (+Y) toward a light coming from above.
        let c = light.contribution(Vec3::ZERO, Vec3::Y);
        assert!(c.x > 0.9, "expected near-full lighting, got {c:?}");
    }

    #[test]
    fn backface_gets_no_direct_light() {
        let light = Light::directional(Vec3::NEG_Y);
        let c = light.contribution(Vec3::ZERO, Vec3::NEG_Y);
        assert_eq!(c, Vec3::ZERO);
    }

    #[test]
    fn point_light_attenuates_with_distance() {
        let near = Light::point(Vec3::new(0.0, 1.0, 0.0));
        let far = Light::point(Vec3::new(0.0, 10.0, 0.0));
        let cn = near.contribution(Vec3::ZERO, Vec3::Y);
        let cf = far.contribution(Vec3::ZERO, Vec3::Y);
        assert!(cn.x > cf.x);
    }

    #[test]
    fn ambient_lifts_unlit_surfaces() {
        let rig = LightRig::ambient_only(Vec3::splat(0.3));
        let lit = rig.shade(&Material::default(), Vec3::ZERO, Vec3::NEG_Y);
        assert!(lit.x > 0.0);
    }

    #[test]
    fn cel_quantizes_to_bands() {
        let q = quantize(Vec3::splat(0.7), 2);
        assert_eq!(q, Vec3::splat(0.5));
    }

    #[test]
    fn fog_blends_toward_color_at_far() {
        let mut rig = LightRig::default();
        rig.fog = Some((Vec3::ONE, 0.0, 10.0));
        let near = rig.apply_fog(Vec3::ZERO, 0.0);
        let far = rig.apply_fog(Vec3::ZERO, 10.0);
        assert_eq!(near, Vec3::ZERO);
        assert_eq!(far, Vec3::ONE);
    }
}
