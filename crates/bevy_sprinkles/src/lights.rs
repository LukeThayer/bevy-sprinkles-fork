use bevy::color::Hsva;
use bevy::prelude::*;

use crate::asset::{FxLightKind, LightData, LightProp, ParticlesAsset};
use crate::drives::EffectDrives;
use crate::runtime::{compute_phase, ParticleSystemRuntime, Particles3d};

/// Links an effect-owned light entity back to the effect that declared it.
#[derive(Component)]
pub struct LightEntity {
    /// The entity that holds the [`Particles3d`] component this light belongs to.
    pub parent_system: Entity,
    /// Index of this light within the parent [`ParticlesAsset::lights`].
    pub light_index: usize,
}

/// Marks an effect whose lights have been spawned, so [`setup_effect_lights`]
/// is idempotent: without this, every frame would spawn another full set of
/// child lights on top of the ones already there.
#[derive(Component)]
pub struct EffectLightsSpawned;

/// Runtime clock for a single effect-owned light.
///
/// Deliberately **not** [`EmitterRuntime`](crate::runtime::EmitterRuntime):
/// that type also carries trail-history, `simulation_steps` and
/// `clear_requested` state that means nothing for a light, and the only
/// system that advances an `EmitterRuntime`
/// (`update_particle_time`, `spawning.rs`) queries
/// `(&EmitterEntity, &mut EmitterRuntime)` — a light entity carries
/// [`LightEntity`], not `EmitterEntity`, so sharing the type would leave its
/// clock frozen at `0.0` forever. This type gets its own advance system,
/// [`advance_light_clocks`], instead.
#[derive(Component, Default)]
pub struct LightRuntime {
    /// Current simulation time in seconds, wrapped against the light's own
    /// [`EmitterTime::total_duration`](crate::asset::EmitterTime::total_duration).
    pub system_time: f32,
    /// Simulation time from the previous frame.
    ///
    /// Currently write-only: [`advance_light_clocks`] updates it every frame
    /// but nothing yet reads it back (there is no `prev_system_phase`
    /// equivalent for lights, unlike [`EmitterRuntime::prev_system_phase`](
    /// crate::runtime::EmitterRuntime::prev_system_phase)). Kept because it
    /// is in the ruling's named minimum field set and is the natural place
    /// to detect a phase wrap within a single frame, should a future task
    /// need that (e.g. a one-shot "just crossed the end of its curve" event).
    pub prev_system_time: f32,
    /// Current emission cycle index (increments each time the lifetime wraps).
    pub cycle: u32,
}

