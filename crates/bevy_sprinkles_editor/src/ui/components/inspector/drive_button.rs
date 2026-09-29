//! The drive affordance beside a drivable inspector field.
//!
//! A small button beside a numeric field shows how many [`Drive`]s target
//! that property (empty when zero) and, on click, points the Drives dock at
//! that drive -- see [`handle_drive_trigger_click`]. It authors nothing
//! itself.
//!
//! **This module used to own a second editor.** The button opened a popover
//! carrying its own row list (variable combobox, `curve_edit`, output
//! min/max, op combobox, mute, delete), so one drive could be open in two
//! surfaces at once, kept in step only by both watching `DirtyState` and
//! rebuilding wholesale. The dock redesign left one editing surface --
//! `components::drives`' editor pane -- reachable from two PLACES, and the
//! popover, its rebuild system and its own "Add drive" button went with the
//! duplication. What stayed here is everything the dock reuses: the six
//! row-marker components, [`spawn_drive_row`] that spawns them, every commit
//! observer behind them, and [`upsert_drive`].
//!
//! **A fresh drive must RESPOND.** [`upsert_drive`] appends a rising ramp,
//! `DriveOp::Multiply`, output `0.0..1.0`: a 0-100% fader on the consumer's
//! authored value, moving with the variable from the moment it is wired.
//!
//! That reverses this module's original rule, "a fresh drive must be a
//! no-op", and the reversal is deliberate. The old default was a constant
//! curve with the output pinned to `1.0..1.0`, which bought "wiring a drive
//! never changes the look" at the price of a drive that could not change it
//! LATER either: `sample` computes `lo + (hi - lo) * unit`, so a `min ==
//! max` output resolves to the same number for every value of the variable.
//! The wire appeared in the list, drew its curve, and was mathematically
//! incapable of doing anything until its output range was widened by hand,
//! with nothing on screen saying so -- and an author who does not already
//! know that formula has no way to find out. A jolt is cheap and visible;
//! a dead wire is neither.
//!
//! The accepted cost, stated rather than hidden: adding a drive CAN change
//! the look immediately. Most sharply when the target variable's default is
//! `0.0`, where the property drops to zero until the variable is scrubbed
//! up. See the `tests` module for the pin on the new contract.
//!
//! **The dock states the target's stage in words**, not as an enum name:
//! that is the first question an author asks of a knob ("does this change
//! what is already in the air?"), and `EmitterProp::stage()` already answers
//! it -- [`stage_label`] just spells it out, and `drives.rs` calls it.
//!
//! **Rows are addressed by their live index into `ParticlesAsset::drives`**,
//! not by identity, so every write here re-borrows the asset and re-checks
//! bounds rather than caching a `&mut Drive`. The dock's editor pane is
//! rebuilt wholesale on any dirty edit, which is what turns a stale index
//! into a correct one on the next tick (mirroring
//! `variables::rebuild_variable_list`'s reasoning: simpler and safer than
//! tracking which specific mutation happened). The one index this module
//! must fix up rather than rebuild is the dock's SELECTION, because a
//! selection survives the rebuild that corrects everything else -- see
//! [`handle_drive_delete_click`].
//!
//! **Target resolution is dynamic, not baked in at spawn time.** Every
//! section in this module (`scale_section`, `colors_section`, ...) is built
//! ONCE when the inspector panel is first added, then reused across every
//! emitter the author selects -- `sync.rs` re-reads the currently inspected
//! emitter's data each frame rather than the section being torn down and
//! rebuilt per emitter. A `drive_button` therefore stores only the
//! `EmitterProp` it drives; the emitter `index` half of its `DriveTarget` is
//! resolved from `InspectedEmitterTracker::current_index` wherever it is
//! needed (label sync, trigger click), never cached on the button itself.
//!
//! **Task 21 widened this to `LightProp`.** A button now stores a
//! [`DrivableProp`] (`Emitter(EmitterProp)` or `Light(LightProp)`) instead of
//! a bare `EmitterProp`; [`current_target`] resolves the right half against
//! the right tracker (`InspectedEmitterTracker` or `InspectedLightTracker`)
//! depending on which variant it holds. Only two `LightProp` variants get a
//! button at all -- `Intensity` and `Range`, the two that have an authored
//! field on `LightData` to hang one off; `Hue`/`Saturation`/`Value` have none
//! (they are drive-only targets, exactly like
//! `EmitterProp::SpawnProbability`) and stay reachable only through
//! `drives.rs`'s target picker.

