use bevy::{
    camera::visibility::NoFrustumCulling, light::NotShadowCaster, pbr::ExtendedMaterial,
    prelude::*, render::storage::ShaderBuffer,
};

use crate::{
    asset::{
        DRIVE_SLOT_COUNT, DrawPassMaterial, DrivenFx, EmitterData, EmitterTrail, FxSettings,
        FxUniform, ParticlesAsset,
    },
    drives::{EffectDrives, EmitterResolved},
    material::{
        FxDefs, ParticleEmitterUniforms, ParticleMaterialExtension, TRAIL_THICKNESS_CURVE_SAMPLES,
    },
    mesh::ParticleMeshCache,
    runtime::{
        ColliderEntity, CurrentMaterialConfig, CurrentMeshConfig, EditorMode, EmitterEntity,
        EmitterRuntime, ParticleBufferHandle, ParticleData, ParticleMaterial,
        ParticleMaterialHandle, ParticleMeshHandle, ParticleSystemRuntime, Particles3d,
        ParticlesCollider3D, SimulationStep, SubEmitterBufferHandle, TrailHistoryEntry,
    },
    textures::GradientTextureCache,
};

const MAX_FRAME_DELTA: f32 = 0.1;
const INACTIVE_GRACE_FACTOR: f32 = 1.2;
const MAX_TRAIL_HISTORY_FPS: f32 = 240.0;
fn create_trail_history_buffer(
    amount: u32,
    frames: u32,
    buffers: &mut Assets<ShaderBuffer>,
) -> Option<Handle<ShaderBuffer>> {
    if frames > 0 {
        let data = vec![TrailHistoryEntry::default(); (amount * frames) as usize];
        Some(buffers.add(ShaderBuffer::from(data)))
    } else {
        None
    }
}

fn compute_trail_history_frames(emitter: &EmitterData) -> u32 {
    let trail_size = emitter.trail_size();
    if trail_size <= 1 {
        return 0;
    }
    let effective_fps = if emitter.time.fixed_fps > 0 {
        emitter.time.fixed_fps as f32
    } else {
        MAX_TRAIL_HISTORY_FPS
    };
    let from_stretch = (emitter.trail.stretch_time * effective_fps).ceil() as u32;
    trail_size.max(from_stretch).max(2)
}

fn get_particle_asset<'a>(
    parent_system: Entity,
    particle_systems: &Query<&Particles3d>,
    assets: &'a Assets<ParticlesAsset>,
) -> Option<&'a ParticlesAsset> {
    let particle_system = particle_systems.get(parent_system).ok()?;
    assets.get(particle_system)
}

pub(crate) fn get_emitter_data<'a>(
    parent_system: Entity,
    emitter_index: usize,
    particle_systems: &Query<&Particles3d>,
    assets: &'a Assets<ParticlesAsset>,
) -> Option<&'a EmitterData> {
    get_particle_asset(parent_system, particle_systems, assets)
        .and_then(|asset| asset.emitters.get(emitter_index))
}

/// What the effect's drive list aims at one emitter, for the callers that
/// reach the asset through an `EmitterEntity` rather than iterating it.
///
/// An unreachable asset answers `DrivenFx::NONE` rather than panicking: both
/// callers have already bailed out of the frame by the time that could
/// happen, and the authored-only answer is the safe one regardless.
fn driven_fx_for(
    parent_system: Entity,
    emitter_index: usize,
    particle_systems: &Query<&Particles3d>,
    assets: &Assets<ParticlesAsset>,
) -> DrivenFx {
    get_particle_asset(parent_system, particle_systems, assets)
        .map(|asset| DrivenFx::for_emitter(&asset.drives, emitter_index))
        .unwrap_or(DrivenFx::NONE)
}

fn get_editor_assets_folders<'a>(
    parent_system: Entity,
    is_editor: bool,
    particle_systems: &Query<&Particles3d>,
    assets: &'a Assets<ParticlesAsset>,
) -> &'a [String] {
    if !is_editor {
        return &[];
    }
    get_particle_asset(parent_system, particle_systems, assets)
        .map(|a| a.sprinkles_editor.assets_folder.as_slice())
        .unwrap_or(&[])
}

