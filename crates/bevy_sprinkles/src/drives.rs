use std::collections::HashMap;

use bevy::prelude::*;

use crate::asset::{
    Drive, DriveOp, DriveTarget, EmitterProp, LightProp, ParticlesAsset, Stage, TransformProp,
    VariableDecl, DRIVE_SLOT_COUNT,
};
use crate::runtime::Particles3d;

/// The host game's per-instance knob values for one effect entity.
///
/// Attach to the same entity that holds [`Particles3d`](crate::Particles3d).
/// Two entities sharing one asset with different values here render
/// differently — that per-instance isolation is the point of the whole
/// variable system, and it is why this is a `Component` and not a `Resource`.
///
/// Keyed by name rather than by [`VariableId`](crate::asset::VariableId)
/// because the host should not have to know an effect's internal ordering, and
/// an effect swapped underneath must not silently reinterpret the host's
/// numbers as different knobs. Name resolution happens once per frame against
/// the asset's declarations, which is cheap next to everything else in a frame.
#[derive(Component, Clone, Default, Debug)]
pub struct ParticleVariables(HashMap<String, f32>);

impl ParticleVariables {
    /// Sets the value for a named variable, overwriting any previous value.
    pub fn set(&mut self, name: &str, value: f32) {
        self.0.insert(name.to_string(), value);
    }

    /// Returns the raw value set for `name`, or `None` if it was never set.
    ///
    /// This does not consult declarations or defaults — use
    /// [`resolve_values`](Self::resolve_values) for that.
    pub fn get(&self, name: &str) -> Option<f32> {
        self.0.get(name).copied()
    }

    /// Clears every value this host has set, reverting all variables to their
    /// declared defaults on the next resolve.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// One value per declaration, in declaration order, so the result indexes
    /// directly by `VariableId`.
    ///
    /// A name this effect does not declare is ignored rather than fatal: hosts
    /// legitimately hold one variable map across effect swaps. A non-finite
    /// value falls back to the default, because NaN reaching a uniform
    /// propagates through the vertex shader and can blank the whole draw —
    /// which an author reads as "my effect disappeared", with nothing pointing
    /// at the number that caused it.
    pub fn resolve_values(&self, decls: &[VariableDecl]) -> Vec<f32> {
        decls
            .iter()
            .map(|d| match self.0.get(&d.name) {
                Some(v) if v.is_finite() => *v,
                _ => d.default,
            })
            .collect()
    }

    /// Names set here that no declaration matches. Used for a warn-once
    /// diagnostic; a typo'd knob is otherwise completely silent.
    pub fn unknown_names<'a>(&'a self, decls: &[VariableDecl]) -> Vec<&'a str> {
        self.0
            .keys()
            .filter(|k| !decls.iter().any(|d| &d.name == *k))
            .map(|k| k.as_str())
            .collect()
    }
}

/// Everything a single effect instance's drives resolved to this frame.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDrives {
    /// Per-emitter resolved values, indexed by emitter index.
    pub emitters: Vec<EmitterResolved>,
    /// Per-light resolved values, indexed by light index.
    pub lights: Vec<LightResolved>,
}

/// `None` in a slot means "no drive touched this"; the consumer keeps the
/// emitter's authored value. That is deliberately distinct from `Some(1.0)`,
/// which means a drive computed exactly one — the difference matters for
/// `DriveOp::Replace` on a property whose authored value is not 1.
#[derive(Clone, Debug)]
pub struct EmitterResolved {
    /// Resolved [`Stage::Spawn`] properties for this emitter, read once at
    /// particle birth.
    pub spawn: HashMap<EmitterProp, f32>,
    /// Resolved [`Stage::Sim`] properties for this emitter, read every step.
    pub sim: HashMap<EmitterProp, f32>,
    /// Resolved [`Stage::Render`] properties, indexed by
    /// [`EmitterProp::slot`] — dense with `ParticleEmitterUniforms::drive_slots`'s
    /// layout (see [`DRIVE_SLOT_COUNT`]'s doc for the lockstep this depends on).
    pub render: [Option<f32>; DRIVE_SLOT_COUNT],
    /// Resolved channels of this emitter's own `Transform`.
    pub transform: HashMap<TransformProp, f32>,
}

impl Default for EmitterResolved {
    fn default() -> Self {
        Self {
            spawn: HashMap::new(),
            sim: HashMap::new(),
            render: [None; DRIVE_SLOT_COUNT],
            transform: HashMap::new(),
        }
    }
}

