use bevy::{
    prelude::*,
    render::{Extract, render_resource::ShaderType, storage::ShaderBuffer},
};
use bytemuck::{Pod, Zeroable};

use crate::{
    asset::{
        AnimatedVelocity, CurveTexture, DrawOrder, EmissionShape, EmitterCollisionMode,
        EmitterData, ParticleFlags, ParticlesAsset, ParticlesColliderShape3D, SolidOrGradientColor,
        SubEmitterMode,
    },
    runtime::{
        EmitterEntity, EmitterRuntime, ParticleBufferHandle, ParticleSystemRuntime, Particles3d,
        ParticlesCollider3D, SubEmitterBufferHandle, compute_phase, is_past_delay,
    },
    textures::{CurveTextureCache, GradientTextureCache},
};

pub const EMISSION_SHAPE_POINT: u32 = 0;
pub const EMISSION_SHAPE_SPHERE: u32 = 1;
pub const EMISSION_SHAPE_SPHERE_SURFACE: u32 = 2;
pub const EMISSION_SHAPE_BOX: u32 = 3;
pub const EMISSION_SHAPE_RING: u32 = 4;

pub const COLLIDER_TYPE_SPHERE: u32 = 0;
pub const COLLIDER_TYPE_BOX: u32 = 1;
pub const MAX_COLLIDERS: usize = 32;

const DEFAULT_FPS: f32 = 60.0;

pub const COLLISION_MODE_DISABLED: u32 = 0;
pub const COLLISION_MODE_RIGID: u32 = 1;
pub const COLLISION_MODE_HIDE_ON_CONTACT: u32 = 2;

pub const SUB_EMITTER_MODE_DISABLED: u32 = 0;
pub const SUB_EMITTER_MODE_CONSTANT: u32 = 1;
pub const SUB_EMITTER_MODE_AT_END: u32 = 2;
pub const SUB_EMITTER_MODE_AT_COLLISION: u32 = 3;
pub const SUB_EMITTER_MODE_AT_START: u32 = 4;

#[derive(Clone, Copy, Default, Pod, Zeroable, ShaderType)]
#[repr(C)]
pub struct CurveUniform {
    pub enabled: u32,
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
    pub min_z: f32,
    pub max_z: f32,
    pub _pad: u32,
}

impl CurveUniform {
    pub fn disabled() -> Self {
        Self {
            enabled: 0,
            min_x: 0.0,
            max_x: 1.0,
            min_y: 0.0,
            max_y: 1.0,
            min_z: 0.0,
            max_z: 1.0,
            _pad: 0,
        }
    }

    pub fn enabled_from(curve: &CurveTexture) -> Self {
        let range_y = curve.effective_range_y();
        let range_z = curve.effective_range_z();
        Self {
            enabled: 1,
            min_x: curve.x.range.min,
            max_x: curve.x.range.max,
            min_y: range_y.min,
            max_y: range_y.max,
            min_z: range_z.min,
            max_z: range_z.max,
            _pad: 0,
        }
    }
}

#[derive(Clone, Copy, Default, Pod, Zeroable, ShaderType)]
#[repr(C)]
pub struct AnimatedVelocityUniform {
    pub min: f32,
    pub max: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub curve: CurveUniform,
}

#[derive(Clone, Copy, Default, Pod, Zeroable, ShaderType)]
#[repr(C)]
pub struct ColliderUniform {
    pub transform: [[f32; 4]; 4],
    pub inverse_transform: [[f32; 4]; 4],
    pub extents: [f32; 3],
    pub collider_type: u32,
}

#[derive(Clone, Copy, Default, Pod, Zeroable, ShaderType)]
#[repr(C)]
pub struct EmitterUniforms {
    pub delta_time: f32,
    pub system_phase: f32,
    pub prev_system_phase: f32,
    pub cycle: u32,

    pub amount: u32,
    pub lifetime: f32,
    pub lifetime_randomness: f32,
    pub emitting: u32,

    pub gravity: [f32; 3],
    pub random_seed: u32,

    pub emission_shape: u32,
    pub emission_sphere_radius: f32,
    pub emission_ring_height: f32,
    pub emission_ring_radius: f32,

    pub emission_ring_inner_radius: f32,
    pub spread: f32,
    pub flatness: f32,
    pub initial_velocity_min: f32,

    pub initial_velocity_max: f32,
    pub inherit_velocity_ratio: f32,
    pub explosiveness: f32,
    pub spawn_time_randomness: f32,

    pub emission_offset: [f32; 3],
    pub _pad1: f32,

    pub emission_scale: [f32; 3],
    pub _pad2: f32,

    pub emission_box_extents: [f32; 3],
    pub _pad3: f32,

    pub emission_ring_axis: [f32; 3],
    pub _pad4: f32,

    pub direction: [f32; 3],
    pub _pad5: f32,

    pub velocity_pivot: [f32; 3],
    pub _pad6: f32,

    pub draw_order: u32,
    pub clear_particles: u32,
    pub scale_min: f32,
    pub scale_max: f32,

    pub scale_over_lifetime: CurveUniform,

    pub use_initial_color_gradient: u32,
    pub turbulence_enabled: u32,
    pub particle_flags: u32,
    /// Fraction of eligible slots that actually spawn, 0..1.
    ///
    /// Exists because `amount` cannot be scaled at runtime: it is also the
    /// per-slot simulation gate, so lowering it strands live particles in
    /// truncated slots where they freeze and never despawn. This gates
    /// spawning without resizing the pool.
    pub spawn_probability: f32,

    pub initial_color: [f32; 4],

    pub alpha_over_lifetime: CurveUniform,
    pub emission_over_lifetime: CurveUniform,

    pub turbulence_noise_strength: f32,
    pub turbulence_noise_scale: f32,
    pub turbulence_noise_speed_random: f32,
    pub turbulence_influence_min: f32,

    pub turbulence_noise_speed: [f32; 3],
    pub turbulence_influence_max: f32,

    pub turbulence_influence_over_lifetime: CurveUniform,

    pub radial_velocity: AnimatedVelocityUniform,

    pub collision_mode: u32,
    pub collision_base_size: f32,
    pub collision_use_scale: u32,
    pub collision_friction: f32,

    pub collision_bounce: f32,
    pub collider_count: u32,
    pub _collision_pad0: f32,
    pub _collision_pad1: f32,

    pub angle_min: f32,
    pub angle_max: f32,
    pub _angle_pad0: f32,
    pub _angle_pad1: f32,

