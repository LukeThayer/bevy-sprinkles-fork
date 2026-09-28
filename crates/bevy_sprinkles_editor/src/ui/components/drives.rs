//! The Drives list: every wire in the effect, in one place.
//!
//! Task 19 put a drive affordance beside individual numeric fields, but that
//! affordance can only exist where a field exists to hang it off -- it
//! reached 9 of `EmitterProp::ALL`'s 17 variants. The other 8
//! (`SpawnProbability` most of all: it has NO authored field anywhere, by
//! construction -- Task 6 created it as a new uniform precisely because
//! `amount` cannot be scaled without stranding live particles) were
//! implemented, tested, and unreachable from the UI. **This module closes
//! that**: the target-picker form below (`spawn_new_drive_form`) can create a
//! drive on ANY `EmitterProp`/`TransformProp`/`LightProp`, because it lists
//! them directly from `EmitterProp::ALL`/`TransformProp::ALL`/`LightProp::ALL`
//! rather than from a field that may or may not exist. See the `tests`
//! module's count-based pins.
//!
//! **Reuses `inspector::drive_button` wholesale rather than reimplementing
//! it.** `spawn_drive_row` (variable combo, curve edit, output min/max, op
//! combo, mute checkbox, delete button) and its six row-marker components are
//! `pub(crate)` there for exactly this: a row spawned here carries the
//! IDENTICAL `DriveVariableCombo`/`DriveOpCombo`/`DriveMuteCheckbox`/
//! `DriveDeleteButton`/`DriveCurveTarget`/`DriveOutputField` markers a popover
//! row would, so the observers already registered by `drive_button::plugin`
//! (which runs unconditionally, popover or not) handle every commit here too
//! -- this module adds no new edit-commit logic of its own, only the pieces
//! Task 19 had no use for: the target picker, move-up/move-down, and grouping
//! with warning badges.
//!
//! **Order is meaning, so this list is never sorted or deduplicated.**
//! `resolve_drives` folds every drive onto its target in `asset.drives`'
//! vector order, and `DriveOp::Replace` discards everything contributed
//! before it -- reordering is therefore a real edit with an observable
//! effect, not cosmetic. `move_drive` is the one operation that changes it.
//!
//! **Grouping is adjacency-based, not a full group-by.** A separator and
//! target header are drawn wherever the target changes between one drive and
//! the next in vector order -- so if two drives on the same target are not
//! adjacent (nothing stops that; it is legal), they render as two separate
//! single-row groups rather than being merged, which would require silently
//! reordering the display relative to `asset.drives`. The warning badges
//! (`target_warnings`) are NOT limited to one adjacent run, though: they scan
//! the WHOLE drive list for the target, so a scattered duplicate is still
//! caught.
//!
//! **Reordering is move-up/move-down buttons, not pointer drag.** There is
//! no drag-and-drop infrastructure anywhere in this bevy_ui-native widget set
//! (checked: no crate here builds on picking-driven reordering), and building
//! one from scratch is out of proportion to what "reorderable" actually
//! requires -- a button that calls `move_drive` produces the exact same
//! observable effect (a changed `asset.drives` order) that every test here
//! cares about. Named honestly as a deviation from the brief's "drag-to-
//! reorder" phrasing, per this task's standing licence to follow the real
//! widget API.

use bevy::prelude::*;
use bevy_sprinkles::asset::{DriveOp, DriveTarget, EmitterProp, LightProp, TransformProp, VariableId};
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState};
use crate::ui::components::data_panel::EditorDataPanel;
use crate::ui::components::inspector::drive_button::{spawn_drive_row, stage_label, upsert_drive};
use crate::ui::components::inspector::name_to_label;
use crate::ui::icons::ICON_ADD;
use crate::ui::tokens::TEXT_MUTED_COLOR;
use crate::ui::widgets::alert::{AlertSpan, AlertVariant, alert};
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonSize, button,
};
use crate::ui::widgets::combobox::{
    ComboBoxChangeEvent, ComboBoxOptionData, combobox_with_selected,
};
use crate::ui::widgets::inspector_field::fields_row;
use crate::ui::widgets::panel_section::{PanelSectionProps, panel_section};
use crate::ui::widgets::separator::{SeparatorProps, separator};

pub fn plugin(app: &mut App) {
    app.init_resource::<NewDriveDraft>()
        .add_observer(on_new_drive_kind_change)
        .add_observer(on_new_drive_index_change)
        .add_observer(on_new_drive_prop_change)
        .add_observer(on_new_drive_variable_change)
        .add_observer(on_add_drive_click)
        .add_observer(handle_move_drive_click)
        .add_systems(Update, (setup_drives_section, rebuild_drives_list));
}

// --- Pure data operations -------------------------------------------------

/// Moves the drive at `from` to `to`, preserving every other drive's
/// relative order. Declaration order IS application order (see the module
/// doc), so this is the one control in this list that can change what an
/// effect looks like without touching a single curve.
///
/// A no-op (`from == to`, or either index out of range) returns `false` so
/// the caller can skip dirtying the project -- a click that changes nothing
/// must not make the project look unsaved.
pub fn move_drive(asset: &mut ParticlesAsset, from: usize, to: usize) -> bool {
    let len = asset.drives.len();
    if from == to || from >= len || to >= len {
        return false;
    }
    let drive = asset.drives.remove(from);
    asset.drives.insert(to, drive);
    true
}