pub fn update_particle_time(
    time: Res<Time>,
    assets: Res<Assets<ParticlesAsset>>,
    system_query: Query<(&Particles3d, &ParticleSystemRuntime)>,
    mut emitter_query: Query<(&EmitterEntity, &mut EmitterRuntime)>,
) {
    for (emitter, mut runtime) in emitter_query.iter_mut() {
        let Ok((particle_system, system_runtime)) = system_query.get(emitter.parent_system) else {
            continue;
        };

        let Some(asset) = assets.get(particle_system) else {
            continue;
        };

        let Some(emitter_data) = asset.emitters.get(runtime.emitter_index) else {
            continue;
        };

        runtime.simulation_steps.clear();

        let clear_requested = runtime.clear_requested;
        runtime.clear_requested = false;

        if runtime.inactive || system_runtime.paused {
            if clear_requested {
                let step = SimulationStep {
                    prev_system_time: runtime.system_time,
                    system_time: runtime.system_time,
                    cycle: runtime.cycle,
                    delta_time: 0.0,
                    clear_requested: true,
                    trail_history_write_index: runtime.trail_history_write_index,
                };
                runtime.simulation_steps.push(step);
            }
            continue;
        }

        let fixed_fps = emitter_data.time.fixed_fps;
        let total_duration = emitter_data.time.total_duration();

        if fixed_fps > 0 {
            let fixed_delta = 1.0 / fixed_fps as f32;
            let frame_delta = time.delta_secs().min(MAX_FRAME_DELTA);
            runtime.accumulated_delta += frame_delta;

            while runtime.accumulated_delta >= fixed_delta
                || (clear_requested && runtime.simulation_steps.is_empty())
            {
                runtime.accumulated_delta -= fixed_delta;

                let prev_time = runtime.system_time;
                runtime.system_time += fixed_delta;

                if runtime.system_time >= total_duration && total_duration > 0.0 {
                    runtime.system_time = runtime.system_time % total_duration;
                    runtime.cycle += 1;
                }

                let step = SimulationStep {
                    prev_system_time: prev_time,
                    system_time: runtime.system_time,
                    cycle: runtime.cycle,
                    delta_time: fixed_delta,
                    clear_requested: if runtime.simulation_steps.is_empty() {
                        clear_requested
                    } else {
                        false
                    },
                    trail_history_write_index: runtime.trail_history_write_index,
                };
                runtime.advance_trail_history();
                runtime.simulation_steps.push(step);
            }

            if !runtime.simulation_steps.is_empty() {
                runtime.prev_system_time = runtime.simulation_steps[0].prev_system_time;
            }
        } else {
            let delta = time.delta_secs();
            let prev_time = runtime.system_time;
            runtime.prev_system_time = runtime.system_time;
            runtime.system_time += delta;

            if runtime.system_time >= total_duration && total_duration > 0.0 {
                runtime.system_time = runtime.system_time % total_duration;
                runtime.cycle += 1;
            }

            let step = SimulationStep {
                prev_system_time: prev_time,
                system_time: runtime.system_time,
                cycle: runtime.cycle,
                delta_time: delta,
                clear_requested,
                trail_history_write_index: runtime.trail_history_write_index,
            };
            runtime.advance_trail_history();
            runtime.simulation_steps.push(step);
        }

        if emitter_data.time.one_shot && runtime.cycle > 0 && !runtime.one_shot_completed {
            runtime.set_emitting(false);
            runtime.one_shot_completed = true;
        }

        if !runtime.emitting {
            runtime.inactive_time += time.delta_secs();
            let grace = emitter_data.time.lifetime * INACTIVE_GRACE_FACTOR;
            if runtime.inactive_time > grace {
                runtime.inactive = true;
            }
        } else {
            runtime.inactive_time = 0.0;
        }
    }
}

fn transform_align_to_u32(align: Option<crate::asset::TransformAlign>) -> u32 {
    use crate::asset::TransformAlign;
    match align {
        None => 0,
        Some(TransformAlign::Billboard) => 1,
        Some(TransformAlign::YToVelocity) => 2,
        Some(TransformAlign::BillboardYToVelocity) => 3,
        Some(TransformAlign::BillboardFixedY) => 4,
    }
}

/// Resolves [`FxSettings::gradient_remap`] into a baked, sampleable texture
/// handle through `cache` -- the exact same [`GradientTextureCache`] (and
/// thus the exact same cache-key-keyed `get_or_create`) that an emitter's own
/// colour-over-lifetime gradient bakes through in
/// [`prepare_gradient_textures`](crate::textures::prepare_gradient_textures)
/// (`textures/baked.rs`). Two emitters whose `gradient_remap` gradients are
/// equal (`Gradient::cache_key`) share one baked handle rather than each
/// paying for their own texture.
///
/// Pulled out of `build_extension` as its own pure function -- taking only
/// plain, `Default`-constructible values (`GradientTextureCache`,
/// `Assets<Image>`), not `AssetServer` or any ECS scaffolding -- specifically
/// so a unit test can drive it directly. `build_extension` itself has no
/// test coverage (see `spawning.rs`'s test module, which only covers
/// `fold_render_slots`), so without this split a revert of the `.map()` call
/// below back to a hardcoded `None` would compile and pass every test in the
/// suite -- exactly the "wired to nothing" defect class this feature's
/// acceptance criterion exists to prevent.
fn resolve_gradient_texture(
    fx: &FxSettings,
    cache: &mut GradientTextureCache,
    images: &mut Assets<Image>,
) -> Option<Handle<Image>> {
    fx.gradient_remap
        .as_ref()
        .map(|gradient| cache.get_or_create(gradient, images))
}

/// Decides which FX `#ifdef` blocks this emitter's fragment shader compiles.
///
/// Split out of [`build_extension`] for the same reason
/// [`resolve_gradient_texture`] was: `build_extension` needs an
/// `AssetServer` and so has no test coverage, and this answer is the half
/// that a drive can change. Reverting any of the four `*_with_drives` calls
/// below to its authored-only sibling is exactly the regression Fix 1
/// repaired, and the tests at the bottom of this file fail on each.
///
/// `soft` and `gradient` take the authored-only predicates because no
/// [`EmitterProp`](crate::asset::EmitterProp) targets either -- there is no
/// drive that could switch them on.
fn build_fx_defs(fx: &FxSettings, driven: DrivenFx) -> FxDefs {
    FxDefs {
        scroll: fx.scroll_enabled_with_drives(driven),
        flow: fx.flow_enabled_with_drives(driven),
        erosion: fx.erosion_enabled_with_drives(driven),
        fresnel: fx.fresnel_enabled_with_drives(driven),
        soft: fx.soft_enabled(),
        gradient: fx.gradient_enabled(),
    }
}

