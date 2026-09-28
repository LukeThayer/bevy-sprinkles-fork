# From particle editor to VFX editor

**Status:** approved design, 2026-09-28
**Repo:** `LukeThayer/bevy-sprinkles-fork` (this one). Not orgonic.

## Intent

Turn `bevy_sprinkles` + `bevy_sprinkles_editor` from a *particle* system into a
*VFX* system, and the editor into a standalone VFX authoring tool. The visual
north star is Valorant-style stylized FX: hard-edged, saturated, readable at a
glance, snappy (sub-300 ms), built from scrolled and eroded texture sheets on
simple geometry with additive cores — not from particle count.

Three pillars, specced together as one subsystem because they share one spine:

1. **Variables** — an effect declares its own named knobs; the host game drives
   them per instance at runtime.
2. **Scrolling textures and mesh FX** — the material features that produce the
   look.
3. **Lighting** — effects that light the scene, not just glow.

## Success criteria

- An effect file declares a variable (say `temperature`), and a host game sets
  it per entity with no knowledge of that effect's internals.
- One variable drives several unrelated properties through separately authored
  curves — the worked example: `temperature` → hue shift (linear), size
  (quadratic), light flicker frequency, and scroll rate.
- Two entities sharing one effect asset, with different variable values, render
  differently. Per-instance isolation is a guarantee, not an accident.
- An author can build a Valorant-grade explosion in the editor: a scrolled and
  eroded cone, a shockwave ring, an additive core, a light flash, sparks.
- Existing `.ron` effects load unchanged.

## Non-goals

Explicitly out of scope, each decided rather than overlooked:

- **Modal (vim) editing.** Dropped by ruling; use the existing editor UX.
- **The orgonic migration.** orgonic has 53 `bevy_sprinkles` references, three
  live assets (`glacial.ron`, `ionic.ron`, `steam.ron`), a parallel in-house
  `src/vfx/` runtime, and F8 Effect/Preview tabs. Retiring that in favour of
  this is real work in another repo and gets its own spec after this lands.
- **Custom mesh import** (`.glb` effect geometry). Built-in primitives only.
- **Particles as light sources.** One aggregate light per effect is the right
  answer; per-particle lights are the classic expensive mistake.
- **Determinism.** Ruled not required — playback is cosmetic, so
  `ParticleSystemRuntime::global_seed` stays time-seeded and two clients may see
  different instances of the same explosion.
- **Per-particle non-uniform scale.** Ruled per-emitter scale is enough, which
  keeps `ParticleData`'s scalar `position.w` scale and avoids a GPU layout
  change.

## What already exists (do not rebuild)

Verified by reading the tree at `b4ce468`:

- GPU compute simulation (`shaders/particle_simulate.wgsl`, 1578 lines), sorting,
  trails, sub-emitters, collision, turbulence, per-particle mesh selection,
  `TransformAlign` billboarding.
- `CurveTexture` (modes, easings, tension) and `Gradient`, both baked to 1D GPU
  textures through `CurveTextureCache`/`GradientTextureCache`, and both with
  mature editor widgets (`curve_edit` 1945 lines, `gradient_edit` 1635,
  `color_picker` 1491).
- `ExtendedMaterial<StandardMaterial, ParticleMaterialExtension>`, with
  `emissive` and `emissive_texture` on the standard half.
- Versioned asset format with migration (`asset/versions/`, `migrate_str`).
- `override.rs`: per-instance overrides with per-emitter maps and per-instance
  baked textures.
- **`ParticleEmitterUniforms` already carries per-instance `tint: Vec4` and
  `size_mul: f32`, written to a GPU storage buffer every frame by
  `write_emitter_uniforms`.** This is the single most important existing fact in
  this spec: the per-frame CPU→GPU path for per-instance scalars is already
  built and proven. Variables extend it rather than inventing it.

### What is genuinely missing

- **Variables**: nothing. `ParticleOverride` is a fixed seven-field struct set
  from code and not authorable in the editor.
- **Scrolling textures**: nothing. The only `scroll` in the codebase is the
  turbulence noise field's drift, which is not UV.
- **Scene lighting**: nothing. `emissive` makes a particle *look* bright; no
  effect emits light.

## Object model

```rust
pub struct ParticlesAsset {
    // ...existing: name, dimension, initial_transform, emitters, colliders,
    //    despawn_on_finish, authors, sprinkles_editor...
    pub variables: Vec<VariableDecl>,  // NEW
    pub drives:    Vec<Drive>,         // NEW
    pub lights:    Vec<LightData>,     // NEW
}
```