    pub angle_over_lifetime: CurveUniform,

    pub angular_velocity: AnimatedVelocityUniform,

    pub orbit_velocity: AnimatedVelocityUniform,

    pub directional_velocity: AnimatedVelocityUniform,

    pub sub_emitter_mode: u32,
    pub sub_emitter_frequency: f32,
    pub sub_emitter_amount: u32,
    pub sub_emitter_keep_velocity: u32,

    pub is_sub_emitter_target: u32,
    pub _sub_emitter_pad0: u32,
    pub _sub_emitter_pad1: u32,
    pub _sub_emitter_pad2: u32,

    pub emitter_transform: [[f32; 4]; 4],

    pub trail_size: u32,
    pub trail_pass: u32,
    pub trail_stretch_time: f32,
    pub trail_history_size: u32,

    pub trail_history_write_index: u32,
    pub trail_effective_fps: f32,
    pub _trail_pad0: u32,
    pub _trail_pad1: u32,
}

#[derive(Resource, Default)]
pub struct ExtractedColliders {
    pub colliders: Vec<ColliderUniform>,
}

#[derive(Resource, Default)]
pub struct ExtractedParticleSystem {
    pub emitters: Vec<(Entity, ExtractedEmitterData)>,
}

pub struct ExtractedEmitterData {
    pub uniform_steps: Vec<EmitterUniforms>,
    pub particle_buffer_handle: Handle<ShaderBuffer>,
    pub indices_buffer_handle: Handle<ShaderBuffer>,
    pub sorted_particles_buffer_handle: Handle<ShaderBuffer>,
    pub amount: u32,
    pub draw_order: u32,
    /// Whether this emitter's blend mode makes the drawn result depend on the
    /// order its particles arrive in — [`DrawPassMaterial::needs_sorting`].
    ///
    /// Kept separate from `draw_order` rather than folded into it, because
    /// `draw_order` is also a simulate-shader uniform: `particle_simulate.wgsl`
    /// branches on `DRAW_ORDER_INDEX` to decide whether to stamp a spawn index
    /// into each particle. Rewriting it here to mean "do not sort" would
    /// silently change what the simulation writes.
    pub needs_sorting: bool,
    pub camera_position: [f32; 3],
    pub camera_forward: [f32; 3],
    pub emitter_transform: Mat4,
    pub gradient_texture_handle: Option<Handle<Image>>,
    pub color_over_lifetime_texture_handle: Option<Handle<Image>>,
    pub scale_over_lifetime_texture_handle: Option<Handle<Image>>,
    pub alpha_over_lifetime_texture_handle: Option<Handle<Image>>,
    pub emission_over_lifetime_texture_handle: Option<Handle<Image>>,
    pub turbulence_influence_over_lifetime_texture_handle: Option<Handle<Image>>,
    pub radial_velocity_curve_texture_handle: Option<Handle<Image>>,
    pub angle_over_lifetime_texture_handle: Option<Handle<Image>>,
    pub angular_velocity_curve_texture_handle: Option<Handle<Image>>,
    pub orbit_velocity_curve_texture_handle: Option<Handle<Image>>,
    pub directional_velocity_curve_texture_handle: Option<Handle<Image>>,
    pub is_sub_emitter_target: bool,
    pub emission_buffer_handle: Option<Handle<ShaderBuffer>>,
    pub source_buffer_handle: Option<Handle<ShaderBuffer>>,
    pub trail_size: u32,
    pub trail_history_buffer_handle: Option<Handle<ShaderBuffer>>,
}

fn curve_uniform_from(curve: &Option<CurveTexture>) -> CurveUniform {
    match curve {
        Some(c) if !c.is_constant() => CurveUniform::enabled_from(c),
        _ => CurveUniform::disabled(),
    }
}

fn animated_velocity_uniform_from(velocity: &AnimatedVelocity) -> AnimatedVelocityUniform {
    AnimatedVelocityUniform {
        min: velocity.velocity.min,
        max: velocity.velocity.max,
        _pad0: 0.0,
        _pad1: 0.0,
        curve: curve_uniform_from(&velocity.velocity_over_lifetime),
    }
}

fn scaled_animated_velocity_uniform_from(
    velocity: &AnimatedVelocity,
    scale: f32,
) -> AnimatedVelocityUniform {
    let mut u = animated_velocity_uniform_from(velocity);
    u.min *= scale;
    u.max *= scale;
    u
}

struct CollisionUniforms {
    mode: u32,
    friction: f32,
    bounce: f32,
}

fn collision_uniforms_from(mode: &Option<EmitterCollisionMode>) -> CollisionUniforms {
    match mode {
        Some(EmitterCollisionMode::Rigid { friction, bounce }) => CollisionUniforms {
            mode: COLLISION_MODE_RIGID,
            friction: *friction,
            bounce: *bounce,
        },
        Some(EmitterCollisionMode::HideOnContact) => CollisionUniforms {
            mode: COLLISION_MODE_HIDE_ON_CONTACT,
            friction: 0.0,
            bounce: 0.0,
        },
        None => CollisionUniforms {
            mode: COLLISION_MODE_DISABLED,
            friction: 0.0,
            bounce: 0.0,
        },
    }
}

struct EmissionShapeUniforms {
    shape: u32,
    sphere_radius: f32,
    box_extents: Vec3,
    ring_axis: Vec3,
    ring_height: f32,
    ring_radius: f32,
    ring_inner_radius: f32,
}

