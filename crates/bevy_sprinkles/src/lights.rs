use bevy::prelude::*;

use crate::asset::{FxLightKind, LightProp, ParticlesAsset};
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
pub fn setup_effect_lights(
    mut commands: Commands,
    assets: Res<Assets<ParticlesAsset>>,
    q: Query<(Entity, &Particles3d), Without<EffectLightsSpawned>>,
) {
    for (entity, particles) in q.iter() {
        let Some(asset) = assets.get(&particles.0) else {
            continue;
        };

        for (i, light) in asset.lights.iter().enumerate() {
            if !light.enabled {
                continue;
            }

            let transform = light.transform.to_transform();
            let mut child = commands.spawn((
                transform,
                Visibility::default(),
                LightEntity {
                    parent_system: entity,
                    light_index: i,
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

            let child = child.id();
            commands.entity(entity).add_child(child);
        }

        commands.entity(entity).insert(EffectLightsSpawned);
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

        runtime.prev_system_time = runtime.system_time;
        runtime.system_time += delta;

        if total_duration > 0.0 && runtime.system_time >= total_duration {
            runtime.system_time %= total_duration;
            runtime.cycle += 1;
        }
    }
}

/// Applies each light's own-clock intensity curve and its ECS-stage drives.
///
/// Intensity and range are recomputed from the authored value every frame
/// rather than accumulated, so this is idempotent: running it twice in a row
/// (or after any number of frames) gives the same answer as running it once
/// for the same clock and drive state.
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

        let intensity = (data.intensity * envelope * intensity_mul).max(0.0);
        let range = (data.range * range_mul).max(0.0);

        if let Some(mut l) = point {
            l.intensity = intensity;
            l.range = range;
        }
        if let Some(mut l) = spot {
            l.intensity = intensity;
            l.range = range;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{
        CurvePoint, CurveTexture, LightData, ParticlesAsset, ParticlesAuthors, ParticlesDimension,
    };
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

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
}
