use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};

use super::{Drive, DriveTarget, EmitterProp};
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
    /// **A zero baseline here means "the drive supplies the whole value".**
    /// The shader multiplies this by the resolved drive slot
    /// (`fx.erosion_soft.x * drive_slots[DRIVE_SLOT_EROSION]`) and no
    /// [`DriveOp`](super::DriveOp) changes that — the authored value is
    /// applied downstream of the fold, never inside it — so a literal zero
    /// would annihilate the wire. [`FxUniform::from_settings`] therefore
    /// hands the GPU a `1.0` baseline when a drive targets
    /// [`EmitterProp::ErosionThreshold`](super::EmitterProp::ErosionThreshold)
    /// on this emitter and this field is exactly `0.0`, and
    /// [`FxSettings::erosion_enabled_with_drives`] compiles the block in.
    /// Authoring a NON-zero threshold still means what it always did: the
    /// drive scales it. Same rule for the other four drivable FX scalars
    /// (`scroll.x`, `scroll.y`, `flow_strength`, `fresnel_power`).
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

/// Which of the five drivable FX scalars a [`Drive`] targets on ONE emitter.
///
/// Those five are the only [`EmitterProp`]s whose baseline lives in
/// [`FxSettings`] rather than in the simulation uniform, and they are the
/// reason this type exists: whether a feature's shader block is compiled, and
/// whether its authored baseline is substituted, cannot be answered from
/// `FxSettings` alone -- that struct has never known anything about drives.
///
/// **A muted drive still counts.** `muted` is the editor's A/B toggle and is
/// flipped live, but the shader defs this answer feeds are baked into the
/// material when it is built, and nothing rebuilds a material when a drive
/// changes -- so a mute-sensitive answer would go stale exactly when it
/// mattered, giving a toggle that half-works rather than one that works.
/// Muting keeps its documented meaning downstream instead, where it is
/// honoured every frame: `drives::sample` returns `None`, the slot folds back
/// to identity, and a substituted baseline of `1.0` then passes that identity
/// through unchanged.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct DrivenFx {
    /// Some drive targets [`EmitterProp::ScrollU`] -- the `x` lane of
    /// [`FxSettings::scroll`].
    pub scroll_u: bool,
    /// Some drive targets [`EmitterProp::ScrollV`] -- the `y` lane of
    /// [`FxSettings::scroll`].
    pub scroll_v: bool,
    /// Some drive targets [`EmitterProp::FlowStrength`].
    pub flow_strength: bool,
    /// Some drive targets [`EmitterProp::ErosionThreshold`].
    pub erosion_threshold: bool,
    /// Some drive targets [`EmitterProp::FresnelPower`].
    pub fresnel_power: bool,
}

impl DrivenFx {
    /// Nothing driven -- the answer for a caller with no drive list to hand,
    /// and the one [`FxUniform`]'s `From<&FxSettings>` assumes.
    pub const NONE: Self = Self {
        scroll_u: false,
        scroll_v: false,
        flow_strength: false,
        erosion_threshold: false,
        fresnel_power: false,
    };

    /// Scans a whole effect's drive list for the wires aimed at one emitter.
    ///
    /// Drives are per-emitter ([`DriveTarget::Emitter`] carries the index), so
    /// a fresnel drive on emitter 0 must not compile the fresnel block into
    /// emitter 1's shader: that emitter would pay the block's fill rate to
    /// multiply by an identity slot forever.
    pub fn for_emitter(drives: &[Drive], emitter_index: usize) -> Self {
        let mut out = Self::NONE;
        for drive in drives {
            let DriveTarget::Emitter { index, prop } = &drive.target else {
                continue;
            };
            if *index as usize != emitter_index {
                continue;
            }
            match prop {
                EmitterProp::ScrollU => out.scroll_u = true,
                EmitterProp::ScrollV => out.scroll_v = true,
                EmitterProp::FlowStrength => out.flow_strength = true,
                EmitterProp::ErosionThreshold => out.erosion_threshold = true,
                EmitterProp::FresnelPower => out.fresnel_power = true,
                _ => {}
            }
        }
        out
    }
}

