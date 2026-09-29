use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};

use crate::TextureRef;

/// The stylized-FX half of a particle material: everything that makes a
/// scrolled, eroded, rim-lit sheet read as volumetric rather than as a sprite.
///
/// Every FEATURE defaults to inert. That is load-bearing: this struct is added
/// to an existing serialized type, so any default that changed a pixel would
/// silently restyle every effect already authored.
///
/// Note the level that promise is pitched at. Each feature is gated by one
/// field -- `fresnel_power`, `erosion_threshold`, `soft_fade`, and so on -- and
/// it is THAT field that defaults to off. A field inside an off feature is free
/// to default to the value that makes the feature work once it is switched on,
/// and `fresnel_boost` does exactly that: defaulting it to zero made Fresnel
/// Power alone compile the shader block, pay its fill rate, and contribute
/// mathematically nothing, while the tooltip sent the author off to blame their
/// geometry.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct FxSettings {
    /// UV scroll rate, in UV units per second.
    pub scroll: Vec2,
    /// UV tiling multiplier.
    pub tiling: Vec2,
    /// A texture whose RG channels offset the base UV -- the difference
    /// between a texture that CHURNS and one that merely slides. Scroll alone
    /// is the clearest tell of cheap VFX.
    pub flow_texture: Option<TextureRef>,
    /// How strongly the flow texture's offset perturbs the base UV. `0`
    /// disables flow regardless of whether a texture is set.
    pub flow_strength: f32,
    /// UV scroll rate applied to the flow texture's own sample, in UV units
    /// per second -- independent of `scroll`, so the flow pattern can churn
    /// at a different rate than the base texture slides.
    pub flow_scroll: Vec2,
    /// Dissolve noise. Fragments below `erosion_threshold` are discarded, with
    /// `erosion_edge` of emissive rim before the cut.
    pub erosion_texture: Option<TextureRef>,
    /// Noise value below which a fragment is discarded. `0` disables erosion
    /// regardless of whether a texture is set.
    ///
    /// **A zero baseline cannot be driven up.** The shader multiplies this by
    /// the resolved drive slot (`fx.erosion_soft.x *
    /// drive_slots[DRIVE_SLOT_EROSION]`), and no
    /// [`DriveOp`](super::DriveOp) changes that — the authored value is
    /// applied downstream of the fold, never inside it. So an effect that
    /// wants `EmitterProp::ErosionThreshold` to dissolve it on command must
    /// author a non-zero threshold here and let the drive scale that, rather
    /// than leaving this at its default and expecting the drive to supply the
    /// whole value. The same holds for every drivable property whose authored
    /// default is `0.0`.
    pub erosion_threshold: f32,
    /// Width, in noise units, of the emissive rim painted just above the
    /// erosion cut.
    pub erosion_edge: f32,
    /// Color of the erosion rim.
    pub erosion_edge_color: [f32; 4],
    /// Rim brightening exponent. `0` disables the whole fresnel block.
    pub fresnel_power: f32,
    /// Rim brightening intensity multiplier, applied to the rim term the
    /// exponent shapes (`base.rgb * fresnel_rim * fresnel_boost` in
    /// `particle_material.wgsl`). Defaults to `1.0` -- neutral, not off --
    /// because zero here silently cancels a fresnel the author just enabled.
    pub fresnel_boost: f32,
    /// Depth-fade distance in world units. `0` disables. Removes the hard
    /// intersection line where a quad clips the floor.
    ///
    /// Requires the consuming app's camera to carry bevy's
    /// [`DepthPrepass`](bevy::core_pipeline::prepass::DepthPrepass)
    /// component (`bevy_sprinkles_editor`'s viewport camera does). Without
    /// it, an effect with `soft_fade > 0.0` fails to compile its fragment
    /// shader -- the depth texture this feature reads only exists on a view
    /// that opted into a depth prepass.
    pub soft_fade: f32,
    /// Sample the base texture's red channel as a mask and colour it through
    /// this gradient, instead of using the texture's own colour.
    pub gradient_remap: Option<super::Gradient>,
}

impl Default for FxSettings {
    fn default() -> Self {
        Self {
            scroll: Vec2::ZERO,
            tiling: Vec2::ONE,
            flow_texture: None,
            flow_strength: 0.0,
            flow_scroll: Vec2::ZERO,
            erosion_texture: None,
            erosion_threshold: 0.0,
            erosion_edge: 0.0,
            erosion_edge_color: [1.0, 0.5, 0.1, 1.0],
            fresnel_power: 0.0,
            fresnel_boost: 1.0,
            soft_fade: 0.0,
            gradient_remap: None,
        }
    }
}