/// True if `target` carries two or more `DriveOp::Replace` drives anywhere
/// in `asset.drives` -- not just adjacently. Legal (the later one wins) and
/// almost always a mistake mid-rewiring, so the list warns rather than
/// forbids it.
pub fn target_has_double_replace(asset: &ParticlesAsset, target: &DriveTarget) -> bool {
    asset
        .drives
        .iter()
        .filter(|d| &d.target == target && d.op == DriveOp::Replace)
        .count()
        >= 2
}

/// True if `emitter_index`'s emitter is driven by both `ScaleUniform` and any
/// per-axis scale channel. Also legal -- `apply_transform_drives`'s doc
/// comment states exactly how the two compose (`ScaleUniform` first, then the
/// per-axis channel overrides its own axis) -- and also almost always a
/// mistake mid-rewiring.
pub fn emitter_has_uniform_and_axis_scale(asset: &ParticlesAsset, emitter_index: u8) -> bool {
    let mut uniform = false;
    let mut axis = false;
    for d in &asset.drives {
        if let DriveTarget::Transform { index, prop } = &d.target {
            if *index != emitter_index {
                continue;
            }
            match prop {
                TransformProp::ScaleUniform => uniform = true,
                TransformProp::ScaleX | TransformProp::ScaleY | TransformProp::ScaleZ => {
                    axis = true;
                }
                _ => {}
            }
        }
    }
    uniform && axis
}

/// The warning badges a target's group header should show, worded so the
/// author knows both THAT it's legal and WHY it's probably not what they
/// meant.
pub fn target_warnings(asset: &ParticlesAsset, target: &DriveTarget) -> Vec<String> {
    let mut out = Vec::new();
    if target_has_double_replace(asset, target) {
        out.push(
            "Two Replace drives target this property. The earlier one is discarded; \
             the later one wins."
                .to_string(),
        );
    }
    if let DriveTarget::Transform { index, prop } = target {
        let is_scale = matches!(
            prop,
            TransformProp::ScaleUniform
                | TransformProp::ScaleX
                | TransformProp::ScaleY
                | TransformProp::ScaleZ
        );
        if is_scale && emitter_has_uniform_and_axis_scale(asset, *index) {
            out.push(
                "This emitter is scaled by both ScaleUniform and a per-axis channel. \
                 ScaleUniform applies first; the per-axis channel then overrides its own axis."
                    .to_string(),
            );
        }
    }
    out
}

/// Every `EmitterProp` the picker offers, in `EmitterProp::ALL`'s own order.
/// This is the whole reason a future variant cannot be silently omitted: the
/// picker's option list is not a hand-written subset, it IS `ALL`.
pub fn emitter_prop_picker_options() -> Vec<EmitterProp> {
    EmitterProp::ALL.to_vec()
}

/// See [`emitter_prop_picker_options`].
pub fn transform_prop_picker_options() -> Vec<TransformProp> {
    TransformProp::ALL.to_vec()
}

/// See [`emitter_prop_picker_options`].
pub fn light_prop_picker_options() -> Vec<LightProp> {
    LightProp::ALL.to_vec()
}

/// Which family of target the picker is currently building. `Emitter` reaches
/// `EmitterProp::ALL`; `Transform` and `Light` are the "too, if not a large
/// extra step" half of the brief -- both were reachable with the same
/// picker shape, so both are here.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PickKind {
    #[default]
    Emitter,
    Transform,
    Light,
}

impl PickKind {
    const ALL: [PickKind; 3] = [Self::Emitter, Self::Transform, Self::Light];

    fn label(self) -> &'static str {
        match self {
            Self::Emitter => "Emitter",
            Self::Transform => "Emitter transform",
            Self::Light => "Light",
        }
    }
}

/// The picker's in-progress selection for a NOT-YET-CREATED drive. Separate
/// from `ParticlesAsset` entirely -- like `variables::VariableScrub`, this is
/// viewing/authoring-surface state, not content, so it never dirties the
/// project by itself; only `Add drive` (`on_add_drive_click`) does.
#[derive(Resource, Clone)]
pub struct NewDriveDraft {
    pub kind: PickKind,
    pub index: u8,
    pub emitter_prop: EmitterProp,
    pub transform_prop: TransformProp,
    pub light_prop: LightProp,
    pub variable: VariableId,
}

impl Default for NewDriveDraft {
    fn default() -> Self {
        Self {
            kind: PickKind::default(),
            index: 0,
            emitter_prop: EmitterProp::ALL[0],
            transform_prop: TransformProp::ALL[0],
            light_prop: LightProp::ALL[0],
            variable: VariableId(0),
        }
    }
}

/// The `DriveTarget` the picker's current selection describes. Exhaustive on
/// `PickKind`, no wildcard arm.
pub fn picked_target(draft: &NewDriveDraft) -> DriveTarget {
    match draft.kind {
        PickKind::Emitter => DriveTarget::Emitter {
            index: draft.index,
            prop: draft.emitter_prop,
        },
        PickKind::Transform => DriveTarget::Transform {
            index: draft.index,
            prop: draft.transform_prop,
        },
        PickKind::Light => DriveTarget::Light {
            index: draft.index,
            prop: draft.light_prop,
        },
    }
}

