//! The two optimisations measured together, on one scene, headlessly.
//!
//! **What these numbers are.** Nobody who can run this file can time a frame:
//! there is no GPU and no window here. So the measurement is of *work
//! eliminated* — emitters extracted, compute dispatches issued, sort
//! dispatches issued — and those are a proxy for frame time, not a
//! substitute for one. They are the right proxy because the cost these two
//! changes attack is per-*emitter* fixed cost rather than per-particle cost:
//! a buffer and a bind group per simulation step, a compute dispatch per step,
//! and an init/bitonic/copy chain per emitter, all paid once per emitter per
//! frame no matter how few particles are alive.
//!
//! **Why the counts are real and not modelled.** The scene is extracted by the
//! actual `extract_particle_systems`, running in an actual render sub-app that
//! [`ExtractPlugin`](bevy::render::extract_plugin::ExtractPlugin) builds
//! without ever asking for an adapter. Its output is then fed to the actual
//! [`plan_sort_dispatches`], which is pure arithmetic over a
//! `DynamicUniformBuffer` scratch buffer. Both halves are the shipping code
//! path; only the device-touching tail of `prepare_particle_sort_bind_groups`
//! is absent, and that tail issues no dispatches.
//!
//! **Why this module exists rather than living in `extract.rs` or `sort.rs`.**
//! It needs both sides at once, and the "before" figures are produced by
//! undoing each change *as data* on the extracted output rather than by
//! remembering a number: hiding nothing reproduces the world before the
//! visibility gate, and forcing every `needs_sorting` to `true` reproduces the
//! world before the blend gate, because `draw_order != 0` was that branch's
//! only condition.

use bevy::asset::AssetPlugin;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::{ExtractSchedule, Render, RenderApp, extract_plugin::ExtractPlugin};
use bevy::render::render_resource::DynamicUniformBuffer;

use crate::asset::{
    DrawOrder, DrawPassMaterial, EmitterData, InitialTransform, ParticlesAsset, ParticlesAuthors,
    ParticlesDimension, SerializableAlphaMode, StandardParticleMaterial,
};
use crate::extract::{
    ExtractedEmitterData, ExtractedParticleSystem, extract_particle_systems,
};
use crate::runtime::{
    EmitterEntity, EmitterRuntime, ParticleBufferHandle, ParticleSystemRuntime, Particles3d,
    SimulationStep,
};
use crate::sort::{SortParams, plan_sort_dispatches};
use crate::textures::{CurveTextureCache, GradientTextureCache};

/// Emitters in the fixture scene: eight additive, two alpha-blended, every one
/// of them asking for `DrawOrder::ViewDepth`.
const ADDITIVE: usize = 8;
const BLENDED: usize = 2;
const EMITTERS: usize = ADDITIVE + BLENDED;

/// Which emitters are hidden — two additive and one blended, so neither fix's
/// effect can be mistaken for the other's.
const HIDDEN: [usize; 3] = [0, 1, 8];

/// Pool size per emitter. 160 rounds up to 256, so a full bitonic sort is 8
/// stages and `1+2+...+8 = 36` level dispatches.
const POOL: u32 = 160;
const FULL_SORT_LEVELS: usize = 36;

/// Simulation steps per emitter per frame, each of which is one compute
/// dispatch in `run_particle_compute_node`.
const STEPS: usize = 2;

/// One frame's measured work.
#[derive(Debug, PartialEq, Eq)]
struct Counts {
    emitters_extracted: usize,
    compute_dispatches: usize,
    sort_dispatches: usize,
}

/// Builds the fixture scene and runs one frame of real extraction.
///
/// `hide` selects which emitters get `InheritedVisibility::HIDDEN`; passing an
/// empty slice is how the "before the visibility gate" figure is produced,
/// since a world where nothing is hidden is a world where the gate cannot fire.
fn extract_frame(hide: &[usize]) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AssetPlugin::default())
        .add_plugins(ExtractPlugin::default());
    app.init_asset::<ParticlesAsset>();
    app.init_resource::<GradientTextureCache>();
    app.init_resource::<CurveTextureCache>();

    // `ExtractPlugin` leaves `update_schedule` unset, and the extract schedule
    // defers its commands to `RenderSystems::ExtractCommands` in `Render` — so
    // without this the extracted resource is built and never inserted.
    let render_app = app.get_sub_app_mut(RenderApp).unwrap();
    render_app.update_schedule = Some(Render.intern());
    render_app.add_systems(ExtractSchedule, extract_particle_systems);

    let emitters: Vec<EmitterData> = (0..EMITTERS)
        .map(|i| {
            let alpha_mode = if i < ADDITIVE {
                SerializableAlphaMode::Add
            } else {
                SerializableAlphaMode::Blend
            };
            let mut emitter = EmitterData::default();
            emitter.emission.particles_amount = POOL;
            emitter.draw_pass.draw_order = DrawOrder::ViewDepth;
            emitter.draw_pass.material = DrawPassMaterial::Standard(StandardParticleMaterial {
                alpha_mode,
                ..StandardParticleMaterial::default()
            });
            emitter
        })
        .collect();

    let handle = {
        let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
        assets.add(ParticlesAsset::new(
            "perf-fixture".into(),
            ParticlesDimension::D3,
            InitialTransform::default(),
            emitters,
            vec![],
            false,
            ParticlesAuthors::default(),
        ))
    };

    let system_entity = app
        .world_mut()
        .spawn((
            Particles3d(handle),
            ParticleSystemRuntime::default(),
            GlobalTransform::default(),
        ))
        .id();

    for index in 0..EMITTERS {
        let mut runtime = EmitterRuntime::new(index, Some(11));
        runtime.simulation_steps = (0..STEPS)
            .map(|s| SimulationStep {
                prev_system_time: s as f32 * 0.1,
                system_time: (s + 1) as f32 * 0.1,
                cycle: 0,
                delta_time: 0.1,
                clear_requested: false,
                trail_history_write_index: 0,
            })
            .collect();

        app.world_mut().spawn((
            EmitterEntity {
                parent_system: system_entity,
            },
            runtime,
            ParticleBufferHandle {
                particle_buffer: Handle::default(),
                indices_buffer: Handle::default(),
                sorted_particles_buffer: Handle::default(),
                emitter_uniforms_buffer: Handle::default(),
                max_particles: POOL,
                amount: POOL,
                trail_size: 1,
                trail_history_buffer: None,
                trail_history_frames: 0,
            },
            GlobalTransform::default(),
            if hide.contains(&index) {
                InheritedVisibility::HIDDEN
            } else {
                InheritedVisibility::VISIBLE
            },
        ));
    }

    app.update();
    app
}