/// Builds the [`ParticleMaterialExtension`] from a buffer pair plus an
/// authored [`FxSettings`]: the clamped GPU uniform, the loaded feature
/// textures, and the shader-def flags that pick which `#ifdef` blocks the
/// fragment compiles.
///
/// `driven` is what the effect's drive list aims at THIS emitter, and it
/// reaches both halves: a drive can switch a feature's block on
/// ([`build_fx_defs`]) and can lift a zero baseline out of the multiply
/// ([`FxUniform::from_settings`]). Either one alone leaves the drive inert.
fn build_extension(
    sorted_particles: Handle<ShaderBuffer>,
    emitter_uniforms: Handle<ShaderBuffer>,
    fx: &FxSettings,
    driven: DrivenFx,
    asset_server: &AssetServer,
    assets_folders: &[String],
    gradient_cache: &mut GradientTextureCache,
    images: &mut Assets<Image>,
) -> ParticleMaterialExtension {
    let flow_texture = fx
        .flow_texture
        .as_ref()
        .map(|t| t.load(asset_server, assets_folders));
    let erosion_texture = fx
        .erosion_texture
        .as_ref()
        .map(|t| t.load(asset_server, assets_folders));
    let gradient_texture = resolve_gradient_texture(fx, gradient_cache, images);

    ParticleMaterialExtension {
        sorted_particles,
        emitter_uniforms,
        fx: FxUniform::from_settings(fx, driven),
        flow_texture,
        erosion_texture,
        gradient_texture,
        defs: build_fx_defs(fx, driven),
    }
}

fn create_particle_material_from_config(
    config: &DrawPassMaterial,
    sorted_particles_buffer: Handle<ShaderBuffer>,
    emitter_uniforms_buffer: Handle<ShaderBuffer>,
    driven: DrivenFx,
    asset_server: &AssetServer,
    assets_folders: &[String],
    gradient_cache: &mut GradientTextureCache,
    images: &mut Assets<Image>,
) -> ParticleMaterial {
    let (base, fx_settings) = match config {
        DrawPassMaterial::Standard(mat) => (
            mat.to_standard_material(asset_server, assets_folders),
            &mat.fx,
        ),
        DrawPassMaterial::CustomShader { .. } => {
            todo!("custom shader support not yet implemented")
        }
    };

    ExtendedMaterial {
        base,
        extension: build_extension(
            sorted_particles_buffer,
            emitter_uniforms_buffer,
            fx_settings,
            driven,
            asset_server,
            assets_folders,
            gradient_cache,
            images,
        ),
    }
}

fn bake_thickness_curve(trail: &EmitterTrail) -> [f32; TRAIL_THICKNESS_CURVE_SAMPLES] {
    let mut samples = [1.0f32; TRAIL_THICKNESS_CURVE_SAMPLES];
    if let Some(ref curve) = trail.thickness_curve {
        for (i, sample) in samples.iter_mut().enumerate() {
            let t = i as f32 / (TRAIL_THICKNESS_CURVE_SAMPLES - 1) as f32;
            *sample = curve.sample(t);
        }
    }
    samples
}