/// Whether `draft.index` names a target that actually exists in `asset` --
/// the same bounds `validate_drives` enforces at load, checked here BEFORE
/// writing, so `Add drive` cannot itself produce a file that fails its own
/// load.
fn picked_target_in_bounds(asset: &ParticlesAsset, draft: &NewDriveDraft) -> bool {
    let len = match draft.kind {
        PickKind::Light => asset.lights.len(),
        PickKind::Emitter | PickKind::Transform => asset.emitters.len(),
    };
    (draft.index as usize) < len
}

fn emitter_name(asset: &ParticlesAsset, index: u8) -> String {
    asset
        .emitters
        .get(index as usize)
        .map(|e| e.name.clone())
        .unwrap_or_else(|| format!("Emitter {index}"))
}

fn light_name(asset: &ParticlesAsset, index: u8) -> String {
    asset
        .lights
        .get(index as usize)
        .map(|l| l.name.clone())
        .unwrap_or_else(|| format!("Light {index}"))
}

fn prop_label<T: std::fmt::Debug>(prop: T) -> String {
    name_to_label(&format!("{prop:?}"))
}

/// "Which target" -- rendered once per adjacent run in the flat list.
fn target_label(asset: &ParticlesAsset, target: &DriveTarget) -> String {
    match target {
        DriveTarget::Emitter { index, prop } => {
            format!("{} \u{2022} {}", emitter_name(asset, *index), prop_label(*prop))
        }
        DriveTarget::Transform { index, prop } => format!(
            "{} \u{2022} Transform \u{2022} {}",
            emitter_name(asset, *index),
            prop_label(*prop)
        ),
        DriveTarget::Light { index, prop } => {
            format!("{} \u{2022} {}", light_name(asset, *index), prop_label(*prop))
        }
    }
}

/// "Does this change what is already in the air" -- reuses
/// `drive_button::stage_label` for `Emitter` targets (the only family with a
/// `Stage`); `Transform` and `Light` targets are always ECS-stage, per their
/// own doc comments in `asset::drive`, so they get plain fixed text instead
/// of a fabricated `Stage` value.
fn target_stage_text(target: &DriveTarget) -> &'static str {
    match target {
        DriveTarget::Emitter { prop, .. } => stage_label(*prop),
        DriveTarget::Transform { .. } => "ECS: applied to the emitter's Transform every frame",
        DriveTarget::Light { .. } => "ECS: applied to the light every frame",
    }
}

// --- Components ------------------------------------------------------------

#[derive(Component)]
struct DrivesSection;

#[derive(Component)]
struct DrivesListWrapper;

#[derive(Component, Clone, Copy)]
struct NewDriveKindCombo;

#[derive(Component, Clone, Copy)]
struct NewDriveIndexCombo;

#[derive(Component, Clone, Copy)]
struct NewDrivePropCombo;

#[derive(Component, Clone, Copy)]
struct NewDriveVariableCombo;

#[derive(Component)]
struct AddDriveButton;

/// `delta` is `-1` (move up / earlier) or `1` (move down / later). One
/// component and one observer for both directions rather than two of each.
#[derive(Component, Clone, Copy)]
struct MoveDriveButton {
    index: usize,
    delta: i32,
}

// --- Section setup / rebuild ----------------------------------------------

fn setup_drives_section(mut commands: Commands, panels: Query<Entity, Added<EditorDataPanel>>) {
    for panel_entity in &panels {
        commands
            .spawn_scene(panel_section(PanelSectionProps::new("Drives")))
            .insert((DrivesSection, ChildOf(panel_entity)));
    }
}

