#import bevy_sprinkles::common::{
    Particle,
    ParticleEmitterUniforms,
    FxUniform,
    PARTICLE_FLAG_ACTIVE,
    TRANSFORM_ALIGN_BILLBOARD,
    TRANSFORM_ALIGN_Y_TO_VELOCITY,
    TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY,
    TRANSFORM_ALIGN_BILLBOARD_FIXED_Y,
    TRAIL_THICKNESS_CURVE_SAMPLES,
    DRIVE_SLOT_TINT,
    DRIVE_SLOT_ALPHA,
    DRIVE_SLOT_SIZE_MUL,
    DRIVE_SLOT_EMISSIVE,
    DRIVE_SLOT_SCROLL_U,
    DRIVE_SLOT_SCROLL_V,
    DRIVE_SLOT_FLOW,
    DRIVE_SLOT_EROSION,
    DRIVE_SLOT_FRESNEL,
}
#import bevy_pbr::{
    mesh_functions,
    mesh_view_bindings::{view, globals},
    view_transformations::{position_world_to_clip, depth_ndc_to_view_z},
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{Vertex, VertexOutput}
#ifdef PREPASS_FRAGMENT
#import bevy_pbr::{
    prepass_io::FragmentOutput,
    pbr_deferred_functions::deferred_output,
    pbr_fragment::pbr_input_from_standard_material,
}
#endif
#else
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, alpha_discard},
}
// FX_SOFT only: `prepass_depth` (bevy_pbr::prepass_utils, gated `#ifdef
// DEPTH_PREPASS` in its own source) only exists when the VIEW/camera has a
// depth prepass enabled -- a property of the camera, independent of this
// material's own FX_SOFT flag. Importing it only under FX_SOFT means an
// effect that doesn't use soft particles never requires a depth prepass;
// gated only here (not also by DEPTH_PREPASS) is deliberate -- a
// soft-particle effect rendered by a camera without one is a documented
// misconfiguration (see FxSettings::soft_fade) that should fail loudly at
// shader-compile time, not be silently patched over.
#ifdef FX_SOFT
#import bevy_pbr::prepass_utils::prepass_depth
#endif
#endif

const STANDARD_MATERIAL_FLAGS_UNLIT_BIT: u32 = 1u << 5u;

#ifdef PREPASS_PIPELINE
#ifndef PREPASS_FRAGMENT
#ifndef MAY_DISCARD
// depth-only prepass (storage buffers not bound, nothing declared here)
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> sorted_particles: array<Particle>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> emitter_uniforms: ParticleEmitterUniforms;
#endif
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> sorted_particles: array<Particle>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> emitter_uniforms: ParticleEmitterUniforms;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> fx: FxUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var flow_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var flow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var erosion_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var erosion_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var gradient_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var gradient_sampler: sampler;
#endif
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> sorted_particles: array<Particle>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> emitter_uniforms: ParticleEmitterUniforms;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<uniform> fx: FxUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var flow_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var flow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var erosion_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var erosion_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var gradient_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var gradient_sampler: sampler;
#endif

// computes a shortest-arc rotation matrix that aligns the Y axis to a direction
fn align_y_to_direction(dir: vec3<f32>) -> mat3x3<f32> {
    let to = normalize(dir);
    let d = to.y; // dot((0,1,0), to)

    // near-opposite: 180° rotation around X
    if d < -0.9999 {
        return mat3x3<f32>(
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, -1.0, 0.0),
            vec3(0.0, 0.0, -1.0),
        );
    }

    // shortest-arc quaternion from (0,1,0) to `to`, simplified to matrix form
    let k = 1.0 / (1.0 + d);
    return mat3x3<f32>(
        vec3(1.0 - to.x * to.x * k, -to.x, -to.x * to.z * k),
        to,
        vec3(-to.x * to.z * k, -to.z, 1.0 - to.z * to.z * k),
    );
}

// aligns Y axis to a direction using a parallel-transported reference "up" vector
fn align_y_with_ref(dir: vec3<f32>, ref_up: vec3<f32>) -> mat3x3<f32> {
    let y_axis = normalize(dir);
    var x_axis = cross(y_axis, ref_up);
    let x_len = length(x_axis);

    // fallback to shortest-arc if ref_up is degenerate
    if x_len < 0.001 {
        return align_y_to_direction(dir);
    }

    x_axis = x_axis / x_len;
    let z_axis = cross(x_axis, y_axis);
    return mat3x3<f32>(x_axis, y_axis, z_axis);
}

fn rot_x(a: f32) -> mat3x3<f32> {
    let c = cos(a); let s = sin(a);
    return mat3x3<f32>(vec3(1.0, 0.0, 0.0), vec3(0.0, c, s), vec3(0.0, -s, c));
}

fn rot_y(a: f32) -> mat3x3<f32> {
    let c = cos(a); let s = sin(a);
    return mat3x3<f32>(vec3(c, 0.0, s), vec3(0.0, 1.0, 0.0), vec3(-s, 0.0, c));
}

fn rot_z(a: f32) -> mat3x3<f32> {
    let c = cos(a); let s = sin(a);
    return mat3x3<f32>(vec3(c, s, 0.0), vec3(-s, c, 0.0), vec3(0.0, 0.0, 1.0));
}

#ifdef PREPASS_PIPELINE
#ifndef PREPASS_FRAGMENT
#ifdef MAY_DISCARD

fn get_head_particle(particle_index: u32) -> Particle {
    let trail_size = emitter_uniforms.trail_size;
    let head_slot = particle_index * max(trail_size, 1u);
    return sorted_particles[head_slot];
}

fn get_trail_color(particle_index: u32, section_frac: f32) -> vec4<f32> {
    let trail_size = emitter_uniforms.trail_size;
    let head_slot = particle_index * trail_size;
    let last_seg = trail_size - 1u;
    let section_f = section_frac * f32(last_seg);
    let seg_lo = min(u32(section_f), last_seg);
    let seg_hi = min(seg_lo + 1u, last_seg);
    let seg_t = section_f - f32(seg_lo);
    let color_lo = sorted_particles[head_slot + seg_lo].color;
    let color_hi = sorted_particles[head_slot + seg_hi].color;
    return mix(color_lo, color_hi, seg_t);
}

