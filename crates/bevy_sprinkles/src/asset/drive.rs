use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use super::{CurveTexture, Range, variables::VariableId};

/// When a resolved drive value is read by the thing that consumes it.
///
/// `Spawn` and `Sim` both land in the SAME simulation uniform buffer, so this
/// is not a routing distinction — it is a behavioural one, and it is the first
/// question an author asks of a knob: "does this change what is already in the
/// air?" `Spawn` = no, `Sim` = yes.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Reflect)]
pub enum Stage {
    /// Read once, when a particle is born.
    Spawn,
    /// Read every simulation step, reshaping particles already in flight.
    Sim,
    /// Read every frame at render time, for all live particles.
    Render,
}

/// How a resolved value combines with the emitter's authored value, and with
/// any earlier drive on the same target.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, Reflect)]
pub enum DriveOp {
    /// Discard everything contributed so far, including the authored value.
    Replace,
    /// Multiply into whatever has been contributed so far.
    #[default]
    Multiply,
    /// Add to whatever has been contributed so far.
    Add,
}

/// The number of render-stage slots carried in `ParticleEmitterUniforms`.
///
/// LAYOUT-LOCKSTEP with `DRIVE_SLOT_COUNT` in `shaders/common.wgsl`. Adding a
/// render property changes this integer and that one, and nothing else about
/// either struct's shape — which is the entire reason drives use a slot array
/// rather than a named uniform field per property.
pub const DRIVE_SLOT_COUNT: usize = 9;

/// A property of an emitter that a [`Drive`] can target.
///
/// Split so that an impossible combination is unrepresentable rather than
/// merely discouraged: `SpawnSize` and `SizeMul` are separate targets instead
/// of one `Size` with a stage flag, so no author can ask for a spawn-time tint
/// or a render-time lifetime — neither of which the architecture can deliver.
///
/// There is deliberately **no `Rate`**. See `SpawnProbability`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
pub enum EmitterProp {
    // --- Spawn: read once, when a particle is born ---
    /// Fraction of eligible slots that actually spawn, 0..1.
    ///
    /// This exists instead of a `Rate` that scales `amount`, because `amount`
    /// is simultaneously the particle-pool size and the per-slot simulation
    /// gate — scaling it strands live particles in truncated slots where they
    /// freeze and never despawn (`extract.rs::apply_spawn_override`'s doc
    /// comment states this). A target named `Rate` would invite exactly that
    /// forbidden implementation.
    SpawnProbability,
    /// See [`super::EmitterTime::lifetime`].
    Lifetime,
    /// Initial speed of a spawned particle.
    InitialSpeed,
    /// Size a particle is spawned at.
    SpawnSize,
    /// Angular spread of the initial spawn direction.
    Spread,
    /// Radius of the emission volume.
    EmissionRadius,

    // --- Sim: read every step; reshapes particles already in flight ---
    /// Downward acceleration applied every step.
    Gravity,
    /// Velocity damping applied every step.
    Drag,
    /// Strength of the per-step turbulence displacement.
    TurbulenceStrength,

    // --- Render: re-read every frame for all live particles ---
    /// Color tint.
    Tint,
    /// Opacity multiplier.
    Alpha,
    /// Multiplier on the particle's rendered size.
    SizeMul,
    /// Multiplier on emissive brightness.
    EmissiveIntensity,
    /// UV scroll speed along U.
    ScrollU,
    /// UV scroll speed along V.
    ScrollV,
    /// Strength of the flow-map displacement.
    FlowStrength,
    /// Threshold at which erosion dissolves a particle.
    ErosionThreshold,
    /// Power of the fresnel falloff.
    FresnelPower,
}

impl EmitterProp {
    /// Every variant, so tests and editor menus cannot drift from the enum.
    pub const ALL: [EmitterProp; 18] = [
        Self::SpawnProbability, Self::Lifetime, Self::InitialSpeed, Self::SpawnSize,
        Self::Spread, Self::EmissionRadius,
        Self::Gravity, Self::Drag, Self::TurbulenceStrength,
        Self::Tint, Self::Alpha, Self::SizeMul, Self::EmissiveIntensity,
        Self::ScrollU, Self::ScrollV, Self::FlowStrength, Self::ErosionThreshold,
        Self::FresnelPower,
    ];

    /// Exhaustive match, no wildcard arm — a new variant is a compile error
    /// until it declares when it is read.
    pub fn stage(self) -> Stage {
        match self {
            Self::SpawnProbability | Self::Lifetime | Self::InitialSpeed
            | Self::SpawnSize | Self::Spread | Self::EmissionRadius => Stage::Spawn,

            Self::Gravity | Self::Drag | Self::TurbulenceStrength => Stage::Sim,

            Self::Tint | Self::Alpha | Self::SizeMul | Self::EmissiveIntensity
            | Self::ScrollU | Self::ScrollV | Self::FlowStrength
            | Self::ErosionThreshold | Self::FresnelPower => Stage::Render,
        }
    }