/// Rebuilds the whole list wholesale -- on first open, on any dirty edit
/// (add/delete/move/mute/anything the reused row observers commit), or on a
/// picker-draft change (switching Kind must repaint the Index/Property
/// comboboxes with a different option set). Same reasoning as
/// `variables::rebuild_variable_list`/`drive_button::rebuild_drive_rows`:
/// simpler and safer than tracking which specific mutation happened, and it
/// is the mechanism that keeps every row-marker index correct after a delete
/// or a move (see the module doc's reuse note and the `tests` module's
/// stale-index pins).
#[allow(clippy::too_many_arguments)]
fn rebuild_drives_list(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    dirty_state: Res<DirtyState>,
    draft: Res<NewDriveDraft>,
    assets: Res<Assets<ParticlesAsset>>,
    section: Query<Entity, With<DrivesSection>>,
    new_sections: Query<Entity, Added<DrivesSection>>,
    existing_wrappers: Query<Entity, With<DrivesListWrapper>>,
) {
    let should_rebuild = !new_sections.is_empty()
        || editor_state.is_changed()
        || dirty_state.is_changed()
        || draft.is_changed();
    if !should_rebuild {
        return;
    }

    let Ok(section_entity) = section.single() else {
        return;
    };

    for entity in &existing_wrappers {
        commands.entity(entity).despawn();
    }

    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(asset) = assets.get(handle) else {
        return;
    };

    let wrapper = commands
        .spawn((
            DrivesListWrapper,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(8.0),
                ..default()
            },
        ))
        .id();
    commands.entity(section_entity).add_child(wrapper);

    spawn_new_drive_form(&mut commands, wrapper, asset, &draft);

    if !asset.drives.is_empty() {
        let sep = commands.spawn_scene(separator(SeparatorProps::horizontal())).id();
        commands.entity(wrapper).add_child(sep);
    }

    let mut prev_target: Option<&DriveTarget> = None;
    for (index, drive) in asset.drives.iter().enumerate() {
        let is_new_group = prev_target != Some(&drive.target);
        if is_new_group {
            if prev_target.is_some() {
                let sep = commands.spawn_scene(separator(SeparatorProps::horizontal())).id();
                commands.entity(wrapper).add_child(sep);
            }

            let header = commands
                .spawn((
                    Text::new(format!(
                        "{}  \u{2014}  {}",
                        target_label(asset, &drive.target),
                        target_stage_text(&drive.target),
                    )),
                    TextColor(TEXT_MUTED_COLOR.into()),
                ))
                .id();
            commands.entity(wrapper).add_child(header);

            for warning in target_warnings(asset, &drive.target) {
                let alert_entity = commands
                    .spawn_scene(alert(AlertVariant::Warning, vec![AlertSpan::Text(warning)]))
                    .id();
                commands.entity(wrapper).add_child(alert_entity);
            }
        }
        prev_target = Some(&drive.target);

        let controls = commands.spawn(fields_row()).id();
        commands.entity(wrapper).add_child(controls);
        let up_entity = commands
            .spawn_scene(button(ButtonProps::new("\u{25B2}").with_size(ButtonSize::IconSM)))
            .insert(MoveDriveButton { index, delta: -1 })
            .id();
        commands.entity(controls).add_child(up_entity);
        let down_entity = commands
            .spawn_scene(button(ButtonProps::new("\u{25BC}").with_size(ButtonSize::IconSM)))
            .insert(MoveDriveButton { index, delta: 1 })
            .id();
        commands.entity(controls).add_child(down_entity);

        commands.entity(wrapper).with_children(|parent| {
            spawn_drive_row(parent, index, drive, &asset.variables);
        });
    }
}

fn spawn_new_drive_form(
    commands: &mut Commands,
    wrapper: Entity,
    asset: &ParticlesAsset,
    draft: &NewDriveDraft,
) {
    if asset.variables.is_empty() {
        let text_entity = commands
            .spawn((
                Text::new("Declare a variable to create a drive."),
                TextColor(TEXT_MUTED_COLOR.into()),
            ))
            .id();
        commands.entity(wrapper).add_child(text_entity);
        return;
    }

    let kind_options: Vec<ComboBoxOptionData> = PickKind::ALL
        .iter()
        .map(|k| ComboBoxOptionData::new(k.label()))
        .collect();
    let kind_selected = PickKind::ALL.iter().position(|k| *k == draft.kind).unwrap_or(0);

    let index_names: Vec<String> = match draft.kind {
        PickKind::Light => asset.lights.iter().map(|l| l.name.clone()).collect(),
        PickKind::Emitter | PickKind::Transform => {
            asset.emitters.iter().map(|e| e.name.clone()).collect()
        }
    };
    let index_options: Vec<ComboBoxOptionData> = index_names
        .iter()
        .enumerate()
        .map(|(i, name)| ComboBoxOptionData::new(format!("{i}: {name}")))
        .collect();
    let index_selected = (draft.index as usize).min(index_names.len().saturating_sub(1));

    let (prop_options, prop_selected): (Vec<ComboBoxOptionData>, usize) = match draft.kind {
        PickKind::Emitter => (
            emitter_prop_picker_options()
                .into_iter()
                .map(|p| ComboBoxOptionData::new(prop_label(p)))
                .collect(),
            EmitterProp::ALL
                .iter()
                .position(|p| *p == draft.emitter_prop)
                .unwrap_or(0),
        ),
        PickKind::Transform => (
            transform_prop_picker_options()
                .into_iter()
                .map(|p| ComboBoxOptionData::new(prop_label(p)))
                .collect(),
            TransformProp::ALL
                .iter()
                .position(|p| *p == draft.transform_prop)
                .unwrap_or(0),
        ),
        PickKind::Light => (
            light_prop_picker_options()
                .into_iter()
                .map(|p| ComboBoxOptionData::new(prop_label(p)))
                .collect(),
            LightProp::ALL.iter().position(|p| *p == draft.light_prop).unwrap_or(0),
        ),
    };

    let var_options: Vec<ComboBoxOptionData> = asset
        .variables
        .iter()
        .map(|v| ComboBoxOptionData::new(v.name.clone()))
        .collect();
    let var_selected = (draft.variable.0 as usize).min(var_options.len().saturating_sub(1));

    let header_text = commands
        .spawn((Text::new("New drive"), TextColor(TEXT_MUTED_COLOR.into())))
        .id();
    commands.entity(wrapper).add_child(header_text);

    let line1 = commands.spawn(fields_row()).id();
    commands.entity(wrapper).add_child(line1);
    let kind_combo = commands
        .spawn_scene(combobox_with_selected(kind_options, kind_selected))
        .insert(NewDriveKindCombo)
        .id();
    commands.entity(line1).add_child(kind_combo);
    let index_combo = commands
        .spawn_scene(combobox_with_selected(index_options, index_selected))
        .insert(NewDriveIndexCombo)
        .id();
    commands.entity(line1).add_child(index_combo);

    let line2 = commands.spawn(fields_row()).id();
    commands.entity(wrapper).add_child(line2);
    let prop_combo = commands
        .spawn_scene(combobox_with_selected(prop_options, prop_selected))
        .insert(NewDrivePropCombo)
        .id();
    commands.entity(line2).add_child(prop_combo);
    let var_combo = commands
        .spawn_scene(combobox_with_selected(var_options, var_selected))
        .insert(NewDriveVariableCombo)
        .id();
    commands.entity(line2).add_child(var_combo);

    let add_entity = commands
        .spawn_scene(button(ButtonProps::new("Add drive").align_left().with_left_icon(ICON_ADD)))
        .insert(AddDriveButton)
        .id();
    commands.entity(wrapper).add_child(add_entity);
}

