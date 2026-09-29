use bevy::{
    core_pipeline::schedule::camera_driver,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
            CachedComputePipelineId, CachedPipelineState, ComputePassDescriptor,
            ComputePipelineDescriptor, DynamicUniformBuffer, PipelineCache, ShaderStages,
            ShaderType,
            binding_types::{storage_buffer, uniform_buffer},
        },
        renderer::{RenderContext, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue},
        storage::GpuShaderBuffer,
    },
};
use std::borrow::Cow;

use crate::compute::ParticleComputeLabel;
use crate::extract::{ExtractedEmitterData, ExtractedParticleSystem};
use crate::runtime::ParticleData;

const SHADER_ASSET_PATH: &str = "embedded://bevy_sprinkles/shaders/particle_sort.wgsl";
const WORKGROUP_SIZE: u32 = 256;

#[derive(Clone, Copy, Default, ShaderType)]
pub struct SortParams {
    pub amount: u32,
    pub draw_order: u32,
    pub stage: u32,
    pub step: u32,
    pub camera_position: Vec3,
    pub _pad1: f32,
    pub camera_forward: Vec3,
    pub _pad2: f32,
    pub emitter_transform: Mat4,
    pub trail_size: u32,
    pub _trail_pad0: u32,
    pub _trail_pad1: u32,
    pub _trail_pad2: u32,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub struct ParticleSortLabel;

#[derive(Resource)]
pub struct ParticleSortPipeline {
    pub bind_group_layout: BindGroupLayoutDescriptor,
    pub init_pipeline: CachedComputePipelineId,
    pub sort_pipeline: CachedComputePipelineId,
    pub copy_pipeline: CachedComputePipelineId,
}

pub fn init_particle_sort_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let bind_group_layout = BindGroupLayoutDescriptor::new(
        "ParticleSortBindGroup",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<SortParams>(true),
                storage_buffer::<ParticleData>(false),
                storage_buffer::<u32>(false),
                storage_buffer::<ParticleData>(false),
            ),
        ),
    );

    let shader = asset_server.load(SHADER_ASSET_PATH);

    let queue_pipeline = |label: &'static str, entry: &'static str| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(label.into()),
            layout: vec![bind_group_layout.clone()],
            shader: shader.clone(),
            entry_point: Some(Cow::from(entry)),
            ..default()
        })
    };

    let init_pipeline = queue_pipeline("particle_sort_init_pipeline", "init_indices");
    let sort_pipeline = queue_pipeline("particle_sort_pipeline", "sort");
    let copy_pipeline = queue_pipeline("particle_sort_copy_pipeline", "copy_sorted");

    commands.insert_resource(ParticleSortPipeline {
        bind_group_layout,
        init_pipeline,
        sort_pipeline,
        copy_pipeline,
    });
}

pub(crate) struct SortDispatch {
    emitter_index: usize,
    dynamic_offset: u32,
    workgroups: u32,
}

#[derive(Resource, Default)]
pub struct ParticleSortBindGroups {
    bind_groups: Vec<BindGroup>,
    pub(crate) init_dispatches: Vec<SortDispatch>,
    pub(crate) sort_levels: Vec<Vec<SortDispatch>>,
    pub(crate) copy_dispatches: Vec<SortDispatch>,
}

