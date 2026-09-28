# VFX Editor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `bevy_sprinkles` into a VFX system whose effects declare their own named knobs that a host game drives per instance, and whose material can produce stylized (Valorant-grade) scrolled, eroded, lit FX.

**Architecture:** One spine — a `Drive` maps `variable → curve → property`. Drives are evaluated on the CPU once per instance per frame (one curve sample each, not per particle) and the resolved value is routed by *stage* into one of three destinations that already exist: the simulation uniform (`EmitterUniforms`, via `extract.rs`), the material uniform (`ParticleEmitterUniforms`, via `spawning.rs::write_emitter_uniforms`), or plain ECS component writes. Pillars 2 and 3 (material features, lights) are consumers of that spine, which is why they are phases of one plan rather than separate plans.

**Tech Stack:** Rust 2024, Bevy 0.19, `ron` 0.10, `serde`, `bytemuck`, WGSL. Editor UI is bevy_ui-native (no egui).

**Spec:** `docs/superpowers/specs/2026-09-28-vfx-editor-design.md`

**Repo/branch:** `/home/luke/src/bevy-sprinkles-fork`, branch `vfx-editor`.

## Global Constraints

- **Bevy 0.19**, edition 2024, `resolver = "3"`. Never bump Bevy in this branch.
- **`EmitterUniforms::amount` must never be mutated at runtime.** It is both the particle-pool size and the per-slot simulation gate (`idx >= amount` skips a slot); scaling it strands live particles in truncated slots where they freeze and never despawn. See the existing doc comment on `extract.rs::apply_spawn_override`. Drivable emission rate uses the new `spawn_probability` uniform instead (Task 6).
- **LAYOUT-LOCKSTEP:** `ParticleEmitterUniforms` in `src/material.rs` and in `src/shaders/common.wgsl` must match field for field, in order. Same for `EmitterUniforms` in `src/extract.rs` and `EmitterParams` in `src/shaders/particle_simulate.wgsl`. A mismatch is silent memory corruption, not a compile error.
- **Existing `.ron` effects must keep loading.** Every new asset field is `#[serde(default)]`.
- **Variable curves are CPU-sampled scalars; lifetime curves stay GPU-baked textures.** Never bake a variable curve into a texture — it yields one value per instance per frame, not a per-particle ramp.
- **No `unwrap()`/`expect()` on asset-derived data.** A hand-authored `.ron` is untrusted input; it fails to load with a message, it does not panic the editor.
- **This machine is NixOS and there is NO `cargo` on `PATH`.** This repo has no
  flake of its own, so every cargo command goes through the *orgonic* dev shell,
  which carries the Rust toolchain, `pkg-config` and the Linux runtime libs a
  Bevy build needs:

  ```bash
  cd /home/luke/src/orgonic && nix develop . --command bash -c \
    'cd /home/luke/src/bevy-sprinkles-fork && <cargo command>'
  ```

  **Every `cargo ...` written anywhere in this plan means that wrapped form.** A
  bare `cargo` fails with `command not found`, which reads as a broken task
  rather than a missing toolchain. Ignore the dev shell's banner warning about a
  "second ~40 GB target tree" — it warns about building *orgonic* bare; this plan
  builds in the fork's own directory, where a `target/` is expected and gitignored.
- **Test command:** `cargo test -p bevy_sprinkles` for runtime tasks,
  `cargo test -p bevy_sprinkles_editor` for editor tasks, and
  `cargo test --workspace` before any phase-closing commit — each through the
  wrapper above.
- **Never pass `--target-dir`.** Use the default `target/`. A Bevy build tree is ~20 GB, so a
  private target dir silently duplicates it and makes your test counts incomparable to the
  recorded baseline. One implementer created an 11 GB `target/dev` this way.
- **Mutation-verify every guard:** delete the guarded code, confirm the new test fails, restore byte for byte. A test that stays green under deletion of its subject is a defect.

## Review Focus

These are the failure modes the spec implies but that no task's happy path exercises. Each has its test assigned to the task that owns the code.

1. **A `.ron` whose drive names an out-of-range emitter/light index or an undeclared variable** must fail to load with a message naming the offender — not panic, and not silently render stock forever. → Task 2.
2. **A host setting a variable name the effect does not declare** (a typo, or an effect swapped underneath) must warn **once** and use the declared default — not warn every frame, and not panic. → Task 3 (fallback) and Task 5 (warn-once).
3. **NaN or infinity** reaching a uniform from an authored curve value, an output range, or a host-set variable must be replaced with a finite value before it reaches the GPU. A NaN propagates through the vertex shader and can blank an entire draw call, which reads as "the effect is gone" rather than "a number was bad". → Task 3 (host values) and Task 4 (authored values).
4. **Two entities sharing one asset, only one carrying `ParticleVariables`**, must render independently: the bare one at declared defaults, not at its neighbour's values. This is the per-instance guarantee and the most likely place a buffer-reuse bug hides. → Task 5.
5. **A curve with zero control points, or an output `Range` whose min equals max**, must resolve to a finite constant rather than dividing by zero. → Task 4.

---

## File Structure

### `crates/bevy_sprinkles`

| File | Responsibility |
|---|---|
| `src/asset/variables.rs` **(new)** | `VariableDecl`, `VariableId`. Pure data. |
| `src/asset/drive.rs` **(new)** | `Drive`, `DriveTarget`, `EmitterProp`, `TransformProp`, `LightProp`, `Stage`, `DriveOp`, and `validate_drives`. Pure data + validation. |
| `src/asset/light.rs` **(new)** | `LightData`, `FxLightKind`. Pure data. |
| `src/asset/mod.rs` *(modify)* | Three new `ParticlesAsset` fields; re-exports. |
| `src/asset/versions/mod.rs` *(modify)* | Bump `0.3` → `0.4`, add migration arm. |
| `src/drives.rs` **(new)** | `ParticleVariables` component, `ResolvedDrives`, the pure `resolve_drives`, and the `evaluate_drives` system. The heart of the spine. |
| `src/lights.rs` **(new)** | Spawning and syncing effect-owned light entities. |
| `src/extract.rs` *(modify)* | `apply_spawn_override` → `apply_sim_drives`; add `spawn_probability`. |
| `src/spawning.rs` *(modify)* | `write_emitter_uniforms` reads `ResolvedDrives` instead of `ParticleOverride`. |
| `src/material.rs` *(modify)* | `drive_slots` array; new material extension fields. |
| `src/override.rs` **(delete)** | Replaced by drives. |
| `src/mesh.rs` *(modify)* | Length-wise UVs on ribbon/tube trails. |
| `src/shaders/common.wgsl` *(modify)* | `drive_slots` in `ParticleEmitterUniforms`. |
| `src/shaders/particle_simulate.wgsl` *(modify)* | `spawn_probability` gate. |
| `src/shaders/particle_material.wgsl` *(modify)* | Scroll, flow, erosion, fresnel, soft particles, gradient remap, lit branch. |

### `crates/bevy_sprinkles_editor`

| File | Responsibility |
|---|---|
| `src/ui/components/variables.rs` **(new)** | Variables panel: CRUD plus the live scrub slider that *is* the preview. |
| `src/ui/components/drives.rs` **(new)** | Drives list view: every drive in one place, mutable and mutable-order. |
| `src/ui/components/inspector/drive_button.rs` **(new)** | The per-field affordance that opens `curve_edit` bound to a variable. |
| `src/ui/components/inspector/light.rs` **(new)** | Light inspector section. |
| `src/ui/components/inspector/material_fx.rs` **(new)** | Scroll/flow/erosion/fresnel/soft/gradient inspector sections. |
| `src/state.rs` *(modify)* | `Inspectable::{Light, Variable}`. |
| `src/ui/components/sidebar.rs` *(modify)* | Lights and Variables tree sections. |

---

# Phase 1 — The Drive spine

## Task 1: Asset types for variables, drives and lights

**Files:**
- Create: `crates/bevy_sprinkles/src/asset/variables.rs`
- Create: `crates/bevy_sprinkles/src/asset/drive.rs`
- Create: `crates/bevy_sprinkles/src/asset/light.rs`
- Modify: `crates/bevy_sprinkles/src/asset/mod.rs` (add `pub mod` lines and the three `ParticlesAsset` fields)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `VariableDecl`, `VariableId(u16)`, `Drive`, `DriveTarget`, `EmitterProp`, `TransformProp`, `LightProp`, `Stage`, `DriveOp`, `LightData`, `FxLightKind`, and `EmitterProp::stage(self) -> Stage`. Every later task uses these names exactly.

- [ ] **Step 1: Confirm the branch and that the tree builds before touching it**

```bash
cd /home/luke/src/bevy-sprinkles-fork
git rev-parse --abbrev-ref HEAD     # expect: vfx-editor
cargo test -p bevy_sprinkles 2>&1 | tail -5
```
Record the passing test count. Every later task compares against it; an unexplained drop is a regression you introduced.

- [ ] **Step 2: Write the failing test for `EmitterProp::stage`**

Create `crates/bevy_sprinkles/src/asset/drive.rs` containing only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_props_and_sim_props_do_not_share_a_stage() {
        // The distinction is the whole point: a Spawn prop is read once at
        // particle birth, a Sim prop every step. Collapsing them was a real
        // bug in an earlier draft of the spec (gravity is applied every step
        // at particle_simulate.wgsl:1455-1456, not at birth).
        assert_eq!(EmitterProp::SpawnSize.stage(), Stage::Spawn);
        assert_eq!(EmitterProp::Gravity.stage(), Stage::Sim);
        assert_eq!(EmitterProp::Tint.stage(), Stage::Render);
    }

    #[test]
    fn every_render_prop_has_a_distinct_slot_and_they_are_dense() {
        let mut seen: Vec<usize> = EmitterProp::ALL
            .iter()
            .filter(|p| p.stage() == Stage::Render)
            .map(|p| p.slot().expect("a Render prop must have a slot"))
            .collect();
        seen.sort_unstable();
        let expected: Vec<usize> = (0..seen.len()).collect();
        assert_eq!(seen, expected, "render slots must be dense and unique");
        assert_eq!(seen.len(), DRIVE_SLOT_COUNT);
    }

    #[test]
    fn a_non_render_prop_has_no_slot() {
        assert!(EmitterProp::Gravity.slot().is_none());
        assert!(EmitterProp::SpawnSize.slot().is_none());
    }
}
```

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p bevy_sprinkles drive:: 2>&1 | tail -20`
Expected: FAIL — the module is not declared in `asset/mod.rs` yet and none of the types exist.

- [ ] **Step 4: Write `variables.rs`**

```rust
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use super::Range;

/// Index of a [`VariableDecl`] within [`ParticlesAsset::variables`].
///
/// Typed rather than a name string on purpose: a `Drive` naming its variable by
/// index can be rejected at load if the index is out of range, instead of
/// silently falling through to render stock forever the way a typo'd name would.
/// Names exist for humans and for the host API; the file stores indices.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize, Reflect)]
pub struct VariableId(pub u16);

/// One knob an effect exposes to the host game.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub struct VariableDecl {
    /// The name the host uses: `vars.set("temperature", 0.8)`.
    pub name: String,
    /// The value used when the host sets nothing.
    pub default: f32,
    /// Authored bounds. Drives the editor slider's range; NOT a clamp on what
    /// the host may set, because a host legitimately overshoots for punch.
    pub range: Range,
}

impl Default for VariableDecl {
    fn default() -> Self {
        Self { name: String::new(), default: 0.0, range: Range { min: 0.0, max: 1.0 } }
    }
}
```

- [ ] **Step 5: Write `drive.rs` above the test module you already created**