use bevy::prelude::*;
use bevy_sprinkles::asset::{
    Drive, DriveOp, DriveTarget, EmitterProp, LightProp, Stage, VariableDecl, VariableId,
};
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState};
use crate::ui::icons::{ICON_CLOSE, ICON_NODE_TREE};
use crate::ui::tokens::BORDER_COLOR;
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonVariant, IconButtonProps, button, icon_button,
};
use crate::ui::widgets::checkbox::{CheckboxCommitEvent, CheckboxProps, checkbox};
use crate::ui::widgets::combobox::{
    ComboBoxChangeEvent, ComboBoxOptionData, combobox_with_selected,
};
use crate::ui::widgets::curve_edit::{CurveEditCommitEvent, CurveEditProps, curve_edit};
use crate::ui::widgets::inspector_field::fields_row;
use crate::ui::widgets::text_edit::{TextEditCommitEvent, TextEditProps, text_edit};
use crate::ui::widgets::utils::find_ancestor;

use crate::ui::components::drives::{SelectedDrive, selection_after_delete};

use super::{InspectedEmitterTracker, InspectedLightTracker};

// --- Pure data operations -----------------------------------------------

/// Every drive on one target, in declaration order -- which is application
/// order, so the list's order is meaningful and must not be sorted.
pub fn drives_on<'a>(asset: &'a ParticlesAsset, target: &DriveTarget) -> Vec<(usize, &'a Drive)> {
    asset
        .drives
        .iter()
        .enumerate()
        .filter(|(_, d)| &d.target == target)
        .collect()
}

/// Appends a drive that WORKS the moment it is wired: a rising ramp,
/// Multiply, output `0.0..1.0` -- a 0-100% fader on the consumer's authored
/// value, at `1.0` (authored value untouched) when the variable sits at the
/// top of its declared range and `0.0` at the bottom.
///
/// Both halves of that are load-bearing and neither works alone. `sample`
/// computes `lo + (hi - lo) * unit`, so the old `1.0..1.0` output collapsed
/// every variable value to one number; and `CurveTexture::default()` is the
/// CONSTANT curve (`Curve::default`'s two points are `(0, 1)` and `(1, 1)`,
/// named "Constant"), so `unit` is `1.0` whatever the variable does. Widening
/// the output alone would have left the drive just as dead -- `0.0 + 1.0 *
/// 1.0` -- which is why this builds its own ramp rather than taking the
/// shared default. The crate-wide `CurveTexture::default()` is deliberately
/// left alone: it is the right default for a curve that is not a fader.
///
/// This is the one place a drive is created. Both authoring routes -- the
/// per-field drive button's "Add drive" and `drives.rs`'s target picker --
/// call it, so the default lives here and only here.
pub fn upsert_drive(asset: &mut ParticlesAsset, target: DriveTarget, variable: VariableId) {
    asset.drives.push(Drive {
        variable,
        target,
        curve: CurveTexture::new(vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]),
        output: ParticleRange { min: 0.0, max: 1.0 },
        op: DriveOp::Multiply,
        muted: false,
    });
}

/// Spells out an `EmitterProp`'s stage as the question an author actually
/// asks, rather than the enum name. Exhaustive on `Stage`, no wildcard arm --
/// a new stage is a compile error here until it says what it means.
///
/// `pub(crate)`: reused verbatim by `drives.rs`'s flat list, which states the
/// same stage next to every row regardless of which kind of target it is.
pub(crate) fn stage_label(prop: EmitterProp) -> &'static str {
    match prop.stage() {
        Stage::Spawn => "Spawn: affects only new particles",
        Stage::Sim => "Sim: affects particles already in flight",
        Stage::Render => "Render: affects all live particles every frame",
    }
}