/// Builds the dispatch plan for one frame's worth of emitters, and nothing
/// else.
///
/// Split out of [`prepare_particle_sort_bind_groups`] because everything here
/// is arithmetic — `DynamicUniformBuffer::push` writes into a plain `Vec<u8>`
/// scratch and touches no GPU
/// (`bevy_render-0.19.0/src/render_resource/uniform_buffer.rs:231`), and the
/// dispatch counts are the only honest measure of this file's cost that a
/// machine with no adapter can take. The caller keeps the half that does need
/// a device: resolving buffers and creating bind groups.
///
/// `emitters` must already be filtered to those whose three storage buffers
/// resolved, in the same order as the caller's buffer list, because
/// `SortDispatch::emitter_index` indexes into that list.
pub(crate) fn plan_sort_dispatches(
    emitters: &[&ExtractedEmitterData],
    dynamic_uniform: &mut DynamicUniformBuffer<SortParams>,
) -> ParticleSortBindGroups {
    let mut result = ParticleSortBindGroups::default();

    for (emitter_idx, emitter_data) in emitters.iter().enumerate() {
        let trail_size = emitter_data.trail_size;
        let total_slots = emitter_data.amount * trail_size;
        let group_count = emitter_data.amount;
        let group_workgroups = (group_count + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
        let total_workgroups = (total_slots + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

        let base_params = SortParams {
            amount: total_slots,
            draw_order: emitter_data.draw_order,
            stage: 0,
            step: 0,
            camera_position: Vec3::from_array(emitter_data.camera_position),
            _pad1: 0.0,
            camera_forward: Vec3::from_array(emitter_data.camera_forward),
            _pad2: 0.0,
            emitter_transform: emitter_data.emitter_transform,
            trail_size,
            _trail_pad0: 0,
            _trail_pad1: 0,
            _trail_pad2: 0,
        };

        let init_offset = dynamic_uniform.push(&base_params);
        result.init_dispatches.push(SortDispatch {
            emitter_index: emitter_idx,
            dynamic_offset: init_offset,
            workgroups: group_workgroups,
        });

        // Two independent reasons to skip the bitonic pass, and both leave a
        // buffer the draw path can still read: `init_indices` has just written
        // the identity permutation, and the `copy_sorted` dispatch below
        // expands it into `sorted_particles_buffer`, which is the buffer the
        // material binds (`spawning.rs` hands it to
        // `create_particle_material_from_config`). Dropping the levels alone
        // therefore costs emission order, not correctness — dropping the init
        // or the copy with them would hand the renderer stale or uninitialised
        // indices.
        //
        // `draw_order == 0` is `DrawOrder::Index`, whose sort key is the
        // particle index: the identity the init pass already wrote.
        //
        // `!needs_sorting` is the new one. Depth order is only visible through
        // an order-dependent blend, and most particle effects are not blended
        // that way — see `DrawPassMaterial::needs_sorting`. **The trade**: an
        // additive, multiplied, opaque or masked emitter that asks for
        // `Lifetime`, `ReverseLifetime` or `ViewDepth` no longer gets it, and
        // draws in emission order instead. For those blends the pixels come
        // out the same, which is the whole argument — but if a future
        // per-particle effect ever *reads* draw position (an OIT tail, a
        // feedback pass, anything order-sensitive that is not the blend
        // equation), this skip becomes wrong and the predicate, not this
        // branch, is where that gets fixed.
        if emitter_data.draw_order != 0 && emitter_data.needs_sorting {
            let n = group_count.next_power_of_two();
            let num_stages = (n as f32).log2().ceil() as u32;

            let mut level = 0usize;
            for stage in 0..num_stages {
                for step_val in (0..=stage).rev() {
                    let offset = dynamic_uniform.push(&SortParams {
                        stage,
                        step: step_val,
                        ..base_params
                    });

                    while result.sort_levels.len() <= level {
                        result.sort_levels.push(Vec::new());
                    }

                    result.sort_levels[level].push(SortDispatch {
                        emitter_index: emitter_idx,
                        dynamic_offset: offset,
                        workgroups: group_workgroups,
                    });

                    level += 1;
                }
            }
        }

        let copy_offset = dynamic_uniform.push(&base_params);
        result.copy_dispatches.push(SortDispatch {
            emitter_index: emitter_idx,
            dynamic_offset: copy_offset,
            workgroups: total_workgroups,
        });
    }

    result
}

pub fn prepare_particle_sort_bind_groups(
    mut commands: Commands,
    pipeline: Res<ParticleSortPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    extracted_systems: Res<ExtractedParticleSystem>,
    gpu_storage_buffers: Res<RenderAssets<GpuShaderBuffer>>,
) {
    let mut ready: Vec<&ExtractedEmitterData> = Vec::new();
    let mut emitter_buffers: Vec<(Buffer, Buffer, Buffer)> = Vec::new();

    for (_entity, emitter_data) in &extracted_systems.emitters {
        let Some(particle_buf) = gpu_storage_buffers.get(&emitter_data.particle_buffer_handle)
        else {
            continue;
        };
        let Some(indices_buf) = gpu_storage_buffers.get(&emitter_data.indices_buffer_handle) else {
            continue;
        };
        let Some(sorted_buf) =
            gpu_storage_buffers.get(&emitter_data.sorted_particles_buffer_handle)
        else {
            continue;
        };

        emitter_buffers.push((
            particle_buf.buffer.clone(),
            indices_buf.buffer.clone(),
            sorted_buf.buffer.clone(),
        ));
        ready.push(emitter_data);
    }

    let mut dynamic_uniform = DynamicUniformBuffer::<SortParams>::default();
    let mut result = plan_sort_dispatches(&ready, &mut dynamic_uniform);

    dynamic_uniform.write_buffer(&render_device, &render_queue);

    if let Some(uniform_binding) = dynamic_uniform.binding() {
        let bind_group_layout = pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout);

        for (particle_buf, indices_buf, sorted_buf) in &emitter_buffers {
            let bind_group = render_device.create_bind_group(
                Some("particle_sort_bind_group"),
                &bind_group_layout,
                &BindGroupEntries::sequential((
                    uniform_binding.clone(),
                    particle_buf.as_entire_binding(),
                    indices_buf.as_entire_binding(),
                    sorted_buf.as_entire_binding(),
                )),
            );
            result.bind_groups.push(bind_group);
        }
    }

    commands.insert_resource(result);
}

pub fn run_particle_sort_node(
    pipeline: Res<ParticleSortPipeline>,
    pipeline_cache: Res<PipelineCache>,
    sort_bind_groups: Res<ParticleSortBindGroups>,
    mut ctx: RenderContext,
) {
    let is_ready = |id| {
        matches!(
            pipeline_cache.get_compute_pipeline_state(id),
            CachedPipelineState::Ok(_)
        )
    };

    if !(is_ready(pipeline.init_pipeline)
        && is_ready(pipeline.sort_pipeline)
        && is_ready(pipeline.copy_pipeline))
    {
        return;
    }

    let Some(init_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.init_pipeline) else {
        return;
    };

    let Some(sort_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.sort_pipeline) else {
        return;
    };

    let Some(copy_pipeline) = pipeline_cache.get_compute_pipeline(pipeline.copy_pipeline) else {
        return;
    };

    if sort_bind_groups.bind_groups.is_empty() {
        return;
    }

    let mut run_pass = |label, pipeline, dispatches: &[SortDispatch]| {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some(label),
                ..default()
            });
        pass.set_pipeline(pipeline);
        for dispatch in dispatches {
            pass.set_bind_group(
                0,
                &sort_bind_groups.bind_groups[dispatch.emitter_index],
                &[dispatch.dynamic_offset],
            );
            pass.dispatch_workgroups(dispatch.workgroups, 1, 1);
        }
    };

    run_pass(
        "particle_sort_init_pass",
        init_pipeline,
        &sort_bind_groups.init_dispatches,
    );

    for level in &sort_bind_groups.sort_levels {
        run_pass("particle_sort_pass", sort_pipeline, level);
    }

    run_pass(
        "particle_sort_copy_pass",
        copy_pipeline,
        &sort_bind_groups.copy_dispatches,
    );
}