impl LightRuntime {
    /// Creates a fresh runtime at the start of its first cycle.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Spawns one child entity per enabled [`LightData`](crate::asset::LightData),
/// once per effect.
///
/// Idempotent via [`EffectLightsSpawned`]: the `Without<EffectLightsSpawned>`
/// filter means an effect is only ever visited by this system once, no matter
/// how many frames pass.
///
/// ## Why the whole per-effect body is one silenced `EntityCommand`
///
/// An effect can be despawned inside the window between this body running and
/// its command buffer being applied, and that is the ordinary case rather than
/// a corner: an effect is normally owned by something short-lived (a
/// projectile, an explosion, a networked entity whose despawn is some other
/// system's deferred command), so "spawned and despawned in the same frame"
/// arrives as soon as one explosion lands inside another's radius. Issued as
/// three separate commands against `entity`, that window had two distinct
/// failures, only one of which was visible:
///
/// - `commands.entity(entity).add_child(..)` and `.insert(EffectLightsSpawned)`
///   resolve the effect at apply time and error if it is gone; the default
///   handler turns that into a process-killing panic. `match_severity` is
///   `#[track_caller]` and `#[inline]`, which is why the reported crash site is
///   `bevy_ecs-0.19.0/src/error/handler.rs:130` — that function's own signature
///   — rather than the `Severity::Panic` arm it dispatches through (:138) or
///   the `panic` handler it lands in (:145). Reported from a real game as
///   `Encountered a panic when applying buffers for system
///   bevy_sprinkles::lights::setup_effect_lights`.
/// - the light child was `commands.spawn`ed BEFORE it was parented, so even a
///   panic-proof `add_child` would have left it ALIVE at the world origin with
///   a dangling `LightEntity { parent_system }`: `ChildOf`'s `on_insert` hook
///   warns and strips the dangling relationship off the CHILD instead of
///   despawning it (`bevy_ecs-0.19.0/src/relationship/mod.rs:194-213`), and
///   nothing reaps a light whose parent is gone — [`sync_effect_lights`] only
///   ever reads `parent_system`. One stray `PointLight`/`SpotLight` per dead
///   effect, owned by nothing, and absent from any crash report.
///
/// One `queue_silenced` closure on the effect buys both at once.
/// `EntityCommand::with_entity` fetches the entity FIRST and propagates the
/// fetch failure with `?`, so the closure — and every `spawn` inside it — is
/// dropped unrun when the effect is already gone
/// (`bevy_ecs-0.19.0/src/system/commands/entity_command.rs:100-103`), and
/// `queue_silenced` discards that error rather than handing it to the
/// panicking handler (`.../system/commands/mod.rs:2011`). Nothing is spawned
/// on the dead path, so there is nothing left to orphan; on the live path the
/// parent is held as an `EntityWorldMut` across the whole closure, so a child
/// can never be spawned into a world where it has since vanished.
///
/// The closure is `move`, which is what forces the clone: `asset` borrows
/// `Res<Assets<ParticlesAsset>>` and cannot cross into a command that runs
/// later. [`LightData`](crate::asset::LightData) is `Clone`, and the clone is
/// narrowed to the ENABLED lights — a disabled light costs nothing, and the
/// rest of the asset (emitters, materials, baked curve textures) is never
/// copied.
pub fn setup_effect_lights(
    mut commands: Commands,
    assets: Res<Assets<ParticlesAsset>>,
    q: Query<(Entity, &Particles3d), Without<EffectLightsSpawned>>,
) {
    for (entity, particles) in q.iter() {
        let Some(asset) = assets.get(&particles.0) else {
            continue;
        };

        let lights: Vec<(usize, LightData)> = asset
            .lights
            .iter()
            .enumerate()
            .filter(|(_, light)| light.enabled)
            .map(|(i, light)| (i, light.clone()))
            .collect();

        commands
            .entity(entity)
            .queue_silenced(move |mut effect: EntityWorldMut| {
                effect.with_children(|parent| {
                    for (i, light) in &lights {
                        let mut child = parent.spawn((
                            light.transform.to_transform(),
                            Visibility::default(),
                            LightEntity {
                                parent_system: entity,
                                light_index: *i,
                            },
                            LightRuntime::new(),
                        ));

                        match light.kind {
                            FxLightKind::Point => {
                                child.insert(PointLight {
                                    color: light.color,
                                    intensity: light.intensity,
                                    range: light.range,
                                    shadow_maps_enabled: light.shadows,
                                    ..default()
                                });
                            }
                            FxLightKind::Spot => {
                                child.insert(SpotLight {
                                    color: light.color,
                                    intensity: light.intensity,
                                    range: light.range,
                                    shadow_maps_enabled: light.shadows,
                                    ..default()
                                });
                            }
                        }
                    }
                });
                effect.insert(EffectLightsSpawned);
            });
    }
}

/// Advances every [`LightRuntime`]'s own clock from [`Time`], wrapping
/// `system_time` and incrementing `cycle` against the light's own
/// [`EmitterTime::total_duration`](crate::asset::EmitterTime::total_duration)
/// — the same wrap `update_particle_time` (`spawning.rs`) applies to an
/// emitter's `EmitterRuntime`, minus the fixed-FPS stepping and trail-history
/// bookkeeping a light has no use for.
///
/// Must run before [`sync_effect_lights`], which reads `system_time` to
/// sample `intensity_over_life`: reversing the order would make every
/// light's envelope sample last frame's phase, one frame stale forever.
///
/// Honors [`ParticleSystemRuntime::paused`] the same way
/// `update_particle_time` (`spawning.rs:105`) does for an emitter: while
/// paused, the clock simply does not advance this frame. `sync_effect_lights`
/// keeps running regardless -- with a frozen `system_time` it recomputes the
/// same phase and therefore the same intensity every frame, so the light
/// holds steady rather than going dark. `bevy_sprinkles_editor`'s pause
/// button (`playback_controls.rs`) sets exactly this flag, so without this
/// guard, pausing the preview would freeze particle motion while leaving any
/// effect light still flashing through its curve underneath it.
///
/// Honors [`EmitterTime::one_shot`](crate::asset::EmitterTime::one_shot) too:
/// once a one-shot light has completed a cycle its clock STOPS, pinned at
/// `total_duration`, instead of wrapping back to the start. Freezing it rather
/// than letting it run on keeps `light_is_off`'s answer a pure function of
/// the stored clock, so the recompute-never-accumulate property survives —
/// `sync_effect_lights` reaches the same (dark) result on frame 10 and frame
/// 10,000. Until this existed, `one_shot` was never consulted anywhere and a
/// "one-shot" light looped forever.
pub fn advance_light_clocks(
    time: Res<Time>,
    assets: Res<Assets<ParticlesAsset>>,
    systems: Query<(&Particles3d, Option<&ParticleSystemRuntime>)>,
    mut lights: Query<(&LightEntity, &mut LightRuntime)>,
) {
    let delta = time.delta_secs();
    for (link, mut runtime) in lights.iter_mut() {
        let Ok((particles, system_runtime)) = systems.get(link.parent_system) else {
            continue;
        };
        if system_runtime.is_some_and(|r| r.paused) {
            continue;
        }
        let Some(asset) = assets.get(&particles.0) else {
            continue;
        };
        let Some(data) = asset.lights.get(link.light_index) else {
            continue;
        };

        let total_duration = data.time.total_duration();

        // A spent one-shot's clock is parked, not wrapped: nothing further to
        // advance, and `light_is_off` reads `cycle` to keep it dark.
        if one_shot_is_spent(&data.time, runtime.cycle) {
            continue;
        }

        runtime.prev_system_time = runtime.system_time;
        runtime.system_time += delta;

        if total_duration > 0.0 && runtime.system_time >= total_duration {
            if data.time.one_shot {
                runtime.system_time = total_duration;
                runtime.cycle = 1;
            } else {
                runtime.system_time %= total_duration;
                runtime.cycle += 1;
            }
        }
    }
}

/// Whether a one-shot light has already run its single cycle.
fn one_shot_is_spent(time: &crate::asset::EmitterTime, cycle: u32) -> bool {
    time.one_shot && cycle >= 1
}

/// Whether a light resolves to zero intensity this frame, regardless of its
/// authored intensity, its envelope curve and its drives.
///
/// Two cases, both of which `compute_phase` alone cannot express, because it
/// returns `0.0` for "not started yet" and `0.0` for "at the very start" alike:
///
/// - **Before `delay` elapses within a cycle.** A flash authored to begin
///   bright (an envelope curve starting at 1, or no curve at all, whose
///   envelope is 1) otherwise sat at FULL brightness for the whole delay
///   window — the opposite of what `delay` means, and the reason a strobing
///   muzzle flash could not be authored the way the spec promises.
/// - **After a one-shot's single cycle.** `one_shot` was read nowhere at all
///   before this, so a "one-shot" light looped forever.
///
/// Derived fresh from the stored clock every frame rather than latched, so it
/// cannot go stale and running it twice gives the same answer once.
fn light_is_off(time: &crate::asset::EmitterTime, runtime: &LightRuntime) -> bool {
    one_shot_is_spent(time, runtime.cycle)
        || !crate::runtime::is_past_delay(runtime.system_time, time)
}

/// Applies hue/saturation/value drives to an authored colour.
///
/// Hue is a rotation in turns (0..1 maps to 0..360 degrees) so a linear curve
/// over a 0..1 variable sweeps the wheel once -- the "linear hue shift" case
/// from the design brief. Saturation and value MULTIPLY, so an undriven
/// channel is exactly the authored colour rather than a re-derived
/// approximation of it. A non-finite drive is dropped per channel, because a
/// NaN here silently blanks a light and reads as "the light broke".
pub(crate) fn apply_hsv(base: Color, hue: Option<f32>, sat: Option<f32>, val: Option<f32>) -> Color {
    let hue = hue.filter(|v| v.is_finite());
    let sat = sat.filter(|v| v.is_finite());
    let val = val.filter(|v| v.is_finite());
    // Filtering non-finite drives out BEFORE this check (rather than just
    // checking the raw Options) matters: a lone NaN drive with the other two
    // absent must read as "no drives at all" and skip the Color -> Hsva ->
    // Color round trip entirely, or the round trip's own float noise would
    // change a colour that every finite input asked to leave untouched.
    if hue.is_none() && sat.is_none() && val.is_none() {
        return base;
    }
    let mut hsva = Hsva::from(base);
    if let Some(h) = hue {
        hsva.hue = (hsva.hue + h * 360.0).rem_euclid(360.0);
    }
    if let Some(s) = sat {
        hsva.saturation = (hsva.saturation * s).clamp(0.0, 1.0);
    }
    if let Some(v) = val {
        hsva.value = (hsva.value * v).max(0.0);
    }
    Color::from(hsva)
}

/// Applies each light's own-clock intensity curve, its `delay`/`one_shot`
/// gating, and its ECS-stage drives.
///
/// Intensity, range AND colour are recomputed from the authored values every
/// frame rather than accumulated, so this is idempotent: running it twice in a
/// row (or after any number of frames) gives the same answer as running it once
/// for the same clock and drive state. Colour is equally derived — `apply_hsv`
/// starts from `data.color` every time and multiplies, it never reads back the
/// colour it wrote last frame.
pub fn sync_effect_lights(
    assets: Res<Assets<ParticlesAsset>>,
    systems: Query<(&Particles3d, Option<&EffectDrives>)>,
    mut lights: Query<(
        &LightEntity,
        &LightRuntime,
        Option<&mut PointLight>,
        Option<&mut SpotLight>,
    )>,
) {
    for (link, runtime, point, spot) in lights.iter_mut() {
        let Ok((particles, drives)) = systems.get(link.parent_system) else {
            continue;
        };
        let Some(asset) = assets.get(&particles.0) else {
            continue;
        };
        let Some(data) = asset.lights.get(link.light_index) else {
            continue;
        };

        let phase = compute_phase(runtime.system_time, &data.time);
        let envelope = data
            .intensity_over_life
            .as_ref()
            .map(|c| c.sample(phase))
            .filter(|v| v.is_finite())
            .unwrap_or(1.0);

        let resolved = drives.and_then(|d| d.0.lights.get(link.light_index));
        let intensity_mul = resolved
            .and_then(|r| r.props.get(&LightProp::Intensity))
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(1.0);
        let range_mul = resolved
            .and_then(|r| r.props.get(&LightProp::Range))
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(1.0);

        // The gate multiplies rather than short-circuiting, so range, colour
        // and every drive still resolve normally while the light is dark --
        // recompute, never latch.
        let gate = if light_is_off(&data.time, runtime) { 0.0 } else { 1.0 };
        let intensity = (data.intensity * envelope * intensity_mul * gate).max(0.0);
        let range = (data.range * range_mul).max(0.0);

        let hue = resolved
            .and_then(|r| r.props.get(&LightProp::Hue))
            .copied();
        let sat = resolved
            .and_then(|r| r.props.get(&LightProp::Saturation))
            .copied();
        let val = resolved
            .and_then(|r| r.props.get(&LightProp::Value))
            .copied();
        let color = apply_hsv(data.color, hue, sat, val);

        if let Some(mut l) = point {
            l.intensity = intensity;
            l.range = range;
            l.color = color;
        }
        if let Some(mut l) = spot {
            l.intensity = intensity;
            l.range = range;
            l.color = color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{
        CurvePoint, CurveTexture, LightData, ParticlesAsset, ParticlesAuthors, ParticlesDimension,
    };
    use bevy::color::{Hsva, Srgba};
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[test]
    fn a_hue_drive_rotates_the_authored_colour_without_changing_its_value() {
        let shifted = apply_hsv(Color::srgb(1.0, 0.0, 0.0), Some(0.5), None, None);
        let hsva = Hsva::from(shifted);
        assert!((hsva.hue - 180.0).abs() < 1.0, "hue should be rotated, got {}", hsva.hue);
        assert!(hsva.value > 0.9, "value must be untouched, got {}", hsva.value);
    }

    #[test]
    fn no_hsv_drives_returns_the_colour_unchanged() {
        let c = Color::srgb(0.25, 0.5, 0.75);
        let out = apply_hsv(c, None, None, None);
        assert_eq!(Srgba::from(out).to_f32_array(), Srgba::from(c).to_f32_array());
    }

    #[test]
    fn a_non_finite_hsv_drive_is_ignored_rather_than_blanking_the_light() {
        let c = Color::srgb(0.25, 0.5, 0.75);
        let out = apply_hsv(c, Some(f32::NAN), None, None);
        assert_eq!(Srgba::from(out).to_f32_array(), Srgba::from(c).to_f32_array());
    }

    fn app_with(lights: Vec<LightData>) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(
            Update,
            (setup_effect_lights, advance_light_clocks, sync_effect_lights).chain(),
        );
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            a.lights = lights;
            assets.add(a)
        };
        let e = app
            .world_mut()
            .spawn((Particles3d(handle), Transform::default()))
            .id();
        app.update();
        (app, e)
    }

    #[test]
    fn one_child_light_entity_is_spawned_per_declared_light() {
        let (app, effect) = app_with(vec![LightData::default(), LightData::default()]);
        let n = app
            .world()
            .iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 2);
    }