fn format_f32(v: f32) -> String {
    let mut text = v.to_string();
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

// --- Public widget API ---------------------------------------------------

/// Which family of numeric field a [`drive_button`] hangs off. See the
/// module doc's Task 21 paragraph.
#[derive(Clone, Copy)]
enum DrivableProp {
    Emitter(EmitterProp),
    Light(LightProp),
}

pub struct DriveButtonProps {
    prop: DrivableProp,
}

impl DriveButtonProps {
    pub fn new(prop: EmitterProp) -> Self {
        Self {
            prop: DrivableProp::Emitter(prop),
        }
    }

    /// A drive button beside a light's own numeric field
    /// (`LightData::intensity`/`range`), targeting `DriveTarget::Light`
    /// instead of `DriveTarget::Emitter`.
    pub fn new_light(prop: LightProp) -> Self {
        Self {
            prop: DrivableProp::Light(prop),
        }
    }
}

#[derive(Component, Default, Clone)]
pub struct EditorDriveButton;

pub fn drive_button(props: DriveButtonProps) -> impl Scene {
    let DriveButtonProps { prop } = props;
    bsn! {
        EditorDriveButton
        template_value(DriveButtonProp(prop))
        Node {
            flex_shrink: 0.0,
        }
    }
}

pub fn plugin(app: &mut App) {
    app.add_observer(handle_drive_trigger_click)
        .add_observer(handle_drive_variable_change)
        .add_observer(handle_drive_op_change)
        .add_observer(handle_drive_mute_commit)
        .add_observer(handle_drive_delete_click)
        .add_observer(handle_drive_curve_commit)
        .add_observer(handle_drive_output_commit)
        .add_systems(
            Update,
            (setup_drive_button, sync_drive_button_label)
                .after(super::update_inspected_emitter_tracker)
                .after(super::update_inspected_light_tracker),
        );
}

// --- Components -----------------------------------------------------------

#[derive(Component, Clone, Copy)]
struct DriveButtonProp(DrivableProp);

impl Default for DriveButtonProp {
    fn default() -> Self {
        // `template_value` needs `Default` for its scene-reflection
        // scaffolding; this placeholder is never the value actually stored
        // -- `drive_button()` always threads the real prop through the
        // template's own argument at spawn time.
        Self(DrivableProp::Emitter(EmitterProp::SpawnProbability))
    }
}

#[derive(Component)]
struct DriveButtonTrigger(Entity);

#[derive(Component)]
struct DriveRow;

// The six row-marker types below, and `spawn_drive_row` that spawns them,
// are `pub(crate)`: `drives.rs`'s flat list reuses this ENTIRE row (variable
// combo, curve edit, output min/max, op combo, mute checkbox, delete button)
// verbatim rather than reimplementing it, which means it also reuses every
// observer below unchanged -- those observers query by component type, not
// by which module spawned the entity, so a `drives.rs` row wired with these
// same markers is handled by the exact same code path as a popover row.
#[derive(Component, Clone, Copy)]
pub(crate) struct DriveVariableCombo(pub(crate) usize);

#[derive(Component, Clone, Copy)]
pub(crate) struct DriveOpCombo(pub(crate) usize);

#[derive(Component, Clone, Copy)]
pub(crate) struct DriveMuteCheckbox(pub(crate) usize);

#[derive(Component, Clone, Copy)]
pub(crate) struct DriveDeleteButton(pub(crate) usize);

#[derive(Component, Clone, Copy)]
pub(crate) struct DriveCurveTarget(pub(crate) usize);

#[derive(Component, Clone, Copy, PartialEq)]
pub(crate) enum OutputBound {
    Min,
    Max,
}

#[derive(Component, Clone, Copy)]
pub(crate) struct DriveOutputField {
    pub(crate) index: usize,
    pub(crate) bound: OutputBound,
}

fn current_target(
    prop: DrivableProp,
    emitter_tracker: &InspectedEmitterTracker,
    light_tracker: &InspectedLightTracker,
) -> Option<DriveTarget> {
    match prop {
        DrivableProp::Emitter(prop) => emitter_tracker
            .current_index
            .map(|index| DriveTarget::Emitter { index, prop }),
        DrivableProp::Light(prop) => light_tracker
            .current_index
            .map(|index| DriveTarget::Light { index, prop }),
    }
}

// --- Trigger button ---------------------------------------------------

fn setup_drive_button(mut commands: Commands, buttons: Query<Entity, Added<EditorDriveButton>>) {
    for entity in &buttons {
        let trigger = commands
            .spawn_scene(button(
                ButtonProps::new("").with_left_icon(ICON_NODE_TREE),
            ))
            .insert(DriveButtonTrigger(entity))
            .id();
        commands.entity(entity).add_child(trigger);
    }
}

/// Keeps the trigger's label in sync with how many drives target this
/// button's property on the currently inspected emitter. Re-derived from the
/// asset each time, never bookkept incrementally -- see the module doc.
fn sync_drive_button_label(
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    tracker: Res<InspectedEmitterTracker>,
    light_tracker: Res<InspectedLightTracker>,
    props: Query<&DriveButtonProp>,
    triggers: Query<(&DriveButtonTrigger, &Children)>,
    mut texts: Query<&mut Text>,
    new_triggers: Query<Entity, Added<DriveButtonTrigger>>,
) {
    let should_sync = assets.is_changed()
        || tracker.is_changed()
        || light_tracker.is_changed()
        || !new_triggers.is_empty();
    if !should_sync {
        return;
    }

    let count_for = |prop: DrivableProp| -> usize {
        let Some(target) = current_target(prop, &tracker, &light_tracker) else {
            return 0;
        };
        let Some(handle) = &editor_state.current_project else {
            return 0;
        };
        let Some(asset) = assets.get(handle) else {
            return 0;
        };
        drives_on(asset, &target).len()
    };

    for (trig, children) in &triggers {
        let Ok(prop) = props.get(trig.0) else {
            continue;
        };
        let count = count_for(prop.0);
        let label = if count == 0 {
            String::new()
        } else {
            count.to_string()
        };
        for child in children.iter() {
            if let Ok(mut text) = texts.get_mut(child) {
                if **text != label {
                    **text = label;
                }
                break;
            }
        }
    }
}

// --- Trigger: one click, one selection in the Drives dock ---------------

/// A field's drive button no longer opens an editor of its own. It points
/// the Drives dock at the drive for this field's target, and the dock's
/// editor pane -- the ONE place a drive is edited -- shows it.
///
/// This replaced a popover that carried a full row list of its own. Two
/// surfaces could hold the same drive open at once, kept in step only by
/// both watching `DirtyState` and rebuilding wholesale; the popover's row
/// list, its header, its own "Add drive" button and the rebuild system
/// behind them are all gone with it, along with the coupling they needed.
///
/// **A field with no drive yet creates one**, rather than doing nothing
/// visible. The old flow was click -> popover -> "Add drive"; there is no
/// popover to hold that second step, and a click that selects nothing would
/// read as a dead button. It appends through [`upsert_drive`], the one place
/// a drive is created, and opens it -- so the affordance still reaches
/// `VariableId(0)` and a `0.0..1.0` fader exactly as before.
///
/// **A field with several drives opens the FIRST**, in apply order. Any
/// choice here is arbitrary; the earliest is the one the dock's own list
/// prints first, so the two surfaces agree on what "this field's drive"
/// means, and the dock's row for each of the others is one click away.
#[allow(clippy::too_many_arguments)]
fn handle_drive_trigger_click(
    trigger: On<ButtonClickEvent>,
    tracker: Res<InspectedEmitterTracker>,
    light_tracker: Res<InspectedLightTracker>,
    triggers: Query<&DriveButtonTrigger>,
    props: Query<&DriveButtonProp>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut selected: ResMut<SelectedDrive>,
) {
    let Ok(drive_trigger) = triggers.get(trigger.entity) else {
        return;
    };
    let Ok(prop) = props.get(drive_trigger.0) else {
        return;
    };
    // Nothing to drive without a currently inspected emitter/light -- the
    // button exists on a section that is shared across every emitter (or
    // light) the author selects (see the module doc), so this can
    // legitimately be transient between selections.
    let Some(target) = current_target(prop.0, &tracker, &light_tracker) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };

    // Read through `get`, not `get_mut`: selecting is not an edit, and
    // `get_mut` raises an `AssetEvent::Modified` that every `assets
    // .is_changed()` watcher in the editor would then answer.
    let existing = {
        let Some(asset) = assets.get(handle) else {
            return;
        };
        drives_on(asset, &target).first().map(|(index, _)| *index)
    };
    if let Some(index) = existing {
        if selected.0 != Some(index) {
            selected.0 = Some(index);
        }
        return;
    }

    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    if asset.variables.is_empty() {
        return;
    }
    upsert_drive(&mut asset, target, VariableId(0));
    selected.0 = Some(asset.drives.len() - 1);
    dirty_state.has_unsaved_changes = true;
}