fn particle_vertex_impl(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

    let particle_index = u32(round(vertex.uv_b.x));
    let trail_size = emitter_uniforms.trail_size;
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif

#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif

    if (trail_size > 1u) {
        let head_slot = particle_index * trail_size;
        let head_particle = sorted_particles[head_slot];
        let head_flags = bitcast<u32>(head_particle.custom.w);
        let is_active = (head_flags & PARTICLE_FLAG_ACTIVE) != 0u;

        let section_frac = vertex.uv_b.y;

        let last_seg = trail_size - 1u;
        let section_f = section_frac * f32(last_seg);
        let seg_lo = min(u32(section_f), last_seg);
        let seg_hi = min(seg_lo + 1u, last_seg);
        let seg_t = section_f - f32(seg_lo);

        let idx_lo = head_slot + seg_lo;
        let idx_hi = head_slot + seg_hi;
        let pos_lo = sorted_particles[idx_lo].position.xyz;
        let pos_hi = sorted_particles[idx_hi].position.xyz;
        let scale_lo = sorted_particles[idx_lo].position.w;
        let scale_hi = sorted_particles[idx_hi].position.w;

        let trail_pos = mix(pos_lo, pos_hi, seg_t);
        let base_scale = mix(scale_lo, scale_hi, seg_t);
        let particle_scale = select(0.0, base_scale, is_active);

        let last_curve_idx = TRAIL_THICKNESS_CURVE_SAMPLES - 1u;
        let curve_idx_f = (1.0 - section_frac) * f32(last_curve_idx);
        let curve_lo = min(u32(curve_idx_f), last_curve_idx);
        let curve_hi = min(curve_lo + 1u, last_curve_idx);
        let curve_t = curve_idx_f - f32(curve_lo);
        let thickness = mix(
            emitter_uniforms.trail_thickness_curve[curve_lo],
            emitter_uniforms.trail_thickness_curve[curve_hi],
            curve_t
        );

        var trail_dir: vec3<f32>;
        if (seg_lo != seg_hi) {
            trail_dir = pos_hi - pos_lo;
        } else {
            let prev_pos = sorted_particles[head_slot + max(seg_lo, 1u) - 1u].position.xyz;
            trail_dir = pos_lo - prev_pos;
        }
        let dir_len = length(trail_dir);
        if (dir_len > 0.001) {
            trail_dir = trail_dir / dir_len;
        } else {
            trail_dir = -normalize(head_particle.velocity.xyz);
        }

        let orient = align_y_to_direction(trail_dir);

        var cross_section = vertex.position;
        cross_section.x *= thickness;
        cross_section.z *= thickness;
        cross_section.y = 0.0;

        let is_local = emitter_uniforms.use_local_coords != 0u;

        if (is_local) {
            let offset = orient * (cross_section * particle_scale);
            let local_pos = trail_pos + offset;
            out.world_position = mesh_functions::mesh_position_local_to_world(
                world_from_local, vec4(local_pos, 1.0)
            );
        } else {
            let emitter_scale = vec3(
                length(world_from_local[0].xyz),
                length(world_from_local[1].xyz),
                length(world_from_local[2].xyz),
            );
            let offset = orient * (cross_section * particle_scale * emitter_scale);
            out.world_position = vec4(trail_pos + offset, 1.0);
        }

        out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_NORMALS
        let rotated_normal = orient * vertex.normal;
        if (is_local) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(rotated_normal, vertex.instance_index);
        } else {
            out.world_normal = rotated_normal;
        }
#endif

#ifdef VERTEX_TANGENTS
        let rotated_tangent_trail = orient * vertex.tangent.xyz;
        if (is_local) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(rotated_tangent_trail, vertex.tangent.w), vertex.instance_index);
        } else {
            out.world_tangent = vec4(rotated_tangent_trail, vertex.tangent.w);
        }
#endif

#ifdef VERTEX_COLORS
        let color_lo = sorted_particles[idx_lo].color;
        let color_hi = sorted_particles[idx_hi].color;
        out.color = vertex.color * mix(color_lo, color_hi, seg_t);
#endif

        return out;
    }

    let particle = sorted_particles[particle_index];

    let flags = bitcast<u32>(particle.custom.w);
    let is_active = (flags & PARTICLE_FLAG_ACTIVE) != 0u;

    let particle_position = particle.position.xyz;
    let particle_scale = select(0.0, particle.position.w * emitter_uniforms.drive_slots[DRIVE_SLOT_SIZE_MUL], is_active);
    let is_local = emitter_uniforms.use_local_coords != 0u;

    var rotated_position = vertex.position;
#ifdef VERTEX_NORMALS
    var rotated_normal = vertex.normal;
#endif
#ifdef VERTEX_TANGENTS
    var rotated_tangent = vertex.tangent.xyz;
#endif

    let transform_align = emitter_uniforms.transform_align;

    if transform_align == TRANSFORM_ALIGN_Y_TO_VELOCITY {
        let alignment_dir = particle.alignment_dir.xyz;
        let dir_length = length(alignment_dir);
        if dir_length > 0.0 {
            let rotation_matrix = align_y_with_ref(alignment_dir, particle.ref_up.xyz);
            rotated_position = rotation_matrix * vertex.position;
#ifdef VERTEX_NORMALS
            rotated_normal = rotation_matrix * vertex.normal;
#endif
#ifdef VERTEX_TANGENTS
            rotated_tangent = rotation_matrix * vertex.tangent.xyz;
#endif
        }
    }

    let angles = particle.angles.xyz;
    var angle_rot = mat3x3<f32>(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0));
    if abs(angles.x) > 0.0001 { angle_rot = rot_x(angles.x) * angle_rot; }
    if abs(angles.y) > 0.0001 { angle_rot = rot_y(angles.y) * angle_rot; }
    if abs(angles.z) > 0.0001 { angle_rot = rot_z(angles.z) * angle_rot; }

    rotated_position = angle_rot * rotated_position;
#ifdef VERTEX_NORMALS
    rotated_normal = angle_rot * rotated_normal;
#endif
#ifdef VERTEX_TANGENTS
    rotated_tangent = angle_rot * rotated_tangent;
#endif

    let emitter_scale = vec3(
        length(world_from_local[0].xyz),
        length(world_from_local[1].xyz),
        length(world_from_local[2].xyz),
    );

    if transform_align == TRANSFORM_ALIGN_BILLBOARD || transform_align == TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY || transform_align == TRANSFORM_ALIGN_BILLBOARD_FIXED_Y {
        let cam_right = normalize(view.world_from_view[0].xyz);
        let cam_up = normalize(view.world_from_view[1].xyz);
        let cam_forward = normalize(view.world_from_view[2].xyz);

        var particle_world_pos: vec3<f32>;
        if is_local {
            particle_world_pos = (world_from_local * vec4(particle_position, 1.0)).xyz;
        } else {
            particle_world_pos = particle_position;
        }
        let scale = vec3(particle_scale) * emitter_scale;

        if transform_align == TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY {
            var v = particle.alignment_dir.xyz;
            if is_local {
                let emitter_rotation = mat3x3<f32>(
                    normalize(world_from_local[0].xyz),
                    normalize(world_from_local[1].xyz),
                    normalize(world_from_local[2].xyz)
                );
                v = emitter_rotation * v;
            }

            var sv = v - cam_forward * dot(cam_forward, v);
            if length(sv) < 0.001 {
                sv = cam_up;
            }
            sv = normalize(sv);

            let right = normalize(cross(sv, cam_forward));

            let scaled_vertex = rotated_position * scale;
            let pos = particle_world_pos
                + right * scaled_vertex.x
                + sv * scaled_vertex.y
                + cam_forward * scaled_vertex.z;

            out.world_position = vec4(pos, 1.0);
            out.position = position_world_to_clip(pos);

#ifdef VERTEX_NORMALS
            out.world_normal = right * rotated_normal.x
                + sv * rotated_normal.y
                + cam_forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                right * rotated_tangent.x + sv * rotated_tangent.y + cam_forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        } else if transform_align == TRANSFORM_ALIGN_BILLBOARD_FIXED_Y {
            let world_up = vec3(0.0, 1.0, 0.0);
            let right = normalize(cross(world_up, cam_forward));
            let forward = cross(right, world_up);

            let scaled_vertex = rotated_position * scale;
            let pos = particle_world_pos
                + right * scaled_vertex.x
                + world_up * scaled_vertex.y
                + forward * scaled_vertex.z;

            out.world_position = vec4(pos, 1.0);
            out.position = position_world_to_clip(pos);

#ifdef VERTEX_NORMALS
            out.world_normal = right * rotated_normal.x
                + world_up * rotated_normal.y
                + forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                right * rotated_tangent.x + world_up * rotated_tangent.y + forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        } else {
            let scaled_vertex = rotated_position * scale;
            let billboard_pos = particle_world_pos
                + cam_right * scaled_vertex.x
                + cam_up * scaled_vertex.y
                + cam_forward * scaled_vertex.z;

            out.world_position = vec4(billboard_pos, 1.0);
            out.position = position_world_to_clip(billboard_pos);

#ifdef VERTEX_NORMALS
            out.world_normal = cam_right * rotated_normal.x
                + cam_up * rotated_normal.y
                + cam_forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                cam_right * rotated_tangent.x + cam_up * rotated_tangent.y + cam_forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        }
    } else {
        let emitter_rotation = mat3x3(
            normalize(world_from_local[0].xyz),
            normalize(world_from_local[1].xyz),
            normalize(world_from_local[2].xyz)
        );

        if is_local {
            let offset = rotated_position * particle_scale;
            let local_position = offset + particle_position;
            out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(local_position, 1.0));
        } else {
            let offset = emitter_rotation * (rotated_position * particle_scale * emitter_scale);
            out.world_position = vec4(particle_position + offset, 1.0);
        }

        out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_NORMALS
        if is_local {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(rotated_normal, vertex.instance_index);
        } else {
            out.world_normal = emitter_rotation * rotated_normal;
        }