pub struct ParticleSortPlugin;

impl Plugin for ParticleSortPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<ParticleSortBindGroups>()
            .add_systems(RenderStartup, init_particle_sort_pipeline)
            .add_systems(
                Render,
                prepare_particle_sort_bind_groups.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(
                RenderGraph,
                run_particle_sort_node
                    .in_set(ParticleSortLabel)
                    .in_set(RenderGraphSystems::Render)
                    .after(ParticleComputeLabel)
                    .before(camera_driver),
            );
    }
}

/// Dispatch-count proof for the blend-mode gate.
///
/// [`plan_sort_dispatches`] is the whole of this file's per-frame cost decision
/// and needs no GPU, so these tests count the real thing rather than a model of
/// it. What they report is dispatches eliminated — a proxy for frame time, not
/// a measurement of it; no test here can time a frame.
#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use crate::asset::{DrawPassMaterial, SerializableAlphaMode, StandardParticleMaterial};

    /// The pool size the whole file's cost scales with. 160 particles round up
    /// to 256, so a full bitonic pass is 8 stages and `1+2+...+8 = 36` levels.
    const POOL: u32 = 160;
    const FULL_SORT_LEVELS: usize = 36;

    /// One emitter's extracted data, carrying only the fields
    /// [`plan_sort_dispatches`] reads. Everything else is the cheapest value
    /// of its type; none of it is looked at here.
    fn emitter(draw_order: u32, needs_sorting: bool) -> ExtractedEmitterData {
        ExtractedEmitterData {
            uniform_steps: Vec::new(),
            particle_buffer_handle: Handle::default(),
            indices_buffer_handle: Handle::default(),
            sorted_particles_buffer_handle: Handle::default(),
            amount: POOL,
            draw_order,
            needs_sorting,
            camera_position: [0.0; 3],
            camera_forward: [0.0, 0.0, -1.0],
            emitter_transform: Mat4::IDENTITY,
            gradient_texture_handle: None,
            color_over_lifetime_texture_handle: None,
            scale_over_lifetime_texture_handle: None,
            alpha_over_lifetime_texture_handle: None,
            emission_over_lifetime_texture_handle: None,
            turbulence_influence_over_lifetime_texture_handle: None,
            radial_velocity_curve_texture_handle: None,
            angle_over_lifetime_texture_handle: None,
            angular_velocity_curve_texture_handle: None,
            orbit_velocity_curve_texture_handle: None,
            directional_velocity_curve_texture_handle: None,
            is_sub_emitter_target: false,
            emission_buffer_handle: None,
            source_buffer_handle: None,
            trail_size: 1,
            trail_history_buffer_handle: None,
        }
    }

    /// `(init dispatches, bitonic level dispatches, copy dispatches)`.
    fn plan(emitters: &[ExtractedEmitterData]) -> (usize, usize, usize) {
        let refs: Vec<&ExtractedEmitterData> = emitters.iter().collect();
        let mut uniform = DynamicUniformBuffer::<SortParams>::default();
        let planned = plan_sort_dispatches(&refs, &mut uniform);
        (
            planned.init_dispatches.len(),
            planned.sort_levels.iter().map(Vec::len).sum(),
            planned.copy_dispatches.len(),
        )
    }

    fn standard(alpha_mode: SerializableAlphaMode) -> DrawPassMaterial {
        DrawPassMaterial::Standard(StandardParticleMaterial {
            alpha_mode,
            ..StandardParticleMaterial::default()
        })
    }

    #[test]
    fn only_the_blends_whose_equation_is_order_dependent_ask_to_be_sorted() {
        for mode in [
            SerializableAlphaMode::Blend,
            SerializableAlphaMode::Premultiplied,
            SerializableAlphaMode::AlphaToCoverage,
        ] {
            assert!(mode.order_dependent(), "{mode:?} must still be sorted");
            assert!(standard(mode).needs_sorting());
        }

        for mode in [
            SerializableAlphaMode::Opaque,
            SerializableAlphaMode::Mask { cutoff: 0.5 },
            SerializableAlphaMode::Add,
            SerializableAlphaMode::Multiply,
        ] {
            assert!(!mode.order_dependent(), "{mode:?} must not be sorted");
            assert!(!standard(mode).needs_sorting());
        }
    }

    #[test]
    fn a_custom_shaders_unknown_blend_is_sorted_rather_than_guessed_at() {
        assert!(
            DrawPassMaterial::CustomShader {
                vertex_shader: None,
                fragment_shader: None,
            }
            .needs_sorting()
        );
    }

    /// The integration hazard, pinned. Skipping the bitonic levels must not
    /// skip the init that writes the identity permutation or the copy that
    /// expands it into `sorted_particles_buffer` — that buffer is what the
    /// material binds, and without those two it would hold uninitialised
    /// indices or last frame's particles.
    #[test]
    fn an_unsorted_emitter_still_gets_the_init_and_copy_the_draw_path_reads() {
        assert_eq!(plan(&[emitter(3, false)]), (1, 0, 1));
    }

    #[test]
    fn an_order_dependent_emitter_still_gets_the_whole_bitonic_pass() {
        assert_eq!(plan(&[emitter(3, true)]), (1, FULL_SORT_LEVELS, 1));
    }

    /// `DrawOrder::Index` was already exempt before this change, and stays so
    /// even when the blend is order-dependent: its sort key is the particle
    /// index, which is exactly what `init_indices` just wrote.
    #[test]
    fn draw_order_index_is_still_exempt_whatever_the_blend_is() {
        assert_eq!(plan(&[emitter(0, true)]), (1, 0, 1));
    }

    /// The headline measurement for the blend gate, on a scene shaped like the
    /// ones that drop frames: ten `ViewDepth` emitters of 160 particles, eight
    /// of them additive and two alpha-blended.
    ///
    /// "Before" is not a remembered number — it is the same planner run with
    /// every emitter marked order-dependent, which is precisely what this file
    /// did when its only gate was `draw_order != 0`.
    #[test]
    fn eight_additive_emitters_in_ten_drop_the_frames_sort_dispatches_by_288() {
        let before: Vec<ExtractedEmitterData> =
            (0..10).map(|_| emitter(3, true)).collect();
        let after: Vec<ExtractedEmitterData> = (0..10)
            .map(|i| emitter(3, i >= 8))
            .collect();

        let (init_before, levels_before, copy_before) = plan(&before);
        let (init_after, levels_after, copy_after) = plan(&after);

        assert_eq!(
            (init_before, levels_before, copy_before),
            (10, 10 * FULL_SORT_LEVELS, 10)
        );
        assert_eq!(
            (init_after, levels_after, copy_after),
            (10, 2 * FULL_SORT_LEVELS, 10)
        );

        let total = |(i, l, c): (usize, usize, usize)| i + l + c;
        assert_eq!(total((init_before, levels_before, copy_before)), 380);
        assert_eq!(total((init_after, levels_after, copy_after)), 92);
    }
}