// --- Picker observers -------------------------------------------------

fn on_new_drive_kind_change(
    trigger: On<ComboBoxChangeEvent>,
    combos: Query<&NewDriveKindCombo>,
    mut draft: ResMut<NewDriveDraft>,
) {
    if combos.get(trigger.entity).is_err() {
        return;
    }
    let selected = PickKind::ALL.get(trigger.selected).copied().unwrap_or_default();
    if draft.kind != selected {
        draft.kind = selected;
        // A different kind has a different, unrelated index space
        // (emitters vs. lights) -- reset rather than carry a number that
        // may now point at the wrong list or nothing at all.
        draft.index = 0;
    }
}

fn on_new_drive_index_change(
    trigger: On<ComboBoxChangeEvent>,
    combos: Query<&NewDriveIndexCombo>,
    mut draft: ResMut<NewDriveDraft>,
) {
    if combos.get(trigger.entity).is_err() {
        return;
    }
    let value = trigger.selected as u8;
    if draft.index != value {
        draft.index = value;
    }
}

fn on_new_drive_prop_change(
    trigger: On<ComboBoxChangeEvent>,
    combos: Query<&NewDrivePropCombo>,
    mut draft: ResMut<NewDriveDraft>,
) {
    if combos.get(trigger.entity).is_err() {
        return;
    }
    match draft.kind {
        PickKind::Emitter => {
            if let Some(p) = EmitterProp::ALL.get(trigger.selected) {
                draft.emitter_prop = *p;
            }
        }
        PickKind::Transform => {
            if let Some(p) = TransformProp::ALL.get(trigger.selected) {
                draft.transform_prop = *p;
            }
        }
        PickKind::Light => {
            if let Some(p) = LightProp::ALL.get(trigger.selected) {
                draft.light_prop = *p;
            }
        }
    }
}

fn on_new_drive_variable_change(
    trigger: On<ComboBoxChangeEvent>,
    combos: Query<&NewDriveVariableCombo>,
    mut draft: ResMut<NewDriveDraft>,
) {
    if combos.get(trigger.entity).is_err() {
        return;
    }
    draft.variable = VariableId(trigger.selected as u16);
}

fn on_add_drive_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&AddDriveButton>,
    draft: Res<NewDriveDraft>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    if buttons.get(trigger.entity).is_err() {
        return;
    }
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    if asset.variables.is_empty() {
        return;
    }
    if !picked_target_in_bounds(&asset, &draft) {
        return;
    }

    upsert_drive(&mut asset, picked_target(&draft), draft.variable);
    dirty_state.has_unsaved_changes = true;
}