fn emission_shape_uniforms_from(shape: &EmissionShape) -> EmissionShapeUniforms {
    match *shape {
        EmissionShape::Point => EmissionShapeUniforms {
            shape: EMISSION_SHAPE_POINT,
            sphere_radius: 0.0,
            box_extents: Vec3::ZERO,
            ring_axis: Vec3::Z,
            ring_height: 0.0,
            ring_radius: 0.0,
            ring_inner_radius: 0.0,
        },
        EmissionShape::Sphere { radius } => EmissionShapeUniforms {
            shape: EMISSION_SHAPE_SPHERE,
            sphere_radius: radius,
            box_extents: Vec3::ZERO,
            ring_axis: Vec3::Z,
            ring_height: 0.0,
            ring_radius: 0.0,
            ring_inner_radius: 0.0,
        },
        EmissionShape::SphereSurface { radius } => EmissionShapeUniforms {
            shape: EMISSION_SHAPE_SPHERE_SURFACE,
            sphere_radius: radius,
            box_extents: Vec3::ZERO,
            ring_axis: Vec3::Z,
            ring_height: 0.0,
            ring_radius: 0.0,
            ring_inner_radius: 0.0,
        },
        EmissionShape::Box { extents } => EmissionShapeUniforms {
            shape: EMISSION_SHAPE_BOX,
            sphere_radius: 0.0,
            box_extents: extents,
            ring_axis: Vec3::Z,
            ring_height: 0.0,
            ring_radius: 0.0,
            ring_inner_radius: 0.0,
        },
        EmissionShape::Ring {
            axis,
            height,
            radius,
            inner_radius,
        } => EmissionShapeUniforms {
            shape: EMISSION_SHAPE_RING,
            sphere_radius: 0.0,
            box_extents: Vec3::ZERO,
            ring_axis: axis,
            ring_height: height,
            ring_radius: radius,
            ring_inner_radius: inner_radius,
        },
    }
}

fn resolve_curve_texture(
    curve: &Option<CurveTexture>,
    cache: &CurveTextureCache,
) -> Option<Handle<Image>> {
    curve
        .as_ref()
        .filter(|c| !c.is_constant())
        .and_then(|c| cache.get(c))
}

/// Applies Spawn- and Sim-stage drives to this emitter's simulation uniform.
///
/// Both stages land here because they share one buffer; they differ in when
/// the compute shader reads them, not in where they live. Spawn values are
/// read once at particle birth; Sim values (gravity, turbulence) are read
/// every step and therefore reshape particles already in flight.
///
/// Deliberately never touches `u.amount` — see the field's constraint in this
/// module and `EmitterProp::SpawnProbability`'s doc. `amount` is the pool size
/// AND the per-slot gate, so scaling it strands live particles.
pub(crate) fn apply_sim_drives(u: &mut EmitterUniforms, r: &crate::drives::EmitterResolved) {
    use crate::asset::EmitterProp as P;

    if let Some(v) = r.spawn.get(&P::SpawnProbability) {
        u.spawn_probability = v.clamp(0.0, 1.0);
    }
    if let Some(v) = r.spawn.get(&P::Lifetime) {
        u.lifetime *= v;
    }
    if let Some(v) = r.spawn.get(&P::InitialSpeed) {
        u.initial_velocity_min *= v;
        u.initial_velocity_max *= v;
    }
    if let Some(v) = r.spawn.get(&P::SpawnSize) {
        u.scale_min *= v;
        u.scale_max *= v;
    }
    if let Some(v) = r.spawn.get(&P::Spread) {
        u.spread *= v;
    }
    if let Some(v) = r.spawn.get(&P::EmissionRadius) {
        u.emission_sphere_radius *= v;
        u.emission_ring_radius *= v;
        u.emission_ring_inner_radius *= v;
    }
    // A separate knob from `EmissionRadius` above, which only reaches those
    // three radius fields: this one scales `emission_scale`, which the shader
    // applies to the sampled spawn position of EVERY emission shape
    // (`shaders/particle_simulate.wgsl:571`), a box emitter included. Ordinary
    // multiplies, against an authored default of `Vec3::ONE`.
    if let Some(v) = r.spawn.get(&P::EmissionScaleX) {
        u.emission_scale[0] *= v;
    }
    if let Some(v) = r.spawn.get(&P::EmissionScaleY) {
        u.emission_scale[1] *= v;
    }
    if let Some(v) = r.spawn.get(&P::EmissionScaleZ) {
        u.emission_scale[2] *= v;
    }
    // The three direction components REPLACE rather than multiply, which is
    // the one place an `Emitter` drive does. `get_emission_velocity` does
    // `normalize(params.direction)` (`shaders/particle_simulate.wgsl:600`), so
    // a uniform scale of this vector is a no-op, and a per-axis multiply could
    // never lift a component off an authored `0.0` -- and two of the three are
    // zero in the default `Vec3::X`. See `EmitterProp::DirX`.
    if let Some(v) = r.spawn.get(&P::DirX) {
        u.direction[0] = *v;
    }
    if let Some(v) = r.spawn.get(&P::DirY) {
        u.direction[1] = *v;
    }
    if let Some(v) = r.spawn.get(&P::DirZ) {
        u.direction[2] = *v;
    }
    if let Some(v) = r.sim.get(&P::Gravity) {
        u.gravity = [u.gravity[0] * v, u.gravity[1] * v, u.gravity[2] * v];
    }
    if let Some(v) = r.sim.get(&P::TurbulenceStrength) {
        u.turbulence_noise_strength *= v;
    }
}