```rust
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use super::{CurveTexture, Range, variables::VariableId};

/// When a resolved drive value is read by the thing that consumes it.
///
/// `Spawn` and `Sim` both land in the SAME simulation uniform buffer, so this
/// is not a routing distinction — it is a behavioural one, and it is the first
/// question an author asks of a knob: "does this change what is already in the
/// air?" `Spawn` = no, `Sim` = yes.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Reflect)]
pub enum Stage { Spawn, Sim, Render }

/// How a resolved value combines with the emitter's authored value, and with
/// any earlier drive on the same target.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, Reflect)]
pub enum DriveOp {
    /// Discard everything contributed so far, including the authored value.
    Replace,
    #[default]
    Multiply,
    Add,
}

/// The number of render-stage slots carried in `ParticleEmitterUniforms`.
///
/// LAYOUT-LOCKSTEP with `DRIVE_SLOT_COUNT` in `shaders/common.wgsl`. Adding a
/// render property changes this integer and that one, and nothing else about
/// either struct's shape — which is the entire reason drives use a slot array
/// rather than a named uniform field per property.
pub const DRIVE_SLOT_COUNT: usize = 9;

/// A property of an emitter that a [`Drive`] can target.
///
/// Split so that an impossible combination is unrepresentable rather than
/// merely discouraged: `SpawnSize` and `SizeMul` are separate targets instead
/// of one `Size` with a stage flag, so no author can ask for a spawn-time tint
/// or a render-time lifetime — neither of which the architecture can deliver.
///
/// There is deliberately **no `Rate`**. See `SpawnProbability`.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Reflect)]
pub enum EmitterProp {
    // --- Spawn: read once, when a particle is born ---
    /// Fraction of eligible slots that actually spawn, 0..1.
    ///
    /// This exists instead of a `Rate` that scales `amount`, because `amount`
    /// is simultaneously the particle-pool size and the per-slot simulation
    /// gate — scaling it strands live particles in truncated slots where they
    /// freeze and never despawn (`extract.rs::apply_spawn_override`'s doc
    /// comment states this). A target named `Rate` would invite exactly that
    /// forbidden implementation.
    SpawnProbability,
    Lifetime,
    InitialSpeed,
    SpawnSize,
    Spread,
    EmissionRadius,

    // --- Sim: read every step; reshapes particles already in flight ---
    Gravity,
    Drag,
    TurbulenceStrength,

    // --- Render: re-read every frame for all live particles ---
    Tint,
    Alpha,
    SizeMul,
    EmissiveIntensity,
    ScrollU,
    ScrollV,
    FlowStrength,
    ErosionThreshold,
    FresnelPower,
}

impl EmitterProp {
    /// Every variant, so tests and editor menus cannot drift from the enum.
    pub const ALL: [EmitterProp; 18] = [
        Self::SpawnProbability, Self::Lifetime, Self::InitialSpeed, Self::SpawnSize,
        Self::Spread, Self::EmissionRadius,
        Self::Gravity, Self::Drag, Self::TurbulenceStrength,
        Self::Tint, Self::Alpha, Self::SizeMul, Self::EmissiveIntensity,
        Self::ScrollU, Self::ScrollV, Self::FlowStrength, Self::ErosionThreshold,
        Self::FresnelPower,
    ];

    /// Exhaustive match, no wildcard arm — a new variant is a compile error
    /// until it declares when it is read.
    pub fn stage(self) -> Stage {
        match self {
            Self::SpawnProbability | Self::Lifetime | Self::InitialSpeed
            | Self::SpawnSize | Self::Spread | Self::EmissionRadius => Stage::Spawn,

            Self::Gravity | Self::Drag | Self::TurbulenceStrength => Stage::Sim,

            Self::Tint | Self::Alpha | Self::SizeMul | Self::EmissiveIntensity
            | Self::ScrollU | Self::ScrollV | Self::FlowStrength
            | Self::ErosionThreshold | Self::FresnelPower => Stage::Render,
        }
    }

    /// Index into `ParticleEmitterUniforms::drive_slots`, for render props only.
    pub fn slot(self) -> Option<usize> {
        Some(match self {
            Self::Tint => 0,
            Self::Alpha => 1,
            Self::SizeMul => 2,
            Self::EmissiveIntensity => 3,
            Self::ScrollU => 4,
            Self::ScrollV => 5,
            Self::FlowStrength => 6,
            Self::ErosionThreshold => 7,
            Self::FresnelPower => 8,
            _ => return None,
        })
    }
}

/// A channel of the emitter entity's `Transform`. Always ECS-stage.
///
/// Driving scale here is how per-axis scale is achieved: the material shader
/// already builds a per-axis `emitter_scale` vec3 from this Transform
/// (`particle_material.wgsl:215,301,318`), so a beam that lengthens along Y
/// without widening in X is a plain component write and no GPU change.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Reflect)]
pub enum TransformProp {
    ScaleX, ScaleY, ScaleZ, ScaleUniform,
    RotX, RotY, RotZ,
    PosX, PosY, PosZ,
}

/// A property of an effect-owned light. Always ECS-stage.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Reflect)]
pub enum LightProp { Intensity, Range, Hue, Saturation, Value }

/// What a [`Drive`] writes to.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub enum DriveTarget {
    Emitter { index: u8, prop: EmitterProp },
    Transform { index: u8, prop: TransformProp },
    Light { index: u8, prop: LightProp },
}

/// One wire: a variable, shaped by a curve, written to a property.
///
/// Several drives may share a target; they apply in declaration order onto the
/// emitter's authored value, and `DriveOp::Replace` discards everything
/// contributed before it. Order is therefore observable, which is why the
/// editor's Drives list is reorderable.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub struct Drive {
    pub variable: VariableId,
    pub target: DriveTarget,
    /// Reuses the existing curve type, so the existing `curve_edit` widget
    /// authors it with no new UI machinery.
    pub curve: CurveTexture,
    /// The curve's 0..1 output remapped to these bounds.
    pub output: Range,
    pub op: DriveOp,
    /// Muted drives resolve to the identity for their op. The editor's A/B
    /// toggle; persisted so a half-built effect survives a save.
    #[serde(default)]
    pub muted: bool,
}
```

- [ ] **Step 6: Write `light.rs`**

```rust
use serde::{Deserialize, Serialize};
use bevy::prelude::*;
use super::{CurveTexture, EmitterTime, InitialTransform};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize, Reflect)]
pub enum FxLightKind { #[default] Point, Spot }

/// A light the effect owns, spawned as a child entity of the effect.
///
/// Reuses [`EmitterTime`] rather than inventing a timing vocabulary, so a
/// muzzle flash's delay / lifetime / one-shot / loop is authored exactly the
/// way an emitter's is.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
pub struct LightData {
    pub name: String,
    pub enabled: bool,
    pub kind: FxLightKind,
    pub transform: InitialTransform,
    pub color: Color,
    /// HDR. Blowing out into bloom is the intended use.
    pub intensity: f32,
    pub range: f32,
    pub time: EmitterTime,
    /// CPU-sampled against this light's own phase. NOT baked to a texture —
    /// it produces one scalar per frame, not a per-particle ramp.
    pub intensity_over_life: Option<CurveTexture>,
    pub shadows: bool,
}

impl Default for LightData {
    fn default() -> Self {
        Self {
            name: "Light".into(),
            enabled: true,
            kind: FxLightKind::Point,
            transform: InitialTransform::default(),
            color: Color::WHITE,
            intensity: 100_000.0,
            range: 8.0,
            time: EmitterTime::default(),
            intensity_over_life: None,
            shadows: false,
        }
    }
}
```

- [ ] **Step 7: Wire the modules and the asset fields in `asset/mod.rs`**

Add near the other `mod` declarations:

```rust
/// Effect-declared variables the host drives per instance.
pub mod variables;
/// Variable-to-property wiring.
pub mod drive;
/// Effect-owned scene lights.
pub mod light;

pub use drive::{
    DRIVE_SLOT_COUNT, Drive, DriveOp, DriveTarget, EmitterProp, LightProp, Stage, TransformProp,
};
pub use light::{FxLightKind, LightData};
pub use variables::{VariableDecl, VariableId};
```

Add to `ParticlesAsset` (after `colliders`):

```rust
    /// Knobs this effect exposes to the host game. See [`VariableDecl`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<VariableDecl>,
    /// Wiring from variables to properties. See [`Drive`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drives: Vec<Drive>,
    /// Lights this effect owns. See [`LightData`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<LightData>,
```

`ParticlesAsset::new` keeps its current signature and fills the three with `Vec::new()`; adding them as parameters would break every existing caller including the crate's own doctests for no benefit.

- [ ] **Step 8: Run the tests**

Run: `cargo test -p bevy_sprinkles drive:: 2>&1 | tail -20`
Expected: PASS, all three.

Then `cargo test -p bevy_sprinkles 2>&1 | tail -5` — expect the Step 1 count plus 3, with nothing newly failing.

- [ ] **Step 9: Mutation-verify the density guard**

Change `EmitterProp::Alpha`'s slot from `1` to `0` (a duplicate). Run
`cargo test -p bevy_sprinkles drive::` — `every_render_prop_has_a_distinct_slot_and_they_are_dense` must FAIL. Restore byte for byte and confirm it passes again. A slot collision silently makes two properties fight over one uniform, so this test earns its keep only if it actually catches that.

- [ ] **Step 10: Commit**

```bash
git add crates/bevy_sprinkles/src/asset/variables.rs \
        crates/bevy_sprinkles/src/asset/drive.rs \
        crates/bevy_sprinkles/src/asset/light.rs \
        crates/bevy_sprinkles/src/asset/mod.rs
git commit -m "feat(asset): variables, drives and lights as pure types

No ECS, no rendering. EmitterProp::stage() is the exhaustive match that
decides when a driven value is read, and slot() is the dense index into
the render-stage uniform array."
```

---

## Task 2: Validation and the format version bump

**Files:**
- Modify: `crates/bevy_sprinkles/src/asset/drive.rs` (add `validate_drives`)
- Modify: `crates/bevy_sprinkles/src/asset/versions/mod.rs` (bump `0.3` → `0.4`)
- Modify: `crates/bevy_sprinkles/src/asset/mod.rs` (call validation from the loader)

**Interfaces:**
- Consumes: Task 1's `Drive`, `DriveTarget`, `VariableDecl`, `ParticlesAsset`.
- Produces: `validate_drives(&ParticlesAsset) -> Result<(), String>`; `MigrationError::Invalid(String)`.