Three new fields, all `#[serde(default)]`, so existing files parse untouched.

### Why there is no `meshes` field

A "mesh effect" was designed in and then cut. An emitter with
`particles_amount: 1`, `one_shot: true`, zero velocity, `use_local_coords: true`
and `transform_align: None` **is** a single pinned cone, ring or tube — and it
arrives with lifetime curves, gradients, timing, delay, sub-emitters and the
whole existing inspector already working. A separate object type would have
re-implemented each of those to reach the same place, and would have made bursts
of N rings with spawn variation *harder* rather than easier.

The cut also removes a structural problem it created: with no separate mesh
object there is no `fx_material.wgsl`, so the scroll/flow/erosion/fresnel work
lands on `particle_material.wgsl` as one shader and one code path, instead of two
implementations kept in sync by discipline.

Authoring convenience is served by an editor **preset** that stamps out that
configuration, not by a runtime type.

### Why `lights` survives the same argument

A light cannot be "a particle that is a light": particles live in a GPU storage
buffer, a Bevy `PointLight` is a CPU-side ECS component, and there is no cheap
readback of particle positions to place lights at them. The asymmetry is real, so
lights stay a separate CPU-side object while meshes do not.

## Variables

```rust
pub struct VariableDecl {
    pub name:    String,   // "temperature"
    pub default: f32,
    pub range:   Range,    // authored bounds; drives the editor slider
}

/// Host-written, per entity. Absent variables fall back to `VariableDecl::default`.
#[derive(Component, Default)]
pub struct ParticleVariables(HashMap<String, f32>);
```

Host usage is `vars.set("temperature", 0.8)` on the entity holding `Particles3d`.
Two flames at different temperatures are two entities with two
`ParticleVariables` — this is the per-instance guarantee.

Names are resolved to `VariableId` indices once per asset load and cached; the
per-frame path is index lookups, not string hashing. A name in
`ParticleVariables` that no `VariableDecl` declares warns once and is ignored.

**`ParticleOverride` is replaced and removed.** Its seven hardcoded fields become
ordinary drives an author wires up. That is a breaking change to the crate's
public API, taken deliberately: keeping both would leave two ways to say the same
thing, one of them undiscoverable from the editor.

## Drives

```rust
pub struct Drive {
    pub variable: VariableId,   // typed, not a name string
    pub target:   DriveTarget,
    pub curve:    CurveTexture, // REUSES the existing type and its editor widget
    pub output:   Range,        // remap the curve's 0..1 output to [min, max]
    pub op:       DriveOp,      // Replace | Multiply | Add
}

pub enum DriveTarget {
    Emitter   { index: u8, prop: EmitterProp },
    Transform { index: u8, prop: TransformProp },  // the emitter entity's Transform
    Light     { index: u8, prop: LightProp },
}
```

Indices are typed and validated at load; a target naming a nonexistent emitter,
light or variable is a load error, not a silent fallback to stock. This follows
the rule orgonic's `EmitterId` doc already states for the same failure class: a
rename must never silently render stock forever.

The worked example is four `Drive`s off one `VariableId`, each with its own curve
shape — which is what makes "linear hue, quadratic size" expressible without a
special case.

### Stage is a property of the property, not a flag

`EmitterProp` variants are split so that an invalid combination is
unrepresentable, rather than carrying a `stage: Spawn | Render` field an author
could set wrongly:

```rust
pub enum Stage { Spawn, Sim, Render }

pub enum EmitterProp {
    // Spawn — read by particle_simulate.wgsl only when a particle is born.
    // Turning the knob does NOT change particles already in flight.
    SpawnProbability, Lifetime, InitialSpeed, SpawnSize, Spread, EmissionRadius,
    // Sim — read by particle_simulate.wgsl every step.
    // Turning the knob DOES change particles already in flight.
    Gravity, TurbulenceStrength,
    // Render — re-read every frame by particle_material.wgsl for all live particles.
    Tint, Alpha, SizeMul, EmissiveIntensity,
    ScrollU, ScrollV, FlowStrength, ErosionThreshold, FresnelPower,
}

impl EmitterProp { pub fn stage(self) -> Stage { /* exhaustive match, no wildcard */ } }
```

