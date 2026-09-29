//! The drive affordance beside a drivable inspector field.
//!
//! A small button beside a numeric field shows how many [`Drive`]s target
//! that property (empty when zero) and, on click, opens a popover to author
//! them: a variable combobox, the existing `curve_edit` widget bound to
//! [`Drive::curve`], output min/max, an op combobox, a mute toggle and a
//! delete. This is the whole reason `curve_edit` needs no new machinery --
//! it is reused exactly as `inspector::colors` opens `gradient_edit`.
//!
//! **A fresh drive must be a no-op.** [`upsert_drive`] appends an identity
//! curve, `DriveOp::Multiply`, output pinned `1.0..1.0`: resolving it
//! multiplies the consumer's authored value by exactly one. Wiring a drive
//! must never itself change the look -- only shaping the curve or widening
//! the output range does. See the `tests` module for the pin.
//!
//! **The popover states the target's stage in words**, not as an enum name:
//! that is the first question an author asks of a knob ("does this change
//! what is already in the air?"), and `EmitterProp::stage()` already answers
//! it -- `stage_label` just spells it out.
//!
//! **Rows are addressed by their live index into `ParticlesAsset::drives`**,
//! not by identity, so every write here re-borrows the asset and re-checks
//! bounds rather than caching a `&mut Drive`. The popover's row list is
//! rebuilt wholesale on any dirty edit or emitter-selection change (mirroring
//! `variables::rebuild_variable_list`'s reasoning: simpler and safer than
//! tracking which specific mutation happened, at the cost of a full rebuild
//! on any edit -- fine for a list this small).
//!
//! **Target resolution is dynamic, not baked in at spawn time.** Every
//! section in this module (`scale_section`, `colors_section`, ...) is built
//! ONCE when the inspector panel is first added, then reused across every
//! emitter the author selects -- `sync.rs` re-reads the currently inspected
//! emitter's data each frame rather than the section being torn down and
//! rebuilt per emitter. A `drive_button` therefore stores only the
//! `EmitterProp` it drives; the emitter `index` half of its `DriveTarget` is
//! resolved from `InspectedEmitterTracker::current_index` wherever it is
//! needed (label sync, popover open, row rebuild, add-click), never cached
//! on the button itself.
//!
//! **Task 21 widened this to `LightProp`.** A button now stores a
//! [`DrivableProp`] (`Emitter(EmitterProp)` or `Light(LightProp)`) instead of
//! a bare `EmitterProp`; [`current_target`] resolves the right half against
//! the right tracker (`InspectedEmitterTracker` or `InspectedLightTracker`)
//! depending on which variant it holds. `LightProp` has no `Stage` (every
//! light drive is ECS-stage, per `asset::drive`'s doc), so the popover header
//! spells that out as fixed text rather than calling `stage_label`, which
//! stays `EmitterProp`-only. Only two `LightProp` variants get a button at
//! all -- `Intensity` and `Range`, the two that have an authored field on
//! `LightData` to hang one off; `Hue`/`Saturation`/`Value` have none (they
//! are drive-only targets, exactly like `EmitterProp::SpawnProbability`) and
//! stay reachable only through `drives.rs`'s target picker.

use bevy::prelude::*;
use bevy_sprinkles::asset::{
    Drive, DriveOp, DriveTarget, EmitterProp, LightProp, Stage, VariableDecl, VariableId,
};
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState};
use crate::ui::icons::{ICON_ADD, ICON_CLOSE, ICON_NODE_TREE};
use crate::ui::tokens::{BORDER_COLOR, TEXT_MUTED_COLOR};
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonVariant, IconButtonProps, button, icon_button,
};
use crate::ui::widgets::checkbox::{CheckboxCommitEvent, CheckboxProps, checkbox};
use crate::ui::widgets::combobox::{
    ComboBoxChangeEvent, ComboBoxOptionData, combobox_with_selected,
};
use crate::ui::widgets::curve_edit::{CurveEditCommitEvent, CurveEditProps, curve_edit};
use crate::ui::widgets::inspector_field::fields_row;
use crate::ui::widgets::popover::{
    PopoverHeaderProps, PopoverPlacement, PopoverProps, PopoverTracker, activate_trigger,
    deactivate_trigger, popover, popover_header,
};
use crate::ui::widgets::text_edit::{TextEditCommitEvent, TextEditProps, text_edit};
use crate::ui::widgets::utils::find_ancestor;

use crate::ui::components::drives::prop_label;

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