/// `pub(crate)`: reused verbatim by `drives.rs`'s flat list. See the module
/// doc's note above `DriveVariableCombo` for why reusing this wholesale, and
/// letting the existing observers below handle the results, is safe -- they
/// dispatch on component type, not on which module spawned the row.
pub(crate) fn spawn_drive_row(
    parent: &mut ChildSpawnerCommands,
    index: usize,
    drive: &Drive,
    variables: &[VariableDecl],
) {
    parent
        .spawn((
            DriveRow,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(6.0),
                width: percent(100),
                padding: UiRect::bottom(px(10.0)),
                border: UiRect::bottom(px(1.0)),
                ..default()
            },
            BorderColor::all(BORDER_COLOR),
        ))
        .with_children(|row| {
            row.spawn(fields_row()).with_children(|line| {
                let var_options: Vec<ComboBoxOptionData> = variables
                    .iter()
                    .map(|v| ComboBoxOptionData::new(v.name.clone()))
                    .collect();
                let selected = (drive.variable.0 as usize).min(var_options.len().saturating_sub(1));
                let line_target = line.target_entity();
                line.commands()
                    .spawn_scene(combobox_with_selected(var_options, selected))
                    .insert(DriveVariableCombo(index))
                    .insert(ChildOf(line_target));

                line.commands()
                    .spawn_scene(icon_button(
                        IconButtonProps::new(ICON_CLOSE).variant(ButtonVariant::Ghost),
                    ))
                    .insert(DriveDeleteButton(index))
                    .insert(ChildOf(line_target));
            });

            row.spawn(fields_row()).with_children(|line| {
                let mut curve_props = CurveEditProps::new().with_label("Curve");
                curve_props.curve = Some(drive.curve.clone());
                let line_target = line.target_entity();
                line.commands()
                    .spawn_scene(curve_edit(curve_props))
                    .insert(DriveCurveTarget(index))
                    .insert(ChildOf(line_target));
            });

            row.spawn(fields_row()).with_children(|line| {
                let line_target = line.target_entity();
                line.commands()
                    .spawn_scene(text_edit(
                        TextEditProps::default()
                            .with_label("Output min")
                            .numeric_f32()
                            .with_default_value(format_f32(drive.output.min)),
                    ))
                    .insert(DriveOutputField {
                        index,
                        bound: OutputBound::Min,
                    })
                    .insert(ChildOf(line_target));

                line.commands()
                    .spawn_scene(text_edit(
                        TextEditProps::default()
                            .with_label("Output max")
                            .numeric_f32()
                            .with_default_value(format_f32(drive.output.max)),
                    ))
                    .insert(DriveOutputField {
                        index,
                        bound: OutputBound::Max,
                    })
                    .insert(ChildOf(line_target));
            });

            row.spawn(fields_row()).with_children(|line| {
                // "Replace" is qualified at the point of CHOICE, because the
                // bare word promises something no op can deliver: these three
                // fold drives onto each other, and the result is applied to
                // the emitter's authored value downstream by multiplication
                // (see `asset::drive::DriveOp`). An author reading plain
                // "Replace" reasonably expects the authored value to be
                // overridden, wires one up, and watches it get multiplied
                // anyway. The label is the only place that misunderstanding
                // can be headed off before it happens.
                let op_options = vec![
                    ComboBoxOptionData::new("Replace earlier drives"),
                    ComboBoxOptionData::new("Multiply"),
                    ComboBoxOptionData::new("Add"),
                ];
                let selected = match drive.op {
                    DriveOp::Replace => 0,
                    DriveOp::Multiply => 1,
                    DriveOp::Add => 2,
                };
                let line_target = line.target_entity();
                line.commands()
                    .spawn_scene(combobox_with_selected(op_options, selected))
                    .insert(DriveOpCombo(index))
                    .insert(ChildOf(line_target));

                line.commands()
                    .spawn_scene(checkbox(CheckboxProps::new("Mute").checked(drive.muted)))
                    .insert(DriveMuteCheckbox(index))
                    .insert(ChildOf(line_target));
            });
        });
}