fn build_base_uniforms(
    emitter: &EmitterData,
    runtime: &EmitterRuntime,
    draw_order: u32,
    es: &EmissionShapeUniforms,
    collision: &CollisionUniforms,
    sub_emitter_uniforms: (u32, f32, u32, u32),
    spawn_transform: Mat4,
) -> EmitterUniforms {
    let turbulence = &emitter.turbulence;

    // uniform scale factor from the spawn transform so physics quantities
    // (gravity, radial velocity, etc.) stay proportional to scaled distances.
    // for local mode spawn_transform is identity, giving 1.0 (no-op).
    let transform_scale = {
        let sx = spawn_transform.x_axis.truncate().length();
        let sy = spawn_transform.y_axis.truncate().length();
        let sz = spawn_transform.z_axis.truncate().length();
        (sx * sy * sz).cbrt().max(f32::EPSILON)
    };

    EmitterUniforms {
        delta_time: 0.0,
        system_phase: 0.0,
        prev_system_phase: 0.0,
        cycle: 0,

        amount: emitter.emission.particles_amount,
        lifetime: emitter.time.lifetime,
        lifetime_randomness: emitter.time.lifetime_randomness,
        emitting: 0,

        gravity: (emitter.accelerations.gravity * transform_scale).into(),
        random_seed: runtime.random_seed,

        emission_shape: es.shape,
        emission_sphere_radius: es.sphere_radius,
        emission_ring_height: es.ring_height,
        emission_ring_radius: es.ring_radius,

        emission_ring_inner_radius: es.ring_inner_radius,
        spread: emitter.velocities.spread,
        flatness: emitter.velocities.flatness,
        initial_velocity_min: emitter.velocities.initial_velocity.min,

        initial_velocity_max: emitter.velocities.initial_velocity.max,
        inherit_velocity_ratio: emitter.velocities.inherit_ratio,
        explosiveness: emitter.time.explosiveness,
        spawn_time_randomness: emitter.time.spawn_time_randomness,

        emission_offset: emitter.emission.offset.into(),
        _pad1: 0.0,

        emission_scale: emitter.emission.scale.into(),
        _pad2: 0.0,

        emission_box_extents: es.box_extents.into(),
        _pad3: 0.0,

        emission_ring_axis: es.ring_axis.into(),
        _pad4: 0.0,

        direction: emitter.velocities.initial_direction.into(),
        _pad5: 0.0,

        velocity_pivot: emitter.velocities.pivot.into(),
        _pad6: 0.0,

        draw_order,
        clear_particles: 0,
        scale_min: emitter.scale.range.min,
        scale_max: emitter.scale.range.max,

        scale_over_lifetime: curve_uniform_from(&emitter.scale.scale_over_lifetime),

        use_initial_color_gradient: match &emitter.colors.initial_color {
            SolidOrGradientColor::Solid { .. } => 0,
            SolidOrGradientColor::Gradient { .. } => 1,
        },
        turbulence_enabled: if turbulence.enabled { 1 } else { 0 },
        particle_flags: {
            let mut flags = emitter.particle_flags;
            if let Some(curve) = &emitter.angle.angle_over_lifetime {
                if curve.y.is_some() || curve.z.is_some() {
                    flags |= ParticleFlags::ANGLE_PER_AXIS;
                }
            }
            flags.bits()
        },
        // 1.0 = ungated (every eligible slot spawns), the authored default
        // before any Spawn-stage drive touches `EmitterProp::SpawnProbability`.
        spawn_probability: 1.0,

        initial_color: match &emitter.colors.initial_color {
            SolidOrGradientColor::Solid { color } => *color,
            SolidOrGradientColor::Gradient { .. } => [1.0, 1.0, 1.0, 1.0],
        },

        alpha_over_lifetime: curve_uniform_from(&emitter.colors.alpha_over_lifetime),
        emission_over_lifetime: curve_uniform_from(&emitter.colors.emission_over_lifetime),

        turbulence_noise_strength: turbulence.noise_strength,
        turbulence_noise_scale: turbulence.noise_scale / transform_scale,
        turbulence_noise_speed_random: turbulence.noise_speed_random,
        turbulence_influence_min: turbulence.influence.min,

        turbulence_noise_speed: turbulence.noise_speed.into(),
        turbulence_influence_max: turbulence.influence.max,

        turbulence_influence_over_lifetime: curve_uniform_from(&turbulence.influence_over_lifetime),

        radial_velocity: scaled_animated_velocity_uniform_from(
            &emitter.velocities.radial_velocity,
            transform_scale,
        ),

        collision_mode: collision.mode,
        collision_base_size: emitter.collision.base_size * transform_scale,
        collision_use_scale: emitter.collision.use_scale as u32,
        collision_friction: collision.friction,
        collision_bounce: collision.bounce,
        collider_count: 0,
        _collision_pad0: 0.0,
        _collision_pad1: 0.0,

        angle_min: emitter.angle.range.min,
        angle_max: emitter.angle.range.max,
        _angle_pad0: 0.0,
        _angle_pad1: 0.0,

        angle_over_lifetime: curve_uniform_from(&emitter.angle.angle_over_lifetime),

        angular_velocity: animated_velocity_uniform_from(&emitter.velocities.angular_velocity),

        orbit_velocity: animated_velocity_uniform_from(&emitter.velocities.orbit_velocity),

        directional_velocity: animated_velocity_uniform_from(
            &emitter.velocities.directional_velocity,
        ),

        sub_emitter_mode: sub_emitter_uniforms.0,
        sub_emitter_frequency: sub_emitter_uniforms.1,
        sub_emitter_amount: sub_emitter_uniforms.2,
        sub_emitter_keep_velocity: sub_emitter_uniforms.3,
        is_sub_emitter_target: 0,
        _sub_emitter_pad0: 0,
        _sub_emitter_pad1: 0,
        _sub_emitter_pad2: 0,

        emitter_transform: spawn_transform.to_cols_array_2d(),

        trail_size: 1,
        trail_pass: 0,
        trail_stretch_time: 0.0,
        trail_history_size: 0,

        trail_history_write_index: 0,
        trail_effective_fps: 60.0,
        _trail_pad0: 0,
        _trail_pad1: 0,
    }
}

/// Whether a hidden emitter's simulation is worth paying for this frame.
///
/// Extraction is the gate in front of *all* of an emitter's per-frame GPU
/// cost: one uniform buffer plus one bind group per simulation step
/// (`prepare_particle_compute_bind_groups`, `compute.rs`), a compute dispatch
/// per step (`run_particle_compute_node`), a sort bind group and its
/// init/sort/copy dispatches (`sort.rs`). All of that is per *emitter*, not
/// per particle, so a level carrying many effects pays it many times over
/// whether or not anyone is looking at them. Dropping an emitter here drops
/// the lot.
///
/// **The trade this makes.** A skipped emitter does not simulate, so its
/// particles freeze mid-flight. Its CPU clock does not freeze with them —
/// `update_particle_time` (`spawning.rs`) keeps advancing `system_time` and
/// refilling `simulation_steps` every frame regardless — so when the emitter
/// becomes visible again it resumes from GPU state as old as the hidden
/// interval, with emission phase that has moved on without it. That can read
/// as a pop, and the longer it was hidden the worse it is. This is a real
/// behaviour change, not a free win: the bargain is that an author who wrote
/// `Visibility::Hidden` has said the effect is off, and an effect that is off
/// has no state anyone is entitled to see continue.
///
/// **Why [`InheritedVisibility`] and not [`ViewVisibility`].** `ViewVisibility`
/// is the frustum-culled signal and would win far more, but it cannot be
/// trusted to mean "no particle of this emitter is on screen". The `Aabb` these
/// entities are culled against is computed from their `Mesh3d`, and
/// `build_particle_mesh` (`mesh.rs`) stacks `particle_count` copies of the base
/// geometry at *identical* origin-local positions — every particle's real
/// position arrives in the vertex shader out of the sorted particle buffer.
/// So the AABB bounds the emitter's origin quad, not the particle cloud, and
/// with the default `use_local_coords: false` the particles stay in world space
/// where they were emitted while the emitter walks off camera. The asset does
/// carry a particle-aware box, [`EmitterDrawPass::visibility_aabb`](
/// crate::asset::EmitterDrawPass::visibility_aabb) — but nothing in this crate
/// ever puts it on an entity; it is read only by the editor's inspector and
/// gizmo. Until something applies it, frustum culling answers a question about
/// the emitter's origin, and freezing a whole effect on that answer would be
/// wrong. Gating on the authored "this is off" instead is a smaller win that
/// is always correct.
///
/// A missing component counts as visible. Every emitter spawned by
/// `setup_particle_systems` carries `Visibility`, which requires
/// `InheritedVisibility`, so in a real app the `Option` is always `Some`; the
/// fallback is there so a harness that never runs visibility propagation gets
/// simulation rather than silence.
pub fn emitter_is_simulated(inherited: Option<&InheritedVisibility>) -> bool {
    inherited.is_none_or(|v| v.get())
}