#endif
#ifdef VERTEX_TANGENTS
        if is_local {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(rotated_tangent, vertex.tangent.w), vertex.instance_index);
        } else {
            out.world_tangent = vec4(emitter_rotation * rotated_tangent, vertex.tangent.w);
        }
#endif
    }

#ifdef VERTEX_COLORS
    out.color = vertex.color * particle.color * vec4<f32>(
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_ALPHA],
    );
#endif

    return out;
}

#endif // MAY_DISCARD
#else // PREPASS_FRAGMENT

fn get_head_particle(particle_index: u32) -> Particle {
    let trail_size = emitter_uniforms.trail_size;
    let head_slot = particle_index * max(trail_size, 1u);
    return sorted_particles[head_slot];
}

fn get_trail_color(particle_index: u32, section_frac: f32) -> vec4<f32> {
    let trail_size = emitter_uniforms.trail_size;
    let head_slot = particle_index * trail_size;
    let last_seg = trail_size - 1u;
    let section_f = section_frac * f32(last_seg);
    let seg_lo = min(u32(section_f), last_seg);
    let seg_hi = min(seg_lo + 1u, last_seg);
    let seg_t = section_f - f32(seg_lo);
    let color_lo = sorted_particles[head_slot + seg_lo].color;
    let color_hi = sorted_particles[head_slot + seg_hi].color;
    return mix(color_lo, color_hi, seg_t);
}

fn particle_vertex_impl(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

    let particle_index = u32(round(vertex.uv_b.x));
    let trail_size = emitter_uniforms.trail_size;
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif

#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif

    if (trail_size > 1u) {
        let head_slot = particle_index * trail_size;
        let head_particle = sorted_particles[head_slot];
        let head_flags = bitcast<u32>(head_particle.custom.w);
        let is_active = (head_flags & PARTICLE_FLAG_ACTIVE) != 0u;

        let section_frac = vertex.uv_b.y;

        let last_seg = trail_size - 1u;
        let section_f = section_frac * f32(last_seg);
        let seg_lo = min(u32(section_f), last_seg);
        let seg_hi = min(seg_lo + 1u, last_seg);
        let seg_t = section_f - f32(seg_lo);

        let idx_lo = head_slot + seg_lo;
        let idx_hi = head_slot + seg_hi;
        let pos_lo = sorted_particles[idx_lo].position.xyz;
        let pos_hi = sorted_particles[idx_hi].position.xyz;
        let scale_lo = sorted_particles[idx_lo].position.w;
        let scale_hi = sorted_particles[idx_hi].position.w;

        let trail_pos = mix(pos_lo, pos_hi, seg_t);
        let base_scale = mix(scale_lo, scale_hi, seg_t);
        let particle_scale = select(0.0, base_scale, is_active);

        let last_curve_idx = TRAIL_THICKNESS_CURVE_SAMPLES - 1u;
        let curve_idx_f = (1.0 - section_frac) * f32(last_curve_idx);
        let curve_lo = min(u32(curve_idx_f), last_curve_idx);
        let curve_hi = min(curve_lo + 1u, last_curve_idx);
        let curve_t = curve_idx_f - f32(curve_lo);
        let thickness = mix(
            emitter_uniforms.trail_thickness_curve[curve_lo],
            emitter_uniforms.trail_thickness_curve[curve_hi],
            curve_t
        );

        var trail_dir: vec3<f32>;
        if (seg_lo != seg_hi) {
            trail_dir = pos_hi - pos_lo;
        } else {
            let prev_pos = sorted_particles[head_slot + max(seg_lo, 1u) - 1u].position.xyz;
            trail_dir = pos_lo - prev_pos;
        }
        let dir_len = length(trail_dir);
        if (dir_len > 0.001) {
            trail_dir = trail_dir / dir_len;
        } else {
            trail_dir = -normalize(head_particle.velocity.xyz);
        }

        let orient = align_y_to_direction(trail_dir);

        var cross_section = vertex.position;
        cross_section.x *= thickness;
        cross_section.z *= thickness;
        cross_section.y = 0.0;

        let is_local = emitter_uniforms.use_local_coords != 0u;

        if (is_local) {
            let offset = orient * (cross_section * particle_scale);
            let local_pos = trail_pos + offset;
            out.world_position = mesh_functions::mesh_position_local_to_world(
                world_from_local, vec4(local_pos, 1.0)
            );
        } else {
            let emitter_scale = vec3(
                length(world_from_local[0].xyz),
                length(world_from_local[1].xyz),
                length(world_from_local[2].xyz),
            );
            let offset = orient * (cross_section * particle_scale * emitter_scale);
            out.world_position = vec4(trail_pos + offset, 1.0);
        }

        out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_NORMALS
        let rotated_normal = orient * vertex.normal;
        if (is_local) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(rotated_normal, vertex.instance_index);
        } else {
            out.world_normal = rotated_normal;
        }
#endif

#ifdef VERTEX_TANGENTS
        let rotated_tangent_trail = orient * vertex.tangent.xyz;
        if (is_local) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(rotated_tangent_trail, vertex.tangent.w), vertex.instance_index);
        } else {
            out.world_tangent = vec4(rotated_tangent_trail, vertex.tangent.w);
        }
#endif

#ifdef VERTEX_COLORS
        let color_lo = sorted_particles[idx_lo].color;
        let color_hi = sorted_particles[idx_hi].color;
        out.color = vertex.color * mix(color_lo, color_hi, seg_t);