// --- Row commits ----------------------------------------------------------

pub(crate) fn handle_drive_variable_change(
    trigger: On<ComboBoxChangeEvent>,
    combos: Query<&DriveVariableCombo>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(combo) = combos.get(trigger.entity) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    let Some(drive) = asset.drives.get_mut(combo.0) else {
        return;
    };
    let new_id = VariableId(trigger.selected as u16);
    if drive.variable != new_id {
        drive.variable = new_id;
        dirty_state.has_unsaved_changes = true;
    }
}

pub(crate) fn handle_drive_op_change(
    trigger: On<ComboBoxChangeEvent>,
    combos: Query<&DriveOpCombo>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(combo) = combos.get(trigger.entity) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    let Some(drive) = asset.drives.get_mut(combo.0) else {
        return;
    };
    let new_op = match trigger.selected {
        0 => DriveOp::Replace,
        2 => DriveOp::Add,
        _ => DriveOp::Multiply,
    };
    if drive.op != new_op {
        drive.op = new_op;
        dirty_state.has_unsaved_changes = true;
    }
}

pub(crate) fn handle_drive_mute_commit(
    trigger: On<CheckboxCommitEvent>,
    checkboxes: Query<&DriveMuteCheckbox>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(cb) = checkboxes.get(trigger.entity) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    let Some(drive) = asset.drives.get_mut(cb.0) else {
        return;
    };
    if drive.muted != trigger.checked {
        drive.muted = trigger.checked;
        dirty_state.has_unsaved_changes = true;
    }
}

/// Deleting shifts every later drive down one, so the dock's selection --
/// an index like every other drive address here -- has to move with it or
/// the editor pane below silently becomes a different drive's. See
/// `drives::selection_after_delete` for which way each case lands.
pub(crate) fn handle_drive_delete_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&DriveDeleteButton>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut selected: ResMut<SelectedDrive>,
) {
    let Ok(delete_button) = buttons.get(trigger.entity) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    if delete_button.0 >= asset.drives.len() {
        return;
    }
    asset.drives.remove(delete_button.0);
    selected.0 = selection_after_delete(selected.0, delete_button.0);
    dirty_state.has_unsaved_changes = true;
}