pub fn extract_particle_systems(
    mut commands: Commands,
    emitter_query: Extract<
        Query<(
            Entity,
            &EmitterEntity,
            &EmitterRuntime,
            &ParticleBufferHandle,
            &GlobalTransform,
            Option<&SubEmitterBufferHandle>,
            Option<&InheritedVisibility>,
        )>,
    >,
    system_query: Extract<
        Query<(
            &Particles3d,
            &ParticleSystemRuntime,
            Option<&crate::drives::EffectDrives>,
        )>,
    >,
    camera_query: Extract<Query<&GlobalTransform, With<Camera3d>>>,
    assets: Extract<Res<Assets<ParticlesAsset>>>,
    gradient_cache: Extract<Res<GradientTextureCache>>,
    curve_cache: Extract<Res<CurveTextureCache>>,
) {
    let mut extracted = ExtractedParticleSystem::default();

    let (camera_position, camera_forward) = camera_query
        .iter()
        .next()
        .map(|t| (t.translation(), t.forward().as_vec3()))
        .unwrap_or((Vec3::ZERO, Vec3::NEG_Z));

    let mut emission_buffer_map: std::collections::HashMap<(Entity, usize), Handle<ShaderBuffer>> =
        std::collections::HashMap::new();
    for (
        _entity,
        emitter_entity,
        runtime,
        _buffer_handle,
        _global_transform,
        sub_emitter_buf,
        _inherited_visibility,
    ) in emitter_query.iter()
    {
        let Some(sub_buf) = sub_emitter_buf else {
            continue;
        };
        let Ok((particle_system, _, _)) = system_query.get(emitter_entity.parent_system) else {
            continue;
        };
        let Some(asset) = assets.get(particle_system) else {
            continue;
        };
        let Some(emitter) = asset.emitters.get(runtime.emitter_index) else {
            continue;
        };
        let Some(ref sub_config) = emitter.sub_emitter else {
            continue;
        };
        emission_buffer_map.insert(
            (emitter_entity.parent_system, sub_config.target_emitter),
            sub_buf.buffer.clone(),
        );
    }

    for (
        entity,
        emitter_entity,
        runtime,
        buffer_handle,
        global_transform,
        sub_emitter_buf,
        inherited_visibility,
    ) in emitter_query.iter()
    {
        // Deliberately gated here and not in the sub-emitter pass above: that
        // pass only records which buffer feeds which target, and a target must
        // keep knowing it is fed from elsewhere even on a frame when its source
        // is hidden, or it would fall back to emitting for itself.
        if !emitter_is_simulated(inherited_visibility) {
            continue;
        }

        let Ok((particle_system, _system_runtime, effect_drives)) =
            system_query.get(emitter_entity.parent_system)
        else {
            continue;
        };

        let Some(asset) = assets.get(particle_system) else {
            continue;
        };

        let Some(emitter) = asset.emitters.get(runtime.emitter_index) else {
            continue;
        };

        if !emitter.enabled || runtime.inactive {
            continue;
        }

        let draw_order = match emitter.draw_pass.draw_order {
            DrawOrder::Index => 0,
            DrawOrder::Lifetime => 1,
            DrawOrder::ReverseLifetime => 2,
            DrawOrder::ViewDepth => 3,
        };

        let es = emission_shape_uniforms_from(&emitter.emission.shape);
        let collision = collision_uniforms_from(&emitter.collision.mode);

        let sub_emitter_uniforms = match &emitter.sub_emitter {
            Some(config) => {
                let mode = match config.mode {
                    SubEmitterMode::Constant => SUB_EMITTER_MODE_CONSTANT,
                    SubEmitterMode::AtEnd => SUB_EMITTER_MODE_AT_END,
                    SubEmitterMode::AtCollision => SUB_EMITTER_MODE_AT_COLLISION,
                    SubEmitterMode::AtStart => SUB_EMITTER_MODE_AT_START,
                };
                let freq = if config.frequency > 0.0 {
                    1.0 / config.frequency
                } else {
                    1.0
                };
                (mode, freq, config.amount, config.keep_velocity as u32)
            }
            None => (SUB_EMITTER_MODE_DISABLED, 1.0, 1, 0),
        };

        let use_local_coords = emitter.draw_pass.use_local_coords;
        let world_matrix = global_transform.to_matrix();

        // local mode: spawn in local space (identity), render via mesh transform (world)
        // global mode: spawn in world space (world), render without transform (identity)
        let (spawn_transform, render_transform) = if use_local_coords {
            (Mat4::IDENTITY, world_matrix)
        } else {
            (world_matrix, Mat4::IDENTITY)
        };

        let trail_size = emitter.trail_size();
        let trail_stretch_time = emitter.trail.stretch_time;

        let effective_fps = if emitter.time.fixed_fps > 0 {
            emitter.time.fixed_fps as f32
        } else {
            let dt = runtime
                .simulation_steps
                .last()
                .map(|s| s.delta_time)
                .unwrap_or(1.0 / DEFAULT_FPS);
            if dt > 0.0 { 1.0 / dt } else { DEFAULT_FPS }
        };
        let trail_history_frames = buffer_handle.trail_history_frames;

        let mut base_uniforms = build_base_uniforms(
            emitter,
            runtime,
            draw_order,
            &es,
            &collision,
            sub_emitter_uniforms,
            spawn_transform,
        );
        let empty_resolved = crate::drives::EmitterResolved::default();
        let resolved = effect_drives
            .and_then(|d| d.0.emitters.get(runtime.emitter_index))
            .unwrap_or(&empty_resolved);
        apply_sim_drives(&mut base_uniforms, resolved);
        base_uniforms.trail_size = trail_size;
        base_uniforms.trail_stretch_time = trail_stretch_time;
        base_uniforms.trail_history_size = trail_history_frames;
        base_uniforms.trail_effective_fps = effective_fps;

        let is_sub_emitter_target = emission_buffer_map
            .contains_key(&(emitter_entity.parent_system, runtime.emitter_index));

        let uniform_steps: Vec<EmitterUniforms> = runtime
            .simulation_steps
            .iter()
            .flat_map(|step| {
                let should_emit = if is_sub_emitter_target {
                    false
                } else {
                    runtime.emitting && is_past_delay(step.system_time, &emitter.time)
                };
                let head_uniforms = EmitterUniforms {
                    delta_time: step.delta_time,
                    system_phase: compute_phase(step.system_time, &emitter.time),
                    prev_system_phase: compute_phase(step.prev_system_time, &emitter.time),
                    cycle: step.cycle,
                    emitting: if should_emit { 1 } else { 0 },
                    clear_particles: if step.clear_requested { 1 } else { 0 },
                    is_sub_emitter_target: if is_sub_emitter_target { 1 } else { 0 },
                    trail_pass: 0,
                    trail_history_write_index: step.trail_history_write_index,
                    ..base_uniforms
                };
                let trail_uniforms = (trail_size > 1).then(|| EmitterUniforms {
                    trail_pass: 1,
                    ..head_uniforms
                });
                std::iter::once(head_uniforms).chain(trail_uniforms)
            })
            .collect();

        let gradient_texture_handle = match &emitter.colors.initial_color {
            SolidOrGradientColor::Gradient { gradient } => gradient_cache.get(gradient),
            SolidOrGradientColor::Solid { .. } => None,
        };

        let color_over_lifetime_texture_handle =
            gradient_cache.get(&emitter.colors.color_over_lifetime);

        let scale_over_lifetime_texture_handle =
            resolve_curve_texture(&emitter.scale.scale_over_lifetime, &curve_cache);
        let alpha_over_lifetime_texture_handle =
            resolve_curve_texture(&emitter.colors.alpha_over_lifetime, &curve_cache);
        let emission_over_lifetime_texture_handle =
            resolve_curve_texture(&emitter.colors.emission_over_lifetime, &curve_cache);
        let turbulence_influence_over_lifetime_texture_handle =
            resolve_curve_texture(&emitter.turbulence.influence_over_lifetime, &curve_cache);
        let radial_velocity_curve_texture_handle = resolve_curve_texture(
            &emitter.velocities.radial_velocity.velocity_over_lifetime,
            &curve_cache,
        );
        let angle_over_lifetime_texture_handle =
            resolve_curve_texture(&emitter.angle.angle_over_lifetime, &curve_cache);
        let angular_velocity_curve_texture_handle = resolve_curve_texture(
            &emitter.velocities.angular_velocity.velocity_over_lifetime,
            &curve_cache,
        );
        let orbit_velocity_curve_texture_handle = resolve_curve_texture(
            &emitter.velocities.orbit_velocity.velocity_over_lifetime,
            &curve_cache,
        );
        let directional_velocity_curve_texture_handle = resolve_curve_texture(
            &emitter
                .velocities
                .directional_velocity
                .velocity_over_lifetime,
            &curve_cache,
        );

        let emission_buffer_handle = sub_emitter_buf.map(|b| b.buffer.clone());
        let source_buffer_handle = if is_sub_emitter_target {
            emission_buffer_map
                .get(&(emitter_entity.parent_system, runtime.emitter_index))
                .cloned()
        } else {
            None
        };

        extracted.emitters.push((
            entity,
            ExtractedEmitterData {
                uniform_steps,
                particle_buffer_handle: buffer_handle.particle_buffer.clone(),
                indices_buffer_handle: buffer_handle.indices_buffer.clone(),
                sorted_particles_buffer_handle: buffer_handle.sorted_particles_buffer.clone(),
                amount: emitter.emission.particles_amount,
                draw_order,
                needs_sorting: emitter.draw_pass.material.needs_sorting(),
                camera_position: camera_position.into(),
                camera_forward: camera_forward.into(),
                emitter_transform: render_transform,
                gradient_texture_handle,
                color_over_lifetime_texture_handle,
                scale_over_lifetime_texture_handle,
                alpha_over_lifetime_texture_handle,
                emission_over_lifetime_texture_handle,
                turbulence_influence_over_lifetime_texture_handle,
                radial_velocity_curve_texture_handle,
                angle_over_lifetime_texture_handle,
                angular_velocity_curve_texture_handle,
                orbit_velocity_curve_texture_handle,
                directional_velocity_curve_texture_handle,
                is_sub_emitter_target,
                emission_buffer_handle,
                source_buffer_handle,
                trail_size,
                trail_history_buffer_handle: buffer_handle.trail_history_buffer.clone(),
            },
        ));
    }

    commands.insert_resource(extracted);
}