#endif

        return out;
    }

    let particle = sorted_particles[particle_index];

    let flags = bitcast<u32>(particle.custom.w);
    let is_active = (flags & PARTICLE_FLAG_ACTIVE) != 0u;

    let particle_position = particle.position.xyz;
    let particle_scale = select(0.0, particle.position.w * emitter_uniforms.drive_slots[DRIVE_SLOT_SIZE_MUL], is_active);
    let is_local = emitter_uniforms.use_local_coords != 0u;

    var rotated_position = vertex.position;
#ifdef VERTEX_NORMALS
    var rotated_normal = vertex.normal;
#endif
#ifdef VERTEX_TANGENTS
    var rotated_tangent = vertex.tangent.xyz;
#endif

    let transform_align = emitter_uniforms.transform_align;

    if transform_align == TRANSFORM_ALIGN_Y_TO_VELOCITY {
        let alignment_dir = particle.alignment_dir.xyz;
        let dir_length = length(alignment_dir);
        if dir_length > 0.0 {
            let rotation_matrix = align_y_with_ref(alignment_dir, particle.ref_up.xyz);
            rotated_position = rotation_matrix * vertex.position;
#ifdef VERTEX_NORMALS
            rotated_normal = rotation_matrix * vertex.normal;
#endif
#ifdef VERTEX_TANGENTS
            rotated_tangent = rotation_matrix * vertex.tangent.xyz;
#endif
        }
    }

    let angles = particle.angles.xyz;
    var angle_rot = mat3x3<f32>(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0));
    if abs(angles.x) > 0.0001 { angle_rot = rot_x(angles.x) * angle_rot; }
    if abs(angles.y) > 0.0001 { angle_rot = rot_y(angles.y) * angle_rot; }
    if abs(angles.z) > 0.0001 { angle_rot = rot_z(angles.z) * angle_rot; }

    rotated_position = angle_rot * rotated_position;
#ifdef VERTEX_NORMALS
    rotated_normal = angle_rot * rotated_normal;
#endif
#ifdef VERTEX_TANGENTS
    rotated_tangent = angle_rot * rotated_tangent;
#endif

    let emitter_scale = vec3(
        length(world_from_local[0].xyz),
        length(world_from_local[1].xyz),
        length(world_from_local[2].xyz),
    );

    if transform_align == TRANSFORM_ALIGN_BILLBOARD || transform_align == TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY || transform_align == TRANSFORM_ALIGN_BILLBOARD_FIXED_Y {
        let cam_right = normalize(view.world_from_view[0].xyz);
        let cam_up = normalize(view.world_from_view[1].xyz);
        let cam_forward = normalize(view.world_from_view[2].xyz);

        var particle_world_pos: vec3<f32>;
        if is_local {
            particle_world_pos = (world_from_local * vec4(particle_position, 1.0)).xyz;
        } else {
            particle_world_pos = particle_position;
        }
        let scale = vec3(particle_scale) * emitter_scale;

        if transform_align == TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY {
            var v = particle.alignment_dir.xyz;
            if is_local {
                let emitter_rotation = mat3x3<f32>(
                    normalize(world_from_local[0].xyz),
                    normalize(world_from_local[1].xyz),
                    normalize(world_from_local[2].xyz)
                );
                v = emitter_rotation * v;
            }

            var sv = v - cam_forward * dot(cam_forward, v);
            if length(sv) < 0.001 {
                sv = cam_up;
            }
            sv = normalize(sv);

            let right = normalize(cross(sv, cam_forward));

            let scaled_vertex = rotated_position * scale;
            let pos = particle_world_pos
                + right * scaled_vertex.x
                + sv * scaled_vertex.y
                + cam_forward * scaled_vertex.z;

            out.world_position = vec4(pos, 1.0);
            out.position = position_world_to_clip(pos);

#ifdef VERTEX_NORMALS
            out.world_normal = right * rotated_normal.x
                + sv * rotated_normal.y
                + cam_forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                right * rotated_tangent.x + sv * rotated_tangent.y + cam_forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        } else if transform_align == TRANSFORM_ALIGN_BILLBOARD_FIXED_Y {
            let world_up = vec3(0.0, 1.0, 0.0);
            let right = normalize(cross(world_up, cam_forward));
            let forward = cross(right, world_up);

            let scaled_vertex = rotated_position * scale;
            let pos = particle_world_pos
                + right * scaled_vertex.x
                + world_up * scaled_vertex.y
                + forward * scaled_vertex.z;

            out.world_position = vec4(pos, 1.0);
            out.position = position_world_to_clip(pos);

#ifdef VERTEX_NORMALS
            out.world_normal = right * rotated_normal.x
                + world_up * rotated_normal.y
                + forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                right * rotated_tangent.x + world_up * rotated_tangent.y + forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        } else {
            let scaled_vertex = rotated_position * scale;
            let billboard_pos = particle_world_pos
                + cam_right * scaled_vertex.x
                + cam_up * scaled_vertex.y
                + cam_forward * scaled_vertex.z;

            out.world_position = vec4(billboard_pos, 1.0);
            out.position = position_world_to_clip(billboard_pos);

#ifdef VERTEX_NORMALS
            out.world_normal = cam_right * rotated_normal.x
                + cam_up * rotated_normal.y
                + cam_forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                cam_right * rotated_tangent.x + cam_up * rotated_tangent.y + cam_forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        }
    } else {
        let emitter_rotation = mat3x3(
            normalize(world_from_local[0].xyz),
            normalize(world_from_local[1].xyz),
            normalize(world_from_local[2].xyz)
        );

        if is_local {
            let offset = rotated_position * particle_scale;
            let local_position = offset + particle_position;
            out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(local_position, 1.0));
        } else {
            let offset = emitter_rotation * (rotated_position * particle_scale * emitter_scale);
            out.world_position = vec4(particle_position + offset, 1.0);
        }

        out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_NORMALS
        if is_local {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(rotated_normal, vertex.instance_index);
        } else {
            out.world_normal = emitter_rotation * rotated_normal;
        }
#endif
#ifdef VERTEX_TANGENTS
        if is_local {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(rotated_tangent, vertex.tangent.w), vertex.instance_index);
        } else {
            out.world_tangent = vec4(emitter_rotation * rotated_tangent, vertex.tangent.w);
        }
#endif
    }

#ifdef VERTEX_COLORS
    out.color = vertex.color * particle.color * vec4<f32>(
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_ALPHA],
    );
#endif

    return out;
}

#endif // PREPASS_FRAGMENT
#else // not PREPASS_PIPELINE (forward pass)

fn get_head_particle(particle_index: u32) -> Particle {
    let trail_size = emitter_uniforms.trail_size;
    let head_slot = particle_index * max(trail_size, 1u);
    return sorted_particles[head_slot];
}

fn get_trail_color(particle_index: u32, section_frac: f32) -> vec4<f32> {
    let trail_size = emitter_uniforms.trail_size;
    let head_slot = particle_index * trail_size;
    let last_seg = trail_size - 1u;
    let section_f = section_frac * f32(last_seg);
    let seg_lo = min(u32(section_f), last_seg);
    let seg_hi = min(seg_lo + 1u, last_seg);
    let seg_t = section_f - f32(seg_lo);
    let color_lo = sorted_particles[head_slot + seg_lo].color;
    let color_hi = sorted_particles[head_slot + seg_hi].color;
    return mix(color_lo, color_hi, seg_t);
}