**Three stages, not two.** `Spawn` and `Sim` both land in the same simulation
uniform buffer, so the distinction is invisible in the plumbing — but it is
exactly what an author needs to predict, and getting it wrong is the difference
between a knob that reshapes a live plume and one that only affects the next
particle born. It is a documented, verified behavioural fact rather than a
routing detail: `params.gravity` is applied every step at
`particle_simulate.wgsl:1455-1456` (`physics_velocity + gravity * dt`), whereas
`get_initial_scale` is called once at birth. An earlier draft of this spec
collapsed these two into one "spawn" stage and was wrong about gravity.

The editor must surface the stage on each drive, because it is the answer to the
first question an author asks of a knob.

`SpawnSize` and `SizeMul` are separate targets rather than one `Size` with a
toggle. The author still declares the stage per drive — by choosing the target —
but cannot ask for a spawn-time tint or a render-time lifetime, neither of which
the architecture can deliver.

`TransformProp` and `LightProp` are always ECS-stage.

### There is no `Drag` target either, for the same class of reason

The spec originally listed `Drag` as a Sim-stage property. It is absent, because
**this engine implements no velocity damping at all** — the only trace is a
`// TODO: requires implementing damping` comment in `asset/mod.rs`. There is no
field for a drive to route to.

A drive target with nothing behind it is worse than a missing feature: the editor
paints a picker over every `EmitterProp`, so an author would select `Drag`, author
a curve for it, and watch nothing happen, with no error and nothing to search for.
That is the "dead dial" defect class, and the sibling project's notes record
catching six of them. Documenting the no-op in a doc comment does not help — doc
comments are read by code readers, not by authors.

Re-add it alongside an implementation of damping, not before.

### Emission rate needs a new uniform, not a scaled `amount`

"Temperature drives emission" is a motivating case, but it cannot be built the
obvious way. `EmitterUniforms::amount` is simultaneously the fixed particle-pool
size **and** the per-slot simulation gate — the compute shader skips a slot when
`idx >= amount` — so scaling it at runtime strands already-live particles in the
truncated slots, where they freeze and never despawn. `apply_spawn_override`
carries a doc comment saying exactly this and deliberately never touches
`amount`: "Runtime density control therefore isn't a spawn-scalar knob."

So drivable emission rate requires a **new** `spawn_probability: f32` uniform
(0..1) that gates spawning per slot, leaving `amount` fixed and live particles
undisturbed. `EmitterProp::SpawnProbability` targets that uniform; there is no
`Rate` target, because a target named `Rate` would invite exactly the
`amount`-scaling implementation the engine forbids.

This is the one place in this spec where a pillar needs new shader work rather
than new routing.

### Several drives on one target

Drives sharing a target apply **in declaration order**, starting from the value
authored on the emitter. `Multiply` and `Add` accumulate onto the running value;
`Replace` discards everything contributed so far, including the authored value.
Order is therefore observable, and the Drives list view is the place it is
edited. Two `Replace` drives on one target is legal and the last one wins — the
editor warns rather than forbids, since it is a normal intermediate state while
re-wiring.

> **Assumption, flagged.** The original ruling was "both stages, declared per
> binding". This derives the stage from the target instead, and in the course of
> doing so found a third stage the two-stage framing had hidden. It preserves
> per-drive stage control; it removes only the ability to declare a stage the
> architecture cannot deliver. Reverting to an explicit flag is a small change,
> but it would re-admit exactly the wrong combinations.

### `TransformProp` — driving emitter size and scale

```rust
pub enum TransformProp {
    ScaleX, ScaleY, ScaleZ, ScaleUniform,
    RotX, RotY, RotZ,
    PosX, PosY, PosZ,
}
```

Driving emitter scale is a first-class requirement, and it is nearly free: the
emitter entity's `Transform` is an ordinary Bevy `Transform`, and
`particle_material.wgsl` **already** applies a per-axis `emitter_scale` vec3 built
from it (lines 215, 301, 318). So a beam that lengthens along Y without widening
in X, driven by a variable, is a plain CPU component write — no GPU change.

This is also what makes the per-particle non-uniform scale non-goal acceptable:
the per-axis need is served at emitter level, where it is already supported.

## Routing: where a driven value lands