/// Counts one extracted frame. `force_sorting` rewrites every emitter's
/// `needs_sorting` to `true` first, which is exactly the behaviour of the sort
/// pass before the blend gate existed.
fn count(app: &mut App, force_sorting: bool) -> Counts {
    let world = app.sub_app_mut(RenderApp).world_mut();
    let mut extracted = world.resource_mut::<ExtractedParticleSystem>();

    if force_sorting {
        for (_, data) in extracted.emitters.iter_mut() {
            data.needs_sorting = true;
        }
    }

    let refs: Vec<&ExtractedEmitterData> =
        extracted.emitters.iter().map(|(_, data)| data).collect();
    let mut uniform = DynamicUniformBuffer::<SortParams>::default();
    let planned = plan_sort_dispatches(&refs, &mut uniform);

    Counts {
        emitters_extracted: refs.len(),
        compute_dispatches: refs.iter().map(|d| d.uniform_steps.len()).sum(),
        sort_dispatches: planned.init_dispatches.len()
            + planned
                .sort_levels
                .iter()
                .map(Vec::len)
                .sum::<usize>()
            + planned.copy_dispatches.len(),
    }
}

/// The measurement, as one assertion per number so a regression names itself.
///
/// Ten emitters of 160 particles, all `ViewDepth`, eight additive and two
/// alpha-blended, three of them hidden:
///
/// | | before | after |
/// |---|---|---|
/// | emitters extracted | 10 | 7 |
/// | compute dispatches | 20 | 14 |
/// | sort dispatches | 380 | 50 |
///
/// The sort figure is where the crate's dispatch count lived: 380 of the 400
/// calls this scene used to make were sort dispatches, and 330 of them were
/// bitonic levels for emitters whose additive blend cannot tell the difference.
/// None of this is a frame-time claim.
#[test]
fn one_scene_measured_before_and_after_both_gates() {
    let mut before = extract_frame(&[]);
    let mut after = extract_frame(&HIDDEN);

    assert_eq!(
        count(&mut before, true),
        Counts {
            emitters_extracted: EMITTERS,
            compute_dispatches: EMITTERS * STEPS,
            sort_dispatches: EMITTERS + EMITTERS * FULL_SORT_LEVELS + EMITTERS,
        }
    );

    let visible = EMITTERS - HIDDEN.len();
    let visible_blended = BLENDED - 1;
    assert_eq!(
        count(&mut after, false),
        Counts {
            emitters_extracted: visible,
            compute_dispatches: visible * STEPS,
            sort_dispatches: visible + visible_blended * FULL_SORT_LEVELS + visible,
        }
    );
}

/// The same scene with the two gates separated, so neither fix can take credit
/// for the other's saving.
#[test]
fn each_gate_is_worth_what_it_is_worth_on_its_own() {
    // Visibility gate alone: three of ten emitters gone, and their compute
    // dispatches with them. The sort still runs on everything left.
    let mut visibility_only = extract_frame(&HIDDEN);
    let counts = count(&mut visibility_only, true);
    assert_eq!(counts.emitters_extracted, 7);
    assert_eq!(counts.compute_dispatches, 14);
    assert_eq!(counts.sort_dispatches, 7 + 7 * FULL_SORT_LEVELS + 7);

    // Blend gate alone: nothing hidden, so every emitter is still extracted and
    // still simulated — only the eight additive ones stop being sorted.
    let mut blend_only = extract_frame(&[]);
    let counts = count(&mut blend_only, false);
    assert_eq!(counts.emitters_extracted, 10);
    assert_eq!(counts.compute_dispatches, 20);
    assert_eq!(counts.sort_dispatches, 10 + BLENDED * FULL_SORT_LEVELS + 10);
}