fn particle_vertex_impl(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

    let particle_index = u32(round(vertex.uv_b.x));
    let trail_size = emitter_uniforms.trail_size;
    var world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif

#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif

    if (trail_size > 1u) {
        let head_slot = particle_index * trail_size;
        let head_particle = sorted_particles[head_slot];
        let head_flags = bitcast<u32>(head_particle.custom.w);
        let is_active = (head_flags & PARTICLE_FLAG_ACTIVE) != 0u;

        let section_frac = vertex.uv_b.y;

        let last_seg = trail_size - 1u;
        let section_f = section_frac * f32(last_seg);
        let seg_lo = min(u32(section_f), last_seg);
        let seg_hi = min(seg_lo + 1u, last_seg);
        let seg_t = section_f - f32(seg_lo);

        let idx_lo = head_slot + seg_lo;
        let idx_hi = head_slot + seg_hi;
        let pos_lo = sorted_particles[idx_lo].position.xyz;
        let pos_hi = sorted_particles[idx_hi].position.xyz;
        let scale_lo = sorted_particles[idx_lo].position.w;
        let scale_hi = sorted_particles[idx_hi].position.w;

        let trail_pos = mix(pos_lo, pos_hi, seg_t);
        let base_scale = mix(scale_lo, scale_hi, seg_t);
        let particle_scale = select(0.0, base_scale, is_active);

        let last_curve_idx = TRAIL_THICKNESS_CURVE_SAMPLES - 1u;
        let curve_idx_f = (1.0 - section_frac) * f32(last_curve_idx);
        let curve_lo = min(u32(curve_idx_f), last_curve_idx);
        let curve_hi = min(curve_lo + 1u, last_curve_idx);
        let curve_t = curve_idx_f - f32(curve_lo);
        let thickness = mix(
            emitter_uniforms.trail_thickness_curve[curve_lo],
            emitter_uniforms.trail_thickness_curve[curve_hi],
            curve_t
        );

        var trail_dir: vec3<f32>;
        if (seg_lo != seg_hi) {
            trail_dir = pos_hi - pos_lo;
        } else {
            let prev_pos = sorted_particles[head_slot + max(seg_lo, 1u) - 1u].position.xyz;
            trail_dir = pos_lo - prev_pos;
        }
        let dir_len = length(trail_dir);
        if (dir_len > 0.001) {
            trail_dir = trail_dir / dir_len;
        } else {
            trail_dir = -normalize(head_particle.velocity.xyz);
        }

        let orient = align_y_to_direction(trail_dir);

        var cross_section = vertex.position;
        cross_section.x *= thickness;
        cross_section.z *= thickness;
        cross_section.y = 0.0;

        let is_local = emitter_uniforms.use_local_coords != 0u;

        if (is_local) {
            let offset = orient * (cross_section * particle_scale);
            let local_pos = trail_pos + offset;
            out.world_position = mesh_functions::mesh_position_local_to_world(
                world_from_local, vec4(local_pos, 1.0)
            );
        } else {
            let emitter_scale = vec3(
                length(world_from_local[0].xyz),
                length(world_from_local[1].xyz),
                length(world_from_local[2].xyz),
            );
            let offset = orient * (cross_section * particle_scale * emitter_scale);
            out.world_position = vec4(trail_pos + offset, 1.0);
        }

        out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_NORMALS
        let rotated_normal = orient * vertex.normal;
        if (is_local) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(rotated_normal, vertex.instance_index);
        } else {
            out.world_normal = rotated_normal;
        }
#endif

#ifdef VERTEX_TANGENTS
        let rotated_tangent_trail = orient * vertex.tangent.xyz;
        if (is_local) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(rotated_tangent_trail, vertex.tangent.w), vertex.instance_index);
        } else {
            out.world_tangent = vec4(rotated_tangent_trail, vertex.tangent.w);
        }
#endif

#ifdef VERTEX_COLORS
        let color_lo = sorted_particles[idx_lo].color;
        let color_hi = sorted_particles[idx_hi].color;
        out.color = vertex.color * mix(color_lo, color_hi, seg_t);
#endif

        return out;
    }

    let particle = sorted_particles[particle_index];

    let flags = bitcast<u32>(particle.custom.w);
    let is_active = (flags & PARTICLE_FLAG_ACTIVE) != 0u;

    let particle_position = particle.position.xyz;
    let particle_scale = select(0.0, particle.position.w * emitter_uniforms.drive_slots[DRIVE_SLOT_SIZE_MUL], is_active);
    let is_local = emitter_uniforms.use_local_coords != 0u;

    var rotated_position = vertex.position;
#ifdef VERTEX_NORMALS
    var rotated_normal = vertex.normal;
#endif
#ifdef VERTEX_TANGENTS
    var rotated_tangent = vertex.tangent.xyz;
#endif

    let transform_align = emitter_uniforms.transform_align;

    if transform_align == TRANSFORM_ALIGN_Y_TO_VELOCITY {
        let alignment_dir = particle.alignment_dir.xyz;
        let dir_length = length(alignment_dir);
        if dir_length > 0.0 {
            let rotation_matrix = align_y_with_ref(alignment_dir, particle.ref_up.xyz);
            rotated_position = rotation_matrix * vertex.position;
#ifdef VERTEX_NORMALS
            rotated_normal = rotation_matrix * vertex.normal;
#endif
#ifdef VERTEX_TANGENTS
            rotated_tangent = rotation_matrix * vertex.tangent.xyz;
#endif
        }
    }

    let angles = particle.angles.xyz;
    var angle_rot = mat3x3<f32>(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0));
    if abs(angles.x) > 0.0001 { angle_rot = rot_x(angles.x) * angle_rot; }
    if abs(angles.y) > 0.0001 { angle_rot = rot_y(angles.y) * angle_rot; }
    if abs(angles.z) > 0.0001 { angle_rot = rot_z(angles.z) * angle_rot; }

    rotated_position = angle_rot * rotated_position;
#ifdef VERTEX_NORMALS
    rotated_normal = angle_rot * rotated_normal;
#endif
#ifdef VERTEX_TANGENTS
    rotated_tangent = angle_rot * rotated_tangent;