```
ParticleVariables (per entity, host-written)
        │
        │  evaluate_drives  —  CPU, Update. One curve sample per drive per instance.
        ▼
   resolved values, grouped by stage
        │
        ├─► Spawn ─┐
        │          ├─► simulation uniform buffer ──► particle_simulate.wgsl
        ├─► Sim   ─┘    Spawn values are read once, at particle birth.
        │               Sim values are read every step, so they reshape
        │               particles already in flight.
        │
        ├─► Render ──► ParticleEmitterUniforms ──► particle_material.wgsl
        │              Extends the existing tint/size_mul precedent. All live
        │              particles re-read it every frame.
        │
        └─► ECS    ──► PointLight/SpotLight fields, emitter Transform.
                       Plain component writes; no GPU plumbing.
```

The stage split needs **no new machinery**. Two of the three stages share one
buffer and differ only in when the shader reads them; the third is the material
uniform that already exists. That is why this design fits the existing
architecture instead of fighting it.

### The distinction a reader will otherwise get wrong

**Lifetime curves stay GPU-baked textures. Variable curves are CPU-sampled
scalars.** They are orthogonal axes, and they compose:

```
final size = SpawnSize × scale_over_lifetime(age) × drive(temperature)
             ^spawn       ^GPU texture, per particle  ^CPU scalar, per instance
```

A variable curve yields one value per instance per frame, not a per-particle
ramp, so baking it into a texture would be pure waste. Anyone extending this will
be tempted to reuse the existing baking path; this paragraph is why not to.

Cost: 100 live effects × 10 drives = 1000 curve samples per frame on the CPU.
Negligible.

### Uniform layout

`ParticleEmitterUniforms` is marked `LAYOUT-LOCKSTEP` with
`ParticleEmitterUniforms` in `shaders/common.wgsl` — the two must match field for
field. Adding a named field per drivable render property would mean editing two
files in sync forever, and getting it wrong is a silent corruption rather than a
compile error.

Instead, render-stage values go into a fixed-size slot array:

```rust
pub drive_slots: [f32; DRIVE_SLOT_COUNT],
```

Each render-stage `EmitterProp` maps to a fixed slot index. Adding a property
changes one integer rather than the struct's shape. A test asserts the Rust
`DRIVE_SLOT_COUNT` matches the WGSL array length, converting the lockstep comment
into something enforced.

`tint` and `size_mul` fold into slots and stop being named fields, alongside the
removal of `ParticleOverride`.

## Material feature set

All of this lands on `particle_material.wgsl`, applying to every particle
including mesh particles and ribbons.

- **UV scroll** — per-axis rate and tiling. The base of the whole look.
- **Flow / distortion map** — a second texture whose RG offsets the base UV, with
  its own scroll rate and strength. This is what makes fire and smoke *churn*;
  scroll alone reads as a sliding texture and is the single clearest tell of
  cheap VFX.
- **Erosion / dissolve** — noise texture, threshold, edge width, edge colour. The
  signature stylized burn-away edge, and the usual way a Valorant effect exits.
- **Fresnel rim** — cheap, and much of why stylized cones read as volumetric.
- **Soft particles** — depth-buffer fade. Hard intersection lines where a quad
  clips the floor are the most common amateur tell; this removes them.
- **Gradient remap** — sample the base texture as a mask and colour it through a
  `Gradient`, reusing the existing type and its editor widget.
- **Alpha mode** — Additive | Blend | Premultiplied, per emitter.

Every numeric field above is a render-stage `EmitterProp`, hence drivable.

### Lit particles

Add a `lit` flag. When set, the fragment routes through `apply_pbr_lighting`
instead of the additive path, so smoke and dust sit in the scene's light.

**This must be a shader-def branch, not a runtime uniform.** Bevy branches the
unlit bit in `pbr.wgsl:81-85`, *outside* `apply_pbr_lighting` — so a material that
calls `apply_pbr_lighting` unconditionally ignores an `unlit` flag entirely. This
exact bug has already been shipped once in a sibling project (orgonic's
`ToonMaterial`, where `unlit: true` never did anything), diagnosed during its SDF
winding fix. Cite that finding in the implementation.

## Ribbons

`ParticleMesh::RibbonTrail` and `TubeTrail` already generate strip geometry
(`mesh.rs`, with `sections` and `RibbonTrailShape`). The gap is only:

- UVs that run along the strip's length, so scroll reads as travel down a beam.
- The material feature set above, which they inherit for free.

This pillar is largely *finishing* an existing feature rather than building one —
a scope win worth stating so it is not planned as though it were greenfield.

## Lighting