impl FxSettings {
    /// Whether UV scroll/tiling differs from the inert default.
    pub fn scroll_enabled(&self) -> bool {
        self.scroll != Vec2::ZERO || self.tiling != Vec2::ONE
    }
    /// Whether the flow-texture UV offset is active.
    pub fn flow_enabled(&self) -> bool {
        self.flow_texture.is_some() && self.flow_strength != 0.0
    }
    /// Whether erosion discard/rim is active.
    pub fn erosion_enabled(&self) -> bool {
        self.erosion_texture.is_some() && (self.erosion_threshold > 0.0 || self.erosion_edge > 0.0)
    }
    /// Whether fresnel rim brightening is active.
    pub fn fresnel_enabled(&self) -> bool {
        self.fresnel_power > 0.0
    }
    /// Whether depth soft-fade is active.
    pub fn soft_enabled(&self) -> bool {
        self.soft_fade > 0.0
    }
    /// Whether the gradient remap is active.
    pub fn gradient_enabled(&self) -> bool {
        self.gradient_remap.is_some()
    }

    /// True when any feature is on. Used only by tests and the editor summary;
    /// the shader defs are decided per feature.
    pub fn enabled(&self) -> bool {
        self.scroll_enabled()
            || self.flow_enabled()
            || self.erosion_enabled()
            || self.fresnel_enabled()
            || self.soft_enabled()
            || self.gradient_enabled()
    }

    /// Computes a hash key for material caching, in the style of
    /// [`super::StandardParticleMaterial::cache_key`] -- folded into that
    /// method so an FX-only edit (e.g. dragging the scroll rate in the
    /// editor) is recognised as a material change and rebuilds the GPU
    /// material, rather than silently leaving the old `FxUniform` bound.
    pub fn cache_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let hash_f32 = |h: &mut std::collections::hash_map::DefaultHasher, v: f32| {
            v.to_bits().hash(h);
        };
        let hash_vec2 = |h: &mut std::collections::hash_map::DefaultHasher, v: Vec2| {
            v.x.to_bits().hash(h);
            v.y.to_bits().hash(h);
        };

        hash_vec2(&mut hasher, self.scroll);
        hash_vec2(&mut hasher, self.tiling);
        self.flow_texture.hash(&mut hasher);
        hash_f32(&mut hasher, self.flow_strength);
        hash_vec2(&mut hasher, self.flow_scroll);
        self.erosion_texture.hash(&mut hasher);
        hash_f32(&mut hasher, self.erosion_threshold);
        hash_f32(&mut hasher, self.erosion_edge);
        for v in self.erosion_edge_color {
            hash_f32(&mut hasher, v);
        }
        hash_f32(&mut hasher, self.fresnel_power);
        hash_f32(&mut hasher, self.fresnel_boost);
        hash_f32(&mut hasher, self.soft_fade);
        match &self.gradient_remap {
            Some(g) => {
                1u8.hash(&mut hasher);
                g.cache_key().hash(&mut hasher);
            }
            None => 0u8.hash(&mut hasher),
        }
        hasher.finish()
    }
}

fn finite(v: f32, fallback: f32) -> f32 {
    if v.is_finite() { v } else { fallback }
}
fn finite2(v: Vec2, fallback: Vec2) -> Vec2 {
    Vec2::new(finite(v.x, fallback.x), finite(v.y, fallback.y))
}

/// GPU-side FX parameters. Packed in vec4s so the WGSL struct needs no padding
/// fields that could drift out of lockstep.
#[derive(Clone, Copy, Default, ShaderType, Reflect, Debug)]
pub struct FxUniform {
    /// xy = scroll rate, zw = tiling.
    pub scroll_tiling: Vec4,
    /// x = flow strength, yz = flow scroll, w = fresnel power.
    pub flow_fresnel: Vec4,
    /// x = erosion threshold, y = erosion edge, z = fresnel boost, w = soft fade.
    pub erosion_soft: Vec4,
    /// Erosion rim color.
    pub erosion_edge_color: Vec4,
}