pub fn setup_particle_systems(
    mut commands: Commands,
    query: Query<(Entity, &Particles3d, Has<EditorMode>), Without<ParticleSystemRuntime>>,
    assets: Res<Assets<ParticlesAsset>>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mesh_cache: ResMut<ParticleMeshCache>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut gradient_cache: ResMut<GradientTextureCache>,
    mut images: ResMut<Assets<Image>>,
) {
    for (system_entity, particle_system, is_editor) in query.iter() {
        let Some(asset) = assets.get(particle_system) else {
            continue;
        };

        if asset.emitters.is_empty() {
            continue;
        }

        let assets_folders = if is_editor {
            asset.sprinkles_editor.assets_folder.as_slice()
        } else {
            &[]
        };

        let initial_transform = asset.initial_transform.to_transform();

        commands
            .entity(system_entity)
            .queue_silenced(move |mut entity: EntityWorldMut| {
                entity
                    .insert(ParticleSystemRuntime::default())
                    .insert_if_new((initial_transform, Visibility::default()));
            });

        let mut emitter_entities: Vec<Entity> = Vec::new();

        for (emitter_index, emitter) in asset.emitters.iter().enumerate() {
            let amount = emitter.emission.particles_amount;
            let trail_size = emitter.trail_size();
            let total_slots = amount * trail_size;

            let particles: Vec<ParticleData> =
                (0..total_slots).map(|_| ParticleData::default()).collect();

            let mut particle_buffer = ShaderBuffer::from(particles.clone());
            particle_buffer.buffer_description.usage |=
                bevy::render::render_resource::BufferUsages::COPY_SRC;
            let particle_buffer_handle = buffers.add(particle_buffer);

            let indices: Vec<u32> = (0..total_slots).collect();
            let indices_buffer_handle = buffers.add(ShaderBuffer::from(indices));

            let sorted_particles_buffer_handle = buffers.add(ShaderBuffer::from(particles));

            let trail_history_frames = compute_trail_history_frames(emitter);
            let trail_history_buffer =
                create_trail_history_buffer(amount, trail_history_frames, &mut buffers);

            let emitter_uniforms = ParticleEmitterUniforms {
                emitter_transform: Mat4::IDENTITY,
                max_particles: total_slots,
                particle_flags: emitter.particle_flags.bits(),
                trail_size,
                transform_align: transform_align_to_u32(emitter.draw_pass.transform_align),
                ..default()
            };
            let mut emitter_uniforms_ssbo = ShaderBuffer::default();
            emitter_uniforms_ssbo.set_data(emitter_uniforms);
            let emitter_uniforms_buffer_handle = buffers.add(emitter_uniforms_ssbo);

            let current_mesh = emitter.draw_pass.mesh.clone();
            let current_material = emitter.draw_pass.material.clone();
            let shadow_caster = emitter.draw_pass.shadow_caster;

            let particle_mesh_handle = mesh_cache.get_or_create(&current_mesh, amount, &mut meshes);

            let material_handle = materials.add(create_particle_material_from_config(
                &current_material,
                sorted_particles_buffer_handle.clone(),
                emitter_uniforms_buffer_handle.clone(),
                DrivenFx::for_emitter(&asset.drives, emitter_index),
                &asset_server,
                assets_folders,
                &mut gradient_cache,
                &mut images,
            ));

            let mut runtime = EmitterRuntime::new(emitter_index, emitter.time.fixed_seed);
            runtime.trail_history_frames = trail_history_frames;

            let mut emitter_cmds = commands.spawn((
                EmitterEntity {
                    parent_system: system_entity,
                },
                runtime,
                ParticleBufferHandle {
                    particle_buffer: particle_buffer_handle.clone(),
                    indices_buffer: indices_buffer_handle.clone(),
                    sorted_particles_buffer: sorted_particles_buffer_handle.clone(),
                    emitter_uniforms_buffer: emitter_uniforms_buffer_handle,
                    max_particles: total_slots,
                    amount,
                    trail_size,
                    trail_history_buffer,
                    trail_history_frames,
                },
                Mesh3d(particle_mesh_handle.clone()),
                MeshMaterial3d(material_handle.clone()),
                CurrentMeshConfig(current_mesh),
                CurrentMaterialConfig(current_material),
                ParticleMeshHandle(particle_mesh_handle),
                ParticleMaterialHandle(material_handle),
                emitter.initial_transform.to_transform(),
                Visibility::default(),
                // Unconditional, not an opt-in: nothing in this crate ever
                // puts a particle-aware AABB on an emitter (see
                // `emitter_is_simulated`'s doc comment in `extract.rs`, which
                // reached this same conclusion for the same reason). Without
                // `NoFrustumCulling`, bevy culls the emitter against the
                // `Mesh3d`-derived AABB, but `build_particle_mesh` (`mesh.rs`)
                // stacks every particle's geometry at *identical*
                // origin-local positions -- each particle's real position
                // only exists in the vertex shader, read out of the sorted
                // particle buffer, so the AABB bounds the emitter's origin
                // quad, not the particle cloud. A wide `EmissionScaleX/Y/Z`
                // or a fast `InitialSpeed` sprays particles far outside that
                // box, and the whole effect vanishes the instant the origin
                // itself leaves the frustum while its particles still fill
                // the screen. The asset's `EmitterDrawPass::visibility_aabb`
                // is authored with the particle cloud in mind, but nothing
                // here ever applies it -- it is read only by the editor's
                // inspector and gizmo -- so there is no correct box to cull
                // against today. This is a *different* gate from
                // `InheritedVisibility`/`emitter_is_simulated`: that one
                // means "the author hid this, stop simulating it"; this one
                // means "there is no honest box to frustum-cull against, so
                // don't". A visible emitter must never be frustum-culled; a
                // hidden one must still be skipped by `emitter_is_simulated`
                // -- the two must keep working independently of each other.
                NoFrustumCulling,
            ));

            if !shadow_caster {
                emitter_cmds.insert(NotShadowCaster);
            }

            let emitter_entity = emitter_cmds.id();

            emitter_entities.push(emitter_entity);
            commands
                .entity(system_entity)
                .queue_silenced(move |mut entity: EntityWorldMut| {
                    entity.add_child(emitter_entity);
                });
        }

        for (emitter_index, emitter) in asset.emitters.iter().enumerate() {
            if let Some(ref sub_config) = emitter.sub_emitter {
                let target_index = sub_config.target_emitter;
                if target_index == emitter_index || target_index >= asset.emitters.len() {
                    continue;
                }

                let target_amount = asset.emitters[target_index].emission.particles_amount;
                let buffer_len = 4 + 12 * target_amount as usize;
                let mut initial_data = vec![0u32; buffer_len];
                initial_data[1] = target_amount;
                let mut buffer = ShaderBuffer::from(initial_data);
                buffer.buffer_description.usage |=
                    bevy::render::render_resource::BufferUsages::COPY_DST;

                let buffer_handle = buffers.add(buffer);
                let target_entity = emitter_entities[target_index];
                let parent_entity = emitter_entities[emitter_index];

                commands
                    .entity(parent_entity)
                    .insert(SubEmitterBufferHandle {
                        buffer: buffer_handle,
                        target_emitter: target_entity,
                        max_particles: target_amount,
                    });
            }
        }

        for (collider_index, collider_data) in asset.colliders.iter().enumerate() {
            let collider_entity = commands
                .spawn((
                    ColliderEntity {
                        parent_system: system_entity,
                        collider_index,
                    },
                    ParticlesCollider3D {
                        enabled: collider_data.enabled,
                        shape: collider_data.shape.clone(),
                    },
                    collider_data.initial_transform.to_transform(),
                    Name::new(collider_data.name.clone()),
                ))
                .id();

            commands
                .entity(system_entity)
                .queue_silenced(move |mut entity: EntityWorldMut| {
                    entity.add_child(collider_entity);
                });
        }
    }
}