/// Resolved properties of one effect-owned light.
#[derive(Clone, Debug, Default)]
pub struct LightResolved {
    /// Resolved light properties, keyed by which one they drive.
    pub props: HashMap<LightProp, f32>,
}

/// Samples one drive to a finite scalar, or `None` if it contributes nothing.
///
/// `values` is indexed by `VariableId`, so this never does a string lookup.
/// Every arithmetic result is checked for finiteness at the boundary rather
/// than trusting the inputs: the curve's control points, the output bounds and
/// the host's variable are three independent places a NaN can enter, and only
/// one of them (the host's) is guarded upstream.
fn sample(drive: &Drive, values: &[f32]) -> Option<f32> {
    if drive.muted {
        return None;
    }
    let t = *values.get(drive.variable.0 as usize)?;
    if !t.is_finite() {
        return None;
    }
    let unit = drive.curve.sample(t.clamp(0.0, 1.0));
    if !unit.is_finite() {
        return None;
    }
    let (lo, hi) = (drive.output.min, drive.output.max);
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    let out = lo + (hi - lo) * unit;
    out.is_finite().then_some(out)
}

/// Folds one contribution onto whatever earlier drives left behind.
///
/// `Replace` discards the accumulator entirely, including the consumer's
/// authored value — which is why a `Replace` after a `Multiply` wipes it, and
/// why the Drives list in the editor is reorderable rather than a set.
fn fold(acc: Option<f32>, value: f32, op: DriveOp) -> Option<f32> {
    let next = match (op, acc) {
        (DriveOp::Replace, _) => value,
        (DriveOp::Multiply, Some(a)) => a * value,
        (DriveOp::Multiply, None) => value,
        (DriveOp::Add, Some(a)) => a + value,
        (DriveOp::Add, None) => value,
    };
    next.is_finite().then_some(next)
}

/// Resolves every drive in `asset` against already-resolved variable `values`.
///
/// Pure: no ECS, no GPU, no frames. Cost is one curve sample per drive per
/// instance — a variable curve yields ONE scalar, not a per-particle ramp,
/// which is exactly why variable curves are sampled here on the CPU while
/// lifetime curves stay baked into GPU textures.
pub fn resolve_drives(values: &[f32], asset: &ParticlesAsset) -> ResolvedDrives {
    let mut out = ResolvedDrives {
        emitters: vec![EmitterResolved::default(); asset.emitters.len()],
        lights: vec![LightResolved::default(); asset.lights.len()],
    };

    for drive in &asset.drives {
        let Some(value) = sample(drive, values) else { continue };
        match &drive.target {
            DriveTarget::Emitter { index, prop } => {
                let Some(e) = out.emitters.get_mut(*index as usize) else { continue };
                match prop.stage() {
                    Stage::Spawn => {
                        let acc = e.spawn.get(prop).copied();
                        if let Some(v) = fold(acc, value, drive.op) { e.spawn.insert(*prop, v); }
                    }
                    Stage::Sim => {
                        let acc = e.sim.get(prop).copied();
                        if let Some(v) = fold(acc, value, drive.op) { e.sim.insert(*prop, v); }
                    }
                    Stage::Render => {
                        let Some(slot) = prop.slot() else { continue };
                        e.render[slot] = fold(e.render[slot], value, drive.op);
                    }
                }
            }
            DriveTarget::Transform { index, prop } => {
                let Some(e) = out.emitters.get_mut(*index as usize) else { continue };
                let acc = e.transform.get(prop).copied();
                if let Some(v) = fold(acc, value, drive.op) { e.transform.insert(*prop, v); }
            }
            DriveTarget::Light { index, prop } => {
                let Some(l) = out.lights.get_mut(*index as usize) else { continue };
                let acc = l.props.get(prop).copied();
                if let Some(v) = fold(acc, value, drive.op) { l.props.insert(*prop, v); }
            }
        }
    }
    out
}

/// This frame's resolved drives for one effect instance.
///
/// Recomputed from scratch every frame rather than mutated incrementally:
/// derived state cannot leak or go stale, and a drive added, muted or reordered
/// in the editor takes effect on the next frame with no invalidation step.
#[derive(Component, Clone, Debug, Default)]
pub struct EffectDrives(pub ResolvedDrives);