#endif

    let emitter_scale = vec3(
        length(world_from_local[0].xyz),
        length(world_from_local[1].xyz),
        length(world_from_local[2].xyz),
    );

    if transform_align == TRANSFORM_ALIGN_BILLBOARD || transform_align == TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY || transform_align == TRANSFORM_ALIGN_BILLBOARD_FIXED_Y {
        let cam_right = normalize(view.world_from_view[0].xyz);
        let cam_up = normalize(view.world_from_view[1].xyz);
        let cam_forward = normalize(view.world_from_view[2].xyz);

        var particle_world_pos: vec3<f32>;
        if is_local {
            particle_world_pos = (world_from_local * vec4(particle_position, 1.0)).xyz;
        } else {
            particle_world_pos = particle_position;
        }
        let scale = vec3(particle_scale) * emitter_scale;

        if transform_align == TRANSFORM_ALIGN_BILLBOARD_Y_TO_VELOCITY {
            var v = particle.alignment_dir.xyz;
            if is_local {
                let emitter_rotation = mat3x3<f32>(
                    normalize(world_from_local[0].xyz),
                    normalize(world_from_local[1].xyz),
                    normalize(world_from_local[2].xyz)
                );
                v = emitter_rotation * v;
            }

            var sv = v - cam_forward * dot(cam_forward, v);
            if length(sv) < 0.001 {
                sv = cam_up;
            }
            sv = normalize(sv);

            let right = normalize(cross(sv, cam_forward));

            let scaled_vertex = rotated_position * scale;
            let pos = particle_world_pos
                + right * scaled_vertex.x
                + sv * scaled_vertex.y
                + cam_forward * scaled_vertex.z;

            out.world_position = vec4(pos, 1.0);
            out.position = position_world_to_clip(pos);

#ifdef VERTEX_NORMALS
            out.world_normal = right * rotated_normal.x
                + sv * rotated_normal.y
                + cam_forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                right * rotated_tangent.x + sv * rotated_tangent.y + cam_forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        } else if transform_align == TRANSFORM_ALIGN_BILLBOARD_FIXED_Y {
            let world_up = vec3(0.0, 1.0, 0.0);
            let right = normalize(cross(world_up, cam_forward));
            let forward = cross(right, world_up);

            let scaled_vertex = rotated_position * scale;
            let pos = particle_world_pos
                + right * scaled_vertex.x
                + world_up * scaled_vertex.y
                + forward * scaled_vertex.z;

            out.world_position = vec4(pos, 1.0);
            out.position = position_world_to_clip(pos);

#ifdef VERTEX_NORMALS
            out.world_normal = right * rotated_normal.x
                + world_up * rotated_normal.y
                + forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                right * rotated_tangent.x + world_up * rotated_tangent.y + forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        } else {
            let scaled_vertex = rotated_position * scale;
            let billboard_pos = particle_world_pos
                + cam_right * scaled_vertex.x
                + cam_up * scaled_vertex.y
                + cam_forward * scaled_vertex.z;

            out.world_position = vec4(billboard_pos, 1.0);
            out.position = position_world_to_clip(billboard_pos);

#ifdef VERTEX_NORMALS
            out.world_normal = cam_right * rotated_normal.x
                + cam_up * rotated_normal.y
                + cam_forward * rotated_normal.z;
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4(
                cam_right * rotated_tangent.x + cam_up * rotated_tangent.y + cam_forward * rotated_tangent.z,
                vertex.tangent.w
            );
#endif
        }
    } else {
        let emitter_rotation = mat3x3(
            normalize(world_from_local[0].xyz),
            normalize(world_from_local[1].xyz),
            normalize(world_from_local[2].xyz)
        );

        if is_local {
            let offset = rotated_position * particle_scale;
            let local_position = offset + particle_position;
            out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(local_position, 1.0));
        } else {
            let offset = emitter_rotation * (rotated_position * particle_scale * emitter_scale);
            out.world_position = vec4(particle_position + offset, 1.0);
        }

        out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_NORMALS
        if is_local {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(rotated_normal, vertex.instance_index);
        } else {
            out.world_normal = emitter_rotation * rotated_normal;
        }
#endif
#ifdef VERTEX_TANGENTS
        if is_local {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(rotated_tangent, vertex.tangent.w), vertex.instance_index);
        } else {
            out.world_tangent = vec4(emitter_rotation * rotated_tangent, vertex.tangent.w);
        }
#endif
    }

#ifdef VERTEX_COLORS
    out.color = vertex.color * particle.color * vec4<f32>(
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_TINT],
        emitter_uniforms.drive_slots[DRIVE_SLOT_ALPHA],
    );
#endif

    return out;
}

#endif // not PREPASS_PIPELINE

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

#ifdef PREPASS_PIPELINE
#ifndef PREPASS_FRAGMENT
#ifndef MAY_DISCARD
    out.position = vec4(2.0, 2.0, 0.0, 1.0);
    out.world_position = vec4(0.0);
    return out;
#else
    return particle_vertex_impl(vertex);
#endif
#else
    return particle_vertex_impl(vertex);
#endif
#else
    return particle_vertex_impl(vertex);
#endif
}

#ifdef PREPASS_PIPELINE
#ifndef PREPASS_FRAGMENT
#ifdef MAY_DISCARD
@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) {
#ifdef VERTEX_UVS_B
    let particle_index = u32(round(in.uv_b.x));
    let particle = get_head_particle(particle_index);
    var particle_color: vec4<f32>;
    if (emitter_uniforms.trail_size > 1u) {
        particle_color = get_trail_color(particle_index, in.uv_b.y);
    } else {
        particle_color = particle.color;
    }
#else
    let particle = sorted_particles[0u];
    let particle_color = particle.color;
#endif

    let flags = bitcast<u32>(particle.custom.w);
    let is_active = (flags & PARTICLE_FLAG_ACTIVE) != 0u;

    if (!is_active || particle_color.a < 0.001) {
        discard;
    }
}
#endif // MAY_DISCARD
#endif // not PREPASS_FRAGMENT
#endif // PREPASS_PIPELINE

// deferred prepass fragment (normal/motion vector/deferred passes)
#ifdef PREPASS_PIPELINE
#ifdef PREPASS_FRAGMENT
@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
#ifdef VERTEX_UVS_B
    let particle_index = u32(round(in.uv_b.x));
    let particle = get_head_particle(particle_index);
    var particle_color: vec4<f32>;
    if (emitter_uniforms.trail_size > 1u) {
        particle_color = get_trail_color(particle_index, in.uv_b.y);
    } else {
        particle_color = particle.color;
    }
#else
    let particle = sorted_particles[0u];
    let particle_color = particle.color;
#endif

    let flags = bitcast<u32>(particle.custom.w);
    let is_active = (flags & PARTICLE_FLAG_ACTIVE) != 0u;

    if (!is_active || particle_color.a < 0.001) {
        discard;
    }

    var frag_in = in;
#ifdef FX_SCROLL
    // drive_slots let a host variable modulate the authored rate at runtime;
    // an untouched slot carries 1.0, so an undriven effect scrolls exactly as
    // authored.
    let scroll_rate = fx.scroll_tiling.xy * vec2<f32>(
        emitter_uniforms.drive_slots[DRIVE_SLOT_SCROLL_U],
        emitter_uniforms.drive_slots[DRIVE_SLOT_SCROLL_V],
    );
    frag_in.uv = in.uv * fx.scroll_tiling.zw + scroll_rate * globals.time;
#endif
#ifdef FX_FLOW
    // Offsetting the base UV by a second, independently scrolling texture is
    // what makes fire and smoke CHURN. A single scrolling layer reads as a
    // sliding sheet no matter how good the texture is.
    let flow_uv = in.uv * fx.scroll_tiling.zw + fx.flow_fresnel.yz * globals.time;
    let flow = textureSample(flow_texture, flow_sampler, flow_uv).rg * 2.0 - 1.0;
    let flow_amount = fx.flow_fresnel.x * emitter_uniforms.drive_slots[DRIVE_SLOT_FLOW];
    frag_in.uv = frag_in.uv + flow * flow_amount;