pub fn extract_colliders(
    mut commands: Commands,
    colliders_query: Extract<Query<(&GlobalTransform, &ParticlesCollider3D)>>,
) {
    let mut colliders = Vec::new();

    for (global_transform, collider) in colliders_query.iter() {
        if !collider.enabled {
            continue;
        }

        let transform = global_transform.to_matrix();
        let inverse = transform.inverse();

        let (extents, collider_type) = match &collider.shape {
            ParticlesColliderShape3D::Sphere { radius } => {
                ([*radius, 0.0, 0.0], COLLIDER_TYPE_SPHERE)
            }
            ParticlesColliderShape3D::Box { size } => ((*size * 0.5).to_array(), COLLIDER_TYPE_BOX),
        };

        colliders.push(ColliderUniform {
            transform: transform.to_cols_array_2d(),
            inverse_transform: inverse.to_cols_array_2d(),
            extents,
            collider_type,
        });

        if colliders.len() >= MAX_COLLIDERS {
            break;
        }
    }

    commands.insert_resource(ExtractedColliders { colliders });
}

#[cfg(test)]
mod drive_tests {
    use super::*;
    use crate::asset::EmitterProp;
    use crate::drives::EmitterResolved;

    fn resolved(pairs: &[(EmitterProp, f32)]) -> EmitterResolved {
        let mut r = EmitterResolved::default();
        for (p, v) in pairs {
            match p.stage() {
                crate::asset::Stage::Spawn => {
                    r.spawn.insert(*p, *v);
                }
                crate::asset::Stage::Sim => {
                    r.sim.insert(*p, *v);
                }
                crate::asset::Stage::Render => panic!("render props do not reach this uniform"),
            }
        }
        r
    }