/// Set once on an entity whose `ParticleVariables` names something the effect
/// does not declare, so the warning is emitted once instead of every frame.
#[derive(Component)]
pub struct UnknownVariablesWarned;

/// Resolves every effect instance's variables and drives for this frame.
///
/// Runs in `Update` before the render-uniform write in `PostUpdate`, so a
/// value set by host code this frame reaches the GPU the same frame.
pub fn evaluate_drives(
    mut commands: Commands,
    assets: Res<Assets<ParticlesAsset>>,
    mut q: Query<(
        Entity,
        &Particles3d,
        Option<&ParticleVariables>,
        Option<&mut EffectDrives>,
        Has<UnknownVariablesWarned>,
    )>,
) {
    for (entity, particles, vars, resolved, warned) in q.iter_mut() {
        let Some(asset) = assets.get(&particles.0) else { continue };

        let empty = ParticleVariables::default();
        let vars = vars.unwrap_or(&empty);

        if !warned {
            let unknown = vars.unknown_names(&asset.variables);
            if !unknown.is_empty() {
                warn!(
                    "effect {:?}: ParticleVariables names {:?}, which this effect does not declare; \
                     using declared defaults. Declared: {:?}",
                    asset.name,
                    unknown,
                    asset.variables.iter().map(|v| &v.name).collect::<Vec<_>>(),
                );
                commands.entity(entity).insert(UnknownVariablesWarned);
            }
        }

        let values = vars.resolve_values(&asset.variables);
        let next = resolve_drives(&values, asset);
        match resolved {
            Some(mut slot) => slot.0 = next,
            None => { commands.entity(entity).insert(EffectDrives(next)); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{Range, VariableDecl};

    fn decls() -> Vec<VariableDecl> {
        vec![
            VariableDecl { name: "temperature".into(), default: 0.25, range: Range { min: 0.0, max: 1.0 } },
            VariableDecl { name: "charge".into(), default: 0.5, range: Range { min: 0.0, max: 1.0 } },
        ]
    }

    #[test]
    fn an_unset_variable_resolves_to_its_declared_default() {
        let v = ParticleVariables::default();
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.5]);
    }

    #[test]
    fn a_set_variable_wins_over_its_default() {
        let mut v = ParticleVariables::default();
        v.set("charge", 0.9);
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.9]);
    }

    #[test]
    fn a_name_the_effect_does_not_declare_is_ignored_not_fatal() {
        // A host that swaps an effect underneath, or fat-fingers a name, gets
        // the declared defaults and a warning -- not a panic and not silence.
        let mut v = ParticleVariables::default();
        v.set("temprature", 0.9); // typo
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.5]);
    }

    #[test]
    fn a_non_finite_host_value_falls_back_to_the_default() {
        // NaN reaching a uniform propagates through the vertex shader and can
        // blank a whole draw call, which reads as "the effect vanished".
        let mut v = ParticleVariables::default();
        v.set("temperature", f32::NAN);
        v.set("charge", f32::INFINITY);
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.5]);
    }

    #[test]
    fn two_instances_do_not_share_state() {
        let mut a = ParticleVariables::default();
        let mut b = ParticleVariables::default();
        a.set("temperature", 1.0);
        b.set("temperature", 0.0);
        assert_eq!(a.resolve_values(&decls())[0], 1.0);
        assert_eq!(b.resolve_values(&decls())[0], 0.0);
    }

    use crate::asset::{
        CurveTexture, CurvePoint, Drive, DriveOp, DriveTarget, EmitterProp, ParticlesAsset,
        ParticlesAuthors, ParticlesDimension, EmitterData, VariableId,
    };

    /// A curve that returns `v` everywhere, so a test asserts on the drive's
    /// arithmetic rather than on curve interpolation.
    fn flat(v: f64) -> CurveTexture {
        CurveTexture::new(vec![CurvePoint::new(0.0, v), CurvePoint::new(1.0, v)])
    }

    /// The identity curve (`sample(t) == t`), for tests that must observe the
    /// resolved *value* of a variable rather than just its presence -- `flat`
    /// is deliberately insensitive to `t` and cannot distinguish two instances
    /// whose only difference is which value they feed in.
    fn ramp() -> CurveTexture {
        CurveTexture::new(vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)])
    }

    fn asset_with_drives(drives: Vec<Drive>) -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(), ParticlesDimension::D3, Default::default(),
            vec![EmitterData::default()], vec![], false, ParticlesAuthors::default(),
        );
        a.variables = vec![VariableDecl { name: "v".into(), default: 0.0, range: Range { min: 0.0, max: 1.0 } }];
        a.drives = drives;
        a
    }

    fn d(prop: EmitterProp, curve: CurveTexture, output: Range, op: DriveOp) -> Drive {
        Drive {
            variable: VariableId(0),
            target: DriveTarget::Emitter { index: 0, prop },
            curve, output, op, muted: false,
        }
    }

    #[test]
    fn a_render_drive_lands_in_its_slot_and_nowhere_else() {
        let a = asset_with_drives(vec![
            d(EmitterProp::SizeMul, flat(1.0), Range { min: 0.0, max: 4.0 }, DriveOp::Replace),
        ]);
        let r = resolve_drives(&[1.0], &a);
        let slot = EmitterProp::SizeMul.slot().unwrap();
        assert_eq!(r.emitters[0].render[slot], Some(4.0));
        for (i, s) in r.emitters[0].render.iter().enumerate() {
            if i != slot { assert_eq!(*s, None, "slot {i} must be untouched"); }
        }
    }

    #[test]
    fn spawn_and_sim_drives_land_in_separate_maps() {
        let a = asset_with_drives(vec![
            d(EmitterProp::SpawnSize, flat(1.0), Range { min: 0.0, max: 2.0 }, DriveOp::Replace),
            d(EmitterProp::Gravity,   flat(1.0), Range { min: 0.0, max: 3.0 }, DriveOp::Replace),
        ]);
        let r = resolve_drives(&[1.0], &a);
        assert_eq!(r.emitters[0].spawn.get(&EmitterProp::SpawnSize), Some(&2.0));
        assert_eq!(r.emitters[0].sim.get(&EmitterProp::Gravity), Some(&3.0));
        assert!(r.emitters[0].spawn.get(&EmitterProp::Gravity).is_none());
        assert!(r.emitters[0].sim.get(&EmitterProp::SpawnSize).is_none());
    }

    #[test]
    fn the_output_range_remaps_the_curve() {
        let a = asset_with_drives(vec![
            d(EmitterProp::Alpha, flat(0.5), Range { min: 2.0, max: 4.0 }, DriveOp::Replace),
        ]);
        let r = resolve_drives(&[1.0], &a);
        assert_eq!(r.emitters[0].render[EmitterProp::Alpha.slot().unwrap()], Some(3.0));
    }

    #[test]
    fn drives_on_one_target_apply_in_declaration_order() {
        // Replace must discard the Multiply that came before it, and be kept
        // by the Multiply that comes after: 1*2 -> replaced by 5 -> *3 = 15.
        let a = asset_with_drives(vec![
            d(EmitterProp::SizeMul, flat(1.0), Range { min: 2.0, max: 2.0 }, DriveOp::Multiply),
            d(EmitterProp::SizeMul, flat(1.0), Range { min: 5.0, max: 5.0 }, DriveOp::Replace),
            d(EmitterProp::SizeMul, flat(1.0), Range { min: 3.0, max: 3.0 }, DriveOp::Multiply),
        ]);
        let r = resolve_drives(&[1.0], &a);
        assert_eq!(r.emitters[0].render[EmitterProp::SizeMul.slot().unwrap()], Some(15.0));
    }

    #[test]
    fn a_muted_drive_contributes_nothing() {
        let mut drive = d(EmitterProp::Alpha, flat(1.0), Range { min: 9.0, max: 9.0 }, DriveOp::Replace);
        drive.muted = true;
        let a = asset_with_drives(vec![drive]);
        let r = resolve_drives(&[1.0], &a);
        assert_eq!(r.emitters[0].render[EmitterProp::Alpha.slot().unwrap()], None);
    }

    #[test]
    fn an_authored_nan_never_reaches_a_slot() {
        // Review Focus 3. A NaN in a uniform propagates through the vertex
        // shader and can blank the whole draw call, which an author reads as
        // "my effect vanished" with nothing pointing at the bad number.
        let a = asset_with_drives(vec![
            d(EmitterProp::Alpha, flat(f64::NAN), Range { min: 0.0, max: 1.0 }, DriveOp::Replace),
        ]);
        let r = resolve_drives(&[1.0], &a);
        let got = r.emitters[0].render[EmitterProp::Alpha.slot().unwrap()];
        assert!(got.is_none() || got.unwrap().is_finite(), "got {got:?}");
    }

    #[test]
    fn a_nan_output_bound_never_reaches_a_slot() {
        let a = asset_with_drives(vec![
            d(EmitterProp::Alpha, flat(1.0), Range { min: 0.0, max: f32::NAN }, DriveOp::Replace),
        ]);
        let r = resolve_drives(&[1.0], &a);
        let got = r.emitters[0].render[EmitterProp::Alpha.slot().unwrap()];
        assert!(got.is_none() || got.unwrap().is_finite(), "got {got:?}");
    }

    #[test]
    fn a_degenerate_curve_and_a_zero_width_range_still_resolve_finitely() {
        // Review Focus 5: zero control points, and min == max.
        let a = asset_with_drives(vec![
            d(EmitterProp::Alpha, CurveTexture::new(vec![]), Range { min: 1.0, max: 1.0 }, DriveOp::Replace),
        ]);
        let r = resolve_drives(&[1.0], &a);
        let got = r.emitters[0].render[EmitterProp::Alpha.slot().unwrap()];
        assert!(got.is_none() || got.unwrap().is_finite(), "got {got:?}");
    }

    #[test]
    fn an_effect_with_no_drives_resolves_to_all_none() {
        let a = asset_with_drives(vec![]);
        let r = resolve_drives(&[0.0], &a);
        assert_eq!(r.emitters.len(), 1);
        assert!(r.emitters[0].render.iter().all(|s| s.is_none()));
        assert!(r.emitters[0].spawn.is_empty());
        assert!(r.emitters[0].sim.is_empty());
    }

    #[test]
    fn a_transform_drive_lands_on_the_emitter_its_index_names() {
        // Pins that Transform's index is an EMITTER index, not a
        // reserved-must-be-zero slot: with two emitters, a drive on index 1
        // must touch emitters[1] and leave emitters[0] alone.
        let mut a = asset_with_drives(vec![Drive {
            variable: VariableId(0),
            target: DriveTarget::Transform { index: 1, prop: TransformProp::ScaleY },
            curve: flat(1.0),
            output: Range { min: 0.0, max: 3.0 },
            op: DriveOp::Replace,
            muted: false,
        }]);
        a.emitters = vec![EmitterData::default(), EmitterData::default()];
        let r = resolve_drives(&[1.0], &a);
        assert_eq!(r.emitters[1].transform.get(&TransformProp::ScaleY), Some(&3.0));
        assert!(r.emitters[0].transform.is_empty(), "the other emitter must be untouched");
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(Update, evaluate_drives);
        app
    }

    #[test]
    fn two_entities_sharing_an_asset_resolve_independently() {
        // Review Focus 4: the per-instance guarantee. The bare entity must sit
        // at declared defaults, NOT at its neighbour's value.
        let mut app = test_app();
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset_with_drives(vec![d(
                EmitterProp::SizeMul, ramp(), Range { min: 0.0, max: 10.0 }, DriveOp::Replace,
            )]))
        };

        let mut hot = ParticleVariables::default();
        hot.set("v", 1.0);
        let hot_e = app.world_mut().spawn((Particles3d(handle.clone()), hot)).id();
        // No ParticleVariables at all -- must fall back to default (0.0).
        let bare_e = app.world_mut().spawn(Particles3d(handle.clone())).id();

        app.update();

        let slot = EmitterProp::SizeMul.slot().unwrap();
        let hot_v = app.world().entity(hot_e).get::<EffectDrives>().unwrap().0.emitters[0].render[slot];
        let bare_v = app.world().entity(bare_e).get::<EffectDrives>().unwrap().0.emitters[0].render[slot];
        assert_eq!(hot_v, Some(10.0));
        assert_eq!(bare_v, Some(0.0), "a bare instance must use declared defaults");
    }

    #[test]
    fn an_unknown_variable_name_warns_only_once_per_entity() {
        // Review Focus 2: a typo must not emit a warning every frame forever.
        let mut app = test_app();
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset_with_drives(vec![]))
        };
        let mut vars = ParticleVariables::default();
        vars.set("nope", 1.0);
        let e = app.world_mut().spawn((Particles3d(handle), vars)).id();

        app.update();
        app.update();
        app.update();

        let warned = app.world().entity(e).get::<UnknownVariablesWarned>();
        assert!(warned.is_some(), "the entity must be marked as already warned");
    }
}