#endif

    var pbr_input = pbr_input_from_standard_material(frag_in, is_front);
#ifdef FX_GRADIENT
    // Treat the base texture as a MASK and colour it through an authored
    // gradient, rather than using its own colour. This is why one greyscale
    // smoke texture can serve a dozen effects, and it is the colour authoring
    // surface -- which is why the Tint drive slot stays a scalar multiplier
    // and does not also try to author colour. Placed right after the base
    // color sample (before FX_EROSION's rim mix and FX_FRESNEL's rgb
    // multiply) so both later stages operate on the remapped colour, the
    // same "immediately after the sample" placement FX_EROSION documents for
    // itself below.
    let gradient_mask = pbr_input.material.base_color.r;
    let gradient_remapped = textureSample(gradient_texture, gradient_sampler, vec2<f32>(gradient_mask, 0.5));
    pbr_input.material.base_color = vec4<f32>(
        gradient_remapped.rgb,
        gradient_remapped.a * pbr_input.material.base_color.a,
    );
#endif
#ifdef FX_EROSION
    // The signature stylized burn-away: sample noise, discard below a moving
    // threshold, and emit a bright rim in the band just above it. Driving the
    // threshold from a variable is how an effect dissolves on command. Sampled
    // at frag_in.uv (post scroll/flow) so the dissolve pattern travels with
    // the churned surface rather than sitting still on top of it. Placed
    // right after the base color sample and before every later stage
    // (emissive scaling, the particle-color multiply, lighting, and -- in the
    // forward fragment -- alpha_discard), so a discarded fragment pays for
    // none of that.
    let erosion_noise = textureSample(erosion_texture, erosion_sampler, frag_in.uv).r;
    let erosion_threshold = clamp(
        fx.erosion_soft.x * emitter_uniforms.drive_slots[DRIVE_SLOT_EROSION],
        0.0, 1.0,
    );
    if (erosion_noise < erosion_threshold) {
        discard;
    }
    let erosion_edge = fx.erosion_soft.y;
    if (erosion_edge > 0.0) {
        // 1 at the cut, falling to 0 `edge` above it.
        let erosion_rim = 1.0 - clamp((erosion_noise - erosion_threshold) / erosion_edge, 0.0, 1.0);
        pbr_input.material.base_color = mix(
            pbr_input.material.base_color,
            fx.erosion_edge_color,
            erosion_rim * fx.erosion_edge_color.a,
        );
    }
#endif
#ifdef FX_FRESNEL
    // Rim brightening: a dot product and a power, but it's much of why a
    // stylized mesh reads as volumetric rather than as a flat painted shape.
    // Uses the raw vertex normal/position (unaffected by FX_SCROLL/FX_FLOW's
    // UV distortion), so it composes independently of them.
    //
    // For a camera-facing billboard the normal points at the camera
    // everywhere, so `dot(n, v)` is ~1 everywhere and the rim is
    // near-uniform -- this earns its keep on mesh particles (cones, spheres,
    // tubes), not quads. An author who tries it on a quad first will
    // conclude it is broken; it isn't, it just has nothing to rim.
    let fresnel_n = normalize(in.world_normal);
    let fresnel_v = normalize(view.world_position.xyz - in.world_position.xyz);
    let fresnel_power = fx.flow_fresnel.w * emitter_uniforms.drive_slots[DRIVE_SLOT_FRESNEL];
    let fresnel_rim = pow(1.0 - saturate(dot(fresnel_n, fresnel_v)), max(fresnel_power, 0.001));
    pbr_input.material.base_color = vec4<f32>(
        pbr_input.material.base_color.rgb
            + pbr_input.material.base_color.rgb * fresnel_rim * fx.erosion_soft.z,
        pbr_input.material.base_color.a,
    );
#endif
    // DRIVE_SLOT_EMISSIVE: an untouched slot carries 1.0, so an undriven
    // effect's emissive contribution is unchanged. Scaling rgb only leaves
    // the exposure-weight alpha channel (read in pbr_functions.wgsl) alone.
    pbr_input.material.emissive = vec4<f32>(
        pbr_input.material.emissive.rgb * emitter_uniforms.drive_slots[DRIVE_SLOT_EMISSIVE],
        pbr_input.material.emissive.a,
    );
    pbr_input.material.base_color = pbr_input.material.base_color * particle_color;
    let out = deferred_output(in, pbr_input);

    return out;
}
#endif // PREPASS_FRAGMENT
#endif // PREPASS_PIPELINE

// forward rendering fragment
#ifndef PREPASS_PIPELINE
@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
#ifdef VERTEX_UVS_B
    let particle_index = u32(round(in.uv_b.x));
    let particle = get_head_particle(particle_index);
    var particle_color: vec4<f32>;
    if (emitter_uniforms.trail_size > 1u) {
        particle_color = get_trail_color(particle_index, in.uv_b.y);
    } else {
        particle_color = particle.color;
    }
#else
    let particle = sorted_particles[0u];
    let particle_color = particle.color;
#endif

    let flags = bitcast<u32>(particle.custom.w);
    let is_active = (flags & PARTICLE_FLAG_ACTIVE) != 0u;

    if (!is_active || particle_color.a < 0.001) {
        discard;
    }

    var frag_in = in;
#ifdef FX_SCROLL
    // drive_slots let a host variable modulate the authored rate at runtime;
    // an untouched slot carries 1.0, so an undriven effect scrolls exactly as
    // authored.
    let scroll_rate = fx.scroll_tiling.xy * vec2<f32>(
        emitter_uniforms.drive_slots[DRIVE_SLOT_SCROLL_U],
        emitter_uniforms.drive_slots[DRIVE_SLOT_SCROLL_V],
    );
    frag_in.uv = in.uv * fx.scroll_tiling.zw + scroll_rate * globals.time;
#endif
#ifdef FX_FLOW
    // Offsetting the base UV by a second, independently scrolling texture is
    // what makes fire and smoke CHURN. A single scrolling layer reads as a
    // sliding sheet no matter how good the texture is.
    let flow_uv = in.uv * fx.scroll_tiling.zw + fx.flow_fresnel.yz * globals.time;
    let flow = textureSample(flow_texture, flow_sampler, flow_uv).rg * 2.0 - 1.0;
    let flow_amount = fx.flow_fresnel.x * emitter_uniforms.drive_slots[DRIVE_SLOT_FLOW];
    frag_in.uv = frag_in.uv + flow * flow_amount;
#endif

    var pbr_input = pbr_input_from_standard_material(frag_in, is_front);
#ifdef FX_GRADIENT
    // Treat the base texture as a MASK and colour it through an authored
    // gradient, rather than using its own colour. This is why one greyscale
    // smoke texture can serve a dozen effects, and it is the colour authoring
    // surface -- which is why the Tint drive slot stays a scalar multiplier
    // and does not also try to author colour. Placed right after the base
    // color sample (before FX_EROSION's rim mix and FX_FRESNEL's rgb
    // multiply) so both later stages operate on the remapped colour, the
    // same "immediately after the sample" placement FX_EROSION documents for
    // itself below.
    let gradient_mask = pbr_input.material.base_color.r;
    let gradient_remapped = textureSample(gradient_texture, gradient_sampler, vec2<f32>(gradient_mask, 0.5));
    pbr_input.material.base_color = vec4<f32>(
        gradient_remapped.rgb,
        gradient_remapped.a * pbr_input.material.base_color.a,
    );