This task owns **Review Focus 1**.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/bevy_sprinkles/src/asset/drive.rs`:

```rust
    use crate::asset::{ParticlesAsset, ParticlesDimension, ParticlesAuthors, EmitterData, VariableDecl};

    fn asset_with(variables: Vec<VariableDecl>, drives: Vec<Drive>) -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(), ParticlesDimension::D3, Default::default(),
            vec![EmitterData::default()], vec![], false, ParticlesAuthors::default(),
        );
        a.variables = variables;
        a.drives = drives;
        a
    }

    fn drive_on(variable: u16, target: DriveTarget) -> Drive {
        Drive {
            variable: VariableId(variable),
            target,
            curve: CurveTexture::default(),
            output: Range { min: 0.0, max: 1.0 },
            op: DriveOp::Multiply,
            muted: false,
        }
    }

    fn one_var() -> Vec<VariableDecl> {
        vec![VariableDecl { name: "temperature".into(), ..Default::default() }]
    }

    #[test]
    fn a_drive_naming_an_undeclared_variable_is_rejected() {
        let a = asset_with(vec![], vec![drive_on(0, DriveTarget::Emitter {
            index: 0, prop: EmitterProp::Tint,
        })]);
        let err = validate_drives(&a).unwrap_err();
        assert!(err.contains("variable"), "message must name the problem: {err}");
    }

    #[test]
    fn a_drive_naming_an_out_of_range_emitter_is_rejected() {
        let a = asset_with(one_var(), vec![drive_on(0, DriveTarget::Emitter {
            index: 7, prop: EmitterProp::Tint,
        })]);
        let err = validate_drives(&a).unwrap_err();
        assert!(err.contains("emitter"), "message must name the problem: {err}");
    }

    #[test]
    fn a_drive_naming_an_out_of_range_light_is_rejected() {
        let a = asset_with(one_var(), vec![drive_on(0, DriveTarget::Light {
            index: 0, prop: LightProp::Intensity,
        })]);
        let err = validate_drives(&a).unwrap_err();
        assert!(err.contains("light"), "message must name the problem: {err}");
    }

    #[test]
    fn two_variables_may_not_share_a_name() {
        let a = asset_with(
            vec![
                VariableDecl { name: "heat".into(), ..Default::default() },
                VariableDecl { name: "heat".into(), ..Default::default() },
            ],
            vec![],
        );
        let err = validate_drives(&a).unwrap_err();
        assert!(err.contains("heat"), "message must name the duplicate: {err}");
    }

    #[test]
    fn a_valid_asset_passes() {
        let a = asset_with(one_var(), vec![drive_on(0, DriveTarget::Emitter {
            index: 0, prop: EmitterProp::Tint,
        })]);
        assert!(validate_drives(&a).is_ok());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles drive:: 2>&1 | tail -20`
Expected: FAIL — `validate_drives` does not exist.

- [ ] **Step 3: Implement `validate_drives` in `drive.rs`**

```rust
use std::collections::HashSet;
use super::ParticlesAsset;

/// Rejects the shapes a hand-authored `.ron` can break invisibly.
///
/// Every one of these would otherwise produce an effect that loads clean and
/// then renders stock forever, which is the single worst failure mode for an
/// authoring tool: the author sees no error and no effect, and has nothing to
/// search for. Returns the FIRST violation, naming the offender — the tests
/// match on that text, so the wording is part of the contract.
pub fn validate_drives(a: &ParticlesAsset) -> Result<(), String> {
    let mut names: HashSet<&str> = HashSet::new();
    for v in &a.variables {
        if v.name.trim().is_empty() {
            return Err("a variable has an empty name".to_string());
        }
        if !names.insert(v.name.as_str()) {
            return Err(format!("duplicate variable name {:?}", v.name));
        }
    }

    for (i, d) in a.drives.iter().enumerate() {
        if d.variable.0 as usize >= a.variables.len() {
            return Err(format!(
                "drive {i} names undeclared variable {:?} ({} declared)",
                d.variable, a.variables.len()
            ));
        }
        let (kind, index, len) = match &d.target {
            DriveTarget::Emitter { index, .. } => ("emitter", *index, a.emitters.len()),
            DriveTarget::Transform { index, .. } => ("emitter", *index, a.emitters.len()),
            DriveTarget::Light { index, .. } => ("light", *index, a.lights.len()),
        };
        if index as usize >= len {
            return Err(format!(
                "drive {i} names {kind} index {index}, but only {len} exist"
            ));
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run to confirm the tests pass**

Run: `cargo test -p bevy_sprinkles drive:: 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Bump the format version and call validation from the loader**

In `crates/bevy_sprinkles/src/asset/versions/mod.rs`:

```rust
const CURRENT_FORMAT_VERSION: &str = "0.4";
```

Add an arm for the previous version, above the `"0.2"` arm. `0.3` needs no field translation — the three new fields are `#[serde(default)]`, so a plain re-parse fills them empty. The arm exists to set `was_migrated`, which is what makes the loader warn and the editor mark the project dirty so a save rewrites the version string:

```rust
        "0.3" => {
            let asset: ParticlesAsset = ron::de::from_bytes(bytes)?;
            Ok(MigrationResult { asset, was_migrated: true })
        }
```

Add the validation error variant:

```rust
    /// The asset parsed but broke an invariant that would make it render
    /// nothing with no error at runtime.
    #[error("{0}")]
    Invalid(String),
```

And run validation on every successful parse, in `migrate`, just before each `Ok(...)` — factor it into one helper so no arm can forget it:

```rust
fn finish(asset: ParticlesAsset, was_migrated: bool) -> Result<MigrationResult, MigrationError> {
    crate::asset::drive::validate_drives(&asset).map_err(MigrationError::Invalid)?;
    Ok(MigrationResult { asset, was_migrated })
}
```

Replace every `Ok(MigrationResult { asset, was_migrated: X })` in `migrate` with `finish(asset, X)`.

- [ ] **Step 6: Write the migration tests**

Add to `versions/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_v0_3_file_without_the_new_fields_still_loads() {
        let ron = r#"(
            sprinkles_version: "0.3",
            name: "old",
            dimension: D3,
            emitters: [],
        )"#;
        let r = migrate_str(ron).expect("a 0.3 file must keep loading");
        assert!(r.was_migrated);
        assert!(r.asset.variables.is_empty());
        assert!(r.asset.drives.is_empty());
        assert!(r.asset.lights.is_empty());
    }

    #[test]
    fn a_file_whose_drive_names_a_missing_variable_fails_to_load() {
        let ron = r#"(
            sprinkles_version: "0.4",
            name: "broken",
            dimension: D3,
            emitters: [],
            drives: [(
                variable: (0),
                target: Emitter(index: 0, prop: Tint),
                curve: (points: []),
                output: (min: 0.0, max: 1.0),
                op: Multiply,
            )],
        )"#;
        let err = migrate_str(ron).expect_err("must not load");
        assert!(matches!(err, MigrationError::Invalid(_)), "got {err:?}");
    }
}
```

If the inline RON above does not match `CurveTexture`'s actual serialized shape, build the asset in Rust, serialize it with `ron::ser`, and assert on that instead — do not weaken the assertion to make a literal parse.

- [ ] **Step 7: Run the whole crate's tests**

Run: `cargo test -p bevy_sprinkles 2>&1 | tail -10`
Expected: PASS. Existing asset tests must be unaffected — if any fail, an existing `.ron` fixture or doctest hardcodes `sprinkles_version: "0.3"` and needs the new arm, which is the migration path working as intended.

- [ ] **Step 8: Mutation-verify**

Delete the `d.variable.0 as usize >= a.variables.len()` check. Confirm
`a_drive_naming_an_undeclared_variable_is_rejected` and
`a_file_whose_drive_names_a_missing_variable_fails_to_load` both fail. Restore byte for byte.

- [ ] **Step 9: Commit**

```bash
git add crates/bevy_sprinkles/src/asset/drive.rs \
        crates/bevy_sprinkles/src/asset/versions/mod.rs \
        crates/bevy_sprinkles/src/asset/mod.rs
git commit -m "feat(asset): validate drives at load, bump format to 0.4

A drive naming a missing variable or an out-of-range emitter now fails
the load with a message naming the offender, rather than loading clean
and rendering stock forever with nothing to search for."
```

---

## Task 3: `ParticleVariables` — the host-facing component

**Files:**
- Create: `crates/bevy_sprinkles/src/drives.rs`
- Modify: `crates/bevy_sprinkles/src/lib.rs` (declare the module, export the type)

**Interfaces:**
- Consumes: Task 1's `VariableDecl`, `VariableId`.
- Produces: `ParticleVariables` with `set(&mut self, name: &str, value: f32)`, `get(&self, name: &str) -> Option<f32>`, and `resolve_values(&self, decls: &[VariableDecl]) -> Vec<f32>`.

This task owns **Review Focus 2**.

- [ ] **Step 1: Write the failing tests**

Create `crates/bevy_sprinkles/src/drives.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{VariableDecl, Range};

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
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: FAIL — module not declared, `ParticleVariables` does not exist.

- [ ] **Step 3: Implement**

Above the test module in `drives.rs`:

```rust
use std::collections::HashMap;
use bevy::prelude::*;
use crate::asset::VariableDecl;

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
    pub fn set(&mut self, name: &str, value: f32) {
        self.0.insert(name.to_string(), value);
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.0.get(name).copied()
    }

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
```

In `lib.rs`, beside the other module declarations:

```rust
/// Effect variables and the drive-resolution spine.
pub mod drives;
```

and add `ParticleVariables` to the `pub use` list and to `prelude.rs`.

- [ ] **Step 4: Run to confirm the tests pass**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: PASS, all five.

- [ ] **Step 5: Mutation-verify the finite guard**

Change `Some(v) if v.is_finite() => *v` to `Some(v) => *v`. Confirm
`a_non_finite_host_value_falls_back_to_the_default` fails. Restore byte for byte.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles/src/drives.rs crates/bevy_sprinkles/src/lib.rs \
        crates/bevy_sprinkles/src/prelude.rs
git commit -m "feat(drives): ParticleVariables, the host's per-instance knobs

Keyed by name so a host need not know an effect's ordering. Unknown
names and non-finite values fall back to declared defaults rather than
panicking or putting a NaN on the GPU."
```

---

## Task 4: `resolve_drives` — the pure heart of the spine

**Files:**
- Modify: `crates/bevy_sprinkles/src/drives.rs`

**Interfaces:**
- Consumes: Task 1's `Drive`/`DriveTarget`/`EmitterProp`/`Stage`/`DriveOp`/`DRIVE_SLOT_COUNT`, Task 3's `ParticleVariables`.
- Produces:
  - `pub struct ResolvedDrives { pub emitters: Vec<EmitterResolved>, pub lights: Vec<LightResolved> }`
  - `pub struct EmitterResolved { pub spawn: HashMap<EmitterProp, f32>, pub sim: HashMap<EmitterProp, f32>, pub render: [Option<f32>; DRIVE_SLOT_COUNT], pub transform: HashMap<TransformProp, f32> }`
  - `pub struct LightResolved { pub props: HashMap<LightProp, f32> }`
  - `pub fn resolve_drives(values: &[f32], asset: &ParticlesAsset) -> ResolvedDrives`

`resolve_drives` is a pure function taking already-resolved variable values. Keeping it pure and ECS-free is the highest-value testability decision in this plan: every stage-routing and precedence question is answered by a table test with no App, no GPU and no frames.

This task owns **Review Focus 3** (authored NaN) and **Review Focus 5** (degenerate curve/range).

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/bevy_sprinkles/src/drives.rs`:

```rust
    use crate::asset::{
        CurveTexture, CurvePoint, Drive, DriveOp, DriveTarget, EmitterProp, ParticlesAsset,
        ParticlesAuthors, ParticlesDimension, EmitterData, VariableId, DRIVE_SLOT_COUNT,
    };

    /// A curve that returns `v` everywhere, so a test asserts on the drive's
    /// arithmetic rather than on curve interpolation.
    fn flat(v: f64) -> CurveTexture {
        CurveTexture::new(vec![CurvePoint::new(0.0, v), CurvePoint::new(1.0, v)])
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
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: FAIL — `resolve_drives` and `ResolvedDrives` do not exist.

- [ ] **Step 3: Implement**

Add to `drives.rs`:

```rust
use std::collections::HashMap;
use crate::asset::{
    Drive, DriveOp, DriveTarget, EmitterProp, LightProp, ParticlesAsset, Stage, TransformProp,
    DRIVE_SLOT_COUNT,
};

/// Everything a single effect instance's drives resolved to this frame.
#[derive(Clone, Debug, Default)]
pub struct ResolvedDrives {
    pub emitters: Vec<EmitterResolved>,
    pub lights: Vec<LightResolved>,
}

/// `None` in a slot means "no drive touched this"; the consumer keeps the
/// emitter's authored value. That is deliberately distinct from `Some(1.0)`,
/// which means a drive computed exactly one — the difference matters for
/// `DriveOp::Replace` on a property whose authored value is not 1.
#[derive(Clone, Debug)]
pub struct EmitterResolved {
    pub spawn: HashMap<EmitterProp, f32>,
    pub sim: HashMap<EmitterProp, f32>,
    pub render: [Option<f32>; DRIVE_SLOT_COUNT],
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

#[derive(Clone, Debug, Default)]
pub struct LightResolved {
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
```

Derive `Hash, Eq` on `EmitterProp`, `TransformProp` and `LightProp` if not already present — they are `HashMap` keys here.

- [ ] **Step 4: Run to confirm the tests pass**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: PASS, all nine plus Task 3's five.

- [ ] **Step 5: Mutation-verify the finiteness boundary**

Change `out.is_finite().then_some(out)` at the end of `sample` to `Some(out)`. Confirm `an_authored_nan_never_reaches_a_slot` and `a_nan_output_bound_never_reaches_a_slot` both fail. Restore byte for byte.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles/src/drives.rs
git commit -m "feat(drives): resolve_drives, the pure variable-to-property fold

Pure and ECS-free so stage routing and drive precedence are answerable
by table tests with no App and no GPU. Finiteness is checked at the
boundary because the curve, the output bounds and the host value are
three independent doors a NaN can come through."
```

---

## Task 5: Render-stage routing — `drive_slots` to the material uniform

**Files:**
- Modify: `crates/bevy_sprinkles/src/material.rs` (`ParticleEmitterUniforms`)
- Modify: `crates/bevy_sprinkles/src/shaders/common.wgsl` (lockstep struct)
- Modify: `crates/bevy_sprinkles/src/spawning.rs` (`write_emitter_uniforms`)
- Modify: `crates/bevy_sprinkles/src/drives.rs` (the `evaluate_drives` system + `EffectDrives` component)
- Modify: `crates/bevy_sprinkles/src/lib.rs` (system registration)

**Interfaces:**
- Consumes: Task 4's `resolve_drives`, `ResolvedDrives`; Task 3's `ParticleVariables`.
- Produces: `#[derive(Component)] pub struct EffectDrives(pub ResolvedDrives);` written each frame onto the effect entity; `ParticleEmitterUniforms::drive_slots: [f32; DRIVE_SLOT_COUNT]`.

This task owns **Review Focus 2** (warn-once) and **Review Focus 4** (per-instance isolation).

- [ ] **Step 1: Write the failing tests**

Append to `drives.rs`'s test module:

```rust
    use bevy::prelude::*;

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
                EmitterProp::SizeMul, flat(1.0), Range { min: 0.0, max: 10.0 }, DriveOp::Replace,
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
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: FAIL — `evaluate_drives`, `EffectDrives`, `UnknownVariablesWarned` do not exist.

- [ ] **Step 3: Add the component, the marker and the system to `drives.rs`**

```rust
use crate::runtime::Particles3d;

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
```

Register it in `lib.rs`'s `Update` set, before the uniform writes:

```rust
                crate::drives::evaluate_drives,
```

- [ ] **Step 4: Add `drive_slots` to both lockstep structs**

In `src/material.rs`, replace the `tint` and `size_mul` fields at the end of `ParticleEmitterUniforms` with:

```rust
    /// Render-stage drive values, indexed by `EmitterProp::slot()`.
    ///
    /// A slot array rather than a named field per property: adding a drivable
    /// property then changes one integer here and one in `common.wgsl`, not
    /// the shape of two structs that must match field for field. `NaN` never
    /// reaches here — `drives::sample` checks finiteness at the boundary.
    ///
    /// Sentinel: a slot no drive touched carries the identity for its consumer
    /// (1.0 for multipliers, which is every current slot), written by
    /// `write_emitter_uniforms`.
    pub drive_slots: [f32; DRIVE_SLOT_COUNT],
```

and in `Default`, `drive_slots: [1.0; DRIVE_SLOT_COUNT]`.

In `src/shaders/common.wgsl`, mirror it exactly:

```wgsl
// LAYOUT-LOCKSTEP: fields (order+types) must match ParticleEmitterUniforms in
// material.rs -- same GPU buffer.
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
```

In `src/shaders/particle_material.wgsl`, replace the two existing reads:
- `emitter_uniforms.size_mul` → `emitter_uniforms.drive_slots[DRIVE_SLOT_SIZE_MUL]`
- any `emitter_uniforms.tint` multiply → `emitter_uniforms.drive_slots[DRIVE_SLOT_TINT]` applied to all three colour channels, with `DRIVE_SLOT_ALPHA` on the alpha channel.

`tint` was a `vec4` and a slot is an `f32`; the slot carries a **scalar multiplier** on the existing particle colour. Full per-channel colour drive is intentionally not in this task — the gradient remap in Task 15 is the colour authoring surface, and two competing colour paths would be one too many.

- [ ] **Step 5: Wire `write_emitter_uniforms` to read `EffectDrives`**

In `src/spawning.rs`, replace the `overrides`/`per_emitter` queries with one `Query<&EffectDrives>` and the `emitter_multipliers` call with:

```rust
        let drive_slots = drives
            .get(emitter.parent_system)
            .ok()
            .and_then(|d| d.0.emitters.get(runtime.emitter_index))
            .map(|e| {
                let mut slots = [1.0f32; DRIVE_SLOT_COUNT];
                for (i, v) in e.render.iter().enumerate() {
                    if let Some(v) = v { slots[i] = *v; }
                }
                slots
            })
            .unwrap_or([1.0; DRIVE_SLOT_COUNT]);
```

and put `drive_slots` into the struct literal in place of `tint` and `size_mul`.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p bevy_sprinkles 2>&1 | tail -10`
Expected: PASS. The `override.rs` tests still pass — that module is deleted in Task 8, not here, so this task's diff stays reviewable on its own.

- [ ] **Step 7: Verify the shader actually compiles**

Run the editor and load any bundled example:

```bash
cargo run -p bevy_sprinkles_editor 2>&1 | grep -i "error\|shader\|wgsl" | head -20
```
Expected: no WGSL validation errors. A lockstep mismatch shows up here as a validation failure or as visibly wrong particle sizes — **not** as a compile error, which is exactly why this step is manual and mandatory.

- [ ] **Step 8: Mutation-verify per-instance isolation**

Change `evaluate_drives` to insert the same `EffectDrives` on every entity (e.g. resolve once outside the loop using the first entity's variables). Confirm `two_entities_sharing_an_asset_resolve_independently` fails. Restore byte for byte.

- [ ] **Step 9: Commit**

```bash
git add crates/bevy_sprinkles/src/drives.rs crates/bevy_sprinkles/src/material.rs \
        crates/bevy_sprinkles/src/spawning.rs crates/bevy_sprinkles/src/lib.rs \
        crates/bevy_sprinkles/src/shaders/common.wgsl \
        crates/bevy_sprinkles/src/shaders/particle_material.wgsl
git commit -m "feat(drives): route render-stage drives into the material uniform

drive_slots replaces the hardcoded tint/size_mul pair, so adding a
drivable render property is one integer in two files rather than a new
field in two structs that must match field for field."
```

---

## Task 6: Spawn and Sim routing, and the `spawn_probability` gate

**Files:**
- Modify: `crates/bevy_sprinkles/src/extract.rs` (`EmitterUniforms`, `apply_spawn_override` → `apply_sim_drives`)
- Modify: `crates/bevy_sprinkles/src/shaders/particle_simulate.wgsl` (`EmitterParams`, the spawn gate)

**Interfaces:**
- Consumes: Task 4's `EmitterResolved`.
- Produces: `pub(crate) fn apply_sim_drives(u: &mut EmitterUniforms, r: &EmitterResolved)`; `EmitterUniforms::spawn_probability: f32`.

**Read the Global Constraints before starting.** `amount` must never be mutated here.

- [ ] **Step 1: Write the failing tests**

Append to `extract.rs`'s existing `tests` module (it already has `apply_spawn_override` tests — replace them, since their subject is being renamed):

```rust
    use crate::drives::EmitterResolved;
    use crate::asset::EmitterProp;

    fn resolved(pairs: &[(EmitterProp, f32)]) -> EmitterResolved {
        let mut r = EmitterResolved::default();
        for (p, v) in pairs {
            match p.stage() {
                crate::asset::Stage::Spawn => { r.spawn.insert(*p, *v); }
                crate::asset::Stage::Sim => { r.sim.insert(*p, *v); }
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
        let mut u = EmitterUniforms { amount: 64, ..Default::default() };
        apply_sim_drives(&mut u, &resolved(&[
            (EmitterProp::SpawnProbability, 0.1),
            (EmitterProp::Lifetime, 0.5),
        ]));
        assert_eq!(u.amount, 64, "amount must never be mutated by apply_sim_drives");
    }

    #[test]
    fn spawn_probability_is_carried_and_clamped_to_unit() {
        let mut u = EmitterUniforms { spawn_probability: 1.0, ..Default::default() };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnProbability, 0.25)]));
        assert_eq!(u.spawn_probability, 0.25);

        let mut u = EmitterUniforms { spawn_probability: 1.0, ..Default::default() };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnProbability, 4.0)]));
        assert_eq!(u.spawn_probability, 1.0, "a probability above one is meaningless");

        let mut u = EmitterUniforms { spawn_probability: 1.0, ..Default::default() };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::SpawnProbability, -3.0)]));
        assert_eq!(u.spawn_probability, 0.0);
    }

    #[test]
    fn a_spawn_drive_multiplies_the_authored_value() {
        let mut u = EmitterUniforms {
            lifetime: 2.0, initial_velocity_min: 1.0, initial_velocity_max: 3.0,
            ..Default::default()
        };
        apply_sim_drives(&mut u, &resolved(&[
            (EmitterProp::Lifetime, 0.5),
            (EmitterProp::InitialSpeed, 2.0),
        ]));
        assert_eq!(u.lifetime, 1.0);
        assert_eq!(u.initial_velocity_min, 2.0);
        assert_eq!(u.initial_velocity_max, 6.0);
    }

    #[test]
    fn a_sim_drive_scales_gravity() {
        let mut u = EmitterUniforms { gravity: [0.0, -10.0, 0.0], ..Default::default() };
        apply_sim_drives(&mut u, &resolved(&[(EmitterProp::Gravity, 0.5)]));
        assert_eq!(u.gravity, [0.0, -5.0, 0.0]);
    }

    #[test]
    fn an_empty_resolution_changes_nothing() {
        let before = EmitterUniforms { lifetime: 2.0, spawn_probability: 1.0, ..Default::default() };
        let mut u = before;
        apply_sim_drives(&mut u, &EmitterResolved::default());
        assert_eq!(u.lifetime, before.lifetime);
        assert_eq!(u.spawn_probability, before.spawn_probability);
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles extract:: 2>&1 | tail -20`
Expected: FAIL — `apply_sim_drives` and `spawn_probability` do not exist.

- [ ] **Step 3: Add `spawn_probability` to both lockstep structs**

In `extract.rs`'s `EmitterUniforms`, add the field in a slot that preserves 16-byte alignment. The struct is laid out in four-`f32` groups; append it to the group that currently ends with a `_pad`, replacing that pad, or add a new group with three pads. Verify with `ShaderType`'s generated size — do not guess:

```rust
    /// Fraction of eligible slots that actually spawn, 0..1.
    ///
    /// Exists because `amount` cannot be scaled at runtime: it is also the
    /// per-slot simulation gate, so lowering it strands live particles in
    /// truncated slots where they freeze and never despawn. This gates
    /// spawning without resizing the pool.
    pub spawn_probability: f32,
```

Mirror it into `EmitterParams` in `shaders/particle_simulate.wgsl` **at the same offset**.

- [ ] **Step 4: Implement `apply_sim_drives`, replacing `apply_spawn_override`**

```rust
/// Applies Spawn- and Sim-stage drives to this emitter's simulation uniform.
///
/// Both stages land here because they share one buffer; they differ in when
/// the compute shader reads them, not in where they live. Spawn values are
/// read once at particle birth; Sim values (gravity, drag, turbulence) are read
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
    if let Some(v) = r.sim.get(&P::Gravity) {
        u.gravity = [u.gravity[0] * v, u.gravity[1] * v, u.gravity[2] * v];
    }
    if let Some(v) = r.sim.get(&P::Drag) {
        u.damping *= v;
    }
    if let Some(v) = r.sim.get(&P::TurbulenceStrength) {
        u.turbulence_noise_strength *= v;
    }
}
```

If a field named above does not exist on `EmitterUniforms` under that exact name (`damping`, `turbulence_noise_strength`, `scale_min`/`scale_max`), find the real one by reading the struct and use it — do not invent a field, and do not silently drop the property.

Update both call sites (`extract.rs:736` and the test at `:917`) to pass an `EmitterResolved` sourced from the effect entity's `EffectDrives`. The extract system already queries the parent system entity; add `EffectDrives` to that query. Where the component is absent, pass `&EmitterResolved::default()`.

- [ ] **Step 5: Gate spawning in the compute shader**

In `particle_simulate.wgsl`, find the branch that decides a slot spawns this step and add, as the last condition before the spawn is committed:

```wgsl
    // Density control without resizing the pool: `amount` is also the per-slot
    // simulation gate, so scaling it would strand live particles in truncated
    // slots. Gating here leaves the pool intact and only skips new births.
    // Hashed on the slot index and cycle so a given slot's decision is stable
    // within a cycle rather than flickering every step.
    if (params.spawn_probability < 1.0) {
        let gate = hash_to_float(hash(idx ^ (params.cycle * 2654435761u)));
        if (gate >= params.spawn_probability) {
            return;
        }
    }
```

Place it where a failed spawn simply does not happen — **not** where it would skip the per-step update of already-live particles. Getting this wrong freezes live particles, the exact failure the `amount` constraint exists to avoid.

- [ ] **Step 6: Run the tests and the shader**

Run: `cargo test -p bevy_sprinkles 2>&1 | tail -10` — expect PASS.
Then `cargo run -p bevy_sprinkles_editor` and load an example: particles must still emit normally at the default `spawn_probability` of 1.0.

- [ ] **Step 7: Mutation-verify the `amount` guard**

Add `u.amount = (u.amount as f32 * 0.5) as u32;` to `apply_sim_drives`. Confirm `amount_is_never_mutated_by_drives` fails. Remove the line byte for byte.

- [ ] **Step 8: Commit**

```bash
git add crates/bevy_sprinkles/src/extract.rs \
        crates/bevy_sprinkles/src/shaders/particle_simulate.wgsl
git commit -m "feat(drives): spawn and sim routing, plus a spawn_probability gate

Drivable emission rate cannot scale amount -- amount is also the
per-slot simulation gate, so shrinking it strands live particles in
truncated slots. A separate probability uniform gates births instead
and leaves the pool and everything in flight alone."
```

---

## Task 7: ECS-stage routing — driving emitter transform scale

**Files:**
- Modify: `crates/bevy_sprinkles/src/drives.rs` (add `apply_transform_drives`)
- Modify: `crates/bevy_sprinkles/src/lib.rs` (register it)

**Interfaces:**
- Consumes: Task 4's `EmitterResolved::transform`, Task 5's `EffectDrives`.
- Produces: `pub fn apply_transform_drives(...)` system.

This is the task that makes "variables drive emitter size/scale" work, and it needs no GPU change: the material shader already builds a per-axis `emitter_scale` vec3 from the emitter entity's `GlobalTransform` (`particle_material.wgsl:215,301,318`).

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_transform_drive_scales_the_emitter_entity_per_axis() {
        let mut app = test_app();
        app.add_systems(Update, apply_transform_drives.after(evaluate_drives));

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = asset_with_drives(vec![Drive {
                variable: VariableId(0),
                target: DriveTarget::Transform { index: 0, prop: TransformProp::ScaleY },
                curve: flat(1.0),
                output: Range { min: 0.0, max: 5.0 },
                op: DriveOp::Replace,
                muted: false,
            }]);
            a.emitters = vec![EmitterData::default()];
            assets.add(a)
        };

        let mut vars = ParticleVariables::default();
        vars.set("v", 1.0);
        let system = app.world_mut().spawn((Particles3d(handle), vars)).id();
        // Stand in for the emitter child that setup_particle_systems creates.
        let emitter = app.world_mut().spawn((
            Transform::from_scale(Vec3::ONE),
            EmitterEntity { parent_system: system },
            EmitterRuntime::new(0, Some(1)),
        )).id();

        app.update();

        let t = app.world().entity(emitter).get::<Transform>().unwrap();
        assert_eq!(t.scale.y, 5.0, "ScaleY must be driven");
        assert_eq!(t.scale.x, 1.0, "X must be untouched -- the point is per-axis");
        assert_eq!(t.scale.z, 1.0);
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: FAIL — `apply_transform_drives` does not exist.

- [ ] **Step 3: Implement**

```rust
/// Writes ECS-stage drives onto each emitter entity's `Transform`.
///
/// This is how per-axis scale is driven. Per-PARTICLE scale is a single scalar
/// (`ParticleData::position.w`) and stays that way by ruling; the per-axis need
/// is served here, at emitter level, where `particle_material.wgsl` already
/// consumes a per-axis `emitter_scale` vec3 built from this Transform.
///
/// Writes absolutely rather than accumulating: the resolved value already
/// folded every drive on this target, so re-applying it each frame is
/// idempotent and cannot drift the way a `*=` on a live Transform would.
pub fn apply_transform_drives(
    drives: Query<&EffectDrives>,
    mut emitters: Query<(&crate::runtime::EmitterEntity, &crate::runtime::EmitterRuntime, &mut Transform)>,
) {
    for (emitter, runtime, mut transform) in emitters.iter_mut() {
        let Ok(d) = drives.get(emitter.parent_system) else { continue };
        let Some(r) = d.0.emitters.get(runtime.emitter_index) else { continue };
        if r.transform.is_empty() {
            continue;
        }
        use crate::asset::TransformProp as T;
        for (prop, v) in &r.transform {
            match prop {
                T::ScaleUniform => transform.scale = Vec3::splat(*v),
                T::ScaleX => transform.scale.x = *v,
                T::ScaleY => transform.scale.y = *v,
                T::ScaleZ => transform.scale.z = *v,
                T::PosX => transform.translation.x = *v,
                T::PosY => transform.translation.y = *v,
                T::PosZ => transform.translation.z = *v,
                T::RotX | T::RotY | T::RotZ => {
                    let (x, y, z) = transform.rotation.to_euler(EulerRot::XYZ);
                    let (x, y, z) = match prop {
                        T::RotX => (*v, y, z),
                        T::RotY => (x, *v, z),
                        _ => (x, y, *v),
                    };
                    transform.rotation = Quat::from_euler(EulerRot::XYZ, x, y, z);
                }
            }
        }
    }
}
```

`ScaleUniform` and an axis drive on the same emitter both write `scale`; declaration order decides, consistent with every other multi-drive target. The editor warns about that pairing in Task 19, it is not forbidden here.

Register in `lib.rs` after `evaluate_drives`:

```rust
                crate::drives::apply_transform_drives.after(crate::drives::evaluate_drives),
```

- [ ] **Step 4: Run to confirm it passes**

Run: `cargo test -p bevy_sprinkles drives:: 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Mutation-verify**

Change `transform.scale.y = *v` to `transform.scale = Vec3::splat(*v)`. Confirm the test fails on the X assertion — that is the per-axis guarantee doing its job. Restore byte for byte.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles/src/drives.rs crates/bevy_sprinkles/src/lib.rs
git commit -m "feat(drives): drive emitter transform scale per axis

No GPU change needed: the material shader already builds a per-axis
emitter_scale from this Transform, which is why per-particle scale
could stay a scalar."
```

---

## Task 8: Delete `ParticleOverride`

**Files:**
- Delete: `crates/bevy_sprinkles/src/override.rs`
- Modify: `crates/bevy_sprinkles/src/lib.rs`, `src/prelude.rs`, `src/textures/mod.rs` (`bake_override_textures`), `src/spawning.rs` (`apply_emissive_override`), `src/extract.rs`

**Interfaces:**
- Consumes: Tasks 5-7 (every consumer of `ParticleOverride` now has a drive-based replacement).
- Produces: nothing new. This task only removes.

A breaking change to the crate's public API, taken deliberately: keeping both would leave two ways to say the same thing, one of them invisible from the editor.

- [ ] **Step 1: Find every reference**

```bash
cd /home/luke/src/bevy-sprinkles-fork
grep -rn "ParticleOverride\|ParticleEmitterOverrides\|OverrideBakedTextures\|effective_override\|emitter_multipliers\|bake_override_textures\|apply_emissive_override\|r#override" crates/ --include=*.rs
```
Write the list down. Each one is either replaced by a drive or deleted; none is left dangling.

- [ ] **Step 2: Map the seven fields to their replacements**

| `ParticleOverride` field | Replacement |
|---|---|
| `tint` | `EmitterProp::Tint` (scalar multiplier) + Task 15's gradient remap for colour |
| `size_mul` | `EmitterProp::SizeMul` |
| `emissive` | `EmitterProp::EmissiveIntensity` |
| `lifetime_mul` | `EmitterProp::Lifetime` |
| `speed_mul` | `EmitterProp::InitialSpeed` |
| `color_keys` | Gradient remap, Task 15 |
| `size_keys` | Author the emitter's `scale_over_lifetime`; a per-instance *curve* swap has no drive equivalent and is dropped |

`color_keys` and `size_keys` swapped a whole per-instance curve or gradient, which `bake_override_textures` existed to re-bake. Drives replace a per-instance *scalar*, not a per-instance *curve*. **That capability is genuinely lost**, and dropping it is what lets `bake_override_textures` and `OverrideBakedTextures` go. If a real effect needs it, it returns as its own feature with its own spec — not as a quiet survival of the type this task deletes.

- [ ] **Step 3: Delete and unwire**

```bash
git rm crates/bevy_sprinkles/src/override.rs
```

Remove from `lib.rs`: the `pub mod r#override;` declaration, the `bake_override_textures` system registration, the `apply_emissive_override` registration, and both names from the `pub use` lists. Remove `OverrideBakedTextures` handling from `spawning.rs::setup_particle_systems`. Remove `bake_override_textures` from `textures/`.

- [ ] **Step 4: Compile and fix the fallout**

Run: `cargo build --workspace 2>&1 | grep -E "^error" | head -20`
Expected: errors only at the sites from Step 1. Fix each by removal, never by re-introducing a shim.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --workspace 2>&1 | tail -10`
Expected: PASS. The count drops by the five `override.rs` tests — that is the deletion, not a regression. Note the new baseline.

- [ ] **Step 6: Commit**

```bash
git add -A crates/
git commit -m "refactor!: remove ParticleOverride in favour of drives

Its seven hardcoded fields become ordinary authored drives, discoverable
from the editor rather than reachable only from host code. Dropped with
it: per-instance curve and gradient SWAPS (color_keys/size_keys), which
drives do not cover -- drives replace a per-instance scalar, not a
per-instance curve. That returns as its own feature if an effect needs
it, rather than surviving quietly."
```

> **Note:** this is the one task in the plan where `git add -A` is appropriate, because the deletion touches many files and `git rm` has already staged the removal. Every other task stages by explicit path.

---

**Phase 1 gate.** Before starting Phase 2, confirm:
- `cargo test --workspace` passes, with a recorded count.
- The editor runs and an example effect renders unchanged.
- An effect with one variable, one drive on `SizeMul`, and a host-set value visibly changes size when the value changes. Build this by hand once; it is the first end-to-end proof the spine works, and everything after it assumes it does.

---

# Phase 2 — Effect-owned lights

## Task 9: Light entities

**Files:**
- Create: `crates/bevy_sprinkles/src/lights.rs`
- Modify: `crates/bevy_sprinkles/src/lib.rs` (module + systems)

**Interfaces:**
- Consumes: Task 1's `LightData`/`FxLightKind`, Task 5's `EffectDrives`.
- Produces: `#[derive(Component)] pub struct LightEntity { pub parent_system: Entity, pub light_index: usize }`; systems `setup_effect_lights`, `sync_effect_lights`.

- [ ] **Step 1: Write the failing tests**

Create `crates/bevy_sprinkles/src/lights.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use crate::asset::{LightData, ParticlesAsset, ParticlesAuthors, ParticlesDimension};
    use crate::runtime::Particles3d;

    fn app_with(lights: Vec<LightData>) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.add_systems(Update, (setup_effect_lights, sync_effect_lights).chain());
        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            let mut a = ParticlesAsset::new(
                "t".into(), ParticlesDimension::D3, Default::default(),
                vec![], vec![], false, ParticlesAuthors::default(),
            );
            a.lights = lights;
            assets.add(a)
        };
        let e = app.world_mut().spawn((Particles3d(handle), Transform::default())).id();
        app.update();
        (app, e)
    }

    #[test]
    fn one_child_light_entity_is_spawned_per_declared_light() {
        let (app, effect) = app_with(vec![LightData::default(), LightData::default()]);
        let n = app.world().iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 2);
    }

    #[test]
    fn a_disabled_light_spawns_nothing() {
        let (app, effect) = app_with(vec![LightData { enabled: false, ..Default::default() }]);
        let n = app.world().iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 0);
    }

    #[test]
    fn an_effect_with_no_lights_spawns_nothing_and_does_not_panic() {
        let (app, effect) = app_with(vec![]);
        let n = app.world().iter_entities()
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
        let n = app.world().iter_entities()
            .filter(|e| e.get::<LightEntity>().map(|l| l.parent_system) == Some(effect))
            .count();
        assert_eq!(n, 1);
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles lights:: 2>&1 | tail -20`
Expected: FAIL — module not declared.

- [ ] **Step 3: Implement**

```rust
use bevy::prelude::*;
use crate::asset::{FxLightKind, LightData, LightProp, ParticlesAsset};
use crate::drives::EffectDrives;
use crate::runtime::{EmitterRuntime, Particles3d};

/// Links an effect-owned light entity back to the effect that declared it.
#[derive(Component)]
pub struct LightEntity {
    pub parent_system: Entity,
    pub light_index: usize,
}

/// Marks an effect whose lights have been spawned, so setup is idempotent.
#[derive(Component)]
pub struct EffectLightsSpawned;

/// Spawns one child entity per enabled [`LightData`], once per effect.
pub fn setup_effect_lights(
    mut commands: Commands,
    assets: Res<Assets<ParticlesAsset>>,
    q: Query<(Entity, &Particles3d), Without<EffectLightsSpawned>>,
) {
    for (entity, particles) in q.iter() {
        let Some(asset) = assets.get(&particles.0) else { continue };
        for (i, light) in asset.lights.iter().enumerate() {
            if !light.enabled {
                continue;
            }
            let transform: Transform = light.transform.clone().into();
            let mut child = commands.spawn((
                transform,
                LightEntity { parent_system: entity, light_index: i },
                EmitterRuntime::new(i, None),
            ));
            match light.kind {
                FxLightKind::Point => {
                    child.insert(PointLight {
                        color: light.color,
                        intensity: light.intensity,
                        range: light.range,
                        shadows_enabled: light.shadows,
                        ..default()
                    });
                }
                FxLightKind::Spot => {
                    child.insert(SpotLight {
                        color: light.color,
                        intensity: light.intensity,
                        range: light.range,
                        shadows_enabled: light.shadows,
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

/// Applies each light's own-clock intensity curve and its ECS-stage drives.
///
/// Intensity is recomputed from the authored value every frame rather than
/// accumulated, so it is idempotent: a curve and a drive on the same light
/// compose to the same result no matter how many frames have passed.
pub fn sync_effect_lights(
    assets: Res<Assets<ParticlesAsset>>,
    systems: Query<(&Particles3d, Option<&EffectDrives>)>,
    mut lights: Query<(&LightEntity, &EmitterRuntime, Option<&mut PointLight>, Option<&mut SpotLight>)>,
) {
    for (link, runtime, point, spot) in lights.iter_mut() {
        let Ok((particles, drives)) = systems.get(link.parent_system) else { continue };
        let Some(asset) = assets.get(&particles.0) else { continue };
        let Some(data) = asset.lights.get(link.light_index) else { continue };

        let phase = runtime.system_phase(&data.time);
        let envelope = data
            .intensity_over_life
            .as_ref()
            .map(|c| c.sample(phase.clamp(0.0, 1.0)))
            .filter(|v| v.is_finite())
            .unwrap_or(1.0);

        let resolved = drives.and_then(|d| d.0.lights.get(link.light_index));
        let mul = resolved
            .and_then(|r| r.props.get(&LightProp::Intensity))
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(1.0);
        let range_mul = resolved
            .and_then(|r| r.props.get(&LightProp::Range))
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(1.0);

        let intensity = (data.intensity * envelope * mul).max(0.0);
        let range = (data.range * range_mul).max(0.0);

        if let Some(mut l) = point { l.intensity = intensity; l.range = range; }
        if let Some(mut l) = spot { l.intensity = intensity; l.range = range; }
    }
}
```

Declare in `lib.rs` (`pub mod lights;`), register both systems in `Update`, `sync_effect_lights` after `evaluate_drives`, and export `LightEntity` from the prelude.

If `InitialTransform` has no `Into<Transform>`, use whatever conversion `setup_particle_systems` already uses for emitters — match the existing pattern rather than adding a second one.

- [ ] **Step 4: Run to confirm the tests pass**

Run: `cargo test -p bevy_sprinkles lights:: 2>&1 | tail -20`
Expected: PASS, all four.

- [ ] **Step 5: Mutation-verify idempotence**

Remove `Without<EffectLightsSpawned>` from the query. Confirm `setup_is_idempotent_across_frames` fails. Restore byte for byte.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles/src/lights.rs crates/bevy_sprinkles/src/lib.rs \
        crates/bevy_sprinkles/src/prelude.rs
git commit -m "feat(lights): effects own scene lights, on their own clock

Reuses EmitterTime so a flash's delay/lifetime/one-shot vocabulary is
the one authors already know, and recomputes intensity from the authored
value each frame so curve and drive compose idempotently."
```

---

## Task 10: Light colour drives

**Files:**
- Modify: `crates/bevy_sprinkles/src/lights.rs`

**Interfaces:**
- Consumes: Task 9's `sync_effect_lights`; Task 4's `LightResolved`.
- Produces: no new names; extends `sync_effect_lights` to handle `LightProp::{Hue, Saturation, Value}`.

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles lights:: 2>&1 | tail -20`
Expected: FAIL — `apply_hsv` does not exist.

- [ ] **Step 3: Implement**

```rust
/// Applies hue/saturation/value drives to an authored colour.
///
/// Hue is a rotation in turns (0..1 maps to 0..360 degrees) so a linear curve
/// over a 0..1 variable sweeps the wheel once -- the "linear hue shift" case
/// from the design brief. Saturation and value MULTIPLY, so an undriven
/// channel is exactly the authored colour rather than a re-derived
/// approximation of it. A non-finite drive is dropped per channel, because a
/// NaN here silently blanks a light and reads as "the light broke".
pub(crate) fn apply_hsv(base: Color, hue: Option<f32>, sat: Option<f32>, val: Option<f32>) -> Color {
    if hue.is_none() && sat.is_none() && val.is_none() {
        return base;
    }
    let mut hsva = Hsva::from(base);
    if let Some(h) = hue.filter(|v| v.is_finite()) {
        hsva.hue = (hsva.hue + h * 360.0).rem_euclid(360.0);
    }
    if let Some(s) = sat.filter(|v| v.is_finite()) {
        hsva.saturation = (hsva.saturation * s).clamp(0.0, 1.0);
    }
    if let Some(v) = val.filter(|v| v.is_finite()) {
        hsva.value = (hsva.value * v).max(0.0);
    }
    Color::from(hsva)
}
```

Add `use bevy::color::{Hsva, Srgba};`. In `sync_effect_lights`, read the three props from `resolved` and set `l.color = apply_hsv(data.color, hue, sat, val);` on both light kinds.

- [ ] **Step 4: Run to confirm the tests pass**

Run: `cargo test -p bevy_sprinkles lights:: 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Mutation-verify**

Remove the `.filter(|v| v.is_finite())` from the hue branch. Confirm `a_non_finite_hsv_drive_is_ignored_rather_than_blanking_the_light` fails. Restore byte for byte.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles/src/lights.rs
git commit -m "feat(lights): hue/saturation/value drives

Hue rotates in turns so a linear curve over a 0..1 variable sweeps the
wheel once; saturation and value multiply so an undriven channel is the
authored colour exactly, not a round-trip approximation of it."
```

---

# Phase 3 — The material feature set

All of Phase 3 lands on `particle_material.wgsl`. There is no second `fx_material.wgsl`, because the mesh-effect object was cut from the design: a mesh effect is an emitter with one pinned mesh particle, so it goes through this same shader and inherits every feature below for free.

**Every feature is gated by a `shader_def`.** `particle_material.wgsl` is already 1274 lines; six ungated feature blocks would cost fill rate on every existing effect that uses none of them. The gate is a requirement, not an optimisation.

**One spec feature has no task, deliberately:** the material's **alpha mode** (Additive / Blend / Premultiplied) already exists as `StandardParticleMaterial::alpha_mode: SerializableAlphaMode` and is already authorable. It is listed in the spec's feature set for completeness of the look, not because it needs building. Confirm it is exposed in the inspector during Task 22; if it is not, adding the field there is part of that task.

## Task 11: `FxSettings`, the material uniform, and UV scroll

**Files:**
- Create: `crates/bevy_sprinkles/src/asset/fx.rs`
- Modify: `crates/bevy_sprinkles/src/asset/particle_material.rs` (add `fx` to `StandardParticleMaterial`)
- Modify: `crates/bevy_sprinkles/src/material.rs` (`ParticleMaterialExtension` uniform + shader defs)
- Modify: `crates/bevy_sprinkles/src/spawning.rs` (`sync_particle_material` fills it)
- Modify: `crates/bevy_sprinkles/src/shaders/particle_material.wgsl`

**Interfaces:**
- Consumes: Task 5's `drive_slots` and the `DRIVE_SLOT_*` WGSL constants.
- Produces: `FxSettings`, `FxUniform`, and the shader def `FX_SCROLL`.

- [ ] **Step 1: Write the failing test**

Create `crates/bevy_sprinkles/src/asset/fx.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fx_is_entirely_off() {
        // A default must cost nothing and change nothing, or every existing
        // effect silently changes appearance when this field appears.
        let fx = FxSettings::default();
        assert_eq!(fx.scroll, Vec2::ZERO);
        assert_eq!(fx.tiling, Vec2::ONE);
        assert!(!fx.enabled());
    }

    #[test]
    fn any_nonzero_scroll_enables_the_feature() {
        let fx = FxSettings { scroll: Vec2::new(0.0, 0.2), ..Default::default() };
        assert!(fx.enabled());
    }

    #[test]
    fn a_non_finite_authored_value_is_clamped_away_on_conversion() {
        let fx = FxSettings { scroll: Vec2::new(f32::NAN, 1.0), ..Default::default() };
        let u = FxUniform::from(&fx);
        assert!(u.scroll_tiling.x.is_finite(), "a NaN must never reach the GPU");
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles fx:: 2>&1 | tail -20`
Expected: FAIL — module not declared.

- [ ] **Step 3: Implement `fx.rs`**

```rust
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};
use super::TextureRef;

/// The stylized-FX half of a particle material: everything that makes a
/// scrolled, eroded, rim-lit sheet read as volumetric rather than as a sprite.
///
/// Every field defaults to inert. That is load-bearing: this struct is added to
/// an existing serialized type, so any default that changed a pixel would
/// silently restyle every effect already authored.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Reflect)]
#[serde(default)]
pub struct FxSettings {
    /// UV scroll rate, in UV units per second.
    pub scroll: Vec2,
    /// UV tiling multiplier.
    pub tiling: Vec2,
    /// A texture whose RG channels offset the base UV -- the difference
    /// between a texture that CHURNS and one that merely slides. Scroll alone
    /// is the clearest tell of cheap VFX.
    pub flow_texture: Option<TextureRef>,
    pub flow_strength: f32,
    pub flow_scroll: Vec2,
    /// Dissolve noise. Fragments below `erosion_threshold` are discarded, with
    /// `erosion_edge` of emissive rim before the cut.
    pub erosion_texture: Option<TextureRef>,
    pub erosion_threshold: f32,
    pub erosion_edge: f32,
    pub erosion_edge_color: [f32; 4],
    /// Rim brightening. 0 disables.
    pub fresnel_power: f32,
    pub fresnel_boost: f32,
    /// Depth-fade distance in world units. 0 disables. Removes the hard
    /// intersection line where a quad clips the floor.
    pub soft_fade: f32,
    /// Sample the base texture's red channel as a mask and colour it through
    /// this gradient, instead of using the texture's own colour.
    pub gradient_remap: Option<super::Gradient>,
}

impl Default for FxSettings {
    fn default() -> Self {
        Self {
            scroll: Vec2::ZERO,
            tiling: Vec2::ONE,
            flow_texture: None,
            flow_strength: 0.0,
            flow_scroll: Vec2::ZERO,
            erosion_texture: None,
            erosion_threshold: 0.0,
            erosion_edge: 0.0,
            erosion_edge_color: [1.0, 0.5, 0.1, 1.0],
            fresnel_power: 0.0,
            fresnel_boost: 0.0,
            soft_fade: 0.0,
            gradient_remap: None,
        }
    }
}

impl FxSettings {
    pub fn scroll_enabled(&self) -> bool {
        self.scroll != Vec2::ZERO || self.tiling != Vec2::ONE
    }
    pub fn flow_enabled(&self) -> bool {
        self.flow_texture.is_some() && self.flow_strength != 0.0
    }
    pub fn erosion_enabled(&self) -> bool {
        self.erosion_texture.is_some() && (self.erosion_threshold > 0.0 || self.erosion_edge > 0.0)
    }
    pub fn fresnel_enabled(&self) -> bool { self.fresnel_power > 0.0 }
    pub fn soft_enabled(&self) -> bool { self.soft_fade > 0.0 }
    pub fn gradient_enabled(&self) -> bool { self.gradient_remap.is_some() }

    /// True when any feature is on. Used only by tests and the editor summary;
    /// the shader defs are decided per feature.
    pub fn enabled(&self) -> bool {
        self.scroll_enabled() || self.flow_enabled() || self.erosion_enabled()
            || self.fresnel_enabled() || self.soft_enabled() || self.gradient_enabled()
    }
}

fn finite(v: f32, fallback: f32) -> f32 { if v.is_finite() { v } else { fallback } }
fn finite2(v: Vec2, fallback: Vec2) -> Vec2 {
    Vec2::new(finite(v.x, fallback.x), finite(v.y, fallback.y))
}

/// GPU-side FX parameters. Packed in vec4s so the WGSL struct needs no padding
/// fields that could drift out of lockstep.
#[derive(Clone, Copy, Default, ShaderType, Debug)]
pub struct FxUniform {
    /// xy = scroll rate, zw = tiling.
    pub scroll_tiling: Vec4,
    /// x = flow strength, yz = flow scroll, w = fresnel power.
    pub flow_fresnel: Vec4,
    /// x = erosion threshold, y = erosion edge, z = fresnel boost, w = soft fade.
    pub erosion_soft: Vec4,
    pub erosion_edge_color: Vec4,
}

impl From<&FxSettings> for FxUniform {
    /// Clamps every authored value to something finite. Authored `.ron` is
    /// untrusted input, and a NaN reaching the fragment shader can blank the
    /// draw -- which an author reads as "my effect vanished".
    fn from(f: &FxSettings) -> Self {
        let scroll = finite2(f.scroll, Vec2::ZERO);
        let tiling = finite2(f.tiling, Vec2::ONE);
        let flow_scroll = finite2(f.flow_scroll, Vec2::ZERO);
        Self {
            scroll_tiling: Vec4::new(scroll.x, scroll.y, tiling.x, tiling.y),
            flow_fresnel: Vec4::new(
                finite(f.flow_strength, 0.0), flow_scroll.x, flow_scroll.y,
                finite(f.fresnel_power, 0.0),
            ),
            erosion_soft: Vec4::new(
                finite(f.erosion_threshold, 0.0).clamp(0.0, 1.0),
                finite(f.erosion_edge, 0.0).clamp(0.0, 1.0),
                finite(f.fresnel_boost, 0.0),
                finite(f.soft_fade, 0.0).max(0.0),
            ),
            erosion_edge_color: Vec4::from_array(f.erosion_edge_color)
                .to_array().map(|v| finite(v, 1.0)).into(),
        }
    }
}
```

Declare `pub mod fx;` in `asset/mod.rs`, re-export `FxSettings`/`FxUniform`, and add to `StandardParticleMaterial`:

```rust
    /// Stylized-FX settings. See [`FxSettings`]; defaults to entirely inert.
    #[serde(default, skip_serializing_if = "is_default_fx")]
    pub fx: FxSettings,
```

with `fn is_default_fx(f: &FxSettings) -> bool { *f == FxSettings::default() }`.

- [ ] **Step 4: Add the uniform and shader defs to the material extension**

In `material.rs`:

```rust
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct ParticleMaterialExtension {
    #[storage(100, read_only)]
    pub sorted_particles: Handle<ShaderBuffer>,
    #[storage(101, read_only)]
    pub emitter_uniforms: Handle<ShaderBuffer>,
    #[uniform(102)]
    pub fx: FxUniform,
    #[texture(103)]
    #[sampler(104)]
    pub flow_texture: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub erosion_texture: Option<Handle<Image>>,
    #[texture(107)]
    #[sampler(108)]
    pub gradient_texture: Option<Handle<Image>>,
    /// Which feature blocks the shader compiles. Not sent to the GPU.
    #[reflect(ignore)]
    pub defs: FxDefs,
}

/// Which FX blocks this material wants compiled in.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash, Reflect)]
pub struct FxDefs {
    pub scroll: bool,
    pub flow: bool,
    pub erosion: bool,
    pub fresnel: bool,
    pub soft: bool,
    pub gradient: bool,
    pub lit: bool,
}
```

Implement `MaterialExtension::specialize` to push a def per enabled flag, keeping the existing depth/cull logic:

```rust
        let defs = &key.bind_group_data;
        let mut push = |on: bool, name: &str| {
            if on {
                descriptor.vertex.shader_defs.push(name.into());
                if let Some(f) = descriptor.fragment.as_mut() {
                    f.shader_defs.push(name.into());
                }
            }
        };
        push(defs.scroll, "FX_SCROLL");
        push(defs.flow, "FX_FLOW");
        push(defs.erosion, "FX_EROSION");
        push(defs.fresnel, "FX_FRESNEL");
        push(defs.soft, "FX_SOFT");
        push(defs.gradient, "FX_GRADIENT");
        push(defs.lit, "FX_LIT");
```

This requires `AsBindGroup`'s `bind_group_data` to be `FxDefs` — add `#[bind_group_data(FxDefs)]` to the struct and `impl From<&ParticleMaterialExtension> for FxDefs`. Pipeline specialization keys on it, so each feature combination compiles once and is cached.

- [ ] **Step 5: Implement scroll in the fragment shader**

In `particle_material.wgsl`, add the bindings and, at the top of the fragment entry point where the base UV is first computed, insert:

```wgsl
@group(3) @binding(102) var<uniform> fx: FxUniform;

var uv = in.uv;
#ifdef FX_SCROLL
    // drive_slots let a host variable modulate the authored rate at runtime;
    // an untouched slot carries 1.0, so an undriven effect scrolls exactly as
    // authored.
    let rate = fx.scroll_tiling.xy * vec2(
        emitter_uniforms.drive_slots[DRIVE_SLOT_SCROLL_U],
        emitter_uniforms.drive_slots[DRIVE_SLOT_SCROLL_V],
    );
    uv = uv * fx.scroll_tiling.zw + rate * globals.time;
#endif
```

Use `uv` in place of `in.uv` for every subsequent texture sample. Import bevy's `globals` binding if the shader does not already have it.

- [ ] **Step 6: Fill the uniform in `sync_particle_material`**

Where `spawning.rs` builds the `ParticleMaterialExtension`, set `fx: FxUniform::from(&std_mat.fx)`, load the three textures from their `TextureRef`s, and set `defs` from the `*_enabled()` predicates.

- [ ] **Step 7: Run the tests and see it move**

Run: `cargo test -p bevy_sprinkles 2>&1 | tail -10` — expect PASS.
Then author an effect with `scroll: (0.0, 0.5)` on a quad with a base texture, run the editor, and confirm the texture visibly scrolls. A shader def that is never pushed produces a still image with no error, so this look-at-it step is mandatory.

- [ ] **Step 8: Commit**

```bash
git add crates/bevy_sprinkles/src/asset/fx.rs crates/bevy_sprinkles/src/asset/mod.rs \
        crates/bevy_sprinkles/src/asset/particle_material.rs \
        crates/bevy_sprinkles/src/material.rs crates/bevy_sprinkles/src/spawning.rs \
        crates/bevy_sprinkles/src/shaders/particle_material.wgsl
git commit -m "feat(fx): FxSettings, a gated material uniform, and UV scroll

Every feature sits behind a shader def so an effect using none of them
compiles the same lean fragment it does today. Defaults are inert, since
this field lands on an existing serialized type."
```

---

## Task 12: Flow-map distortion

**Files:** modify `crates/bevy_sprinkles/src/shaders/particle_material.wgsl`.

**Interfaces:** consumes Task 11's `fx` uniform, `flow_texture`, `FX_FLOW`.

- [ ] **Step 1: Implement, immediately after the scroll block**

```wgsl
#ifdef FX_FLOW
    // Offsetting the base UV by a second, independently scrolling texture is
    // what makes fire and smoke CHURN. A single scrolling layer reads as a
    // sliding sheet no matter how good the texture is.
    let flow_uv = in.uv * fx.scroll_tiling.zw + fx.flow_fresnel.yz * globals.time;
    let flow = textureSample(flow_texture, flow_sampler, flow_uv).rg * 2.0 - 1.0;
    let flow_amount = fx.flow_fresnel.x * emitter_uniforms.drive_slots[DRIVE_SLOT_FLOW];
    uv = uv + flow * flow_amount;
#endif
```

- [ ] **Step 2: Verify visually**

Run the editor with an effect carrying a flow texture and `flow_strength: 0.1`. The base texture must distort and churn, not merely translate. Compare against `flow_strength: 0.0` — if the two look identical, `FX_FLOW` is not being pushed and Task 11's `defs` wiring is wrong.

- [ ] **Step 3: Commit**

```bash
git add crates/bevy_sprinkles/src/shaders/particle_material.wgsl
git commit -m "feat(fx): flow-map UV distortion

Scroll alone reads as a sliding sheet; a second independently scrolling
texture offsetting the base UV is what makes fire churn."
```

---

## Task 13: Erosion / dissolve

**Files:** modify `crates/bevy_sprinkles/src/shaders/particle_material.wgsl`.

**Interfaces:** consumes Task 11's `erosion_texture`, `FX_EROSION`, `DRIVE_SLOT_EROSION`.

- [ ] **Step 1: Implement, after the base colour has been sampled**

```wgsl
#ifdef FX_EROSION
    // The signature stylized burn-away: sample noise, discard below a moving
    // threshold, and emit a bright rim in the band just above it. Driving the
    // threshold from a variable is how an effect dissolves on command.
    let noise = textureSample(erosion_texture, erosion_sampler, uv).r;
    let threshold = clamp(
        fx.erosion_soft.x * emitter_uniforms.drive_slots[DRIVE_SLOT_EROSION],
        0.0, 1.0,
    );
    if (noise < threshold) {
        discard;
    }
    let edge = fx.erosion_soft.y;
    if (edge > 0.0) {
        // 1 at the cut, falling to 0 `edge` above it.
        let rim = 1.0 - clamp((noise - threshold) / edge, 0.0, 1.0);
        color = mix(color, fx.erosion_edge_color, rim * fx.erosion_edge_color.a);
    }
#endif
```

Place this **before** any alpha-based early-out the shader already performs, so a discarded fragment costs nothing further.

- [ ] **Step 2: Verify visually**

Drive `ErosionThreshold` from a variable across 0→1 with the editor's slider: the particle must burn away from the noise pattern with a visible hot rim, and reach fully invisible at 1.0. If it never fully vanishes, the threshold clamp or the noise texture's channel is wrong.

- [ ] **Step 3: Commit**

```bash
git add crates/bevy_sprinkles/src/shaders/particle_material.wgsl
git commit -m "feat(fx): erosion dissolve with an emissive rim"
```

---

## Task 14: Fresnel rim and soft particles

**Files:** modify `crates/bevy_sprinkles/src/shaders/particle_material.wgsl`, and `crates/bevy_sprinkles/src/material.rs` if the depth prepass texture is not already bound. Also modify `crates/bevy_sprinkles_editor/src/viewport.rs` to add `DepthPrepass` (ruling R5).

**Interfaces:** consumes Task 11's `FX_FRESNEL`, `FX_SOFT`, `DRIVE_SLOT_FRESNEL`.

Soft particles need the depth prepass. `ParticleMaterialExtension` already declares `prepass_vertex_shader`/`prepass_fragment_shader`, so the prepass runs; this task reads it.

- [ ] **Step 1: Implement fresnel**

```wgsl
#ifdef FX_FRESNEL
    // Rim brightening. Much of why a stylized cone reads as volumetric rather
    // than as a flat painted shape, for a dot product and a power.
    let n = normalize(in.world_normal);
    let v = normalize(view.world_position.xyz - in.world_position.xyz);
    let power = fx.flow_fresnel.w * emitter_uniforms.drive_slots[DRIVE_SLOT_FRESNEL];
    let rim = pow(1.0 - saturate(dot(n, v)), max(power, 0.001));
    color = vec4(color.rgb + color.rgb * rim * fx.erosion_soft.z, color.a);
#endif
```

For a camera-facing billboard the normal faces the camera everywhere, so fresnel is near-uniform and nearly useless. It earns its keep on mesh particles — cones, spheres, tubes. Say so in the editor's tooltip (Task 22), because an author who tries it on a quad first will conclude it is broken.

- [ ] **Step 2: Implement soft particles**

```wgsl
#ifdef FX_SOFT
    // Fade as the fragment approaches whatever opaque geometry is behind it.
    // The hard intersection line where a quad clips the floor is the single
    // most common tell of amateur VFX; this is the whole fix.
    let frag_coord = in.position;
    let scene_depth = prepass_depth(frag_coord, 0u);
    let this_depth = frag_coord.z;
    // Reversed-Z: nearer is LARGER. depth_ndc_to_view_z converts both to view
    // space so the subtraction is in world units rather than in a nonlinear
    // depth space, where a fixed fade distance would behave differently at
    // every camera range.
    let scene_view_z = view_z_from_depth(scene_depth);
    let this_view_z = view_z_from_depth(this_depth);
    let fade = saturate(abs(scene_view_z - this_view_z) / max(fx.erosion_soft.w, 0.0001));
    color = vec4(color.rgb, color.a * fade);
#endif
```

Use bevy 0.19's actual prepass helpers — `bevy_pbr::prepass_utils::prepass_depth` and the view's depth-to-view-z conversion. **Verify the exact function names against the vendored bevy source before writing them**; guessing a WGSL import produces a shader compile error at runtime, not at build time.

Soft particles require the depth prepass to be enabled on the camera. If it is not, `prepass_depth` returns garbage and particles flicker. Add a one-line note to the crate docs, and enable `DepthPrepass` on the editor's viewport camera **in this task** (`crates/bevy_sprinkles_editor/src/viewport.rs`) — moved here from Task 21 by pre-flight ruling R5, because this feature's own verification step cannot pass without it.

- [ ] **Step 3: Verify visually**

Put an emitter so its quads intersect the ground plane. With `soft_fade: 0.0` there is a hard line; with `soft_fade: 0.5` it fades out smoothly. This is a look-at-it check — no test can assert it.

- [ ] **Step 4: Commit**

```bash
git add crates/bevy_sprinkles/src/shaders/particle_material.wgsl crates/bevy_sprinkles/src/material.rs
git commit -m "feat(fx): fresnel rim and depth-faded soft particles

Soft particles convert the hard quad-meets-floor intersection line --
the most common amateur-VFX tell -- into a fade."
```

---

## Task 15: Gradient remap

**Files:** modify `crates/bevy_sprinkles/src/shaders/particle_material.wgsl`; modify `crates/bevy_sprinkles/src/spawning.rs` to bake the gradient.

**Interfaces:** consumes Task 11's `gradient_texture`, `FX_GRADIENT`, and the existing `GradientTextureCache`.

- [ ] **Step 1: Bake the gradient into the material**

In `spawning.rs`, where `ParticleMaterialExtension` is built, resolve `fx.gradient_remap` through the existing `GradientTextureCache` exactly as the emitter's colour gradient is resolved today, and put the handle in `gradient_texture`. Reuse the cache — do not bake a second time — so two emitters sharing a gradient share one texture.

- [ ] **Step 2: Implement in the shader, immediately after the base texture sample**

```wgsl
#ifdef FX_GRADIENT
    // Treat the base texture as a MASK and colour it through an authored
    // gradient, rather than using its own colour. This is why one greyscale
    // smoke texture can serve a dozen effects, and it is the colour authoring
    // surface -- which is why the Tint drive slot stays a scalar multiplier
    // and does not also try to author colour.
    let mask = base.r;
    let remapped = textureSample(gradient_texture, gradient_sampler, vec2(mask, 0.5));
    base = vec4(remapped.rgb, remapped.a * base.a);
#endif
```

- [ ] **Step 3: Verify visually**

Take a greyscale smoke texture and a black→orange→white gradient. The particle must take the gradient's colours, keyed by the texture's luminance. Confirm that with `FX_GRADIENT` off the original texture colour returns.

- [ ] **Step 4: Commit**

```bash
git add crates/bevy_sprinkles/src/shaders/particle_material.wgsl crates/bevy_sprinkles/src/spawning.rs
git commit -m "feat(fx): gradient remap, so one greyscale mask serves many looks"
```

---

## Task 16: Lit particles — and making the existing `unlit` flag actually work

**Files:** modify `crates/bevy_sprinkles/src/shaders/particle_material.wgsl`, `crates/bevy_sprinkles/src/material.rs`.

**Interfaces:** consumes Task 11's `FxDefs::lit`; `StandardParticleMaterial::unlit` already exists.

**Read this before writing code.** Bevy branches the unlit bit in `pbr.wgsl:81-85` — *outside* `apply_pbr_lighting`. A fragment shader that calls `apply_pbr_lighting` unconditionally therefore **ignores `unlit` entirely**. This exact bug shipped in a sibling project (orgonic's `ToonMaterial`, where `unlit: true` never did anything, diagnosed during its SDF winding fix). So lighting must be a **compile-time branch**, not a runtime uniform read.

- [ ] **Step 1: Write the failing test**

In `material.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{FxSettings, StandardParticleMaterial};

    #[test]
    fn the_lit_def_tracks_the_materials_unlit_flag_inversely() {
        // unlit is the AUTHORED field; FX_LIT is the shader def. They must be
        // exact inverses, because a runtime read of `unlit` inside the
        // fragment does nothing -- bevy branches it in pbr.wgsl:81-85, outside
        // apply_pbr_lighting, which this shader calls directly.
        let lit_mat = StandardParticleMaterial { unlit: false, ..Default::default() };
        let unlit_mat = StandardParticleMaterial { unlit: true, ..Default::default() };
        assert!(FxDefs::from_material(&lit_mat, &FxSettings::default()).lit);
        assert!(!FxDefs::from_material(&unlit_mat, &FxSettings::default()).lit);
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles material:: 2>&1 | tail -20`
Expected: FAIL — `FxDefs::from_material` does not exist.

- [ ] **Step 3: Implement the def**

```rust
impl FxDefs {
    /// Builds the shader-def set for one authored material.
    ///
    /// `lit` is the inverse of the authored `unlit`, resolved HERE at pipeline
    /// specialization rather than in the fragment: bevy branches the unlit bit
    /// in `pbr.wgsl:81-85`, outside `apply_pbr_lighting`, so a shader that
    /// calls `apply_pbr_lighting` unconditionally cannot honour a runtime
    /// `unlit` read at all. Making it a def is the only thing that works.
    pub fn from_material(m: &StandardParticleMaterial, fx: &FxSettings) -> Self {
        Self {
            scroll: fx.scroll_enabled(),
            flow: fx.flow_enabled(),
            erosion: fx.erosion_enabled(),
            fresnel: fx.fresnel_enabled(),
            soft: fx.soft_enabled(),
            gradient: fx.gradient_enabled(),
            lit: !m.unlit,
        }
    }
}
```

Use it in `spawning.rs` where `defs` is filled.

- [ ] **Step 4: Branch in the shader**

```wgsl
#ifdef FX_LIT
    // Smoke and dust must sit in the scene's light. Guarded by a def, never a
    // uniform -- see FxDefs::from_material.
    var pbr_in = pbr_input_from_standard_material(in, is_front);
    pbr_in.material.base_color = color;
    out_color = apply_pbr_lighting(pbr_in);
#else
    out_color = color;
#endif
```

Match the existing fragment's variable names and return shape; do not restructure the function.

- [ ] **Step 5: Run and verify**

Run: `cargo test -p bevy_sprinkles 2>&1 | tail -10` — expect PASS.
Then in the editor: a particle with `unlit: false` must visibly darken when the scene light is removed; with `unlit: true` it must not change at all. If both behave the same, the def is not reaching the fragment.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles/src/material.rs crates/bevy_sprinkles/src/shaders/particle_material.wgsl
git commit -m "feat(fx): lit particles, as a shader def rather than a uniform

bevy branches the unlit bit in pbr.wgsl:81-85, outside the
apply_pbr_lighting this shader calls, so a runtime unlit read does
nothing. A sibling project shipped exactly that bug."
```

---

## Task 17: Length-wise UVs on ribbon and tube trails

**Files:** modify `crates/bevy_sprinkles/src/mesh.rs`.

**Interfaces:** none new. `ParticleMesh::RibbonTrail`/`TubeTrail` already generate strip geometry with `sections`.

This pillar is mostly *finishing* an existing feature: the geometry exists, the material features from Tasks 11-16 apply to it automatically, and only the UV layout is missing.

- [ ] **Step 1: Write the failing test**

In `mesh.rs`:

```rust
    #[test]
    fn a_ribbon_trails_uvs_run_from_zero_to_one_along_its_length() {
        // Without this, a scrolling texture on a beam scrolls ACROSS it
        // instead of ALONG it, which is the one direction a beam must not go.
        let mesh = build_ribbon_trail_mesh(8, RibbonTrailShape::Flat);
        let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().as_float3_or_uv();
        let vs: Vec<f32> = uvs.iter().map(|uv| uv[1]).collect();
        assert!((vs.iter().cloned().fold(f32::MAX, f32::min) - 0.0).abs() < 1e-5);
        assert!((vs.iter().cloned().fold(f32::MIN, f32::max) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_ribbon_trails_uvs_span_zero_to_one_across_its_width() {
        let mesh = build_ribbon_trail_mesh(8, RibbonTrailShape::Flat);
        let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().as_float3_or_uv();
        let us: Vec<f32> = uvs.iter().map(|uv| uv[0]).collect();
        assert!(us.iter().any(|u| *u < 1e-5));
        assert!(us.iter().any(|u| (*u - 1.0).abs() < 1e-5));
    }
```

Adapt the accessor and the builder's real name to what `mesh.rs` actually exposes; if the builder is private, make it `pub(crate)` rather than testing through the public API and losing the precision.

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles mesh:: 2>&1 | tail -20`
Expected: FAIL.

- [ ] **Step 3: Implement**

In the ribbon and tube generators, set `V` from the section index and `U` from the position around the cross-section:

```rust
    // V runs 0..1 head-to-tail so a scrolling texture travels ALONG the beam;
    // U runs 0..1 across the cross-section so it wraps once around a tube.
    let v = section as f32 / (sections.max(1) - 1).max(1) as f32;
    let u = cross_index as f32 / cross_count.max(1) as f32;
    uvs.push([u, v]);
```

Emit one UV per vertex, in the same order the positions are pushed. A mismatched count is a panic at mesh upload, not a compile error.

- [ ] **Step 4: Run and verify visually**

Run: `cargo test -p bevy_sprinkles mesh::` — expect PASS.
Then in the editor, a ribbon trail with a striped texture and `scroll: (0.0, 1.0)` must show the stripes travelling from head to tail.

- [ ] **Step 5: Commit**

```bash
git add crates/bevy_sprinkles/src/mesh.rs
git commit -m "feat(mesh): length-wise UVs on ribbon and tube trails

The geometry already existed; without UVs running along it, a scrolling
texture on a beam scrolled across the beam instead of down it."
```

---

**Phase 3 gate.** `cargo test --workspace` passes. Build one effect using scroll + flow + erosion + gradient + a light together and look at it: this is the first moment the Valorant target is testable at all, and it is worth a screenshot in the PR.

---

# Phase 4 — The editor

**Before starting any Phase 4 task**, read `crates/bevy_sprinkles_editor/src/ui/components/inspector/mod.rs` and one small sibling component (`inspector/turbulence.rs`, 124 lines, is the best model). This editor is bevy_ui-native with its own widget set — there is no egui and no immediate-mode idiom. Every task below says which existing component to mirror; mirroring it is not optional polish, it is how the panel ends up looking like the rest of the app.

## Task 18: The Variables panel

**Files:**
- Create: `crates/bevy_sprinkles_editor/src/ui/components/variables.rs`
- Modify: `src/state.rs` (`Inspectable::Variable`), `src/ui/components/sidebar.rs`, `src/ui/components/mod.rs`

**Interfaces:**
- Consumes: `VariableDecl`, `ParticlesAsset::variables`, `ParticleVariables`.
- Produces: `VariableScrub` resource — the editor's live per-variable values, applied to the previewed effect entity's `ParticleVariables`.

**Mirror:** `ui/components/inspector/turbulence.rs` for section layout; `ui/components/sidebar.rs`'s emitter list for add/remove/select.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubbing_a_variable_reaches_the_previewed_entitys_particle_variables() {
        // The slider IS the preview mechanism -- there is no separate preview
        // concept -- so this wiring is the feature, not a convenience.
        let mut scrub = VariableScrub::default();
        scrub.set("temperature", 0.75);
        let mut vars = ParticleVariables::default();
        scrub.apply_to(&mut vars);
        assert_eq!(vars.get("temperature"), Some(0.75));
    }

    #[test]
    fn a_renamed_variable_drops_its_stale_scrub_value() {
        // Otherwise a rename leaves a ghost entry that silently drives nothing
        // and shows a stale number next to the new name.
        let mut scrub = VariableScrub::default();
        scrub.set("heat", 0.9);
        scrub.retain_declared(&[VariableDecl { name: "temperature".into(), ..Default::default() }]);
        assert_eq!(scrub.get("heat"), None);
    }

    #[test]
    fn an_undeclared_variable_scrubs_to_its_declared_default() {
        let scrub = VariableScrub::default();
        let decls = vec![VariableDecl { name: "t".into(), default: 0.3, ..Default::default() }];
        assert_eq!(scrub.value_or_default("t", &decls), 0.3);
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p bevy_sprinkles_editor variables:: 2>&1 | tail -20`
Expected: FAIL.

- [ ] **Step 3: Implement `VariableScrub`**

```rust
use std::collections::HashMap;
use bevy::prelude::*;
use bevy_sprinkles::prelude::*;
use bevy_sprinkles::asset::VariableDecl;

/// The editor's live knob values for the previewed effect.
///
/// Separate from the asset because these are a VIEWING state, not authored
/// content: scrubbing temperature to 0.9 to see what happens must never dirty
/// the project or end up saved. The authored value is `VariableDecl::default`.
#[derive(Resource, Default)]
pub struct VariableScrub(HashMap<String, f32>);

impl VariableScrub {
    pub fn set(&mut self, name: &str, v: f32) { self.0.insert(name.to_string(), v); }
    pub fn get(&self, name: &str) -> Option<f32> { self.0.get(name).copied() }

    pub fn value_or_default(&self, name: &str, decls: &[VariableDecl]) -> f32 {
        self.get(name).unwrap_or_else(|| {
            decls.iter().find(|d| d.name == name).map(|d| d.default).unwrap_or(0.0)
        })
    }

    /// Drops entries for variables that no longer exist, so a rename or a
    /// delete cannot leave a ghost that shows a stale number and drives
    /// nothing.
    pub fn retain_declared(&mut self, decls: &[VariableDecl]) {
        self.0.retain(|k, _| decls.iter().any(|d| &d.name == k));
    }

    pub fn apply_to(&self, vars: &mut ParticleVariables) {
        for (k, v) in &self.0 {
            vars.set(k, *v);
        }
    }
}
```

- [ ] **Step 4: Build the panel UI**

Add `Inspectable::Variable` to `state.rs`'s enum. Add a **Variables** section to the sidebar outliner listing `asset.variables` by name, with an add button (appends `VariableDecl::default()` named `variable_N`, unique) and a per-row delete.

Selecting a variable inspects it: name text field, default, range min/max, and **a slider bound to `VariableScrub`** spanning the declared range. Editing name/default/range marks the project dirty; moving the slider does **not**.

Add a system that each frame calls `scrub.retain_declared(...)` and `scrub.apply_to(...)` on the previewed entity's `ParticleVariables`, inserting the component if absent.

Deleting a variable that drives something must not corrupt the file: `VariableId`s are positional, so removing index 1 shifts index 2 down. Either renumber every `Drive::variable` above the removed index, or refuse the delete with a message naming the drives. **Renumber** — refusing makes the editor feel broken. Pin it:

```rust
    #[test]
    fn deleting_a_variable_renumbers_the_drives_above_it() {
        let mut asset = /* 3 variables, a drive on VariableId(2) */;
        remove_variable(&mut asset, 1);
        assert_eq!(asset.drives[0].variable, VariableId(1), "ids are positional and must shift");
    }

    #[test]
    fn deleting_a_variable_removes_the_drives_that_used_it() {
        let mut asset = /* 2 variables, a drive on VariableId(0) */;
        remove_variable(&mut asset, 0);
        assert!(asset.drives.is_empty(), "a drive with no variable would fail validation on load");
    }
```

A drive left pointing at a deleted variable fails `validate_drives` on the next load, so the editor would happily write a file it cannot reopen. That is the worst possible bug in an authoring tool and these two tests are the whole defence.

- [ ] **Step 5: Run and use it**

Run: `cargo test -p bevy_sprinkles_editor 2>&1 | tail -10` — expect PASS.
Then: add a variable, add a drive on `SizeMul` (Task 19 makes this easy; until then hand-edit the `.ron`), move the slider, and watch the particles change size live. **Save, quit, reopen** — the file must load without a validation error.

- [ ] **Step 6: Commit**

```bash
git add crates/bevy_sprinkles_editor/src/ui/components/variables.rs \
        crates/bevy_sprinkles_editor/src/ui/components/mod.rs \
        crates/bevy_sprinkles_editor/src/ui/components/sidebar.rs \
        crates/bevy_sprinkles_editor/src/state.rs
git commit -m "feat(editor): the Variables panel, whose slider is the preview

Scrub values are viewing state and never dirty the project. Deleting a
variable renumbers the positional ids in every drive above it, because
writing a file that fails its own load validation is the worst bug an
authoring tool can have."
```

---

## Task 19: The drive affordance on inspector fields

**Files:**
- Create: `crates/bevy_sprinkles_editor/src/ui/components/inspector/drive_button.rs`
- Modify: `src/ui/components/inspector/mod.rs`

**Interfaces:**
- Consumes: Task 18's Variables panel; `Drive`, `DriveTarget`, `EmitterProp`, `DriveOp`; the existing `curve_edit` widget.
- Produces: `fn drive_button(...)` used by every drivable inspector field.

**Mirror:** how `inspector/colors.rs` opens `gradient_edit` — the same open-a-widget-bound-to-a-field pattern, which is why `curve_edit` needs no new machinery here.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn adding_a_drive_for_a_field_that_has_none_appends_one() {
        let mut asset = /* 1 variable, 1 emitter, no drives */;
        upsert_drive(&mut asset, DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul }, VariableId(0));
        assert_eq!(asset.drives.len(), 1);
        assert_eq!(asset.drives[0].variable, VariableId(0));
    }

    #[test]
    fn the_button_reports_how_many_drives_target_a_field() {
        let mut asset = /* 1 variable, 1 emitter */;
        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul };
        assert_eq!(drives_on(&asset, &target).len(), 0);
        upsert_drive(&mut asset, target.clone(), VariableId(0));
        upsert_drive(&mut asset, target.clone(), VariableId(0));
        assert_eq!(drives_on(&asset, &target).len(), 2, "several drives on one target are legal");
    }

    #[test]
    fn a_new_drive_defaults_to_an_identity_curve_and_multiply() {
        // Adding a drive must not change the look until the author shapes it.
        // A drive that immediately altered the effect would make "what did I
        // just do?" unanswerable.
        let mut asset = /* 1 variable, 1 emitter */;
        upsert_drive(&mut asset, DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul }, VariableId(0));
        let d = &asset.drives[0];
        assert_eq!(d.op, DriveOp::Multiply);
        assert_eq!(d.output, Range { min: 1.0, max: 1.0 });
    }
```

- [ ] **Step 2: Run to confirm failure, then implement**

Run: `cargo test -p bevy_sprinkles_editor drive_button:: 2>&1 | tail -20` → FAIL.

```rust
/// Every drive on one target, in declaration order -- which is application
/// order, so the list's order is meaningful and must not be sorted.
pub fn drives_on<'a>(asset: &'a ParticlesAsset, target: &DriveTarget) -> Vec<(usize, &'a Drive)> {
    asset.drives.iter().enumerate().filter(|(_, d)| &d.target == target).collect()
}

/// Appends a drive that is deliberately a NO-OP until shaped: identity curve,
/// Multiply, output pinned to 1..1. Adding a wire must never change the look.
pub fn upsert_drive(asset: &mut ParticlesAsset, target: DriveTarget, variable: VariableId) {
    asset.drives.push(Drive {
        variable,
        target,
        curve: CurveTexture::default(),
        output: Range { min: 1.0, max: 1.0 },
        op: DriveOp::Multiply,
        muted: false,
    });
}
```

- [ ] **Step 3: Build the button**

Beside each drivable numeric field, paint a small button showing the drive count (empty when zero). Clicking opens a popover (mirror `ui/widgets/popover.rs`) with: a variable combobox, the `curve_edit` widget bound to `Drive::curve`, output min/max, an op combobox, a mute toggle and a delete.

The popover header must state the target's **stage** — "Spawn: affects only new particles", "Sim: affects particles already in flight", "Render: affects all live particles every frame". That is the first question an author asks of a knob, and `EmitterProp::stage()` already answers it.

Add the button to the numeric fields in `inspector/scale.rs`, `colors.rs`, `velocities.rs`, `accelerations.rs`, `turbulence.rs`, `time.rs` and the new `material_fx.rs` — one per `EmitterProp` variant. `EmitterProp::ALL` is the checklist; a variant with no field to hang off is a gap to report, not to skip silently.

- [ ] **Step 4: Run, use, commit**

Run: `cargo test -p bevy_sprinkles_editor 2>&1 | tail -10` — expect PASS.
Then wire temperature → SizeMul entirely through the UI, shape the curve, and scrub. Save and reopen.

```bash
git add crates/bevy_sprinkles_editor/src/ui/components/inspector/
git commit -m "feat(editor): drive any numeric field from a variable

Reuses curve_edit rather than adding a curve UI. A new drive is an
identity no-op so wiring one never changes the look, and the popover
names the stage because that is the first thing an author asks."
```

---

## Task 20: The Drives list view

**Files:** create `crates/bevy_sprinkles_editor/src/ui/components/drives.rs`; modify `sidebar.rs`.

**Interfaces:** consumes Task 19's `drives_on`/`upsert_drive`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn reordering_a_drive_changes_application_order() {
        // Order is observable -- Replace discards everything before it -- so
        // the list must be reorderable and must NOT sort itself.
        let mut asset = /* 3 drives on one target */;
        move_drive(&mut asset, 2, 0);
        assert_eq!(asset.drives[0].output.min, /* the one that was last */);
    }

    #[test]
    fn muting_a_drive_persists_but_does_not_delete_it() {
        let mut asset = /* 1 drive */;
        asset.drives[0].muted = true;
        assert_eq!(asset.drives.len(), 1);
    }
```

- [ ] **Step 2: Implement**

A flat list of every drive: variable name → target (emitter/light name + property) → op → output range, with a mute toggle, drag-to-reorder and delete. Group by target with a separator, and show a warning badge on any target carrying two `Replace` drives (legal, last wins, and almost always a mistake mid-rewiring).

Also badge a target driven by both `ScaleUniform` and a per-axis scale prop — same reason.

- [ ] **Step 3: Run, commit**

```bash
git add crates/bevy_sprinkles_editor/src/ui/components/drives.rs \
        crates/bevy_sprinkles_editor/src/ui/components/sidebar.rs
git commit -m "feat(editor): a Drives list, so the wiring is legible at once

Reorderable and never self-sorting, because declaration order is
application order and Replace discards what came before it."
```

---

## Task 21: Lights in the outliner and inspector

**Files:** create `crates/bevy_sprinkles_editor/src/ui/components/inspector/light.rs`; modify `state.rs`, `sidebar.rs`(the `DepthPrepass` camera change moved to Task 14 by ruling R5).

**Interfaces:** consumes Task 9's `LightData`; Task 14's soft particles need the prepass.

**Mirror:** `inspector/collider_properties.rs` — it is the closest existing "a list of non-emitter objects on the asset" inspector.

- [ ] **Step 1: Add `Inspectable::Light`, a Lights tree section, and the inspector**

Fields: name, enabled, kind (Point/Spot), transform, colour (via the existing `color_picker`), intensity, range, shadows, the `EmitterTime` block (mirror `inspector/time.rs`), and `intensity_over_life` via `curve_edit`. Each numeric field gets Task 19's drive button with `DriveTarget::Light`.

- [ ] **Step 2: Confirm the depth prepass is already enabled**

Task 14 added `DepthPrepass` to the viewport camera (ruling R5). Confirm it is still there; do not add it twice. If it is missing, Task 14 regressed and that is a finding, not a fix to make here.

- [ ] **Step 3: Write the failing test**

```rust
    #[test]
    fn deleting_a_light_renumbers_the_drives_above_it() {
        // Same positional-index hazard as variables (Task 18): a stale index
        // writes a file that fails its own load validation.
        let mut asset = /* 3 lights, a drive on Light index 2 */;
        remove_light(&mut asset, 1);
        assert!(matches!(asset.drives[0].target, DriveTarget::Light { index: 1, .. }));
    }

    #[test]
    fn deleting_a_light_removes_the_drives_that_targeted_it() {
        let mut asset = /* 1 light, a drive on Light index 0 */;
        remove_light(&mut asset, 0);
        assert!(asset.drives.is_empty());
    }
```

- [ ] **Step 4: Run, use, commit**

Add a light, drive its intensity from a variable through a curve, scrub, and watch it flicker. Save and reopen.

```bash
git add crates/bevy_sprinkles_editor/src/ui/components/inspector/light.rs \
        crates/bevy_sprinkles_editor/src/state.rs \
        crates/bevy_sprinkles_editor/src/ui/components/sidebar.rs
git commit -m "feat(editor): author effect-owned lights

Light indices are positional like variable ids, so a delete renumbers
the drives above it rather than writing a file that fails its own load."
```

---

## Task 22: The material FX inspector section

**Files:** create `crates/bevy_sprinkles_editor/src/ui/components/inspector/material_fx.rs`; modify `inspector/mod.rs`.

**Interfaces:** consumes Task 11's `FxSettings`.

- [ ] **Step 1: Build the section**

Under the emitter's Draw Pass inspector, add an **FX** section with collapsible sub-sections mirroring `inspector/turbulence.rs`:

- **Scroll** — rate (vector2), tiling (vector2)
- **Flow** — texture (via `ui/widgets/texture_edit.rs`), strength, scroll
- **Erosion** — texture, threshold, edge width, edge colour
- **Fresnel** — power, boost
- **Soft** — fade distance
- **Gradient remap** — the existing `gradient_edit` widget

Each numeric field gets Task 19's drive button where a matching `EmitterProp` exists (`ScrollU`, `ScrollV`, `FlowStrength`, `ErosionThreshold`, `FresnelPower`).

- [ ] **Step 2: Add the two tooltips that prevent a wrong conclusion**

- On **Fresnel**: "Near-uniform on camera-facing billboards — use on mesh particles (cone, sphere, tube)." Without this an author tries it on a quad, sees almost nothing, and concludes it is broken.
- On **Soft**: "Requires a depth prepass on the camera."

- [ ] **Step 3: Write the failing test**

```rust
    #[test]
    fn toggling_a_feature_off_restores_the_inert_default_exactly() {
        // Otherwise a disabled feature leaves residue that silently re-enables
        // itself via the *_enabled() predicates on the next load.
        let mut fx = FxSettings { fresnel_power: 2.0, fresnel_boost: 1.0, ..Default::default() };
        clear_fresnel(&mut fx);
        assert_eq!(fx, FxSettings::default());
    }
```

- [ ] **Step 4: Run, use, commit**

Author a scrolled, eroded, gradient-remapped cone entirely through the UI. Save and reopen.

```bash
git add crates/bevy_sprinkles_editor/src/ui/components/inspector/material_fx.rs \
        crates/bevy_sprinkles_editor/src/ui/components/inspector/mod.rs
git commit -m "feat(editor): author the FX material features"
```

---

## Task 23: The Mesh FX preset

**Files:** modify `crates/bevy_sprinkles_editor/src/ui/components/sidebar.rs` (or wherever "add emitter" lives).

**Interfaces:** consumes nothing new.

This is what replaced the mesh-effect object that the design cut. A mesh FX is an emitter configured a particular way; this makes that configuration one click instead of five fields, which is the entire reason the separate object type was unnecessary.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn the_mesh_fx_preset_produces_a_single_pinned_mesh_particle() {
        let e = mesh_fx_emitter();
        assert_eq!(e.emission.particles_amount, 1, "one particle -- it IS the mesh");
        assert!(e.time.one_shot);
        assert_eq!(e.velocities.initial_velocity, Range::new(0.0, 0.0), "it must not drift");
        assert!(e.draw_pass.use_local_coords, "it must follow the effect");
        assert!(e.draw_pass.transform_align.is_none(), "it must not billboard");
    }
```

- [ ] **Step 2: Implement**

```rust
/// An emitter configured as a single pinned mesh rather than a particle spray.
///
/// The design cut a separate `MeshEffectData` object because an emitter shaped
/// like this already IS one -- and arrives with lifetime curves, gradients,
/// timing, sub-emitters and the whole inspector already working. This function
/// is the convenience that cut bought.
pub fn mesh_fx_emitter() -> EmitterData {
    let mut e = EmitterData::default();
    e.name = "Mesh FX".into();
    e.emission.particles_amount = 1;
    e.time.one_shot = true;
    e.velocities.initial_velocity = Range::new(0.0, 0.0);
    e.velocities.spread = 0.0;
    e.draw_pass.use_local_coords = true;
    e.draw_pass.transform_align = None;
    e.draw_pass.mesh = ParticleMesh::Cylinder {
        top_radius: 0.0, bottom_radius: 1.0, height: 2.0,
        radial_segments: 16, rings: 1, cap_top: false, cap_bottom: false,
    };
    e
}
```

Add "Add Mesh FX" beside "Add Emitter". Adapt field names to the real `EmitterData` shape.

- [ ] **Step 3: Run, use, commit**

Click it, set a scrolled texture, drive `Transform ScaleY` from a variable, scrub — a cone that lengthens without widening. That is the whole mesh pillar working through the cut design.

```bash
git add crates/bevy_sprinkles_editor/src/ui/components/sidebar.rs
git commit -m "feat(editor): an Add Mesh FX preset

The convenience the cut mesh-effect object bought: a pinned single-mesh
emitter in one click."
```

---

## Done criteria

- `cargo test --workspace` passes; count recorded against Task 1's baseline.
- An effect declares `temperature`; four drives shape hue, size, light intensity and scroll rate from it through four differently-shaped curves; scrubbing the slider moves all four. That is the brief's worked example, end to end.
- Two entities sharing that asset with different `ParticleVariables` render differently at the same time.
- An effect authored entirely in the UI saves, quits, reopens, and renders identically.
- A v0.3 file from before this branch still loads.

## Follow-on, not in this plan

The orgonic migration: 53 `bevy_sprinkles` references, three live assets (`glacial.ron`, `ionic.ron`, `steam.ron`), retiring the parallel in-house `src/vfx/` runtime and its F8 Effect/Preview tabs, and replacing that project's `ParticleOverride` use with drives. Separate repo, separate spec. **orgonic must stay pinned to the pre-branch fork commit until that spec runs**, because Task 8 removes `ParticleOverride` and orgonic uses it.