    #[test]
    fn amount_is_never_mutated_by_drives() {
        // amount is BOTH the pool size and the per-slot simulation gate
        // (idx >= amount skips a slot), so lowering it strands live particles
        // in truncated slots where they freeze and never despawn.
        let mut u = EmitterUniforms {
            amount: 64,
            ..Default::default()
        };
        apply_sim_drives(
            &mut u,
            &resolved(&[
                (EmitterProp::SpawnProbability, 0.1),
                (EmitterProp::Lifetime, 0.5),
            ]),
        );
        assert_eq!(u.amount, 64, "amount must never be mutated by apply_sim_drives");
    }

    #[test]
    fn spawn_probability_is_carried_and_clamped_to_unit() {
        let mut u = EmitterUniforms {
            spawn_probability: 1.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnProbability, 0.25)]));
        assert_eq!(u.spawn_probability, 0.25);

        let mut u = EmitterUniforms {
            spawn_probability: 1.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnProbability, 4.0)]));
        assert_eq!(u.spawn_probability, 1.0, "a probability above one is meaningless");

        let mut u = EmitterUniforms {
            spawn_probability: 1.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnProbability, -3.0)]));
        assert_eq!(u.spawn_probability, 0.0);
    }

    #[test]
    fn a_spawn_drive_multiplies_the_authored_value() {
        let mut u = EmitterUniforms {
            lifetime: 2.0,
            initial_velocity_min: 1.0,
            initial_velocity_max: 3.0,
            ..Default::default()
        };
        apply_sim_drives(
            &mut u,
            &resolved(&[
                (EmitterProp::Lifetime, 0.5),
                (EmitterProp::InitialSpeed, 2.0),
            ]),
        );
        assert_eq!(u.lifetime, 1.0);
        assert_eq!(u.initial_velocity_min, 2.0);
        assert_eq!(u.initial_velocity_max, 6.0);
    }

    #[test]
    fn a_sim_drive_scales_gravity() {
        let mut u = EmitterUniforms {
            gravity: [0.0, -10.0, 0.0],
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::Gravity, 0.5)]));
        assert_eq!(u.gravity, [0.0, -5.0, 0.0]);
    }

    #[test]
    fn an_empty_resolution_changes_nothing() {
        let before = EmitterUniforms {
            lifetime: 2.0,
            spawn_probability: 1.0,
            ..Default::default()
        };
        let mut u = before;
        apply_sim_drives(&mut u, &EmitterResolved::default());
        assert_eq!(u.lifetime, before.lifetime);
        assert_eq!(u.spawn_probability, before.spawn_probability);
    }

    #[test]
    fn a_spawn_size_drive_scales_both_scale_bounds() {
        let mut u = EmitterUniforms {
            scale_min: 1.0,
            scale_max: 2.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnSize, 3.0)]));
        assert_eq!(u.scale_min, 3.0);
        assert_eq!(u.scale_max, 6.0);
    }

    #[test]
    fn a_spread_drive_scales_spread() {
        let mut u = EmitterUniforms {
            spread: 45.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::Spread, 2.0)]));
        assert_eq!(u.spread, 90.0);
    }

    #[test]
    fn an_emission_radius_drive_scales_all_three_radius_fields() {
        // This property fans out to three fields; a future edit could easily
        // update two of three and leave the drive silently half-applied.
        let mut u = EmitterUniforms {
            emission_sphere_radius: 1.0,
            emission_ring_radius: 2.0,
            emission_ring_inner_radius: 4.0,
            emission_scale: [2.0, 3.0, 5.0],
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::EmissionRadius, 0.5)]));
        assert_eq!(u.emission_sphere_radius, 0.5);
        assert_eq!(u.emission_ring_radius, 1.0);
        assert_eq!(u.emission_ring_inner_radius, 2.0);
        // And NOT `emission_scale`: these are two different knobs, and the
        // similar names invite an edit that collapses them. `EmissionRadius`
        // does nothing at all to a box emitter, which has no radius field.
        assert_eq!(
            u.emission_scale, [2.0, 3.0, 5.0],
            "EmissionRadius must leave the per-axis emission scale alone -- \
             that is EmissionScaleX/Y/Z's job",
        );
    }

    #[test]
    fn each_emission_scale_axis_multiplies_its_own_uniform_lane() {
        // Distinct authored values AND distinct factors per axis, so a swapped
        // or duplicated index cannot land on the right product by accident.
        let mut u = EmitterUniforms {
            emission_scale: [1.0, 2.0, 4.0],
            emission_sphere_radius: 3.0,
            ..Default::default()
        };
        apply_sim_drives(
            &mut u,
            &resolved(&[
                (EmitterProp::EmissionScaleX, 0.5),
                (EmitterProp::EmissionScaleY, 3.0),
                (EmitterProp::EmissionScaleZ, 0.25),
            ]),
        );
        assert_eq!(u.emission_scale, [0.5, 6.0, 1.0]);
        // The converse of the assertion in the EmissionRadius test above.
        assert_eq!(
            u.emission_sphere_radius, 3.0,
            "EmissionScale* must not reach the radius fields",
        );
    }

    #[test]
    fn an_emission_scale_drive_on_one_axis_leaves_the_other_two_as_authored() {
        // A non-uniform emission volume is the whole point of splitting this
        // into three props rather than reusing one scalar.
        let mut u = EmitterUniforms {
            emission_scale: [1.0, 1.0, 1.0],
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::EmissionScaleX, 4.0)]));
        assert_eq!(u.emission_scale, [4.0, 1.0, 1.0]);
    }

    #[test]
    fn each_direction_component_lands_in_its_own_uniform_lane() {
        // Three near-identical `if let` arms writing three indices of one
        // array is exactly the shape a copy-paste typo hides in, so the three
        // driven values are distinct and none is 0.0 or 1.0: any swapped or
        // duplicated index moves an assertion.
        let mut u = EmitterUniforms {
            direction: [1.0, 0.0, 0.0],
            ..Default::default()
        };
        apply_sim_drives(
            &mut u,
            &resolved(&[
                (EmitterProp::DirX, -0.25),
                (EmitterProp::DirY, 0.75),
                (EmitterProp::DirZ, 3.5),
            ]),
        );
        assert_eq!(u.direction, [-0.25, 0.75, 3.5]);
    }

    #[test]
    fn a_direction_drive_replaces_the_authored_component_rather_than_scaling_it() {
        // The regression this property exists for. `Vec3::X` is the authored
        // default, so `y` is 0.0; under the multiply every other Emitter prop
        // uses, a drive asking for y = 2.0 would resolve to 0.0 * 2.0 = 0.0
        // and the emitter would never turn. Aiming would be unreachable from
        // the default direction, which is the one authors start from.
        let mut u = EmitterUniforms {
            direction: [1.0, 0.0, 0.0],
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::DirY, 2.0)]));
        assert_eq!(
            u.direction[1], 2.0,
            "DirY must WRITE the driven value, not scale the authored 0.0 by it",
        );
        assert_eq!(u.direction[0], 1.0, "an undriven component keeps its authored value");
        assert_eq!(u.direction[2], 0.0);
    }

    #[test]
    fn a_turbulence_strength_drive_scales_turbulence_noise_strength() {
        let mut u = EmitterUniforms {
            turbulence_noise_strength: 4.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::TurbulenceStrength, 0.25)]));
        assert_eq!(u.turbulence_noise_strength, 1.0);
    }

    /// `EmitterParams` in `particle_simulate.wgsl` shares this struct's GPU
    /// buffer and must declare the same fields. Nothing links the two, so a
    /// field added on the Rust side alone compiles clean, tests clean, and
    /// corrupts every field after it at runtime. This is a tripwire, not a
    /// proof: it pins the field this task adds.
    #[test]
    fn the_wgsl_emitter_params_declares_spawn_probability() {
        let src = include_str!("shaders/particle_simulate.wgsl");
        assert!(
            src.contains("spawn_probability: f32"),
            "particle_simulate.wgsl must declare spawn_probability to match EmitterUniforms",
        );
    }
}