pub fn cleanup_particle_entities(
    mut commands: Commands,
    mut removed_systems: RemovedComponents<Particles3d>,
    emitter_entities: Query<Entity, With<EmitterEntity>>,
    emitter_parent_query: Query<&EmitterEntity>,
    collider_entities: Query<(Entity, &ColliderEntity)>,
) {
    for removed_system in removed_systems.read() {
        for emitter_entity in emitter_entities.iter() {
            if let Ok(emitter) = emitter_parent_query.get(emitter_entity) {
                if emitter.parent_system == removed_system {
                    commands.entity(emitter_entity).despawn();
                }
            }
        }

        for (entity, collider) in collider_entities.iter() {
            if collider.parent_system == removed_system {
                commands.entity(entity).despawn();
            }
        }
    }
}

pub fn sync_collider_data(
    particle_systems: Query<&Particles3d>,
    assets: Res<Assets<ParticlesAsset>>,
    mut collider_query: Query<(&ColliderEntity, &mut ParticlesCollider3D, &mut Transform)>,
) {
    if !assets.is_changed() {
        return;
    }

    for (collider, mut collider3d, mut transform) in collider_query.iter_mut() {
        let Some(collider_data) =
            get_particle_asset(collider.parent_system, &particle_systems, &assets)
                .and_then(|asset| asset.colliders.get(collider.collider_index))
        else {
            continue;
        };

        collider3d.enabled = collider_data.enabled;
        collider3d.shape = collider_data.shape.clone();
        *transform = collider_data.initial_transform.to_transform();
    }
}

pub fn sync_particle_mesh(
    particle_systems: Query<&Particles3d>,
    mut emitter_query: Query<(
        &EmitterEntity,
        &EmitterRuntime,
        &ParticleBufferHandle,
        &mut CurrentMeshConfig,
        &mut ParticleMeshHandle,
        &mut Mesh3d,
    )>,
    assets: Res<Assets<ParticlesAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mesh_cache: ResMut<ParticleMeshCache>,
) {
    for (emitter, runtime, buffer_handle, mut current_config, mut mesh_handle, mut mesh3d) in
        emitter_query.iter_mut()
    {
        let Some(emitter_data) = get_emitter_data(
            emitter.parent_system,
            runtime.emitter_index,
            &particle_systems,
            &assets,
        ) else {
            continue;
        };

        let new_mesh = emitter_data.draw_pass.mesh.clone();

        if current_config.0 != new_mesh {
            let new_mesh_handle =
                mesh_cache.get_or_create(&new_mesh, buffer_handle.amount, &mut meshes);
            mesh3d.0 = new_mesh_handle.clone();
            current_config.0 = new_mesh;
            mesh_handle.0 = new_mesh_handle;
        }
    }
}

pub(crate) fn sync_particle_buffers(
    particle_systems: Query<&Particles3d>,
    editor_modes: Query<Has<EditorMode>>,
    mut emitter_query: Query<(
        &EmitterEntity,
        &mut EmitterRuntime,
        &mut ParticleBufferHandle,
        &mut ParticleMeshHandle,
        &mut Mesh3d,
        &mut CurrentMeshConfig,
        &mut ParticleMaterialHandle,
        &mut MeshMaterial3d<ParticleMaterial>,
        &mut CurrentMaterialConfig,
    )>,
    assets: Res<Assets<ParticlesAsset>>,
    asset_server: Res<AssetServer>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mesh_cache: ResMut<ParticleMeshCache>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut gradient_cache: ResMut<GradientTextureCache>,
    mut images: ResMut<Assets<Image>>,
) {
    for (
        emitter,
        mut runtime,
        mut buffer_handle,
        mut mesh_handle,
        mut mesh3d,
        mut current_config,
        mut material_handle,
        mut material3d,
        mut current_material_config,
    ) in emitter_query.iter_mut()
    {
        let Some(emitter_data) = get_emitter_data(
            emitter.parent_system,
            runtime.emitter_index,
            &particle_systems,
            &assets,
        ) else {
            continue;
        };

        let new_amount = emitter_data.emission.particles_amount;
        let new_trail_size = emitter_data.trail_size();
        let new_trail_history_frames = compute_trail_history_frames(emitter_data);

        if buffer_handle.amount == new_amount
            && buffer_handle.trail_size == new_trail_size
            && buffer_handle.trail_history_frames == new_trail_history_frames
        {
            continue;
        }

        let new_total = new_amount * new_trail_size;
        let particles: Vec<ParticleData> =
            (0..new_total).map(|_| ParticleData::default()).collect();

        let mut new_particle_buffer = ShaderBuffer::from(particles.clone());
        new_particle_buffer.buffer_description.usage |=
            bevy::render::render_resource::BufferUsages::COPY_SRC;
        let new_particle_buf = buffers.add(new_particle_buffer);
        let new_indices_buf = buffers.add(ShaderBuffer::from((0..new_total).collect::<Vec<u32>>()));
        let new_sorted_buf = buffers.add(ShaderBuffer::from(particles));

        let emitter_uniforms = ParticleEmitterUniforms {
            max_particles: new_total,
            particle_flags: emitter_data.particle_flags.bits(),
            trail_size: new_trail_size,
            transform_align: transform_align_to_u32(emitter_data.draw_pass.transform_align),
            trail_thickness_curve: bake_thickness_curve(&emitter_data.trail),
            ..default()
        };
        let mut emitter_uniforms_ssbo = ShaderBuffer::default();
        emitter_uniforms_ssbo.set_data(emitter_uniforms);
        let new_uniforms_buf = buffers.add(emitter_uniforms_ssbo);

        buffer_handle.particle_buffer = new_particle_buf;
        buffer_handle.indices_buffer = new_indices_buf;
        buffer_handle.sorted_particles_buffer = new_sorted_buf.clone();
        buffer_handle.emitter_uniforms_buffer = new_uniforms_buf.clone();
        buffer_handle.max_particles = new_total;
        buffer_handle.amount = new_amount;
        buffer_handle.trail_size = new_trail_size;

        buffer_handle.trail_history_buffer =
            create_trail_history_buffer(new_amount, new_trail_history_frames, &mut buffers);
        buffer_handle.trail_history_frames = new_trail_history_frames;
        runtime.trail_history_write_index = 0;
        runtime.trail_history_frames = new_trail_history_frames;

        let is_editor = editor_modes.get(emitter.parent_system).unwrap_or(false);
        let assets_folders =
            get_editor_assets_folders(emitter.parent_system, is_editor, &particle_systems, &assets);

        let new_material = materials.add(create_particle_material_from_config(
            &emitter_data.draw_pass.material,
            new_sorted_buf,
            new_uniforms_buf,
            driven_fx_for(
                emitter.parent_system,
                runtime.emitter_index,
                &particle_systems,
                &assets,
            ),
            &asset_server,
            assets_folders,
            &mut gradient_cache,
            &mut images,
        ));
        material3d.0 = new_material.clone();
        material_handle.0 = new_material;
        current_material_config.0 = emitter_data.draw_pass.material.clone();

        let new_mesh_handle =
            mesh_cache.get_or_create(&emitter_data.draw_pass.mesh, new_amount, &mut meshes);
        mesh3d.0 = new_mesh_handle.clone();
        mesh_handle.0 = new_mesh_handle.clone();
        current_config.0 = emitter_data.draw_pass.mesh.clone();
    }
}