/// Appends a drive that is deliberately a NO-OP until shaped: identity curve,
/// Multiply, output pinned to 1..1. Adding a wire must never change the look.
pub fn upsert_drive(asset: &mut ParticlesAsset, target: DriveTarget, variable: VariableId) {
    asset.drives.push(Drive {
        variable,
        target,
        curve: CurveTexture::default(),
        output: ParticleRange { min: 1.0, max: 1.0 },
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
        PopoverTracker
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
        .add_observer(handle_drive_add_click)
        .add_systems(
            Update,
            (
                setup_drive_button,
                sync_drive_button_label,
                rebuild_drive_rows,
            )
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
struct DriveButtonPopoverMarker(Entity);

#[derive(Component)]
struct DriveRowsContainer(Entity);

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

#[derive(Component)]
struct DriveAddButton(Entity);

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

/// "Does this change what is already in the air" in words, for this
/// button's popover header. `Emitter` reuses [`stage_label`] (the only
/// family with a real `Stage`); `Light` targets are always ECS-stage, per
/// `asset::drive`'s own doc on `DriveTarget::Light`, so they get fixed text
/// instead of a fabricated `Stage` value. Mirrors `drives.rs`'s
/// `target_stage_text` wording exactly, so the same target reads the same
/// way whether it is opened from a field's own button or from the flat list.
fn drivable_stage_text(prop: DrivableProp) -> &'static str {
    match prop {
        DrivableProp::Emitter(prop) => stage_label(prop),
        DrivableProp::Light(_) => "ECS: applied to the light every frame",
    }
}

/// The property's own name, via `drives.rs`'s `prop_label` (reused, not
/// reimplemented -- see that function's doc).
fn drivable_prop_label(prop: DrivableProp) -> String {
    match prop {
        DrivableProp::Emitter(prop) => prop_label(prop),
        DrivableProp::Light(prop) => prop_label(prop),
    }
}

/// The popover header text: property name first, stage second. Task 22's
/// fix round -- `drivable_stage_text` alone cannot tell two buttons apart
/// once they share a `Stage` (`ScrollU`/`ScrollV` are both `Render`), which
/// is exactly the case that task introduced (two drive buttons on one
/// field, for the first time in this codebase). Naming the prop in the
/// header, rather than only labelling the two buttons, generalizes: any
/// FUTURE field with paired drive targets inherits the disambiguation for
/// free instead of needing its own button labels.
fn drivable_header_text(prop: DrivableProp) -> String {
    format!(
        "{} \u{2022} {}",
        drivable_prop_label(prop),
        drivable_stage_text(prop)
    )
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

// --- Popover open/close -------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn handle_drive_trigger_click(
    trigger: On<ButtonClickEvent>,
    mut commands: Commands,
    tracker: Res<InspectedEmitterTracker>,
    light_tracker: Res<InspectedLightTracker>,
    triggers: Query<&DriveButtonTrigger>,
    props: Query<&DriveButtonProp>,
    mut trackers: Query<&mut PopoverTracker>,
    existing_popovers: Query<(Entity, &DriveButtonPopoverMarker)>,
    mut button_styles: Query<(&mut BackgroundColor, &mut BorderColor, &mut ButtonVariant)>,
) {
    let Ok(drive_trigger) = triggers.get(trigger.entity) else {
        return;
    };
    let drive_button_entity = drive_trigger.0;
    let Ok(mut popover_tracker) = trackers.get_mut(drive_button_entity) else {
        return;
    };

    for (popover_entity, marker) in &existing_popovers {
        if marker.0 == drive_button_entity {
            commands.entity(popover_entity).try_despawn();
            popover_tracker.popover = None;
            deactivate_trigger(trigger.entity, &mut button_styles);
            return;
        }
    }

    let Ok(prop) = props.get(drive_button_entity) else {
        return;
    };

    // Nothing to drive without a currently inspected emitter/light -- the
    // button exists on a section that is shared across every emitter (or
    // light) the author selects (see the module doc), so this can
    // legitimately be transient between selections.
    if current_target(prop.0, &tracker, &light_tracker).is_none() {
        return;
    }

    activate_trigger(trigger.entity, &mut button_styles);

    let popover_entity = commands
        .spawn_scene(popover(
            PopoverProps::new(trigger.entity)
                .with_placement(PopoverPlacement::Right)
                .with_padding(0.0)
                .with_node(Node {
                    width: px(320.0),
                    ..default()
                }),
        ))
        .insert(DriveButtonPopoverMarker(drive_button_entity))
        .id();

    popover_tracker.open(popover_entity, trigger.entity);

    commands
        .spawn_scene(popover_header(PopoverHeaderProps::new(
            drivable_header_text(prop.0),
            popover_entity,
        )))
        .insert(ChildOf(popover_entity));

    commands.entity(popover_entity).with_children(|parent| {
        parent.spawn((
            DriveRowsContainer(drive_button_entity),
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(10.0),
                padding: UiRect::all(px(12.0)),
                width: percent(100),
                ..default()
            },
        ));
    });
}

// --- Row list: build/rebuild --------------------------------------------

/// Rebuilds every open drive popover's row list wholesale: on first open
/// (`Added<DriveRowsContainer>`), on any dirty edit (an add/delete/edit
/// anywhere -- including ones this module itself just committed), or on an
/// emitter-selection change (the target this button now means has changed).
fn rebuild_drive_rows(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    tracker: Res<InspectedEmitterTracker>,
    light_tracker: Res<InspectedLightTracker>,
    dirty_state: Res<DirtyState>,
    props: Query<&DriveButtonProp>,
    containers: Query<(Entity, &DriveRowsContainer)>,
    new_containers: Query<Entity, Added<DriveRowsContainer>>,
    children_query: Query<&Children>,
) {
    let should_rebuild = !new_containers.is_empty()
        || dirty_state.is_changed()
        || tracker.is_changed()
        || light_tracker.is_changed();
    if !should_rebuild {
        return;
    }

    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(asset) = assets.get(handle) else {
        return;
    };

    for (container_entity, container) in &containers {
        let Ok(prop) = props.get(container.0) else {
            continue;
        };

        if let Ok(children) = children_query.get(container_entity) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }

        let Some(target) = current_target(prop.0, &tracker, &light_tracker) else {
            continue;
        };
        let rows = drives_on(asset, &target);
        let variables = asset.variables.clone();
        let variable_count = variables.len();
        let drive_button_entity = container.0;

        commands.entity(container_entity).with_children(|parent| {
            for (index, drive) in &rows {
                spawn_drive_row(parent, *index, drive, &variables);
            }
            if variable_count == 0 {
                parent.spawn((
                    Text::new("Declare a variable to drive this field."),
                    TextColor(TEXT_MUTED_COLOR.into()),
                ));
            } else {
                let add_target = parent.target_entity();
                parent
                    .commands()
                    .spawn_scene(button(
                        ButtonProps::new("Add drive")
                            .align_left()
                            .with_left_icon(ICON_ADD),
                    ))
                    .insert(DriveAddButton(drive_button_entity))
                    .insert(ChildOf(add_target));
            }
        });
    }
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

pub(crate) fn handle_drive_delete_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&DriveDeleteButton>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
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

fn handle_drive_add_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&DriveAddButton>,
    props: Query<&DriveButtonProp>,
    tracker: Res<InspectedEmitterTracker>,
    light_tracker: Res<InspectedLightTracker>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(add_button) = buttons.get(trigger.entity) else {
        return;
    };
    let Ok(prop) = props.get(add_button.0) else {
        return;
    };
    let Some(target) = current_target(prop.0, &tracker, &light_tracker) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    if asset.variables.is_empty() {
        return;
    }

    upsert_drive(&mut asset, target, VariableId(0));
    dirty_state.has_unsaved_changes = true;
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
    fn a_new_drive_defaults_to_an_identity_curve_and_multiply() {
        // Adding a drive must not change the look until the author shapes it.
        // A drive that immediately altered the effect would make "what did I
        // just do?" unanswerable.
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
        assert_eq!(d.output, ParticleRange { min: 1.0, max: 1.0 });
    }

    #[test]
    fn resolving_a_freshly_added_drive_leaves_the_render_target_at_the_identity_multiplier() {
        // The property above ("must not change the look") verified end to
        // end: run the real resolver a host would run, and check the
        // resolved render slot is exactly 1.0 -- multiplying the consumer's
        // authored value by one, not replacing or nudging it.
        let mut asset = asset_with_one_variable_one_emitter();
        let target = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::SizeMul,
        };
        upsert_drive(&mut asset, target, VariableId(0));

        let resolved = resolve_drives(&[0.5], &asset);
        let slot = EmitterProp::SizeMul.slot().expect("SizeMul is a Render prop");
        assert_eq!(resolved.emitters[0].render[slot], Some(1.0));
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

    /// Task 22's fix round: `ScrollU` and `ScrollV` share a `Stage` (both
    /// `Render`), so `drivable_stage_text` alone cannot tell their two
    /// popovers apart. Pins that the header text -- what an author actually
    /// reads when they click one of the two identical trigger icons -- does.
    #[test]
    fn the_popover_header_names_the_property_not_just_the_stage() {
        let u = drivable_header_text(DrivableProp::Emitter(EmitterProp::ScrollU));
        let v = drivable_header_text(DrivableProp::Emitter(EmitterProp::ScrollV));
        assert_ne!(u, v, "two props sharing a Stage must not share a header");
        assert!(u.to_lowercase().contains("scroll"));
        assert!(v.to_lowercase().contains("scroll"));
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
}
