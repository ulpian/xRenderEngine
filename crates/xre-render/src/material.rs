//! [`Material`]: how a surface responds to light.
//!
//! Phase 2 uses `base_color`, `kd` (diffuse strength), `emissive`, and the
//! optional cel-quantization level count. `ks` is reserved for a later specular
//! term, and `texture` is a handle resolved by the renderer once textures land
//! in Phase 3 (`RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.4).

use xre_core::math::Vec3;

/// An opaque handle to a texture owned by the renderer/asset store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TexHandle(pub u32);

/// Per-surface shading parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Base (albedo) color, linear `0.0..=1.0`.
    pub base_color: Vec3,
    /// Diffuse strength.
    pub kd: f32,
    /// Specular strength (reserved; unused in Phase 2).
    pub ks: f32,
    /// Self-illumination added after lighting.
    pub emissive: Vec3,
    /// If `Some(n)`, quantize the lit luminance into `n` cel bands.
    pub cel_levels: Option<u32>,
    /// Optional diffuse texture.
    pub texture: Option<TexHandle>,
    /// Skip lighting entirely: the surface renders at full brightness
    /// (`texel * base_color`, or `base_color` when untextured). Used by the image
    /// viewer so a picture's colors stay faithful regardless of light position.
    pub unlit: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: Vec3::splat(0.8),
            kd: 1.0,
            ks: 0.0,
            emissive: Vec3::ZERO,
            cel_levels: None,
            texture: None,
            unlit: false,
        }
    }
}

impl Material {
    /// A flat-colored material.
    #[must_use]
    pub fn colored(rgb: Vec3) -> Self {
        Self {
            base_color: rgb,
            ..Self::default()
        }
    }

    /// Builder: set cel-shading band count.
    #[must_use]
    pub const fn cel(mut self, levels: u32) -> Self {
        self.cel_levels = Some(levels);
        self
    }

    /// Builder: set the emissive color.
    #[must_use]
    pub const fn emissive(mut self, emissive: Vec3) -> Self {
        self.emissive = emissive;
        self
    }

    /// Builder: render at full brightness, bypassing lighting.
    #[must_use]
    pub const fn unlit(mut self) -> Self {
        self.unlit = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_light_gray_opaque() {
        let m = Material::default();
        assert_eq!(m.base_color, Vec3::splat(0.8));
        assert!(m.cel_levels.is_none());
    }

    #[test]
    fn builders_compose() {
        let m = Material::colored(Vec3::X).cel(4).emissive(Vec3::splat(0.1));
        assert_eq!(m.base_color, Vec3::X);
        assert_eq!(m.cel_levels, Some(4));
        assert_eq!(m.emissive, Vec3::splat(0.1));
    }

    #[test]
    fn unlit_builder_sets_flag() {
        assert!(!Material::default().unlit);
        assert!(Material::colored(Vec3::ONE).unlit().unlit);
    }
}