/// Folds one emitter's resolved render-stage drives into the flat slot array
/// the GPU uniform carries.
///
/// This is the one place `Option<f32>` collapses to a plain `f32`: `None` --
/// no drive touched this slot, or there is no resolved state at all yet (the
/// entity has no `EffectDrives`, or this emitter has none) -- becomes the
/// identity `1.0`, the multiplier that leaves the emitter's authored value
/// unchanged. `Some(v)` becomes `v`, even when `v` is `0.0` -- a resolved zero
/// must reach the GPU as zero, not silently fall back to identity.
/// `resolve_drives`/`EffectDrives` keep the `None`/`Some(1.0)` distinction
/// intact all the way up to this boundary.
fn fold_render_slots(resolved: Option<&EmitterResolved>) -> [f32; DRIVE_SLOT_COUNT] {
    let mut slots = [1.0f32; DRIVE_SLOT_COUNT];
    if let Some(e) = resolved {
        for (i, v) in e.render.iter().enumerate() {
            if let Some(v) = v {
                slots[i] = *v;
            }
        }
    }
    slots
}

pub fn write_emitter_uniforms(
    particle_systems: Query<&Particles3d>,
    drives: Query<&EffectDrives>,
    emitter_query: Query<(
        &EmitterEntity,
        &EmitterRuntime,
        &ParticleBufferHandle,
        &GlobalTransform,
    )>,
    assets: Res<Assets<ParticlesAsset>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    for (emitter, runtime, buffer_handle, global_transform) in emitter_query.iter() {
        let Some(emitter_data) = get_emitter_data(
            emitter.parent_system,
            runtime.emitter_index,
            &particle_systems,
            &assets,
        ) else {
            continue;
        };

        let trail_size = emitter_data.trail_size();
        let trail_thickness_curve = bake_thickness_curve(&emitter_data.trail);

        let drive_slots = fold_render_slots(
            drives
                .get(emitter.parent_system)
                .ok()
                .and_then(|d| d.0.emitters.get(runtime.emitter_index)),
        );

        let uniforms = ParticleEmitterUniforms {
            emitter_transform: global_transform.to_matrix(),
            max_particles: buffer_handle.max_particles,
            particle_flags: emitter_data.particle_flags.bits(),
            use_local_coords: emitter_data.draw_pass.use_local_coords as u32,
            trail_size,
            transform_align: transform_align_to_u32(emitter_data.draw_pass.transform_align),
            trail_thickness_curve,
            drive_slots,
            max_screen_size: emitter_data.draw_pass.max_screen_size,
        };

        if let Some(mut buffer) = buffers.get_mut(&buffer_handle.emitter_uniforms_buffer) {
            buffer.set_data(uniforms);
        }
    }
}