/// Headless proof that the visibility gate in [`extract_particle_systems`]
/// removes real per-frame work.
///
/// These tests run the **real** extract system in a **real** render sub-app —
/// [`ExtractPlugin`](bevy::render::extract_plugin::ExtractPlugin) builds one
/// without ever touching a GPU, so `ExtractedParticleSystem` can be read back
/// and counted on a machine with no adapter at all. What they measure is
/// therefore work *eliminated*, not frame time: nobody here can time a frame.
/// Emitters extracted and compute dispatches issued are the honest proxies,
/// and they are the two quantities the gate actually moves.
#[cfg(test)]
mod visibility_tests {
    use super::*;
    use crate::asset::{
        EmitterData, InitialTransform, ParticlesAsset, ParticlesAuthors, ParticlesDimension,
    };
    use crate::runtime::{ParticleSystemRuntime, SimulationStep};
    use bevy::asset::AssetPlugin;
    use bevy::ecs::schedule::ScheduleLabel;
    use bevy::render::{Render, RenderApp, extract_plugin::ExtractPlugin};

    /// Steps per emitter in the fixture. Each one becomes one entry in
    /// `uniform_steps`, which is one compute dispatch in
    /// `run_particle_compute_node` — so the dispatch count the tests assert is
    /// the count that shader actually issues, not a stand-in for it.
    const STEPS_PER_EMITTER: usize = 2;

    /// Builds an app whose render sub-app runs `extract_particle_systems` for
    /// real, with one emitter entity per entry in `visible`.
    ///
    /// `update_schedule` has to be set by hand: `ExtractPlugin` alone leaves it
    /// `None`, and the extract schedule deliberately defers its commands to
    /// `RenderSystems::ExtractCommands` in `Render` — so without this the
    /// `ExtractedParticleSystem` resource is built and then never inserted.
    fn extract_app(visible: &[bool]) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .add_plugins(ExtractPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<GradientTextureCache>();
        app.init_resource::<CurveTextureCache>();

        let render_app = app.get_sub_app_mut(RenderApp).unwrap();
        render_app.update_schedule = Some(Render.intern());
        render_app.add_systems(ExtractSchedule, extract_particle_systems);

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(ParticlesAsset::new(
                "fixture".into(),
                ParticlesDimension::D3,
                InitialTransform::default(),
                vec![EmitterData::default()],
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

        for &is_visible in visible {
            let mut runtime = EmitterRuntime::new(0, Some(7));
            runtime.simulation_steps = (0..STEPS_PER_EMITTER)
                .map(|i| SimulationStep {
                    prev_system_time: i as f32 * 0.1,
                    system_time: (i + 1) as f32 * 0.1,
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
                    max_particles: 160,
                    amount: 160,
                    trail_size: 1,
                    trail_history_buffer: None,
                    trail_history_frames: 0,
                },
                GlobalTransform::default(),
                if is_visible {
                    InheritedVisibility::VISIBLE
                } else {
                    InheritedVisibility::HIDDEN
                },
            ));
        }

        app.update();
        app
    }

    /// `(emitters extracted, compute dispatches those emitters will issue)`.
    fn counts(app: &App) -> (usize, usize) {
        let extracted = app
            .sub_app(RenderApp)
            .world()
            .resource::<ExtractedParticleSystem>();
        (
            extracted.emitters.len(),
            extracted
                .emitters
                .iter()
                .map(|(_, data)| data.uniform_steps.len())
                .sum(),
        )
    }

    #[test]
    fn an_emitter_with_no_visibility_component_at_all_is_still_simulated() {
        assert!(emitter_is_simulated(None));
    }

    #[test]
    fn a_hidden_emitter_is_dropped_from_the_extract_and_a_visible_one_is_not() {
        let app = extract_app(&[true, false, true, false, true]);
        assert_eq!(counts(&app).0, 3);
    }

    /// The headline measurement for the visibility gate, stated as work
    /// removed: eight emitters in the scene, three of them hidden, and the
    /// render world is handed five. Frame time is not measured and cannot be
    /// from here.
    #[test]
    fn hiding_three_of_eight_emitters_removes_their_extract_and_their_dispatches() {
        let all_visible = extract_app(&[true; 8]);
        let three_hidden = extract_app(&[
            true, true, false, true, false, true, false, true,
        ]);

        assert_eq!(counts(&all_visible), (8, 8 * STEPS_PER_EMITTER));
        assert_eq!(counts(&three_hidden), (5, 5 * STEPS_PER_EMITTER));
    }
}