/// The baseline an FX scalar hands the GPU, given whether a drive targets it.
///
/// The shader applies a drive by MULTIPLYING the slot into the authored value
/// (`fx.flow_fresnel.w * drive_slots[DRIVE_SLOT_FRESNEL]` and its three
/// siblings), so an authored `0.0` annihilates whatever the drive resolved to
/// and the wire is inert no matter which [`DriveOp`](super::DriveOp) it uses.
/// Substituting `1.0` for exactly that case turns the multiply into a
/// pass-through: `0.0` comes to mean "no baseline, the drive supplies the
/// whole value" instead of "permanently off".
///
/// A NON-ZERO authored value is left alone deliberately, so a drive on a
/// `fresnel_power: 2.0` still scales that 2.0 rather than silently discarding
/// an authored figure the author can see in the inspector.
fn driven_baseline(v: f32, driven: bool) -> f32 {
    if driven && v == 0.0 { 1.0 } else { v }
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

    // --- Drive-aware gating -------------------------------------------
    //
    // The `*_enabled` predicates above answer "did the AUTHOR switch this
    // on", and that is what several callers want (the editor's FX summary
    // and the per-feature reset buttons in `inspector::material_fx`, which
    // clear exactly the fields those predicates read). The shader defs want
    // a different question -- "can this feature ever do anything at runtime"
    // -- and a drive is the other way one of these scalars becomes non-zero.
    // Asking the authored-only question there is half of why a drive on a
    // zero baseline was inert twice over: the block was never compiled in,
    // and the uniform carried a zero. `driven_baseline` is the other half.
    //
    // A TEXTURE requirement is NOT relaxed. `flow` and `erosion` sample a
    // texture binding; with nothing bound there is no pattern for a drive to
    // scale, so a drive satisfies only the SCALAR half of those two gates.

    /// [`scroll_enabled`](Self::scroll_enabled), or some drive scrolls a lane.
    pub fn scroll_enabled_with_drives(&self, driven: DrivenFx) -> bool {
        self.scroll_enabled() || driven.scroll_u || driven.scroll_v
    }
    /// [`flow_enabled`](Self::flow_enabled), or a drive supplies the strength
    /// -- a flow texture is still mandatory.
    pub fn flow_enabled_with_drives(&self, driven: DrivenFx) -> bool {
        self.flow_texture.is_some() && (self.flow_strength != 0.0 || driven.flow_strength)
    }
    /// [`erosion_enabled`](Self::erosion_enabled), or a drive supplies the
    /// threshold -- an erosion texture is still mandatory.
    pub fn erosion_enabled_with_drives(&self, driven: DrivenFx) -> bool {
        self.erosion_texture.is_some()
            && (self.erosion_threshold > 0.0 || self.erosion_edge > 0.0 || driven.erosion_threshold)
    }
    /// [`fresnel_enabled`](Self::fresnel_enabled), or a drive supplies the power.
    pub fn fresnel_enabled_with_drives(&self, driven: DrivenFx) -> bool {
        self.fresnel_power > 0.0 || driven.fresnel_power
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

impl FxUniform {
    /// Builds the GPU uniform, substituting a `1.0` baseline for any of the
    /// five drivable scalars that a drive targets and the author left at
    /// zero. See [`driven_baseline`] for why, and [`DrivenFx`] for what
    /// counts as driven.
    ///
    /// The substitution belongs HERE rather than in the shader: WGSL in this
    /// crate is validated only at pipeline specialization, which no test and
    /// no headless run reaches, so the same rule expressed in
    /// `particle_material.wgsl` would be unverifiable. Expressed as this
    /// function it is ordinary Rust with ordinary tests.
    pub fn from_settings(f: &FxSettings, driven: DrivenFx) -> Self {
        let mut scroll = finite2(f.scroll, Vec2::ZERO);
        // Per COMPONENT, not per vector: `ScrollU` is `.x` and `ScrollV` is
        // `.y`, so a drive on one lane must leave the other lane's authored
        // value (very often a deliberate zero) exactly as authored.
        scroll.x = driven_baseline(scroll.x, driven.scroll_u);
        scroll.y = driven_baseline(scroll.y, driven.scroll_v);
        let tiling = finite2(f.tiling, Vec2::ONE);
        let flow_scroll = finite2(f.flow_scroll, Vec2::ZERO);
        Self {
            scroll_tiling: Vec4::new(scroll.x, scroll.y, tiling.x, tiling.y),
            flow_fresnel: Vec4::new(
                driven_baseline(finite(f.flow_strength, 0.0), driven.flow_strength),
                flow_scroll.x,
                flow_scroll.y,
                driven_baseline(finite(f.fresnel_power, 0.0), driven.fresnel_power),
            ),
            erosion_soft: Vec4::new(
                driven_baseline(finite(f.erosion_threshold, 0.0), driven.erosion_threshold)
                    .clamp(0.0, 1.0),
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

impl From<&FxSettings> for FxUniform {
    /// The undriven conversion: every authored value clamped to something
    /// finite and nothing substituted. Authored `.ron` is untrusted input,
    /// and a NaN reaching the fragment shader can blank the draw -- which an
    /// author reads as "my effect vanished".
    ///
    /// Kept as the `From` impl because it is the answer for a caller that has
    /// no drive list; the material builder has one and calls
    /// [`FxUniform::from_settings`] instead.
    fn from(f: &FxSettings) -> Self {
        Self::from_settings(f, DrivenFx::NONE)
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

    // --- A drive on a zero authored baseline -------------------------
    //
    // Two independent mechanisms used to kill such a drive, and these cover
    // the second: the shader multiplies the slot into the authored value, so
    // a literal `0.0` annihilated it. (`spawning::build_fx_defs`'s tests
    // cover the first, which is that the block was never compiled at all.)

    use crate::asset::{CurveTexture, DriveOp, Range, VariableId};

    fn drive_targeting(prop: EmitterProp, emitter: u8) -> Drive {
        Drive {
            variable: VariableId(0),
            target: DriveTarget::Emitter {
                index: emitter,
                prop,
            },
            curve: CurveTexture::default(),
            output: Range { min: 0.0, max: 1.0 },
            op: DriveOp::Multiply,
            muted: false,
        }
    }

    #[test]
    fn a_driven_zero_baseline_reaches_the_gpu_as_one() {
        let fx = FxSettings::default(); // every one of the five is 0.0
        let driven = DrivenFx {
            scroll_u: true,
            scroll_v: true,
            flow_strength: true,
            erosion_threshold: true,
            fresnel_power: true,
        };
        let u = FxUniform::from_settings(&fx, driven);
        assert_eq!(u.scroll_tiling.x, 1.0, "scroll U");
        assert_eq!(u.scroll_tiling.y, 1.0, "scroll V");
        assert_eq!(u.flow_fresnel.x, 1.0, "flow strength");
        assert_eq!(u.flow_fresnel.w, 1.0, "fresnel power");
        assert_eq!(u.erosion_soft.x, 1.0, "erosion threshold");
    }

    #[test]
    fn an_authored_non_zero_baseline_is_not_replaced_by_a_drive() {
        // The other half of the rule, and the one a "just make the drive
        // authoritative" fix would break: an author who wrote `2.0` can see
        // that 2.0 in the inspector, so the drive must scale it rather than
        // silently discard it.
        let fx = FxSettings {
            scroll: Vec2::new(0.25, 0.5),
            flow_strength: 3.0,
            erosion_threshold: 0.4,
            fresnel_power: 2.0,
            ..Default::default()
        };
        let driven = DrivenFx {
            scroll_u: true,
            scroll_v: true,
            flow_strength: true,
            erosion_threshold: true,
            fresnel_power: true,
        };
        let u = FxUniform::from_settings(&fx, driven);
        assert_eq!(u.scroll_tiling.x, 0.25);
        assert_eq!(u.scroll_tiling.y, 0.5);
        assert_eq!(u.flow_fresnel.x, 3.0);
        assert_eq!(u.erosion_soft.x, 0.4);
        assert_eq!(u.flow_fresnel.w, 2.0);
    }

    #[test]
    fn a_scroll_u_drive_substitutes_only_the_u_lane() {
        // `scroll` is one `Vec2` but two independent props, so substituting
        // the whole vector would start the V lane scrolling on its own.
        let u = FxUniform::from_settings(
            &FxSettings::default(),
            DrivenFx {
                scroll_u: true,
                ..DrivenFx::NONE
            },
        );
        assert_eq!(u.scroll_tiling.x, 1.0);
        assert_eq!(u.scroll_tiling.y, 0.0, "the undriven lane stays authored");
    }

    #[test]
    fn an_undriven_zero_baseline_still_reaches_the_gpu_as_zero() {
        // Zero means "off" for an effect with no drive at all, and must keep
        // meaning that -- otherwise every stock effect gains a fresnel rim.
        let u = FxUniform::from(&FxSettings::default());
        assert_eq!(u.flow_fresnel.w, 0.0);
        assert_eq!(u.erosion_soft.x, 0.0);
        assert_eq!(u.scroll_tiling.x, 0.0);
    }

    #[test]
    fn the_authored_only_predicates_are_unmoved_by_a_drive() {
        // Their callers (the editor's FX summary, `material_fx`'s per-feature
        // reset buttons) ask "did the author switch this on", and the answer
        // must not start including drives.
        let fx = FxSettings::default();
        let all = DrivenFx {
            scroll_u: true,
            scroll_v: true,
            flow_strength: true,
            erosion_threshold: true,
            fresnel_power: true,
        };
        assert!(!fx.scroll_enabled());
        assert!(!fx.flow_enabled());
        assert!(!fx.erosion_enabled());
        assert!(!fx.fresnel_enabled());
        assert!(!fx.enabled());
        assert!(
            fx.fresnel_enabled_with_drives(all),
            "and the drive-aware sibling must differ, or this test is vacuous"
        );
    }

    #[test]
    fn a_drive_on_another_emitter_leaves_this_one_undriven() {
        // Drives carry an emitter index; compiling emitter 1's fresnel block
        // into emitter 0 makes it pay that block's fill rate forever to
        // multiply by an identity slot.
        let drives = vec![drive_targeting(EmitterProp::FresnelPower, 1)];
        assert!(!DrivenFx::for_emitter(&drives, 0).fresnel_power);
        assert!(DrivenFx::for_emitter(&drives, 1).fresnel_power);
    }

    #[test]
    fn for_emitter_reads_each_of_the_five_props_and_ignores_the_rest() {
        let drives: Vec<Drive> = [
            EmitterProp::ScrollU,
            EmitterProp::ScrollV,
            EmitterProp::FlowStrength,
            EmitterProp::ErosionThreshold,
            EmitterProp::FresnelPower,
        ]
        .into_iter()
        .map(|p| drive_targeting(p, 0))
        .collect();
        assert_eq!(
            DrivenFx::for_emitter(&drives, 0),
            DrivenFx {
                scroll_u: true,
                scroll_v: true,
                flow_strength: true,
                erosion_threshold: true,
                fresnel_power: true,
            }
        );

        // A prop that is not an FX scalar must not set any flag: `Lifetime`
        // is applied in `spawning.rs`, nowhere near `FxUniform`.
        let unrelated = vec![drive_targeting(EmitterProp::Lifetime, 0)];
        assert_eq!(DrivenFx::for_emitter(&unrelated, 0), DrivenFx::NONE);
    }

    #[test]
    fn a_muted_drive_still_counts_as_driven() {
        // Pins the ruling in `DrivenFx`'s doc, which is a real choice and not
        // an oversight: `muted` is flipped live but the shader defs this
        // feeds are baked into the material, and nothing rebuilds a material
        // when a drive changes. Mute keeps its meaning downstream, where the
        // slot folds to identity every frame and the substituted `1.0`
        // baseline passes it through.
        let mut drive = drive_targeting(EmitterProp::FresnelPower, 0);
        drive.muted = true;
        assert!(DrivenFx::for_emitter(&[drive], 0).fresnel_power);
    }
}