    #[test]
    fn a_disabled_light_spawns_nothing() {
        let (app, effect) = app_with(vec![LightData {
            enabled: false,
            ..Default::default()
        }]);
        let n = app
            .world()
            .iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 0);
    }

    #[test]
    fn an_effect_with_no_lights_spawns_nothing_and_does_not_panic() {
        let (app, effect) = app_with(vec![]);
        let n = app
            .world()
            .iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 0);
    }

    #[test]
    fn setup_is_idempotent_across_frames() {
        // A second frame must not spawn a second set -- the classic duplicate-
        // child bug, and invisible in a screenshot because the lights overlap.
        let (mut app, effect) = app_with(vec![LightData::default()]);
        app.update();
        app.update();
        let n = app
            .world()
            .iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 1);
    }

    /// The bug the RULING exists to prevent: an `EmitterRuntime`-backed clock
    /// would never tick, because nothing queries `(&EmitterEntity, &mut
    /// EmitterRuntime)` for a light entity. A test that only checks "a light
    /// entity exists" would pass even with a frozen clock -- this one drives
    /// the app forward on a manually-controlled clock and asserts the
    /// intensity a non-constant `intensity_over_life` curve produces actually
    /// changes between frames.
    #[test]
    fn the_lights_own_clock_advances_and_moves_its_intensity_curve() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(
            Update,
            (setup_effect_lights, advance_light_clocks, sync_effect_lights).chain(),
        );
        // Deterministic per-frame time step instead of wall-clock time.
        app.world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            let light = LightData {
                intensity: 1000.0,
                // A ramp from 0 at phase 0 to 1 at phase 1, so a phase change
                // is directly observable as an intensity change.
                intensity_over_life: Some(CurveTexture::new(vec![
                    CurvePoint::new(0.0, 0.0),
                    CurvePoint::new(1.0, 1.0),
                ])),
                time: crate::asset::EmitterTime {
                    lifetime: 1.0,
                    delay: 0.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            a.lights = vec![light];
            assets.add(a)
        };
        let effect = app
            .world_mut()
            .spawn((Particles3d(handle), Transform::default()))
            .id();

        // Frame 1: spawns the child (commands not yet applied), clock and
        // sync run against nothing yet.
        app.update();
        // Frame 2: the light entity now exists; its clock advances 0.1s from
        // 0.0, and sync reads that.
        app.update();

        let child = app
            .world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .expect("light child must exist by frame 2")
            .id();

        let first = app.world().get::<PointLight>(child).unwrap().intensity;

        // Several more frames: the clock keeps advancing and the curve keeps
        // moving the intensity, right up until the cycle wraps.
        app.update();
        let second = app.world().get::<PointLight>(child).unwrap().intensity;

        assert_ne!(
            first, second,
            "intensity must change between frames as the light's own clock advances"
        );
        assert!(
            second > first,
            "the ramp curve is increasing, so a later phase must read a higher intensity: {first} -> {second}"
        );
    }

    /// A live gap the review round flagged: `bevy_sprinkles_editor`'s pause
    /// button sets `ParticleSystemRuntime::paused`
    /// (`playback_controls.rs:98,143`, read at `viewport.rs:382,635,770,827`),
    /// so this is not hypothetical -- without the guard, pausing the preview
    /// freezes particle motion while the light keeps flashing through its
    /// curve underneath it.
    #[test]
    fn a_paused_system_freezes_its_lights_clock_and_resumes_when_unpaused() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(
            Update,
            (setup_effect_lights, advance_light_clocks, sync_effect_lights).chain(),
        );
        app.world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            let light = LightData {
                intensity: 1000.0,
                intensity_over_life: Some(CurveTexture::new(vec![
                    CurvePoint::new(0.0, 0.0),
                    CurvePoint::new(1.0, 1.0),
                ])),
                // Long enough that a handful of 0.1s frames never wraps the
                // cycle, which would otherwise confound "unchanged" with
                // "wrapped back to the same phase by coincidence".
                time: crate::asset::EmitterTime {
                    lifetime: 10.0,
                    delay: 0.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            a.lights = vec![light];
            assets.add(a)
        };
        let effect = app
            .world_mut()
            .spawn((
                Particles3d(handle),
                Transform::default(),
                ParticleSystemRuntime::default(), // paused: false
            ))
            .id();

        app.update(); // spawns the child (deferred)
        app.update(); // child exists; clock advances 0.0 -> 0.1

        let child = app
            .world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .expect("light child must exist by frame 2")
            .id();

        let before_pause = app.world().get::<PointLight>(child).unwrap().intensity;
        assert!(
            before_pause > 0.0,
            "sanity: the clock must have moved off phase 0 before pausing"
        );

        app.world_mut()
            .get_mut::<ParticleSystemRuntime>(effect)
            .unwrap()
            .paused = true;

        app.update();
        app.update();
        app.update();

        let while_paused = app.world().get::<PointLight>(child).unwrap().intensity;
        assert_eq!(
            before_pause, while_paused,
            "a paused system's light clock must not advance"
        );

        app.world_mut()
            .get_mut::<ParticleSystemRuntime>(effect)
            .unwrap()
            .paused = false;

        app.update();

        let after_resume = app.world().get::<PointLight>(child).unwrap().intensity;
        assert!(
            after_resume > while_paused,
            "unpausing must let the clock advance again: {while_paused} -> {after_resume}"
        );
    }

    #[test]
    fn a_spot_light_gets_the_same_computed_intensity_as_a_point_light_would() {
        // The point/spot branches in setup_effect_lights and sync_effect_lights
        // are parallel code paths; nothing before this test proved the spot
        // branch actually computes a value rather than, say, leaving
        // SpotLight's `default()` intensity untouched.
        let light = LightData {
            kind: FxLightKind::Spot,
            intensity: 500.0,
            intensity_over_life: None,
            ..Default::default()
        };
        let (app, effect) = app_with(vec![light]);

        let spot = app
            .world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .expect("spot light child must exist");

        assert!(
            spot.get::<PointLight>().is_none(),
            "a Spot-kind light must not also carry a PointLight"
        );
        let spot_light = spot
            .get::<SpotLight>()
            .expect("a Spot-kind light must carry a SpotLight");
        assert_eq!(
            spot_light.intensity, 500.0,
            "with no curve and no drives, a spot's intensity must equal the authored value"
        );
    }

    #[test]
    fn no_curve_leaves_the_envelope_at_one_so_intensity_is_the_authored_value() {
        let light = LightData {
            intensity: 777.0,
            intensity_over_life: None,
            ..Default::default()
        };
        let (app, effect) = app_with(vec![light]);

        let child = app
            .world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .expect("light child must exist");

        assert_eq!(
            child.get::<PointLight>().unwrap().intensity,
            777.0,
            "with intensity_over_life: None, the envelope must default to 1.0, \
             not zero the light out"
        );
    }

    #[test]
    fn no_effect_drives_component_leaves_the_drive_multipliers_at_one() {
        // A fallback that defaulted a missing EffectDrives to 0.0 instead of
        // 1.0 would make every undriven light invisible -- app_with never
        // attaches EffectDrives at all, so this is the case that would have
        // caught it.
        let light = LightData {
            intensity: 250.0,
            range: 12.0,
            intensity_over_life: None,
            ..Default::default()
        };
        let (app, effect) = app_with(vec![light]);

        let child = app
            .world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .expect("light child must exist");
        let point = child.get::<PointLight>().unwrap();

        assert_eq!(point.intensity, 250.0, "no EffectDrives must mean an identity intensity multiplier");
        assert_eq!(point.range, 12.0, "no EffectDrives must mean an identity range multiplier");
    }

    /// Task 9's review found the spot branch had no numeric colour assertion
    /// until it was asked for -- this pins colour on BOTH light kinds so the
    /// same gap can't reopen for hue/saturation/value.
    #[test]
    fn a_hue_drive_rotates_colour_on_both_a_point_and_a_spot_light() {
        use crate::drives::{EffectDrives, LightResolved, ResolvedDrives};

        let point_light = LightData {
            kind: FxLightKind::Point,
            color: Color::srgb(1.0, 0.0, 0.0),
            intensity_over_life: None,
            ..Default::default()
        };
        let spot_light = LightData {
            kind: FxLightKind::Spot,
            color: Color::srgb(1.0, 0.0, 0.0),
            intensity_over_life: None,
            ..Default::default()
        };

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(
            Update,
            (setup_effect_lights, advance_light_clocks, sync_effect_lights).chain(),
        );

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            a.lights = vec![point_light, spot_light];
            assets.add(a)
        };

        let mut hue_props = std::collections::HashMap::new();
        hue_props.insert(LightProp::Hue, 0.5_f32);
        let drives = EffectDrives(ResolvedDrives {
            emitters: vec![],
            lights: vec![
                LightResolved { props: hue_props.clone() },
                LightResolved { props: hue_props },
            ],
        });

        let effect = app
            .world_mut()
            .spawn((Particles3d(handle), Transform::default(), drives))
            .id();

        app.update();
        app.update();

        let point = app
            .world()
            .iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .find_map(|e| e.get::<PointLight>().cloned())
            .expect("a point light child must exist");
        let spot = app
            .world()
            .iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .find_map(|e| e.get::<SpotLight>().cloned())
            .expect("a spot light child must exist");

        let point_hsva = Hsva::from(point.color);
        let spot_hsva = Hsva::from(spot.color);
        assert!(
            (point_hsva.hue - 180.0).abs() < 1.0,
            "point light's hue must be rotated 180 degrees, got {}",
            point_hsva.hue
        );
        assert!(
            (spot_hsva.hue - 180.0).abs() < 1.0,
            "spot light's hue must be rotated 180 degrees, got {}",
            spot_hsva.hue
        );
    }

    /// `sync_effect_lights` must recompute colour from the authored value
    /// every frame -- never from the light's own current colour. Accumulating
    /// would drift the hue further every frame; this drives several frames on
    /// a steady hue drive and asserts the colour holds exactly, not creeping.
    #[test]
    fn colour_is_recomputed_from_the_authored_value_each_frame_rather_than_accumulated() {
        use crate::drives::{EffectDrives, LightResolved, ResolvedDrives};

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(
            Update,
            (setup_effect_lights, advance_light_clocks, sync_effect_lights).chain(),
        );
        app.world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            let light = LightData {
                color: Color::srgb(1.0, 0.0, 0.0),
                intensity_over_life: None,
                ..Default::default()
            };
            a.lights = vec![light];
            assets.add(a)
        };

        let mut hue_props = std::collections::HashMap::new();
        hue_props.insert(LightProp::Hue, 0.25_f32);
        let drives = EffectDrives(ResolvedDrives {
            emitters: vec![],
            lights: vec![LightResolved { props: hue_props }],
        });

        let effect = app
            .world_mut()
            .spawn((Particles3d(handle), Transform::default(), drives))
            .id();

        app.update(); // spawns the child (deferred)

        let mut samples = Vec::new();
        for _ in 0..5 {
            app.update();
            let child_color = app
                .world()
                .iter_entities()
                .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
                .and_then(|e| e.get::<PointLight>())
                .expect("light child must exist")
                .color;
            samples.push(Srgba::from(child_color).to_f32_array());
        }

        for pair in samples.windows(2) {
            assert_eq!(
                pair[0], pair[1],
                "a steady hue drive must produce the same colour every frame, not drift: {samples:?}"
            );
        }
    }

    // --- I3: `delay` and `one_shot` are read, not just painted ------------

    /// Builds an app whose one light uses `time` and an envelope curve that
    /// starts BRIGHT, which is the shape that exposed the delay bug: a ramp
    /// starting at 0 would have masked it, since `compute_phase` returns 0.0
    /// during the delay window and a 0-at-phase-0 curve is dark there anyway.
    fn app_with_flash(time: crate::asset::EmitterTime) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(
            Update,
            (setup_effect_lights, advance_light_clocks, sync_effect_lights).chain(),
        );
        app.world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            a.lights = vec![LightData {
                intensity: 1000.0,
                // Flat 1.0: a muzzle flash that is fully on the instant it
                // starts, so any brightness observed before `delay` elapses is
                // the defect and not the curve.
                intensity_over_life: Some(CurveTexture::new(vec![
                    CurvePoint::new(0.0, 1.0),
                    CurvePoint::new(1.0, 1.0),
                ])),
                time,
                ..Default::default()
            }];
            assets.add(a)
        };
        let effect = app
            .world_mut()
            .spawn((Particles3d(handle), Transform::default()))
            .id();
        app.update();
        (app, effect)
    }

    fn light_intensity(app: &App, effect: Entity) -> f32 {
        app.world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .and_then(|e| e.get::<PointLight>())
            .expect("light child must exist")
            .intensity
    }

    #[test]
    fn a_delayed_light_stays_dark_until_its_delay_elapses() {
        // 0.5s delay, 1.0s lifetime, 0.1s frames. `compute_phase` returns 0.0
        // for the whole delay window, so without the gate this light sits at
        // full brightness for half a second before it is supposed to exist.
        let (mut app, effect) = app_with_flash(crate::asset::EmitterTime {
            lifetime: 1.0,
            delay: 0.5,
            ..Default::default()
        });

        // Frames 2..=5 land the clock at 0.1, 0.2, 0.3, 0.4 -- all inside the
        // delay window.
        for _ in 0..4 {
            app.update();
            assert_eq!(
                light_intensity(&app, effect),
                0.0,
                "a light must be dark before its delay elapses"
            );
        }

        // Frame 6 lands at 0.5, exactly when the delay is spent.
        app.update();
        assert!(
            light_intensity(&app, effect) > 0.0,
            "the light must come up once its delay has elapsed"
        );
    }

    #[test]
    fn a_one_shot_light_goes_dark_after_its_cycle_and_stays_dark() {
        // 0.3s total against 0.1s frames: the cycle completes on the frame
        // that takes the clock to 0.3, and must never light again.
        let (mut app, effect) = app_with_flash(crate::asset::EmitterTime {
            lifetime: 0.3,
            delay: 0.0,
            one_shot: true,
            ..Default::default()
        });

        app.update();
        assert!(
            light_intensity(&app, effect) > 0.0,
            "sanity: a one-shot light must actually fire before it stops"
        );

        // Walk to the frame the cycle completes on. It must arrive: a bound of
        // six 0.1s frames is twice the 0.3s cycle, so a light still lit here
        // is one that never stops.
        let mut frames_to_dark = None;
        for frame in 0..6 {
            app.update();
            if light_intensity(&app, effect) == 0.0 {
                frames_to_dark = Some(frame);
                break;
            }
        }
        assert!(
            frames_to_dark.is_some(),
            "a one-shot light must go dark within twice its own cycle"
        );

        // Then it must STAY dark. Without `one_shot` honoured the clock wraps
        // and the flat envelope brings the light straight back up -- the "a
        // one-shot light loops forever" defect, invisible to any test that
        // samples a single frame.
        for frame in 0..20 {
            app.update();
            assert_eq!(
                light_intensity(&app, effect),
                0.0,
                "a spent one-shot light must stay dark (frame {frame} after it went out)"
            );
        }
    }

    #[test]
    fn a_spent_one_shot_lights_clock_stops_rather_than_running_on() {
        let (mut app, effect) = app_with_flash(crate::asset::EmitterTime {
            lifetime: 0.3,
            delay: 0.0,
            one_shot: true,
            ..Default::default()
        });
        for _ in 0..10 {
            app.update();
        }

        let child = app
            .world()
            .iter_entities()
            .find(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .expect("light child must exist")
            .id();
        let runtime = app.world().get::<LightRuntime>(child).unwrap();
        assert_eq!(
            runtime.cycle, 1,
            "a one-shot light must complete exactly one cycle, not keep counting"
        );
        assert_eq!(
            runtime.system_time, 0.3,
            "the clock parks at total_duration instead of wrapping or running on"
        );
    }

    #[test]
    fn a_looping_light_still_wraps_and_relights() {
        // The counterpart guard: the one-shot stop must not have frozen every
        // light. Same timings, `one_shot: false`.
        let (mut app, effect) = app_with_flash(crate::asset::EmitterTime {
            lifetime: 0.3,
            delay: 0.0,
            one_shot: false,
            ..Default::default()
        });
        for _ in 0..20 {
            app.update();
        }
        assert!(
            light_intensity(&app, effect) > 0.0,
            "a looping light must keep relighting after its cycle wraps"
        );
    }

    /// Drives `setup_effect_lights` across the exact window the reported crash
    /// lives in: the body runs, THEN the effect dies, THEN the buffers are
    /// applied. `App::update` cannot express that — the scheduler puts a sync
    /// point between a system with deferred params and anything ordered after
    /// it, so an in-schedule despawner would always find the spawns already
    /// flushed. `run_without_applying_deferred` and `apply_deferred` are the
    /// two halves `System::run` fuses together
    /// (`bevy_ecs-0.19.0/src/system/system.rs:126-148`), so splitting them is
    /// the window, not an approximation of it.
    fn despawned_inside_the_command_window(lights: Vec<LightData>) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(),
                ParticlesDimension::D3,
                Default::default(),
                vec![],
                vec![],
                false,
                ParticlesAuthors::default(),
            );
            a.lights = lights;
            assets.add(a)
        };
        let effect = app
            .world_mut()
            .spawn((Particles3d(handle), Transform::default()))
            .id();

        let mut system = IntoSystem::into_system(setup_effect_lights);
        let _ = system.initialize(app.world_mut());
        system
            .run_without_applying_deferred((), app.world_mut())
            .expect("the system body itself must run");
        app.world_mut().entity_mut(effect).despawn();
        system.apply_deferred(app.world_mut());
        app
    }

    /// The reported crash, from a real game: one flame explodes inside
    /// another's radius, so an effect is spawned and despawned in the same
    /// frame, and `insert<EffectLightsSpawned>` / `add_child` resolve a dead
    /// entity while the buffers flush.
    #[test]
    fn an_effect_that_dies_before_its_buffers_flush_applies_them_without_panicking() {
        let app = despawned_inside_the_command_window(vec![LightData::default()]);
        assert_eq!(
            app.world().iter_entities().filter(|e| e.contains::<Particles3d>()).count(),
            0,
            "the effect really must be gone -- otherwise this test proves nothing"
        );
    }

    /// The half a panic-only fix would miss. The light child used to be
    /// spawned BEFORE it was parented, so guarding only `add_child` and
    /// `insert` stops the crash and still leaks: `ChildOf`'s `on_insert` hook
    /// strips the dangling relationship off the child and keeps the child
    /// (`bevy_ecs-0.19.0/src/relationship/mod.rs:194-213`), leaving a
    /// `PointLight` at the world origin that nothing owns and nothing reaps.
    #[test]
    fn an_effect_that_dies_before_its_buffers_flush_leaves_no_orphan_light() {
        let app = despawned_inside_the_command_window(vec![
            LightData::default(),
            LightData {
                kind: FxLightKind::Spot,
                ..Default::default()
            },
        ]);
        let orphans = app.world().iter_entities().filter(|e| e.contains::<LightEntity>()).count();
        assert_eq!(orphans, 0, "a dead effect must spawn no light at all");
        let stray_lights = app
            .world()
            .iter_entities()
            .filter(|e| e.contains::<PointLight>() || e.contains::<SpotLight>())
            .count();
        assert_eq!(stray_lights, 0, "no light component may survive at the world origin");
    }

    /// The live path the guard must not have cost: a surviving effect still
    /// gets one child per enabled light, each actually PARENTED to it (not
    /// merely pointing at it via `LightEntity`), and carries the idempotency
    /// marker itself.
    #[test]
    fn a_surviving_effect_gets_its_lights_as_real_children_and_the_marker() {
        let (app, effect) = app_with(vec![
            LightData::default(),
            LightData {
                enabled: false,
                ..Default::default()
            },
            LightData {
                kind: FxLightKind::Spot,
                ..Default::default()
            },
        ]);
        assert!(
            app.world().get::<EffectLightsSpawned>(effect).is_some(),
            "the marker belongs on the effect, or the next frame spawns a second set"
        );
        let children = app
            .world()
            .get::<Children>(effect)
            .expect("the effect must own its lights through the hierarchy")
            .iter()
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2, "one child per ENABLED light, disabled ones skipped");
        for child in children {
            let link = app
                .world()
                .get::<LightEntity>(child)
                .expect("every child here is a light");
            assert_eq!(link.parent_system, effect);
            assert!(
                app.world().get::<PointLight>(child).is_some()
                    || app.world().get::<SpotLight>(child).is_some(),
                "a light child must carry the light component its kind asked for"
            );
        }
        // Index 1 is the disabled light, so the two children must be the
        // asset's lights 0 and 2 -- a fix that renumbered them while filtering
        // would send `sync_effect_lights` to the wrong `LightData` forever.
        let mut indices = app
            .world()
            .iter_entities()
            .filter_map(|e| e.get::<LightEntity>())
            .map(|l| l.light_index)
            .collect::<Vec<_>>();
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 2]);
    }
}