pub(crate) fn handle_drive_curve_commit(
    trigger: On<CurveEditCommitEvent>,
    targets: Query<&DriveCurveTarget>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(target) = targets.get(trigger.entity) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    let Some(drive) = asset.drives.get_mut(target.0) else {
        return;
    };
    drive.curve = trigger.curve.clone();
    dirty_state.has_unsaved_changes = true;
}

pub(crate) fn handle_drive_output_commit(
    trigger: On<TextEditCommitEvent>,
    fields: Query<&DriveOutputField>,
    parents: Query<&ChildOf>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Some((_, field)) = find_ancestor(trigger.entity, &fields, &parents) else {
        return;
    };
    let Ok(value) = trigger.text.trim().parse::<f32>() else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    let Some(drive) = asset.drives.get_mut(field.index) else {
        return;
    };

    let changed = match field.bound {
        OutputBound::Min => {
            if (drive.output.min - value).abs() > f32::EPSILON {
                drive.output.min = value;
                true
            } else {
                false
            }
        }
        OutputBound::Max => {
            if (drive.output.max - value).abs() > f32::EPSILON {
                drive.output.max = value;
                true
            } else {
                false
            }
        }
    };

    if changed {
        dirty_state.has_unsaved_changes = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy_sprinkles::drives::resolve_drives;

    fn asset_with_one_variable_one_emitter() -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default()],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        a.variables = vec![VariableDecl {
            name: "temperature".into(),
            ..Default::default()
        }];
        a
    }

    #[test]
    fn adding_a_drive_for_a_field_that_has_none_appends_one() {
        let mut asset = asset_with_one_variable_one_emitter();
        upsert_drive(
            &mut asset,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SizeMul,
            },
            VariableId(0),
        );
        assert_eq!(asset.drives.len(), 1);
        assert_eq!(asset.drives[0].variable, VariableId(0));
    }

    #[test]
    fn the_button_reports_how_many_drives_target_a_field() {
        let mut asset = asset_with_one_variable_one_emitter();
        let target = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::SizeMul,
        };
        assert_eq!(drives_on(&asset, &target).len(), 0);
        upsert_drive(&mut asset, target.clone(), VariableId(0));
        upsert_drive(&mut asset, target.clone(), VariableId(0));
        assert_eq!(
            drives_on(&asset, &target).len(),
            2,
            "several drives on one target are legal"
        );
    }

    #[test]
    fn a_new_drive_defaults_to_a_zero_to_one_fader_on_multiply() {
        // Replaces the old `..._defaults_to_an_identity_curve_and_multiply`,
        // which pinned `1.0..1.0` -- the value that made a fresh drive
        // incapable of responding to its variable at all. See the module
        // doc for why that guarantee was traded away.
        let mut asset = asset_with_one_variable_one_emitter();
        upsert_drive(
            &mut asset,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SizeMul,
            },
            VariableId(0),
        );
        let d = &asset.drives[0];
        assert_eq!(d.op, DriveOp::Multiply);
        assert_eq!(d.output, ParticleRange { min: 0.0, max: 1.0 });
    }

    #[test]
    fn resolving_a_freshly_added_drive_tracks_its_variable_end_to_end() {
        // The property verified through the real resolver a host would run,
        // at BOTH ends of the declared range. One end alone would pass for
        // the old constant-curve default too: that resolved to 1.0
        // everywhere, which is indistinguishable from this at the top of
        // the range and nothing like it at the bottom.
        let mut asset = asset_with_one_variable_one_emitter();
        let target = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::SizeMul,
        };
        upsert_drive(&mut asset, target, VariableId(0));
        let slot = EmitterProp::SizeMul
            .slot()
            .expect("SizeMul is a Render prop");

        // `temperature` is declared over the default range, 0.0..=1.0.
        let top = resolve_drives(&[1.0], &asset);
        assert_eq!(
            top.emitters[0].render[slot],
            Some(1.0),
            "at the top of the range the authored value is passed through"
        );

        let bottom = resolve_drives(&[0.0], &asset);
        assert_eq!(
            bottom.emitters[0].render[slot],
            Some(0.0),
            "and at the bottom it is faded out -- a wire that MOVES, which \
             is the whole point of the new default"
        );
    }

    #[test]
    fn a_drive_naming_a_variable_no_longer_declared_is_out_of_range_by_construction() {
        // upsert_drive never fabricates a variable id out of thin air -- it
        // only ever writes the id it was given. This pins that upsert_drive
        // itself does no validation (validate_drives is the load-time gate,
        // exercised in `asset::drive`'s own tests), by constructing a drive
        // against an id one past the only declared variable and confirming
        // it lands exactly as asked.
        let mut asset = asset_with_one_variable_one_emitter();
        upsert_drive(
            &mut asset,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SizeMul,
            },
            VariableId(1),
        );
        assert_eq!(asset.drives[0].variable, VariableId(1));
    }

    #[test]
    fn stage_label_names_every_stage_in_words_an_author_asks() {
        assert!(stage_label(EmitterProp::SpawnSize).starts_with("Spawn"));
        assert!(stage_label(EmitterProp::Gravity).starts_with("Sim"));
        assert!(stage_label(EmitterProp::SizeMul).starts_with("Render"));
    }

    /// A minimal App carrying the REAL commit observer, not a hand-rolled
    /// stand-in for it -- same reasoning as
    /// `inspector::variable::tests::test_app`. Pins the property the brief
    /// calls out by name: "authoring a drive is a real edit and should dirty
    /// the project."
    fn test_app_with_one_drive() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<DirtyState>();
        app.add_observer(handle_drive_mute_commit);

        let mut asset = asset_with_one_variable_one_emitter();
        upsert_drive(
            &mut asset,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SizeMul,
            },
            VariableId(0),
        );

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset)
        };

        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: None,
        });

        let checkbox_entity = app.world_mut().spawn(DriveMuteCheckbox(0)).id();
        (app, checkbox_entity)
    }

    #[test]
    fn muting_a_drive_dirties_the_project() {
        let (mut app, checkbox_entity) = test_app_with_one_drive();

        app.world_mut().trigger(CheckboxCommitEvent {
            entity: checkbox_entity,
            checked: true,
        });

        assert!(
            app.world().resource::<DirtyState>().has_unsaved_changes,
            "authoring a drive is a real edit and must dirty the project"
        );

        let asset = app
            .world()
            .resource::<Assets<ParticlesAsset>>()
            .get(
                app.world()
                    .resource::<EditorState>()
                    .current_project
                    .as_ref()
                    .unwrap(),
            )
            .unwrap();
        assert!(asset.drives[0].muted);
    }

    #[test]
    fn re_committing_the_same_mute_state_does_not_spuriously_dirty() {
        // The read-compare-write half of the same property: a checkbox
        // commit that changes nothing must not still flip the dirty flag,
        // or every popover repaint would look like an edit.
        let (mut app, checkbox_entity) = test_app_with_one_drive();

        app.world_mut().trigger(CheckboxCommitEvent {
            entity: checkbox_entity,
            checked: false,
        });

        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "committing the already-current value must not dirty the project"
        );
    }

    // --- The one output editor staying in step -----------------------
    //
    // There is one editing surface now, the Drives dock's editor pane, and
    // it holds no edited copy: `spawn_drive_row` reads `drive.output` out of
    // the asset every time it is rebuilt. What the pane CAN do is fail to
    // rebuild, and it watches exactly one signal for that,
    // `DirtyState::is_changed()` -- so a commit path that mutated a drive
    // without dirtying would leave the pane showing the old number
    // indefinitely. These pin the output commit's end of that contract
    // (delete and reorder are pinned in `drives.rs`).

    fn app_with_an_output_field(min: f32, max: f32) -> (App, Entity, Handle<ParticlesAsset>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<DirtyState>();
        app.add_observer(handle_drive_output_commit);

        let mut asset = asset_with_one_variable_one_emitter();
        upsert_drive(
            &mut asset,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SizeMul,
            },
            VariableId(0),
        );
        asset.drives[0].output = ParticleRange { min, max };

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset)
        };
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });

        let field = app
            .world_mut()
            .spawn(DriveOutputField {
                index: 0,
                bound: OutputBound::Min,
            })
            .id();
        (app, field, handle)
    }

    #[test]
    fn an_output_edit_dirties_so_the_other_surface_rebuilds() {
        // One row's commit is the only thing that tells the OTHER surface
        // its copy of this number is out of date. Drop the dirty flip and
        // the popover and the Drives list disagree until something else
        // happens to dirty the project.
        let (mut app, field, handle) = app_with_an_output_field(1.0, 1.0);

        app.world_mut().trigger(TextEditCommitEvent {
            entity: field,
            text: "0.25".into(),
        });

        let asset = app
            .world()
            .resource::<Assets<ParticlesAsset>>()
            .get(&handle)
            .unwrap();
        assert_eq!(asset.drives[0].output.min, 0.25, "the asset is the truth");
        assert!(
            app.world().resource::<DirtyState>().has_unsaved_changes,
            "without this flip neither row list rebuilds and the two \
             surfaces show different numbers for the same drive"
        );
    }

    #[test]
    fn re_committing_the_same_output_value_does_not_spuriously_dirty() {
        // The read-compare-write half, mirroring the mute pair above: a
        // repaint of one surface must not read as an edit to the other.
        let (mut app, field, _) = app_with_an_output_field(1.0, 1.0);

        app.world_mut().trigger(TextEditCommitEvent {
            entity: field,
            text: "1.0".into(),
        });

        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "committing the already-current value must not dirty the project"
        );
    }

    // --- The field button as a pointer at the dock ----------------------
    //
    // The button stopped being an editor and became a selector. These pin
    // the three answers it can give: open the existing drive, create one
    // and open it, or do nothing at all because there is no variable to
    // wire it to.

    fn app_with_a_trigger(asset: ParticlesAsset) -> (App, Entity, Handle<ParticlesAsset>) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<DirtyState>();
        app.init_resource::<SelectedDrive>();
        app.insert_resource(InspectedEmitterTracker {
            current_index: Some(0),
        });
        app.init_resource::<InspectedLightTracker>();
        app.add_observer(handle_drive_trigger_click);

        let handle = {
            let mut assets = app.world_mut().resource_mut::<Assets<ParticlesAsset>>();
            assets.add(asset)
        };
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });

        let button_entity = app
            .world_mut()
            .spawn(DriveButtonProp(DrivableProp::Emitter(EmitterProp::SizeMul)))
            .id();
        let trigger = app
            .world_mut()
            .spawn(DriveButtonTrigger(button_entity))
            .id();
        (app, trigger, handle)
    }

    #[test]
    fn clicking_a_fields_button_opens_the_first_drive_on_that_field() {
        // Two drives on the target, and an unrelated one before them, so
        // "the first drive on this field" and "the first drive in the file"
        // are different indices.
        let mut asset = asset_with_one_variable_one_emitter();
        upsert_drive(
            &mut asset,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::Alpha,
            },
            VariableId(0),
        );
        let target = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::SizeMul,
        };
        upsert_drive(&mut asset, target.clone(), VariableId(0));
        upsert_drive(&mut asset, target, VariableId(0));

        let (mut app, trigger, _) = app_with_a_trigger(asset);
        app.world_mut().trigger(ButtonClickEvent { entity: trigger });

        assert_eq!(app.world().resource::<SelectedDrive>().0, Some(1));
        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "opening an existing drive is not an edit"
        );
    }

    #[test]
    fn clicking_a_field_with_no_drive_creates_one_and_opens_it() {
        // There is no popover left to hold a second "Add drive" step, and a
        // click that selected nothing would read as a dead button.
        let (mut app, trigger, handle) =
            app_with_a_trigger(asset_with_one_variable_one_emitter());
        app.world_mut().trigger(ButtonClickEvent { entity: trigger });

        assert_eq!(app.world().resource::<SelectedDrive>().0, Some(0));
        assert!(app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app
            .world()
            .resource::<Assets<ParticlesAsset>>()
            .get(&handle)
            .unwrap();
        assert_eq!(
            asset.drives[0].target,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SizeMul
            },
        );
        assert_eq!(
            asset.drives[0].output,
            ParticleRange { min: 0.0, max: 1.0 },
            "still the 0-100% fader `upsert_drive` has always appended"
        );
    }

    #[test]
    fn clicking_a_field_with_no_variables_declared_creates_nothing() {
        let mut asset = asset_with_one_variable_one_emitter();
        asset.variables.clear();

        let (mut app, trigger, handle) = app_with_a_trigger(asset);
        app.world_mut().trigger(ButtonClickEvent { entity: trigger });

        assert_eq!(app.world().resource::<SelectedDrive>().0, None);
        assert!(!app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app
            .world()
            .resource::<Assets<ParticlesAsset>>()
            .get(&handle)
            .unwrap();
        assert!(asset.drives.is_empty());
    }

    #[test]
    fn clicking_a_field_with_no_emitter_inspected_does_nothing() {
        // The sections these buttons live on are shared across every
        // emitter, so "nothing inspected" is a real transient state rather
        // than an impossible one.
        let (mut app, trigger, handle) =
            app_with_a_trigger(asset_with_one_variable_one_emitter());
        app.insert_resource(InspectedEmitterTracker {
            current_index: None,
        });
        app.world_mut().trigger(ButtonClickEvent { entity: trigger });

        assert_eq!(app.world().resource::<SelectedDrive>().0, None);
        let asset = app
            .world()
            .resource::<Assets<ParticlesAsset>>()
            .get(&handle)
            .unwrap();
        assert!(asset.drives.is_empty());
    }
}