#endif
#ifdef FX_EROSION
    // The signature stylized burn-away: sample noise, discard below a moving
    // threshold, and emit a bright rim in the band just above it. Driving the
    // threshold from a variable is how an effect dissolves on command. Sampled
    // at frag_in.uv (post scroll/flow) so the dissolve pattern travels with
    // the churned surface rather than sitting still on top of it. Placed
    // right after the base color sample and before every later stage
    // (emissive scaling, the particle-color multiply, lighting, and -- in the
    // forward fragment -- alpha_discard), so a discarded fragment pays for
    // none of that.
    let erosion_noise = textureSample(erosion_texture, erosion_sampler, frag_in.uv).r;
    let erosion_threshold = clamp(
        fx.erosion_soft.x * emitter_uniforms.drive_slots[DRIVE_SLOT_EROSION],
        0.0, 1.0,
    );
    if (erosion_noise < erosion_threshold) {
        discard;
    }
    let erosion_edge = fx.erosion_soft.y;
    if (erosion_edge > 0.0) {
        // 1 at the cut, falling to 0 `edge` above it.
        let erosion_rim = 1.0 - clamp((erosion_noise - erosion_threshold) / erosion_edge, 0.0, 1.0);
        pbr_input.material.base_color = mix(
            pbr_input.material.base_color,
            fx.erosion_edge_color,
            erosion_rim * fx.erosion_edge_color.a,
        );
    }
#endif
#ifdef FX_FRESNEL
    // Rim brightening: a dot product and a power, but it's much of why a
    // stylized mesh reads as volumetric rather than as a flat painted shape.
    // Uses the raw vertex normal/position (unaffected by FX_SCROLL/FX_FLOW's
    // UV distortion), so it composes independently of them.
    //
    // For a camera-facing billboard the normal points at the camera
    // everywhere, so `dot(n, v)` is ~1 everywhere and the rim is
    // near-uniform -- this earns its keep on mesh particles (cones, spheres,
    // tubes), not quads. An author who tries it on a quad first will
    // conclude it is broken; it isn't, it just has nothing to rim.
    let fresnel_n = normalize(in.world_normal);
    let fresnel_v = normalize(view.world_position.xyz - in.world_position.xyz);
    let fresnel_power = fx.flow_fresnel.w * emitter_uniforms.drive_slots[DRIVE_SLOT_FRESNEL];
    let fresnel_rim = pow(1.0 - saturate(dot(fresnel_n, fresnel_v)), max(fresnel_power, 0.001));
    pbr_input.material.base_color = vec4<f32>(
        pbr_input.material.base_color.rgb
            + pbr_input.material.base_color.rgb * fresnel_rim * fx.erosion_soft.z,
        pbr_input.material.base_color.a,
    );
#endif
    // DRIVE_SLOT_EMISSIVE: an untouched slot carries 1.0, so an undriven
    // effect's emissive contribution is unchanged. Scaling rgb only leaves
    // the exposure-weight alpha channel (read in pbr_functions.wgsl) alone.
    pbr_input.material.emissive = vec4<f32>(
        pbr_input.material.emissive.rgb * emitter_uniforms.drive_slots[DRIVE_SLOT_EMISSIVE],
        pbr_input.material.emissive.a,
    );
    pbr_input.material.base_color = pbr_input.material.base_color * particle_color;
#ifdef FX_SOFT
    // Fade as the fragment approaches whatever opaque geometry is behind it
    // -- the hard intersection line where a quad clips the floor is the
    // single most common tell of amateur VFX; this is the whole fix.
    //
    // Forward fragment ONLY: `prepass_depth` reads the depth texture the
    // depth prepass *produced*, i.e. a texture that must already be
    // complete. The other two fragment functions in this file (the
    // depth-only discard prepass and the deferred/normal/motion prepass)
    // execute DURING that same prepass and cannot meaningfully read their
    // own not-yet-finished output -- at best garbage, at worst a
    // driver-undefined read of a texture bound as both render target and
    // sampled resource in the same pass. Only this forward fragment runs in
    // a later pass, after the depth prepass has completed for the frame, so
    // it's the only one of the three where this read is coherent. See the
    // `#ifdef FX_SOFT #import ... prepass_depth #endif` above for why the
    // camera must carry `DepthPrepass` for this to compile at all.
    //
    // `in.position.z` is already device/NDC depth (the @builtin(position)
    // fragment input is post-perspective-divide), the same space
    // `prepass_depth` returns -- both convert through
    // `depth_ndc_to_view_z` into linear view-space units before
    // subtracting, so the fade distance means the same thing at every
    // camera range. Under reversed-Z, nearer is a LARGER raw depth value,
    // but view-space z is uniformly negative (view space: -z is forward),
    // so the plain subtraction is already correct without a sign flip.
    let soft_scene_depth = prepass_depth(in.position, 0u);
    let soft_this_view_z = depth_ndc_to_view_z(in.position.z);
    let soft_scene_view_z = depth_ndc_to_view_z(soft_scene_depth);
    let soft_fade = saturate(
        abs(soft_scene_view_z - soft_this_view_z) / max(fx.erosion_soft.w, 0.0001),
    );
    pbr_input.material.base_color.a = pbr_input.material.base_color.a * soft_fade;
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    let particle_alpha = pbr_input.material.base_color.a;

    var out: FragmentOutput;

    // Runtime-branched on the material's `unlit` flag, deliberately -- NOT a
    // shader def. An earlier plan draft assumed this needed `#ifdef`-gating
    // because bevy's own `pbr.wgsl:81-85` branches the unlit bit outside
    // `apply_pbr_lighting`, so a shader calling that fn unconditionally
    // would ignore `unlit` entirely (this exact bug shipped in a sibling
    // project's ToonMaterial). It does not apply here: this fragment reads
    // `STANDARD_MATERIAL_FLAGS_UNLIT_BIT` itself and only calls
    // `apply_pbr_lighting` in the else branch below, so `unlit` already
    // reaches the fragment correctly through `StandardParticleMaterial::
    // to_standard_material`'s `unlit: self.unlit` forward
    // (asset/particle_material.rs). See
    // `bevy_sprinkles::asset::particle_material::tests` for the Rust-side
    // pin and this file's `assert_occurs_only_in_forward_fragment` test in
    // `material.rs` for the shader-side one. There is no `FX_LIT` def --
    // one was pushed in `specialize` with no `#ifdef` anywhere to read it,
    // and was removed rather than wired up, since this runtime branch
    // already does the job.
    let is_unlit = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) != 0u;

    if is_unlit {
        out.color = pbr_input.material.base_color + pbr_input.material.emissive;
    } else {
        out.color = apply_pbr_lighting(pbr_input);
    }

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#ifndef PREMULTIPLY_ALPHA
    out.color.a = particle_alpha;
#endif

    return out;
}
#endif // not PREPASS_PIPELINE
