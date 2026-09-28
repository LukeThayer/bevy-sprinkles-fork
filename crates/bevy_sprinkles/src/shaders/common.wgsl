#define_import_path bevy_sprinkles::common

struct Particle {
    position: vec4<f32>,       // xyz, scale
    velocity: vec4<f32>,       // xyz, lifetime
    color: vec4<f32>,
    custom: vec4<f32>,         // age, spawn_index, seed, flags
    alignment_dir: vec4<f32>,  // xyz direction for ALIGN_Y_TO_VELOCITY, w unused
    ref_up: vec4<f32>,         // xyz reference up for parallel-transported alignment
    angles: vec4<f32>,         // xyz = per-axis rotation angles in radians
}

const TRAIL_THICKNESS_CURVE_SAMPLES: u32 = 16u;

// LAYOUT-LOCKSTEP: fields (order+types) must match ParticleEmitterUniforms in
// material.rs — same GPU buffer.
struct ParticleEmitterUniforms {
    emitter_transform: mat4x4<f32>,
    max_particles: u32,
    particle_flags: u32,
    use_local_coords: u32,
    trail_size: u32,
    transform_align: u32,
    trail_thickness_curve: array<f32, 16>,
    drive_slots: array<f32, 9>,
}

// LAYOUT-LOCKSTEP with DRIVE_SLOT_COUNT in asset/drive.rs.
const DRIVE_SLOT_TINT: u32 = 0u;
const DRIVE_SLOT_ALPHA: u32 = 1u;
const DRIVE_SLOT_SIZE_MUL: u32 = 2u;
const DRIVE_SLOT_EMISSIVE: u32 = 3u;
const DRIVE_SLOT_SCROLL_U: u32 = 4u;
const DRIVE_SLOT_SCROLL_V: u32 = 5u;
const DRIVE_SLOT_FLOW: u32 = 6u;
const DRIVE_SLOT_EROSION: u32 = 7u;
const DRIVE_SLOT_FRESNEL: u32 = 8u;

// LAYOUT-LOCKSTEP: fields (order+types) must match FxUniform in
// asset/fx.rs — same GPU buffer.
struct FxUniform {
    // xy = scroll rate, zw = tiling.
    scroll_tiling: vec4<f32>,
    // x = flow strength, yz = flow scroll, w = fresnel power.
    flow_fresnel: vec4<f32>,
    // x = erosion threshold, y = erosion edge, z = fresnel boost, w = soft fade.
    erosion_soft: vec4<f32>,
    erosion_edge_color: vec4<f32>,
}

struct CurveUniform {
    enabled: u32,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    min_z: f32,
    max_z: f32,
    _pad: u32,
}

// per-particle flags (stored in particle.custom.w)
const PARTICLE_FLAG_ACTIVE: u32 = 1u;

// emitter-level particle flags (from EmitterParams.particle_flags)
const EMITTER_FLAG_ROTATE_Y: u32 = 2u;
const EMITTER_FLAG_DISABLE_Z: u32 = 4u;
const EMITTER_FLAG_ANGLE_PER_AXIS: u32 = 8u;

// transform align mode values
const TRANSFORM_ALIGN_DISABLED: u32 = 0u;
const TRANSFORM_ALIGN_BILLBOARD: u32 = 1u;
const TRANSFORM_ALIGN_Y_TO_VELOCITY: u32 = 2u;
const TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY: u32 = 3u;
const TRANSFORM_ALIGN_BILLBOARD_FIXED_Y: u32 = 4u;

struct TrailHistoryEntry {
    position: vec4<f32>,
    velocity: vec4<f32>,
}

// sub emitter emission buffer
struct SubEmissionEntry {
    position: vec4<f32>,    // xyz + scale
    velocity: vec4<f32>,    // xyz + w unused
    flags: u32,
}

const EMISSION_FLAG_HAS_POSITION: u32 = 1u;
const EMISSION_FLAG_HAS_VELOCITY: u32 = 2u;

// sub emitter mode constants
const SUB_EMITTER_MODE_DISABLED: u32 = 0u;
const SUB_EMITTER_MODE_CONSTANT: u32 = 1u;
const SUB_EMITTER_MODE_AT_END: u32 = 2u;
const SUB_EMITTER_MODE_AT_COLLISION: u32 = 3u;
const SUB_EMITTER_MODE_AT_START: u32 = 4u;

// multiply-xorshift integer hash
// https://nullprogram.com/blog/2018/07/31/
fn hash(n: u32) -> u32 {
    var x = n;
    x = ((x >> 16u) ^ x) * 0x45d9f3bu;
    x = ((x >> 16u) ^ x) * 0x45d9f3bu;
    x = (x >> 16u) ^ x;
    return x;
}

fn hash_to_float(n: u32) -> f32 {
    return f32(hash(n)) / f32(0xFFFFFFFFu);
}

fn random_range(seed: u32, variation: f32) -> f32 {
    return (hash_to_float(seed) * 2.0 - 1.0) * variation;
}

fn random_vec3(seed: u32, variation: vec3<f32>) -> vec3<f32> {
    return vec3(
        random_range(seed, variation.x),
        random_range(seed + 1u, variation.y),
        random_range(seed + 2u, variation.z)
    );
}