pub fn sync_particle_material(
    particle_systems: Query<&Particles3d>,
    editor_modes: Query<Has<EditorMode>>,
    mut emitter_query: Query<(
        &EmitterEntity,
        &EmitterRuntime,
        &mut CurrentMaterialConfig,
        &mut ParticleMaterialHandle,
        &mut MeshMaterial3d<ParticleMaterial>,
    )>,
    assets: Res<Assets<ParticlesAsset>>,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<ParticleMaterial>>,
    mut gradient_cache: ResMut<GradientTextureCache>,
    mut images: ResMut<Assets<Image>>,
) {
    for (emitter, runtime, mut current_config, mut material_handle, mut material3d) in
        emitter_query.iter_mut()
    {
        let Some(emitter_data) = get_emitter_data(
            emitter.parent_system,
            runtime.emitter_index,
            &particle_systems,
            &assets,
        ) else {
            continue;
        };

        let new_material = emitter_data.draw_pass.material.clone();

        if current_config.0.cache_key() != new_material.cache_key() {
            let (sorted_particles_handle, emitter_uniforms_handle) = {
                let Some(existing_material) = materials.get(&material_handle.0) else {
                    continue;
                };
                (
                    existing_material.extension.sorted_particles.clone(),
                    existing_material.extension.emitter_uniforms.clone(),
                )
            };

            let is_editor = editor_modes.get(emitter.parent_system).unwrap_or(false);
            let assets_folders = get_editor_assets_folders(
                emitter.parent_system,
                is_editor,
                &particle_systems,
                &assets,
            );

            let new_material_handle = materials.add(create_particle_material_from_config(
                &new_material,
                sorted_particles_handle,
                emitter_uniforms_handle,
                driven_fx_for(
                    emitter.parent_system,
                    runtime.emitter_index,
                    &particle_systems,
                    &assets,
                ),
                &asset_server,
                assets_folders,
                &mut gradient_cache,
                &mut images,
            ));

            material3d.0 = new_material_handle.clone();
            current_config.0 = new_material;
            material_handle.0 = new_material_handle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real `setup_particle_systems`, not a hand-built stand-in --
    /// the point of this test is to pin what that system actually attaches
    /// to a spawned emitter entity, not a fixture that only claims to match
    /// it. Every resource here is `Assets<T>`/cache storage, so nothing
    /// needs a render device to run headlessly.
    fn app_with_particle_setup() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_asset::<Mesh>();
        app.init_asset::<ShaderBuffer>();
        app.init_asset::<ParticleMaterial>();
        app.init_asset::<Image>();
        app.init_resource::<ParticleMeshCache>();
        app.init_resource::<GradientTextureCache>();
        app.add_systems(Update, setup_particle_systems);
        app
    }

    /// The bug this pins: nothing in this crate inserts `NoFrustumCulling`
    /// or a particle-aware `Aabb` on an emitter, so bevy culls it against
    /// the mesh-derived AABB -- which bounds the emitter's *origin quad*,
    /// not the particle cloud (see the doc comment on the `NoFrustumCulling`
    /// insert in `setup_particle_systems`). Mutating that insert away must
    /// fail this test.
    #[test]
    fn a_spawned_emitter_carries_no_frustum_culling() {
        let mut app = app_with_particle_setup();

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let a = ParticlesAsset::new(
                "t".into(),
                crate::asset::ParticlesDimension::D3,
                Default::default(),
                vec![EmitterData::default()],
                vec![],
                false,
                crate::asset::ParticlesAuthors::default(),
            );
            assets.add(a)
        };
        app.world_mut().spawn(Particles3d(handle));

        app.update();

        let emitter = app
            .world_mut()
            .query::<(Entity, &EmitterEntity)>()
            .iter(app.world())
            .next()
            .map(|(e, _)| e)
            .expect("setup_particle_systems must spawn an emitter child");

        assert!(
            app.world().get::<NoFrustumCulling>(emitter).is_some(),
            "a spawned emitter must never be frustum-culled -- there is no \
             box today that bounds its actual particle cloud"
        );
    }

    #[test]
    fn no_resolved_state_yields_identity_everywhere() {
        // No `EffectDrives` on the entity, or the entity's emitter index has
        // no `EmitterResolved` -- `write_emitter_uniforms` passes `None` here
        // in both cases.
        assert_eq!(fold_render_slots(None), [1.0; DRIVE_SLOT_COUNT]);
    }

    #[test]
    fn an_untouched_slot_is_identity() {
        let resolved = EmitterResolved::default(); // render: [None; DRIVE_SLOT_COUNT]
        assert_eq!(fold_render_slots(Some(&resolved)), [1.0; DRIVE_SLOT_COUNT]);
    }

    #[test]
    fn a_resolved_zero_reaches_the_slot_as_zero_not_identity() {
        // The case that catches a lazy `unwrap_or(1.0)` written as
        // `filter(|v| *v != 0.0)`: a drive that genuinely computed 0.0 must
        // not be mistaken for "untouched" and folded back to identity.
        let mut resolved = EmitterResolved::default();
        resolved.render[0] = Some(0.0);
        let slots = fold_render_slots(Some(&resolved));
        assert_eq!(slots[0], 0.0);
        for (i, s) in slots.iter().enumerate().skip(1) {
            assert_eq!(*s, 1.0, "slot {i} must stay untouched");
        }
    }

    #[test]
    fn a_resolved_non_default_value_passes_through() {
        let mut resolved = EmitterResolved::default();
        resolved.render[2] = Some(4.0);
        let slots = fold_render_slots(Some(&resolved));
        assert_eq!(slots[2], 4.0);
    }

    fn one_stop_gradient(r: f32) -> crate::asset::Gradient {
        crate::asset::Gradient {
            stops: vec![crate::asset::GradientStop {
                color: [r, 0.0, 0.0, 1.0],
                position: 0.0,
            }],
            ..Default::default()
        }
    }

    /// This is the one that catches a revert of `resolve_gradient_texture`'s
    /// `.map()` call back to a hardcoded `None`: an authored
    /// `gradient_remap` must actually resolve to a baked handle, not just
    /// have a WGSL reader waiting for one.
    #[test]
    fn an_authored_gradient_resolves_to_a_baked_handle() {
        let fx = FxSettings {
            gradient_remap: Some(one_stop_gradient(1.0)),
            ..Default::default()
        };
        let mut cache = GradientTextureCache::default();
        let mut images = Assets::<Image>::default();
        let handle = resolve_gradient_texture(&fx, &mut cache, &mut images);
        assert!(handle.is_some(), "an authored gradient must bake a texture");
    }

    #[test]
    fn no_gradient_remap_resolves_to_no_texture() {
        let fx = FxSettings::default();
        let mut cache = GradientTextureCache::default();
        let mut images = Assets::<Image>::default();
        let handle = resolve_gradient_texture(&fx, &mut cache, &mut images);
        assert!(
            handle.is_none(),
            "an unauthored effect must not bake anything"
        );
    }

    #[test]
    fn two_equal_gradients_resolve_to_the_same_baked_handle() {
        let fx_a = FxSettings {
            gradient_remap: Some(one_stop_gradient(0.5)),
            ..Default::default()
        };
        let fx_b = FxSettings {
            gradient_remap: Some(one_stop_gradient(0.5)),
            ..Default::default()
        };
        let mut cache = GradientTextureCache::default();
        let mut images = Assets::<Image>::default();
        let handle_a = resolve_gradient_texture(&fx_a, &mut cache, &mut images).unwrap();
        let handle_b = resolve_gradient_texture(&fx_b, &mut cache, &mut images).unwrap();
        assert_eq!(
            handle_a.id(),
            handle_b.id(),
            "two emitters authoring an equal gradient must share one baked texture"
        );
    }

    // --- Drive-aware shader-def gating -------------------------------
    //
    // A def that is never pushed means the `#ifdef` block never compiles, so
    // the feature is absent from the shader entirely -- no uniform value and
    // no drive can switch it back on at draw time. These pin the first half
    // of Fix 1; `asset::fx`'s tests pin the second (the zero baseline).

    fn driven(prop: crate::asset::EmitterProp) -> DrivenFx {
        DrivenFx::for_emitter(
            &[crate::asset::Drive {
                variable: crate::asset::VariableId(0),
                target: crate::asset::DriveTarget::Emitter { index: 0, prop },
                curve: crate::asset::CurveTexture::default(),
                output: crate::asset::Range { min: 0.0, max: 1.0 },
                op: crate::asset::DriveOp::Multiply,
                muted: false,
            }],
            0,
        )
    }

    fn a_texture() -> Option<crate::TextureRef> {
        Some(crate::TextureRef::Asset("noise.png".into()))
    }

    #[test]
    fn a_drive_on_a_zero_authored_scalar_compiles_its_block_in() {
        use crate::asset::EmitterProp::*;
        let bare = FxSettings::default();
        let with_textures = FxSettings {
            flow_texture: a_texture(),
            erosion_texture: a_texture(),
            ..Default::default()
        };

        assert!(
            build_fx_defs(&bare, driven(ScrollU)).scroll,
            "a ScrollU drive must push FX_SCROLL"
        );
        assert!(
            build_fx_defs(&bare, driven(ScrollV)).scroll,
            "a ScrollV drive must push FX_SCROLL"
        );
        assert!(
            build_fx_defs(&with_textures, driven(FlowStrength)).flow,
            "a FlowStrength drive must push FX_FLOW"
        );
        assert!(
            build_fx_defs(&with_textures, driven(ErosionThreshold)).erosion,
            "an ErosionThreshold drive must push FX_EROSION"
        );
        assert!(
            build_fx_defs(&bare, driven(FresnelPower)).fresnel,
            "a FresnelPower drive must push FX_FRESNEL"
        );
    }

    #[test]
    fn an_undriven_zero_authored_scalar_still_compiles_nothing() {
        // Without this, the test above would pass for the wrong reason (defs
        // pushed unconditionally) and every stock effect would pay for four
        // shader blocks it never uses.
        let defs = build_fx_defs(
            &FxSettings {
                flow_texture: a_texture(),
                erosion_texture: a_texture(),
                ..Default::default()
            },
            DrivenFx::NONE,
        );
        assert_eq!(defs, FxDefs::default());
    }

    #[test]
    fn a_flow_or_erosion_drive_with_no_texture_compiles_nothing() {
        // A drive satisfies only the SCALAR half of these two gates: both
        // blocks sample a texture binding, and with nothing bound there is
        // no pattern for the drive to scale.
        use crate::asset::EmitterProp::*;
        let bare = FxSettings::default();
        assert!(
            !build_fx_defs(&bare, driven(FlowStrength)).flow,
            "no flow texture means nothing for FX_FLOW to sample"
        );
        assert!(
            !build_fx_defs(&bare, driven(ErosionThreshold)).erosion,
            "no erosion texture means nothing for FX_EROSION to sample"
        );
    }

    #[test]
    fn a_drive_never_compiles_the_two_undrivable_blocks() {
        // `soft` and `gradient` have no `EmitterProp` targeting them, so
        // they must stay on the authored-only predicates -- a drive-aware
        // gate there could only ever be dead code that misleads a reader.
        let defs = build_fx_defs(
            &FxSettings::default(),
            DrivenFx {
                scroll_u: true,
                scroll_v: true,
                flow_strength: true,
                erosion_threshold: true,
                fresnel_power: true,
            },
        );
        assert!(!defs.soft);
        assert!(!defs.gradient);
    }
}