impl From<&FxSettings> for FxUniform {
    /// Clamps every authored value to something finite. Authored `.ron` is
    /// untrusted input, and a NaN reaching the fragment shader can blank the
    /// draw -- which an author reads as "my effect vanished".
    fn from(f: &FxSettings) -> Self {
        let scroll = finite2(f.scroll, Vec2::ZERO);
        let tiling = finite2(f.tiling, Vec2::ONE);
        let flow_scroll = finite2(f.flow_scroll, Vec2::ZERO);
        Self {
            scroll_tiling: Vec4::new(scroll.x, scroll.y, tiling.x, tiling.y),
            flow_fresnel: Vec4::new(
                finite(f.flow_strength, 0.0),
                flow_scroll.x,
                flow_scroll.y,
                finite(f.fresnel_power, 0.0),
            ),
            erosion_soft: Vec4::new(
                finite(f.erosion_threshold, 0.0).clamp(0.0, 1.0),
                finite(f.erosion_edge, 0.0).clamp(0.0, 1.0),
                finite(f.fresnel_boost, 0.0),
                finite(f.soft_fade, 0.0).max(0.0),
            ),
            erosion_edge_color: {
                // The alpha lane is a `mix` WEIGHT, not a colour channel: the
                // shader blends the rim in with `erosion_rim * edge_color.a`.
                // An authored 4.0 there overshoots the mix past the rim colour
                // into extrapolation, and a negative one past the base colour
                // the other way -- both of which read as "the rim broke".
                let c = f.erosion_edge_color.map(|v| finite(v, 1.0));
                Vec4::new(c[0], c[1], c[2], c[3].clamp(0.0, 1.0))
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fx_is_entirely_off() {
        // A default must cost nothing and change nothing, or every existing
        // effect silently changes appearance when this field appears.
        let fx = FxSettings::default();
        assert_eq!(fx.scroll, Vec2::ZERO);
        assert_eq!(fx.tiling, Vec2::ONE);
        assert!(!fx.enabled());
    }

    #[test]
    fn any_nonzero_scroll_enables_the_feature() {
        let fx = FxSettings {
            scroll: Vec2::new(0.0, 0.2),
            ..Default::default()
        };
        assert!(fx.enabled());
    }

    #[test]
    fn a_non_finite_authored_value_is_clamped_away_on_conversion() {
        let fx = FxSettings {
            scroll: Vec2::new(f32::NAN, 1.0),
            ..Default::default()
        };
        let u = FxUniform::from(&fx);
        assert!(u.scroll_tiling.x.is_finite(), "a NaN must never reach the GPU");
    }

    #[test]
    fn a_non_finite_erosion_edge_color_is_clamped_away_on_conversion() {
        let fx = FxSettings {
            erosion_edge_color: [f32::NAN, f32::INFINITY, -f32::INFINITY, 1.0],
            ..Default::default()
        };
        let u = FxUniform::from(&fx);
        assert!(
            u.erosion_edge_color.to_array().iter().all(|v| v.is_finite()),
            "a NaN or infinity must never reach the GPU"
        );
    }

    #[test]
    fn two_settings_that_differ_only_by_scroll_hash_differently() {
        let a = FxSettings::default();
        let b = FxSettings {
            scroll: Vec2::new(0.0, 0.5),
            ..Default::default()
        };
        assert_ne!(a.cache_key(), b.cache_key());
    }

    #[test]
    fn identical_settings_hash_identically() {
        let a = FxSettings {
            scroll: Vec2::new(0.3, -0.1),
            ..Default::default()
        };
        let b = a.clone();
        assert_eq!(a.cache_key(), b.cache_key());
    }

    /// I8: `fresnel_power` alone used to switch on a shader block that
    /// multiplied its rim term by a zero boost -- compiled, paid fill rate,
    /// contributed exactly nothing. Asserts the uniform lane the shader reads
    /// (`erosion_soft.z`), not pixels.
    #[test]
    fn fresnel_power_alone_actually_contributes() {
        let fx = FxSettings {
            fresnel_power: 2.0,
            ..Default::default()
        };
        assert!(fx.fresnel_enabled(), "power alone must switch the feature on");
        let u = FxUniform::from(&fx);
        assert_ne!(
            u.erosion_soft.z, 0.0,
            "the boost the shader multiplies the rim by must not be zero when \
             the author has only set the power"
        );
    }

    #[test]
    fn a_default_fresnel_boost_is_neutral_rather_than_off() {
        assert_eq!(FxSettings::default().fresnel_boost, 1.0);
    }

    /// Minor (deferred, now fixed): `erosion_edge_color.a` is a `mix` WEIGHT
    /// (`erosion_rim * edge_color.a`), so out-of-range values extrapolate the
    /// blend past both endpoints instead of interpolating between them. The
    /// rgb lanes are colour and stay unclamped -- HDR rims are the point.
    #[test]
    fn an_out_of_range_erosion_edge_alpha_is_clamped_to_a_valid_mix_weight() {
        let hot = FxUniform::from(&FxSettings {
            erosion_edge_color: [8.0, 0.0, 0.0, 4.0],
            ..Default::default()
        });
        assert_eq!(hot.erosion_edge_color.w, 1.0);
        assert_eq!(hot.erosion_edge_color.x, 8.0, "rgb stays HDR");

        let cold = FxUniform::from(&FxSettings {
            erosion_edge_color: [1.0, 1.0, 1.0, -2.0],
            ..Default::default()
        });
        assert_eq!(cold.erosion_edge_color.w, 0.0);
    }
}