```rust
pub struct LightData {
    pub name:      String,
    pub enabled:   bool,
    pub kind:      FxLightKind,   // Point | Spot
    pub transform: InitialTransform,
    pub color:     Color,
    pub intensity: f32,           // HDR
    pub range:     f32,
    pub time:      EmitterTime,   // REUSED — its own clock, for flash envelopes
    pub intensity_over_life: Option<CurveTexture>,  // CPU-sampled
    pub shadows:   bool,
}

pub enum LightProp { Intensity, Range, Hue, Saturation, Value }
```

Each `LightData` spawns a child entity carrying a `PointLight` or `SpotLight`,
parented to the effect. Reusing `EmitterTime` gives a light the same delay /
lifetime / one-shot / loop vocabulary an emitter has, so a muzzle flash is
authored the way everything else is.

Driving `LightProp::Intensity` from a variable through a curve is the "flicker
frequency from temperature" case from the original brief.

## Editor surface

Built on the existing infrastructure, per ruling — no new editing paradigm.

- **Variables panel** in the sidebar: add, rename, delete, set default and range.
  Each variable gets a live slider that scrubs it in the viewport. **This slider
  is the preview mechanism** — no separate preview concept is needed, because
  driving the knob and seeing the result *is* the workflow.
- **Drive affordance** on every drivable inspector field: a small control that
  opens the existing `curve_edit` widget bound to a chosen variable, plus the
  output range and op. Reuses `curve_edit` and `gradient_edit` wholesale.
- **Sidebar tree** gains **Lights** as a sibling of Emitters and Colliders,
  extending the existing `Inspectable` / `Inspecting` selection machinery.
- **Drives list view**: every drive in the effect in one place, so the wiring is
  legible without hunting field by field. A drive can be muted here to A/B it.
- **Mesh FX preset**: creates an emitter pre-configured as a single pinned mesh
  (amount 1, one-shot, zero velocity, local coords, no align), so the pattern
  that replaced the cut mesh object is one click rather than five fields.

## Format versioning

`ParticlesAsset` carries `sprinkles_version` and loads through
`versions::migrate`, with `v0_1.rs` as the precedent. This change bumps the
format version and adds a migration that supplies empty `variables`, `drives` and
`lights`, and rewrites any `ParticleOverride` usage encountered in editor project
data. Round-trip and migration tests follow the existing pattern.

## Testing

- **Format**: round-trip; migration from the previous version; validation
  rejects out-of-range target indices and undeclared variable names.
- **Drive evaluation**: `resolve(variables, drives) -> ResolvedDrives` is a pure
  function, table-tested across curve shapes, ops and output ranges. Keeping this
  pure is the highest-value testability decision in the design.
- **Stage routing**, one assertion per stage against the same live particle set:
  a `Spawn` drive must **not** alter particles already alive; a `Sim` drive
  **must**; a `Render` drive **must**, on the very next frame. All three
  asserted together — the contrast is what makes the split meaningful rather
  than decorative, and a `Spawn`/`Sim` pair is precisely what an earlier draft of
  this spec got wrong.
- **Per-instance isolation**: two entities, one asset, different
  `ParticleVariables` → different resolved uniforms. This is the headline
  guarantee and gets a named test.
- **Uniform lockstep**: `DRIVE_SLOT_COUNT` matches the WGSL array length.
- **Missing variable**: an undeclared name warns once and falls back to the
  default rather than panicking or rendering stock silently.

Per the sibling project's convention, every guard added is mutation-verified:
remove the guarded code, confirm the test fails, restore byte for byte. A test
that stays green under deletion of its subject is a defect.

## Risks

- **Upstream divergence.** This is a large departure from
  `doceazedo/sprinkles`. Merging upstream later gets materially harder. Accepted
  — the fork is the product now.
- **`ParticleOverride` removal is a breaking API change**, and orgonic currently
  uses it. Since the orgonic migration is a separate sub-project, orgonic stays
  pinned to the current fork commit until that spec runs. Worth confirming that
  pin holds before this lands.
- **Shader complexity.** `particle_material.wgsl` is already 1274 lines and this
  adds six feature blocks. Shader-defs should gate each so an effect using none
  of them compiles a lean variant, or fill rate will regress across every
  existing effect.
- **Editor field count.** The inspector grows a drive affordance on most numeric
  fields. If that is added field by field by hand it will be inconsistent; it
  should come from the existing reflect-driven inspector path.

## Open question

None blocking. The one flagged assumption is stage-derived-from-target, in
**Drives** above.