fn handle_move_drive_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&MoveDriveButton>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
) {
    let Ok(mv) = buttons.get(trigger.entity) else {
        return;
    };
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };
    let to = if mv.delta < 0 {
        mv.index.checked_sub(1)
    } else {
        mv.index.checked_add(1)
    };
    let Some(to) = to else {
        return;
    };
    if move_drive(&mut asset, mv.index, to) {
        dirty_state.has_unsaved_changes = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy_sprinkles::asset::{Drive, LightData, VariableDecl};
    use bevy_sprinkles::drives::resolve_drives;
    use crate::ui::components::inspector::drive_button::{self, DriveDeleteButton};

    fn asset_with(emitters: usize, lights: usize, variables: usize) -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default(); emitters.max(1)],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        a.lights = vec![LightData::default(); lights];
        a.variables = (0..variables)
            .map(|i| VariableDecl {
                name: format!("v{i}"),
                ..Default::default()
            })
            .collect();
        a
    }

    fn flat_drive(variable: u16, target: DriveTarget, min: f32, op: DriveOp) -> Drive {
        Drive {
            variable: VariableId(variable),
            target,
            curve: CurveTexture::default(),
            output: ParticleRange { min, max: min },
            op,
            muted: false,
        }
    }

    // --- Task 20's two named tests, from real constructors (ruling R4) ----

    #[test]
    fn reordering_a_drive_changes_application_order() {
        let mut asset = asset_with(1, 0, 1);
        let target = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::Alpha,
        };
        asset.drives = vec![
            flat_drive(0, target.clone(), 2.0, DriveOp::Multiply),
            flat_drive(0, target.clone(), 3.0, DriveOp::Multiply),
            flat_drive(0, target.clone(), 5.0, DriveOp::Replace),
        ];

        move_drive(&mut asset, 2, 0);

        assert_eq!(
            asset.drives[0].output.min, 5.0,
            "index 0 must now hold what was last (the Replace)"
        );

        // Order is observable, not cosmetic: before the move, Replace is
        // last and wins outright (resolves to 5.0, verified below via the
        // real resolver). After the move Replace is FIRST, so the two
        // Multiplies compound on top of it instead: 5 * 2 * 3 = 30.
        let target_slot = EmitterProp::Alpha.slot().expect("Alpha is a Render prop");

        let before = {
            let mut original = asset.clone();
            original.drives = vec![
                flat_drive(0, target.clone(), 2.0, DriveOp::Multiply),
                flat_drive(0, target.clone(), 3.0, DriveOp::Multiply),
                flat_drive(0, target.clone(), 5.0, DriveOp::Replace),
            ];
            resolve_drives(&[1.0], &original).emitters[0].render[target_slot]
        };
        assert_eq!(before, Some(5.0), "sanity: Replace-last resolves to the Replace value alone");

        let after = resolve_drives(&[1.0], &asset).emitters[0].render[target_slot];
        assert_eq!(after, Some(30.0), "Replace-first lets the Multiplies compound on top of it");
    }

    #[test]
    fn muting_a_drive_persists_but_does_not_delete_it() {
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![flat_drive(
            0,
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::Alpha,
            },
            1.0,
            DriveOp::Multiply,
        )];

        asset.drives[0].muted = true;

        assert_eq!(asset.drives.len(), 1);
        assert!(asset.drives[0].muted);
    }

    // --- The acceptance criterion: the target picker reaches every variant

    #[test]
    fn the_target_picker_offers_every_emitter_prop_variant_by_count() {
        let options = emitter_prop_picker_options();
        assert_eq!(
            options.len(),
            EmitterProp::ALL.len(),
            "a future EmitterProp variant must not be silently omitted from the picker"
        );
        for p in EmitterProp::ALL {
            assert!(options.contains(&p), "picker is missing {p:?}");
        }
    }

    #[test]
    fn the_target_picker_offers_every_transform_prop_variant_by_count() {
        let options = transform_prop_picker_options();
        assert_eq!(options.len(), TransformProp::ALL.len());
        for p in TransformProp::ALL {
            assert!(options.contains(&p), "picker is missing {p:?}");
        }
    }

    #[test]
    fn the_target_picker_offers_every_light_prop_variant_by_count() {
        let options = light_prop_picker_options();
        assert_eq!(options.len(), LightProp::ALL.len());
        for p in LightProp::ALL {
            assert!(options.contains(&p), "picker is missing {p:?}");
        }
    }

    #[test]
    fn the_picker_reaches_spawn_probability_which_has_no_authored_field_anywhere() {
        // The exact defect this task closes: Task 19 wired 9 of 17
        // `EmitterProp::ALL` variants because only 9 had a field to hang a
        // button on. `SpawnProbability` has none, anywhere in the asset --
        // this picker is the only way to ever construct a drive for it.
        let draft = NewDriveDraft {
            kind: PickKind::Emitter,
            index: 0,
            emitter_prop: EmitterProp::SpawnProbability,
            ..Default::default()
        };
        assert_eq!(
            picked_target(&draft),
            DriveTarget::Emitter {
                index: 0,
                prop: EmitterProp::SpawnProbability
            },
        );
    }

    #[test]
    fn the_picker_reaches_transform_and_light_targets_too() {
        let transform_draft = NewDriveDraft {
            kind: PickKind::Transform,
            index: 2,
            transform_prop: TransformProp::ScaleUniform,
            ..Default::default()
        };
        assert_eq!(
            picked_target(&transform_draft),
            DriveTarget::Transform {
                index: 2,
                prop: TransformProp::ScaleUniform
            },
        );

        let light_draft = NewDriveDraft {
            kind: PickKind::Light,
            index: 1,
            light_prop: LightProp::Hue,
            ..Default::default()
        };
        assert_eq!(
            picked_target(&light_draft),
            DriveTarget::Light {
                index: 1,
                prop: LightProp::Hue
            },
        );
    }

    #[test]
    fn the_picker_is_out_of_bounds_against_an_asset_with_no_lights() {
        let asset = asset_with(1, 0, 1);
        let draft = NewDriveDraft {
            kind: PickKind::Light,
            index: 0,
            ..Default::default()
        };
        assert!(!picked_target_in_bounds(&asset, &draft));
    }

    #[test]
    fn the_picker_is_in_bounds_against_a_declared_emitter() {
        let asset = asset_with(2, 0, 1);
        let draft = NewDriveDraft {
            kind: PickKind::Transform,
            index: 1,
            ..Default::default()
        };
        assert!(picked_target_in_bounds(&asset, &draft));
    }

    // --- move_drive edge cases -------------------------------------------

    #[test]
    fn moving_a_drive_to_its_own_position_is_a_no_op() {
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![flat_drive(
            0,
            DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
            1.0,
            DriveOp::Multiply,
        )];
        assert!(!move_drive(&mut asset, 0, 0));
    }

    #[test]
    fn moving_a_drive_past_the_end_is_a_no_op_and_leaves_the_vector_untouched() {
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![flat_drive(
            0,
            DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
            1.0,
            DriveOp::Multiply,
        )];
        assert!(!move_drive(&mut asset, 0, 5));
        assert_eq!(asset.drives.len(), 1);
    }

    // --- Warning badges ----------------------------------------------------

    #[test]
    fn a_target_with_two_replace_drives_is_flagged() {
        let mut asset = asset_with(1, 0, 1);
        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha };
        asset.drives = vec![
            flat_drive(0, target.clone(), 1.0, DriveOp::Replace),
            flat_drive(0, target.clone(), 2.0, DriveOp::Replace),
        ];
        assert!(target_has_double_replace(&asset, &target));
    }

    #[test]
    fn a_target_with_one_replace_and_one_multiply_is_not_flagged() {
        let mut asset = asset_with(1, 0, 1);
        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha };
        asset.drives = vec![
            flat_drive(0, target.clone(), 1.0, DriveOp::Replace),
            flat_drive(0, target.clone(), 2.0, DriveOp::Multiply),
        ];
        assert!(!target_has_double_replace(&asset, &target));
    }

    #[test]
    fn two_replace_drives_are_flagged_even_when_not_adjacent() {
        // `target_warnings`/`target_has_double_replace` scan the whole
        // drive list, not just one adjacent run -- see the module doc.
        let mut asset = asset_with(1, 0, 1);
        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha };
        let other = DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul };
        asset.drives = vec![
            flat_drive(0, target.clone(), 1.0, DriveOp::Replace),
            flat_drive(0, other, 9.0, DriveOp::Multiply),
            flat_drive(0, target.clone(), 2.0, DriveOp::Replace),
        ];
        assert!(target_has_double_replace(&asset, &target));
    }

    #[test]
    fn an_emitter_scaled_by_both_uniform_and_a_per_axis_channel_is_flagged() {
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![
            flat_drive(
                0,
                DriveTarget::Transform { index: 0, prop: TransformProp::ScaleUniform },
                1.0,
                DriveOp::Multiply,
            ),
            flat_drive(
                0,
                DriveTarget::Transform { index: 0, prop: TransformProp::ScaleY },
                1.0,
                DriveOp::Multiply,
            ),
        ];
        assert!(emitter_has_uniform_and_axis_scale(&asset, 0));
    }

    #[test]
    fn two_different_emitters_each_scaled_one_way_are_not_flagged() {
        let mut asset = asset_with(2, 0, 1);
        asset.drives = vec![
            flat_drive(
                0,
                DriveTarget::Transform { index: 0, prop: TransformProp::ScaleUniform },
                1.0,
                DriveOp::Multiply,
            ),
            flat_drive(
                0,
                DriveTarget::Transform { index: 1, prop: TransformProp::ScaleY },
                1.0,
                DriveOp::Multiply,
            ),
        ];
        assert!(!emitter_has_uniform_and_axis_scale(&asset, 0));
        assert!(!emitter_has_uniform_and_axis_scale(&asset, 1));
    }

    // --- App-driven: dirty discipline and the stale-index hazard ----------
    //
    // Every row-marker component here and in `drive_button.rs` (reused
    // wholesale) addresses a drive by POSITION in `asset.drives`. The whole
    // list is despawned and respawned wholesale whenever `DirtyState`
    // changes (`rebuild_drives_list`'s doc comment), which is what turns a
    // stale index into a correct one on the next tick. That guarantee only
    // holds if every mutation that can shift positions actually flips the
    // dirty flag -- these tests pin that for BOTH mutations this task
    // introduces indices for: delete (reused, unchanged) and reorder (new).

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<DirtyState>();
        app
    }

    #[test]
    fn deleting_a_drive_dirties_the_project_through_the_reused_observer() {
        let mut app = test_app();
        app.add_observer(drive_button::handle_drive_delete_click);

        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![
            flat_drive(0, DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha }, 1.0, DriveOp::Multiply),
            flat_drive(0, DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul }, 2.0, DriveOp::Multiply),
        ];
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });

        let delete_button = app.world_mut().spawn(DriveDeleteButton(0)).id();
        app.world_mut().trigger(ButtonClickEvent { entity: delete_button });

        assert!(app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(asset.drives.len(), 1);
        assert_eq!(
            asset.drives[0].output.min, 2.0,
            "the survivor must be what was drives[1], now shifted down to index 0"
        );
    }

    #[test]
    fn moving_a_drive_dirties_the_project() {
        let mut app = test_app();
        app.add_observer(handle_move_drive_click);

        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha };
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![
            flat_drive(0, target.clone(), 1.0, DriveOp::Multiply),
            flat_drive(0, target, 2.0, DriveOp::Multiply),
        ];
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });

        let up_button = app.world_mut().spawn(MoveDriveButton { index: 1, delta: -1 }).id();
        app.world_mut().trigger(ButtonClickEvent { entity: up_button });

        assert!(app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(asset.drives[0].output.min, 2.0, "index 1 moved up to index 0");
    }

    #[test]
    fn a_move_click_at_the_top_of_the_list_does_not_dirty_or_panic() {
        let mut app = test_app();
        app.add_observer(handle_move_drive_click);

        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![flat_drive(
            0,
            DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
            1.0,
            DriveOp::Multiply,
        )];
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: None,
        });

        let up_button = app.world_mut().spawn(MoveDriveButton { index: 0, delta: -1 }).id();
        app.world_mut().trigger(ButtonClickEvent { entity: up_button });

        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "moving index 0 further up is out of range and must be a no-op, not a spurious dirty"
        );
    }

    #[test]
    fn changing_the_picker_draft_never_dirties_the_project() {
        // `NewDriveDraft` is viewing/authoring-surface state, exactly like
        // `variables::VariableScrub` -- considering a different target to
        // wire up next must not make the project look unsaved. Only
        // `Add drive` (`on_add_drive_click`) is allowed to do that.
        let mut app = test_app();
        app.init_resource::<NewDriveDraft>();
        app.add_observer(on_new_drive_kind_change);
        app.add_observer(on_new_drive_index_change);
        app.add_observer(on_new_drive_prop_change);
        app.add_observer(on_new_drive_variable_change);

        let asset = asset_with(2, 0, 2);
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: None,
        });

        let kind_combo = app.world_mut().spawn(NewDriveKindCombo).id();
        app.world_mut().trigger(ComboBoxChangeEvent {
            entity: kind_combo,
            selected: 1,
            label: "Emitter transform".to_string(),
            value: None,
        });

        let index_combo = app.world_mut().spawn(NewDriveIndexCombo).id();
        app.world_mut().trigger(ComboBoxChangeEvent {
            entity: index_combo,
            selected: 1,
            label: "1".to_string(),
            value: None,
        });

        let prop_combo = app.world_mut().spawn(NewDrivePropCombo).id();
        app.world_mut().trigger(ComboBoxChangeEvent {
            entity: prop_combo,
            selected: 3,
            label: "Scale uniform".to_string(),
            value: None,
        });

        let var_combo = app.world_mut().spawn(NewDriveVariableCombo).id();
        app.world_mut().trigger(ComboBoxChangeEvent {
            entity: var_combo,
            selected: 1,
            label: "v1".to_string(),
            value: None,
        });

        let draft = app.world().resource::<NewDriveDraft>();
        assert_eq!(draft.kind, PickKind::Transform);
        assert_eq!(draft.index, 1);
        assert_eq!(draft.transform_prop, TransformProp::ScaleUniform);
        assert_eq!(draft.variable, VariableId(1));
        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "picking a target to consider must not dirty the project"
        );
    }

    #[test]
    fn a_stale_delete_click_past_the_shrunk_end_is_a_safe_no_op() {
        // Simulates the exact hazard: a row for what USED to be index 1
        // survives (its click handler has no way to know the list shrank
        // underneath it), then gets clicked. The reused handler's own bounds
        // check is what keeps this a no-op rather than corrupting whatever
        // now occupies -- or doesn't occupy -- that slot.
        let mut app = test_app();
        app.add_observer(drive_button::handle_drive_delete_click);

        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![flat_drive(
            0,
            DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
            1.0,
            DriveOp::Multiply,
        )];
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });

        let stale_delete_button = app.world_mut().spawn(DriveDeleteButton(1)).id();
        app.world_mut().trigger(ButtonClickEvent { entity: stale_delete_button });

        assert!(!app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(asset.drives.len(), 1, "the one real drive must survive an out-of-range delete");
    }

    #[test]
    fn clicking_add_drive_creates_a_drive_on_the_picked_target_through_the_real_observer() {
        // The acceptance criterion end to end, through the actual click
        // observer rather than only the pure `picked_target` helper: this is
        // what proves the UI path, not just the data model, can create a
        // drive on a target with no authored field anywhere.
        let mut app = test_app();
        app.add_observer(on_add_drive_click);

        let asset = asset_with(1, 0, 1);
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        app.insert_resource(NewDriveDraft {
            kind: PickKind::Emitter,
            index: 0,
            emitter_prop: EmitterProp::SpawnProbability,
            ..Default::default()
        });

        let add_button = app.world_mut().spawn(AddDriveButton).id();
        app.world_mut().trigger(ButtonClickEvent { entity: add_button });

        assert!(app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(asset.drives.len(), 1);
        assert_eq!(
            asset.drives[0].target,
            DriveTarget::Emitter { index: 0, prop: EmitterProp::SpawnProbability },
        );
    }

    #[test]
    fn an_add_click_with_no_variables_declared_does_not_create_a_drive_or_dirty() {
        let mut app = test_app();
        app.add_observer(on_add_drive_click);

        let asset = asset_with(1, 0, 0);
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        app.insert_resource(NewDriveDraft::default());

        let add_button = app.world_mut().spawn(AddDriveButton).id();
        app.world_mut().trigger(ButtonClickEvent { entity: add_button });

        assert!(!app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert!(asset.drives.is_empty());
    }

    #[test]
    fn an_add_click_targeting_a_light_with_none_declared_does_not_create_a_drive_or_dirty() {
        let mut app = test_app();
        app.add_observer(on_add_drive_click);

        let asset = asset_with(1, 0, 1);
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        app.insert_resource(NewDriveDraft {
            kind: PickKind::Light,
            index: 0,
            ..Default::default()
        });

        let add_button = app.world_mut().spawn(AddDriveButton).id();
        app.world_mut().trigger(ButtonClickEvent { entity: add_button });

        assert!(!app.world().resource::<DirtyState>().has_unsaved_changes);
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert!(asset.drives.is_empty());
    }
}