    /// Index into `ParticleEmitterUniforms::drive_slots`, for render props only.
    pub fn slot(self) -> Option<usize> {
        Some(match self {
            Self::Tint => 0,
            Self::Alpha => 1,
            Self::SizeMul => 2,
            Self::EmissiveIntensity => 3,
            Self::ScrollU => 4,
            Self::ScrollV => 5,
            Self::FlowStrength => 6,
            Self::ErosionThreshold => 7,
            Self::FresnelPower => 8,
            _ => return None,
        })
    }
}

/// A channel of the emitter entity's `Transform`. Always ECS-stage.
///
/// Driving scale here is how per-axis scale is achieved: the material shader
/// already builds a per-axis `emitter_scale` vec3 from this Transform
/// (`particle_material.wgsl:215,301,318`), so a beam that lengthens along Y
/// without widening in X is a plain component write and no GPU change.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
pub enum TransformProp {
    /// Scale along X.
    ScaleX,
    /// Scale along Y.
    ScaleY,
    /// Scale along Z.
    ScaleZ,
    /// Scale applied uniformly to all three axes.
    ScaleUniform,
    /// Rotation about X, in degrees.
    RotX,
    /// Rotation about Y, in degrees.
    RotY,
    /// Rotation about Z, in degrees.
    RotZ,
    /// Position along X.
    PosX,
    /// Position along Y.
    PosY,
    /// Position along Z.
    PosZ,
}

/// A property of an effect-owned light. Always ECS-stage.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Reflect)]
pub enum LightProp {
    /// The light's intensity.
    Intensity,
    /// The light's range.
    Range,
    /// Hue channel of the light's color, driven independently of saturation/value.
    Hue,
    /// Saturation channel of the light's color.
    Saturation,
    /// Value (brightness) channel of the light's color.
    Value,
}

/// What a [`Drive`] writes to.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub enum DriveTarget {
    /// An emitter, addressed by its index into `ParticlesAsset::emitters`.
    Emitter {
        /// Index of the target emitter.
        index: u8,
        /// The property of that emitter being driven.
        prop: EmitterProp,
    },
    /// The effect entity's own `Transform`.
    Transform {
        /// Index, reserved for a future multi-transform target; `0` today.
        index: u8,
        /// The transform channel being driven.
        prop: TransformProp,
    },
    /// An effect-owned light, addressed by its index into `ParticlesAsset::lights`.
    Light {
        /// Index of the target light.
        index: u8,
        /// The property of that light being driven.
        prop: LightProp,
    },
}

/// One wire: a variable, shaped by a curve, written to a property.
///
/// Several drives may share a target; they apply in declaration order onto the
/// emitter's authored value, and `DriveOp::Replace` discards everything
/// contributed before it. Order is therefore observable, which is why the
/// editor's Drives list is reorderable.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub struct Drive {
    /// The variable that drives this wire.
    pub variable: VariableId,
    /// The property this wire writes to.
    pub target: DriveTarget,
    /// Reuses the existing curve type, so the existing `curve_edit` widget
    /// authors it with no new UI machinery.
    pub curve: CurveTexture,
    /// The curve's 0..1 output remapped to these bounds.
    pub output: Range,
    /// How the resolved value combines with what came before it.
    pub op: DriveOp,
    /// Muted drives resolve to the identity for their op. The editor's A/B
    /// toggle; persisted so a half-built effect survives a save.
    #[serde(default)]
    pub muted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_props_and_sim_props_do_not_share_a_stage() {
        // The distinction is the whole point: a Spawn prop is read once at
        // particle birth, a Sim prop every step. Collapsing them was a real
        // bug in an earlier draft of the spec (gravity is applied every step
        // at particle_simulate.wgsl:1455-1456, not at birth).
        assert_eq!(EmitterProp::SpawnSize.stage(), Stage::Spawn);
        assert_eq!(EmitterProp::Gravity.stage(), Stage::Sim);
        assert_eq!(EmitterProp::Tint.stage(), Stage::Render);
    }

    #[test]
    fn every_render_prop_has_a_distinct_slot_and_they_are_dense() {
        let mut seen: Vec<usize> = EmitterProp::ALL
            .iter()
            .filter(|p| p.stage() == Stage::Render)
            .map(|p| p.slot().expect("a Render prop must have a slot"))
            .collect();
        seen.sort_unstable();
        let expected: Vec<usize> = (0..seen.len()).collect();
        assert_eq!(seen, expected, "render slots must be dense and unique");
        assert_eq!(seen.len(), DRIVE_SLOT_COUNT);
    }

    #[test]
    fn a_non_render_prop_has_no_slot() {
        assert!(EmitterProp::Gravity.slot().is_none());
        assert!(EmitterProp::SpawnSize.slot().is_none());
    }
}
