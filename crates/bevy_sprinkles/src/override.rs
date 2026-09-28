use std::collections::HashMap;

use bevy::prelude::*;

use crate::asset::{CurveTexture, Gradient};

/// Per-instance runtime override for a particle system. Attach to the same
/// entity that holds [`Particles3d`](crate::runtime::Particles3d). Every field is
/// `Option`; `None` means "use the asset's authored value". An entity with no
/// `ParticleOverride` renders exactly as stock.
#[derive(Component, Clone, Default)]
pub struct ParticleOverride {
    /// Multiplies particle color every frame (render-time, all live particles). `None` = identity.
    pub tint: Option<LinearRgba>,
    /// Multiplies particle scale every frame (render-time). `None` = identity.
    pub size_mul: Option<f32>,
    /// Sets this instance's material emissive (color x intensity). `None` = asset value.
    pub emissive: Option<LinearRgba>,
    /// Multiplies emitter lifetime (spawn-time). `None` = identity.
    pub lifetime_mul: Option<f32>,
    /// Multiplies initial velocity magnitude (spawn-time). `None` = identity.
    pub speed_mul: Option<f32>,
    /// Replaces this instance's color-over-lifetime gradient (identity layer). `None` = asset gradient.
    pub color_keys: Option<Gradient>,
    /// Replaces this instance's scale-over-lifetime curve (identity layer). `None` = asset curve.
    pub size_keys: Option<CurveTexture>,
}

/// Internal: holds the stable per-instance baked texture handles for the identity
/// layer, plus the last-baked key hashes so we only re-bake when keys change.
/// Attached to each emitter entity by `setup_particle_systems` and managed by
/// `bake_override_textures`; users never touch it directly.
#[derive(Component, Default)]
pub(crate) struct OverrideBakedTextures {
    pub color: Option<Handle<Image>>,
    pub size: Option<Handle<Image>>,
    pub color_hash: Option<u64>,
    pub size_hash: Option<u64>,
}

/// Resolves the render-time multipliers, falling back to identity per field.
///
/// Unused between Task 5 (which moved `write_emitter_uniforms` onto
/// `EffectDrives`, its only non-test caller) and Task 8 (which deletes this
/// module along with the rest of `ParticleOverride`). Kept and allowed rather
/// than deleted early so this task's diff stays reviewable on its own.
#[allow(dead_code)]
pub fn emitter_multipliers(o: Option<&ParticleOverride>) -> (Vec4, f32) {
    let tint = o
        .and_then(|o| o.tint)
        .map(|c| Vec4::new(c.red, c.green, c.blue, c.alpha))
        .unwrap_or(Vec4::ONE);
    let size_mul = o.and_then(|o| o.size_mul).unwrap_or(1.0);
    (tint, size_mul)
}

/// Per-emitter overrides, keyed by the emitter's authored name (`EmitterData.name`).
/// When present on an entity it is AUTHORITATIVE: each emitter's override is
/// `get(name)`, and emitters whose name is absent render stock. A whole-system
/// [`ParticleOverride`] on the same entity is then ignored (no layered default).
#[derive(Component, Clone, Default)]
pub struct ParticleEmitterOverrides(pub HashMap<String, ParticleOverride>);

/// Resolves the effective override for one emitter. `per_emitter`, when present,
/// wins entirely; otherwise the whole-system `whole` applies.
pub fn effective_override<'a>(
    emitter_name: &str,
    whole: Option<&'a ParticleOverride>,
    per_emitter: Option<&'a ParticleEmitterOverrides>,
) -> Option<&'a ParticleOverride> {
    match per_emitter {
        Some(m) => m.0.get(emitter_name),
        None => whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_override_yields_identity_multipliers() {
        assert_eq!(emitter_multipliers(None), (Vec4::ONE, 1.0));
    }

    #[test]
    fn override_multipliers_pass_through() {
        let o = ParticleOverride {
            tint: Some(LinearRgba::new(1.0, 0.0, 0.0, 1.0)),
            size_mul: Some(2.0),
            ..Default::default()
        };
        let (tint, size) = emitter_multipliers(Some(&o));
        assert_eq!(tint, Vec4::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(size, 2.0);
    }

    #[test]
    fn partial_override_falls_back_per_field() {
        let o = ParticleOverride {
            size_mul: Some(0.5),
            ..Default::default()
        };
        let (tint, size) = emitter_multipliers(Some(&o));
        assert_eq!(tint, Vec4::ONE); // tint None -> identity
        assert_eq!(size, 0.5);
    }

    #[test]
    fn per_emitter_map_is_authoritative_when_present() {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "Fire".to_string(),
            ParticleOverride {
                size_mul: Some(2.0),
                ..Default::default()
            },
        );
        let per = ParticleEmitterOverrides(map);
        let whole = ParticleOverride {
            size_mul: Some(9.0),
            ..Default::default()
        };

        // hit -> the mapped override (NOT the whole-system one)
        assert_eq!(
            effective_override("Fire", Some(&whole), Some(&per))
                .unwrap()
                .size_mul,
            Some(2.0)
        );
        // miss -> None (stock), even though a whole-system override exists
        assert!(effective_override("Smoke", Some(&whole), Some(&per)).is_none());
    }

    #[test]
    fn whole_system_applies_when_no_map() {
        let whole = ParticleOverride {
            size_mul: Some(3.0),
            ..Default::default()
        };
        assert_eq!(
            effective_override("anything", Some(&whole), None)
                .unwrap()
                .size_mul,
            Some(3.0)
        );
        assert!(effective_override("anything", None, None).is_none());
    }
}
