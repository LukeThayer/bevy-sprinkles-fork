//! The Drives dock: every wire in the effect, grouped by the variable that
//! moves it, with exactly one drive open for editing below the list.
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
//! rather than from a field that may or may not exist.
//!
//! # Why this is a dock and not a section
//!
//! The list used to live in the 224px-wide data panel, under four other
//! sections, with all six controls of every drive stacked open: four
//! `fields_row()`s per drive, so a twelve-drive effect was about 48 stacked
//! rows in a column narrower than the curve editor it had to hold. The
//! author's own words for it were "grows too long and unwieldy".
//!
//! So the list collapsed to ONE LINE PER DRIVE and moved to its own panel on
//! the right ([`drives_dock`], 360px). The six controls did not shrink or
//! move -- they are the same [`spawn_drive_row`] as before, shown for the ONE
//! drive selected in the list, in the pane beneath it.
//!
//! The list takes the dock's whole leftover height and scrolls inside it,
//! rather than stopping at a fixed cap and leaving the rest of a tall dock
//! empty. The editor pane keeps what its content needs, and the list is the
//! side that yields when the window is too short for both -- a selection has
//! to land somewhere the author can see, and there is no scroll-to-entity
//! facility here to rescue one that lands below the fold.
//!
//! # Order is meaning, and grouping hides it -- so the list prints it
//!
//! `resolve_drives` (`bevy_sprinkles::drives`) folds every drive onto its
//! target in `asset.drives`' VECTOR ORDER, and `fold`'s `DriveOp::Replace`
//! arm discards the accumulator outright. Order is therefore a real edit with
//! an observable effect, and grouping by variable puts two drives that fight
//! over one target in two different groups, where the thing that decides the
//! fight is invisible. Three answers, none of them optional:
//!
//! 1. **Every row prints its index into `asset.drives`** -- the apply-order
//!    number itself, not a position within its group, with a legend line
//!    above the list saying so. Two rows in different groups can still be
//!    compared at a glance because both carry the same kind of number.
//! 2. **Every row that shares a target says so** ([`row_note`]): "2 of 3"
//!    when another drive is on the same target, and "discarded" when a LATER
//!    unmuted `DriveOp::Replace` on that target throws this one away --
//!    which is not a heuristic but exactly what `fold` does. The editor pane
//!    spells the same fact out in a sentence ([`precedence_text`]).
//! 3. **Reordering is scoped to the target** ([`target_neighbour`]), not to
//!    the flat list. Two drives on DIFFERENT targets never meet in
//!    `resolve_drives` -- each target has its own accumulator -- so their
//!    relative order is unobservable, and a button that moved a drive one
//!    flat step past an unrelated drive would change the file and nothing
//!    else, in a list where the rows are not even adjacent. "Earlier"/"Later"
//!    therefore swap a drive with the nearest OTHER DRIVE ON ITS OWN TARGET,
//!    which is complete: every order that `resolve_drives` can distinguish is
//!    reachable, and every click visibly changes a printed number.
//!
//! # Where the duplicate-target warnings went
//!
//! [`target_warnings`] used to be painted under a group header, and the
//! groups were adjacency runs over the flat list -- so a target with two
//! non-adjacent drives got two headers and the warning rendered TWICE for
//! one problem. There are no target headers left, and the warnings are
//! painted once, in the editor pane, for the selected drive's target only
//! ([`warnings_for_selection`]): one selected drive, one target, one copy of
//! each warning, whether or not its drives are adjacent.
//!
//! # One editing surface, reachable from two places
//!
//! A field's own drive button no longer opens a popover of its own. It sets
//! [`SelectedDrive`] to the first drive on that field's target -- creating
//! one if the field has none -- so the dock's editor pane is the only place
//! a drive is ever edited. See `inspector::drive_button`'s module doc for the
//! other half.
//!
//! **Reuses `inspector::drive_button` wholesale rather than reimplementing
//! it.** `spawn_drive_row` (variable combo, curve edit, output min/max, op
//! combo, mute checkbox, delete button) and its six row-marker components are
//! `pub(crate)` there for exactly this: a row spawned here carries the
//! IDENTICAL `DriveVariableCombo`/`DriveOpCombo`/`DriveMuteCheckbox`/
//! `DriveDeleteButton`/`DriveCurveTarget`/`DriveOutputField` markers, so the
//! observers already registered by `drive_button::plugin` handle every commit
//! here too -- this module adds no new edit-commit logic of its own, only the
//! pieces that module has no use for: the target picker, the grouped list,
//! target-scoped reordering, and the selection.
//!
//! **Reordering is Earlier/Later buttons, not pointer drag.** There is no
//! drag-and-drop infrastructure anywhere in this bevy_ui-native widget set,
//! and building one from scratch is out of proportion to what "reorderable"
//! requires -- a button that calls `move_drive` produces the exact same
//! observable effect (a changed `asset.drives` order) that every test here
//! cares about.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy_sprinkles::asset::{DriveOp, DriveTarget, EmitterProp, LightProp, TransformProp, VariableId};
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState};
use crate::ui::components::inspector::drive_button::{spawn_drive_row, stage_label, upsert_drive};
use crate::ui::components::inspector::name_to_label;
use crate::ui::icons::ICON_ADD;
use crate::ui::tokens::{TEXT_DISPLAY_COLOR, TEXT_MUTED_COLOR, TEXT_SIZE_SM};
use crate::ui::widgets::alert::{AlertSpan, AlertVariant, alert};
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonVariant, button,
};
use crate::ui::widgets::combobox::{
    ComboBoxChangeEvent, ComboBoxOptionData, combobox_with_selected,
};
use crate::ui::widgets::inspector_field::fields_row;
use crate::ui::widgets::panel::{PanelDirection, PanelProps, panel};
use crate::ui::widgets::panel_section::{PanelSectionProps, panel_section};
use crate::ui::widgets::scroll::scrollbar;
use crate::ui::widgets::separator::{SeparatorProps, separator};

/// The dock's resting width. Wider than the data panel's 224 because the
/// editor pane holds the curve editor and a two-field output row, which is
/// what made those controls unreadable in the old home.
const DOCK_WIDTH: u32 = 360;
const DOCK_MIN_WIDTH: u32 = 280;
const DOCK_MAX_WIDTH: u32 = 560;

pub fn plugin(app: &mut App) {
    app.init_resource::<NewDriveDraft>()
        .init_resource::<SelectedDrive>()
        .add_observer(on_new_drive_kind_change)
        .add_observer(on_new_drive_index_change)
        .add_observer(on_new_drive_prop_change)
        .add_observer(on_new_drive_variable_change)
        .add_observer(on_add_drive_click)
        .add_observer(on_drive_row_click)
        .add_observer(handle_move_drive_click)
        .add_systems(Update, (setup_drives_dock, rebuild_drives_dock));
}

// --- Pure data operations -------------------------------------------------

/// Moves the drive at `from` to `to`, preserving every other drive's
/// relative order. Declaration order IS application order (see the module
/// doc), so this is the one control in this dock that can change what an
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

/// The flat index the drive at `index` swaps with when moved one step
/// earlier (`delta < 0`) or later among **the drives on its own target**,
/// or `None` when it is already the first or last of them.
///
/// Target-scoped rather than flat because `resolve_drives` gives every
/// target its own accumulator: two drives on different targets never meet,
/// so their relative order in `asset.drives` is not observable in the
/// rendered effect at all. A flat step would therefore be able to dirty the
/// project without changing anything an author can see -- and, in a list
/// grouped by variable, without moving the row it was clicked on.
///
/// Feeding the result straight to [`move_drive`] swaps the two, in both
/// directions: `remove(from)` shifts a later partner down one, so
/// `insert(to)` lands after it; an earlier partner's index is untouched by
/// the removal, so `insert(to)` lands before it.
pub fn target_neighbour(asset: &ParticlesAsset, index: usize, delta: i32) -> Option<usize> {
    let drive = asset.drives.get(index)?;
    if delta < 0 {
        asset.drives[..index]
            .iter()
            .rposition(|d| d.target == drive.target)
    } else {
        asset
            .drives
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, d)| d.target == drive.target)
            .map(|(i, _)| i)
    }
}

/// Every drive sharing `index`'s target, by flat index, in apply order --
/// `index` itself included, so the length is the contention count and the
/// position of `index` within it is its rank.
pub fn siblings_on_target(asset: &ParticlesAsset, index: usize) -> Vec<usize> {
    let Some(drive) = asset.drives.get(index) else {
        return Vec::new();
    };
    asset
        .drives
        .iter()
        .enumerate()
        .filter(|(_, d)| d.target == drive.target)
        .map(|(i, _)| i)
        .collect()
}

/// True when this drive's contribution cannot reach the effect because a
/// LATER drive on the same target replaces the accumulator it would have
/// landed in.
///
/// Read straight off `bevy_sprinkles::drives`: `fold`'s `(DriveOp::Replace,
/// _) => value` arm discards whatever earlier drives accumulated, so every
/// contribution strictly before the last `Replace` on a target is dead. The
/// muted check is not a nicety either -- `sample` returns `None` for a muted
/// drive and `resolve_drives` `continue`s before ever calling `fold`, so a
/// muted `Replace` discards nothing and the drives before it are still live.
pub fn discarded_by_a_later_replace(asset: &ParticlesAsset, index: usize) -> bool {
    let Some(drive) = asset.drives.get(index) else {
        return false;
    };
    asset
        .drives
        .iter()
        .skip(index + 1)
        .any(|d| d.target == drive.target && d.op == DriveOp::Replace && !d.muted)
}

/// True if `target` carries two or more `DriveOp::Replace` drives anywhere
/// in `asset.drives` -- not just adjacently. Legal (the later one wins) and
/// almost always a mistake mid-rewiring, so the dock warns rather than
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

/// The warning badges a target should show, worded so the author knows both
/// THAT it's legal and WHY it's probably not what they meant.
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

/// The warnings the editor pane paints, for the selected drive's target
/// alone.
///
/// This is the whole of the old adjacency double-render fix: the previous
/// list drew [`target_warnings`] under a group header, and its groups were
/// adjacency runs, so a target whose drives were scattered produced two
/// headers and printed each warning twice for one problem. One selection is
/// one target, so each warning is produced exactly once regardless of where
/// that target's drives sit in the vector.
pub fn warnings_for_selection(asset: &ParticlesAsset, selected: Option<usize>) -> Vec<String> {
    let Some(drive) = selected.and_then(|i| asset.drives.get(i)) else {
        return Vec::new();
    };
    target_warnings(asset, &drive.target)
}

/// Every `EmitterProp` the picker offers, in `EmitterProp::ALL`'s own order.
/// The picker's option list is not a hand-written subset, it IS `ALL` -- so
/// that half of "a future variant cannot be silently omitted" is structural.
/// The OTHER half, that `ALL` itself cannot silently fall behind the enum,
/// is a separate check in `bevy_sprinkles`:
/// `asset::drive::tests::emitter_prop_all_matches_the_reflected_enum_exactly`
/// compares `ALL` against the enum's own `#[derive(Reflect)]` variant
/// metadata, not against itself. (The count-based test below,
/// `the_target_picker_offers_every_emitter_prop_variant_by_count`, only
/// guards THIS function's own shape against drifting from `ALL` -- e.g. a
/// future filter narrowing it -- it is not a substitute for that other
/// check, which is why both exist.)
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

/// Which drive the dock's editor pane is showing, by flat index into
/// `ParticlesAsset::drives`.
///
/// Exactly one, which is the point: the old design had a drive's six
/// controls open in the list AND, potentially, in that field's own popover
/// at the same time, two surfaces for one drive kept in step only by both
/// watching `DirtyState`. There is one surface now, and both routes to a
/// drive -- clicking its row here, clicking its field's drive button over in
/// the inspector -- write this.
///
/// Viewing state, never content: setting it must not dirty the project.
///
/// An index, like every other drive address in the editor, so it shifts
/// under a delete or a reorder. Those two are the only mutations that can
/// shift it and both fix it up through [`selection_after_delete`] /
/// [`selection_after_move`]; anything that slips past them lands out of
/// range, which the rebuild treats as "nothing selected" rather than
/// painting the wrong drive.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SelectedDrive(pub Option<usize>);

/// Where a selection lands after the drive at `deleted` is removed.
///
/// The selected drive itself going away clears the selection rather than
/// sliding onto its neighbour: the editor pane below would otherwise silently
/// become a different drive's, under a header the author was not reading.
pub fn selection_after_delete(selected: Option<usize>, deleted: usize) -> Option<usize> {
    match selected {
        Some(i) if i == deleted => None,
        Some(i) if i > deleted => Some(i - 1),
        other => other,
    }
}

/// Where a selection lands after `move_drive(from, to)`.
///
/// Mirrors `Vec::remove` + `Vec::insert` exactly: the moved element lands on
/// `to`, and everything between the two indices shifts one place towards
/// `from`. The selection follows the DRIVE, not the slot -- a reorder is the
/// one edit whose whole purpose is to change which index a drive has, so
/// keeping the number would mean the editor pane jumped to a different drive
/// on every click of Earlier.
pub fn selection_after_move(selected: Option<usize>, from: usize, to: usize) -> Option<usize> {
    let i = selected?;
    if i == from {
        return Some(to);
    }
    if from < to && i > from && i <= to {
        return Some(i - 1);
    }
    if from > to && i >= to && i < from {
        return Some(i + 1);
    }
    Some(i)
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

/// `pub(crate)`: `inspector::drive_button` reuses this verbatim to name a
/// driven property, for the same reason this module needs it --
/// `EmitterProp`/`LightProp`/`TransformProp` have no hand-written display
/// name anywhere else, and two props can share a `Stage` (e.g.
/// `ScrollU`/`ScrollV`, both `Render`), so stage text alone cannot tell them
/// apart.
/// One variant is qualified rather than sentence-cased: `EmitterProp::Tint` is
/// a scalar BRIGHTNESS multiplier applied identically to R, G and B (one drive
/// resolves to one `f32`, and one render prop owns one uniform slot), so the
/// bare word "Tint" promises a colour control this cannot be. Qualified here
/// rather than at each call site, so the picker, the list and the editor
/// pane all say the same thing. `TransformProp` and `LightProp` have no
/// `Tint` variant, so matching on the debug string cannot catch an unrelated
/// one.
pub(crate) fn prop_label<T: std::fmt::Debug>(prop: T) -> String {
    let name = format!("{prop:?}");
    if name == "Tint" {
        return "Tint (brightness)".to_string();
    }
    name_to_label(&name)
}

/// "Which target" -- one line per drive in the collapsed list, and the tail
/// of the editor pane's header.
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

// --- The collapsed list's model ------------------------------------------

/// One line of the collapsed list.
pub struct DriveRowModel {
    /// Index into `ParticlesAsset::drives`, which IS the apply-order number
    /// the row prints. See the module doc.
    pub index: usize,
    /// "Body \u{2022} Emissive intensity".
    pub label: String,
    /// The right-hand note: contention, discard, mute. Empty when there is
    /// nothing to say. See [`row_note`].
    pub note: String,
}

/// One variable's worth of drives.
pub struct DriveGroupModel {
    /// The variable's declared name, or a placeholder naming the id when no
    /// declaration covers it.
    pub name: String,
    pub rows: Vec<DriveRowModel>,
}

/// The short note at the right-hand end of a collapsed row: what an author
/// needs in order to know that this row is in a fight, without opening it.
///
/// Empty when the drive is the only one on its target and is not muted --
/// the ordinary case, where a note would be noise.
pub fn row_note(asset: &ParticlesAsset, index: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    let siblings = siblings_on_target(asset, index);
    if siblings.len() > 1 {
        let rank = siblings.iter().position(|i| *i == index).unwrap_or(0) + 1;
        parts.push(format!("{rank} of {}", siblings.len()));
    }
    if discarded_by_a_later_replace(asset, index) {
        parts.push("discarded".to_string());
    }
    if asset.drives.get(index).is_some_and(|d| d.muted) {
        parts.push("muted".to_string());
    }
    parts.join(" \u{00b7} ")
}

/// The editor pane's sentence about where this drive sits in its target's
/// fold -- the long form of [`row_note`], with room to say why it matters.
pub fn precedence_text(asset: &ParticlesAsset, index: usize) -> String {
    let siblings = siblings_on_target(asset, index);
    if siblings.len() <= 1 {
        return format!("Apply order #{index} \u{2014} the only drive on this target.");
    }
    let rank = siblings.iter().position(|i| *i == index).unwrap_or(0) + 1;
    let head = format!(
        "Apply order #{index} \u{2014} {rank} of {} on this target, folded earliest first.",
        siblings.len()
    );
    if discarded_by_a_later_replace(asset, index) {
        return format!(
            "{head} A later Replace on this target discards everything before it, \
             including this drive."
        );
    }
    head
}

/// The editor pane's header: which variable moves this drive, and what it
/// moves.
///
/// Names the PROPERTY, not only its stage: two props can share a `Stage`
/// (`ScrollU` and `ScrollV` are both `Render`), so a header built from stage
/// text alone cannot tell two drives apart -- and the inspector has two drive
/// buttons on one field for exactly that pair.
pub fn editing_header_text(asset: &ParticlesAsset, index: usize) -> Option<String> {
    let drive = asset.drives.get(index)?;
    let variable = asset
        .variables
        .get(drive.variable.0 as usize)
        .map(|v| v.name.clone())
        .unwrap_or_else(|| format!("variable {}", drive.variable.0));
    Some(format!(
        "{variable} \u{2192} {}",
        target_label(asset, &drive.target)
    ))
}

/// Every drive, grouped by the variable that moves it, in the order the
/// dock paints them.
///
/// Groups run in VARIABLE DECLARATION order, not in first-appearance order,
/// so that reordering drives -- the one edit whose whole purpose is to move
/// indices around -- cannot also make the groups jump. A variable id no
/// declaration covers (which `validate_drives` rejects at load, but the
/// editor can hold transiently between a delete and its renumber) sorts
/// after every declared one rather than being dropped: a drive the author
/// cannot see is a drive they cannot fix.
///
/// Rows within a group stay in `asset.drives` order, which is apply order.
pub fn group_drives_by_variable(asset: &ParticlesAsset) -> Vec<DriveGroupModel> {
    let mut ids: Vec<VariableId> = Vec::new();
    for drive in &asset.drives {
        if !ids.contains(&drive.variable) {
            ids.push(drive.variable);
        }
    }
    ids.sort_by_key(|v| {
        let i = v.0 as usize;
        let declared = i < asset.variables.len();
        (!declared, i)
    });

    ids.into_iter()
        .map(|variable| {
            let name = asset
                .variables
                .get(variable.0 as usize)
                .map(|v| v.name.clone())
                .unwrap_or_else(|| format!("variable {} (undeclared)", variable.0));
            let rows = asset
                .drives
                .iter()
                .enumerate()
                .filter(|(_, d)| d.variable == variable)
                .map(|(index, drive)| DriveRowModel {
                    index,
                    label: target_label(asset, &drive.target),
                    note: row_note(asset, index),
                })
                .collect();
            DriveGroupModel { name, rows }
        })
        .collect()
}

// --- Components ------------------------------------------------------------

#[derive(Component, Default, Clone)]
pub struct EditorDrivesDock;

#[derive(Component)]
struct DrivesListSection;

#[derive(Component)]
struct DriveEditorSection;

#[derive(Component)]
struct DrivesListWrapper;

#[derive(Component)]
struct DriveEditorWrapper;

/// A clickable line in the collapsed list, addressed by flat drive index.
#[derive(Component, Clone, Copy)]
struct DriveListRow(usize);

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

/// `delta` is `-1` (earlier) or `1` (later), applied among the drives on
/// this drive's OWN target -- see [`target_neighbour`]. One component and
/// one observer for both directions rather than two of each.
#[derive(Component, Clone, Copy)]
struct MoveDriveButton {
    index: usize,
    delta: i32,
}

// --- Dock setup / rebuild -------------------------------------------------

/// The Drives dock, pinned to the right edge of the main row.
///
/// Its own panel rather than a fifth section of the 224px data panel: the
/// editor pane holds the curve editor, and the six controls of one drive
/// need roughly the inspector's width to be readable at all. See the module
/// doc for what the old home did to a twelve-drive effect.
pub fn drives_dock() -> impl Scene {
    bsn! {
        EditorDrivesDock
        panel(
            PanelProps::new(PanelDirection::Right)
                .with_width(DOCK_WIDTH)
                .with_min_width(DOCK_MIN_WIDTH)
                .with_max_width(DOCK_MAX_WIDTH),
        )
    }
}

fn setup_drives_dock(mut commands: Commands, docks: Query<Entity, Added<EditorDrivesDock>>) {
    for dock_entity in &docks {
        commands
            .entity(dock_entity)
            .with_child(scrollbar(dock_entity));

        // The list takes the dock's leftover height and scrolls inside it;
        // the editor pane below keeps what its content needs. The order
        // matters under compression: `fill`'s zero `min_height` is what
        // makes the list the section that yields on a short window, so the
        // editor pane a click selects into can never be pushed off the
        // bottom.
        commands
            .spawn_scene(panel_section(PanelSectionProps::new("Drives").fill()))
            .insert((DrivesListSection, ChildOf(dock_entity)));

        commands
            .spawn_scene(panel_section(PanelSectionProps::new("Editing")))
            .insert((DriveEditorSection, ChildOf(dock_entity)));
    }
}

/// Rebuilds both halves of the dock wholesale -- on first open, on any dirty
/// edit (add/delete/move/mute/anything the reused row observers commit), on a
/// picker-draft change (switching Kind must repaint the Index/Property
/// comboboxes with a different option set), and on a selection change.
///
/// Same reasoning as `variables::rebuild_variable_list`: simpler and safer
/// than tracking which specific mutation happened, and it is the mechanism
/// that keeps every row-marker index correct after a delete or a move (see
/// the `tests` module's stale-index pins). It is also why the row notes and
/// the editor pane's precedence sentence can be derived fresh from the asset
/// each time rather than bookkept -- the same "derived beats delta" rule the
/// drive resolver itself follows.
#[allow(clippy::too_many_arguments)]
fn rebuild_drives_dock(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    dirty_state: Res<DirtyState>,
    draft: Res<NewDriveDraft>,
    selected: Res<SelectedDrive>,
    assets: Res<Assets<ParticlesAsset>>,
    list_section: Query<Entity, With<DrivesListSection>>,
    editor_section: Query<Entity, With<DriveEditorSection>>,
    new_sections: Query<Entity, Added<DrivesListSection>>,
    existing_wrappers: Query<Entity, Or<(With<DrivesListWrapper>, With<DriveEditorWrapper>)>>,
) {
    let should_rebuild = !new_sections.is_empty()
        || editor_state.is_changed()
        || dirty_state.is_changed()
        || draft.is_changed()
        || selected.is_changed();
    if !should_rebuild {
        return;
    }

    let (Ok(list_entity), Ok(editor_entity)) = (list_section.single(), editor_section.single())
    else {
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

    spawn_list(&mut commands, list_entity, asset, &draft, selected.0);
    spawn_editor(&mut commands, editor_entity, asset, selected.0);
}

/// The collapsed list: a legend, one line per drive grouped by variable, a
/// separator, then the target picker.
fn spawn_list(
    commands: &mut Commands,
    section: Entity,
    asset: &ParticlesAsset,
    draft: &NewDriveDraft,
    selected: Option<usize>,
) {
    // Grows with its filling section, and like it may shrink to nothing:
    // the chain from the dock down to the scrolling list is only as tall as
    // its shortest link, so every link in it grows and every one carries a
    // zero `min_height` (`panel_section::section_fill_sizing`).
    let wrapper = commands
        .spawn((
            DrivesListWrapper,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(8.0),
                flex_grow: 1.0,
                min_height: px(0),
                ..default()
            },
        ))
        .id();
    commands.entity(section).add_child(wrapper);

    if !asset.drives.is_empty() {
        let legend = commands
            .spawn((
                Text::new(
                    "# is apply order. Drives on one target fold in that order, \
                     later over earlier.",
                ),
                TextFont {
                    font_size: FontSize::Px(TEXT_SIZE_SM),
                    ..default()
                },
                TextColor(TEXT_MUTED_COLOR.into()),
            ))
            .id();
        commands.entity(wrapper).add_child(legend);
    }

    // `Hovered` because `scroll::update_scrollbar` reveals a scrollbar only
    // while its container is hovered, and the container here is this list
    // rather than the dock.
    //
    // The last link in the growing chain, and the one that scrolls: it takes
    // whatever height the wrapper has left after the legend and the target
    // picker, and the drives overflow inside it rather than lengthening the
    // dock. It used to be capped at a flat 320px instead, which left the
    // dock's lower half empty on a tall window.
    let list = commands
        .spawn((
            Hovered::default(),
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(4.0),
                flex_grow: 1.0,
                min_height: px(0),
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .id();
    commands.entity(wrapper).add_child(list);
    let list_scrollbar = commands.spawn(scrollbar(list)).id();
    commands.entity(list).add_child(list_scrollbar);

    for group in group_drives_by_variable(asset) {
        let header = commands
            .spawn(Node {
                width: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            })
            .id();
        commands.entity(list).add_child(header);
        let name = commands
            .spawn((
                Text::new(group.name.clone()),
                TextColor(TEXT_DISPLAY_COLOR.into()),
            ))
            .id();
        let count = commands
            .spawn((
                Text::new(format!("({})", group.rows.len())),
                TextColor(TEXT_MUTED_COLOR.into()),
            ))
            .id();
        commands.entity(header).add_children(&[name, count]);

        for row in &group.rows {
            let indent = commands
                .spawn(Node {
                    width: percent(100),
                    padding: UiRect::left(px(10.0)),
                    ..default()
                })
                .id();
            commands.entity(list).add_child(indent);

            let variant = if selected == Some(row.index) {
                ButtonVariant::Active
            } else {
                ButtonVariant::Ghost
            };
            let mut props = ButtonProps::new(format!("{}  {}", row.index, row.label))
                .with_variant(variant)
                .align_left();
            if !row.note.is_empty() {
                props = props.with_subtitle(row.note.clone());
            }
            let row_entity = commands
                .spawn_scene(button(props))
                .insert(DriveListRow(row.index))
                .id();
            commands.entity(indent).add_child(row_entity);
        }
    }

    if !asset.drives.is_empty() {
        let sep = commands
            .spawn_scene(separator(SeparatorProps::horizontal()))
            .id();
        commands.entity(wrapper).add_child(sep);
    }

    spawn_new_drive_form(commands, wrapper, asset, draft);
}

/// The editor pane: header, precedence sentence, reorder buttons, the
/// target's warnings, then the one reused six-control drive row.
fn spawn_editor(
    commands: &mut Commands,
    section: Entity,
    asset: &ParticlesAsset,
    selected: Option<usize>,
) {
    let wrapper = commands
        .spawn((
            DriveEditorWrapper,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(8.0),
                ..default()
            },
        ))
        .id();
    commands.entity(section).add_child(wrapper);

    // An out-of-range selection is treated as no selection rather than
    // clamped: see `SelectedDrive`'s doc for why painting SOME drive would be
    // worse than painting none.
    let Some(index) = selected.filter(|i| *i < asset.drives.len()) else {
        let text = commands
            .spawn((
                Text::new("Select a drive above to edit it."),
                TextColor(TEXT_MUTED_COLOR.into()),
            ))
            .id();
        commands.entity(wrapper).add_child(text);
        return;
    };
    let drive = &asset.drives[index];

    if let Some(header) = editing_header_text(asset, index) {
        let header_entity = commands
            .spawn((Text::new(header), TextColor(TEXT_DISPLAY_COLOR.into())))
            .id();
        commands.entity(wrapper).add_child(header_entity);
    }

    let stage = commands
        .spawn((
            Text::new(target_stage_text(&drive.target)),
            TextFont {
                font_size: FontSize::Px(TEXT_SIZE_SM),
                ..default()
            },
            TextColor(TEXT_MUTED_COLOR.into()),
        ))
        .id();
    commands.entity(wrapper).add_child(stage);

    let precedence = commands
        .spawn((
            Text::new(precedence_text(asset, index)),
            TextFont {
                font_size: FontSize::Px(TEXT_SIZE_SM),
                ..default()
            },
            TextColor(TEXT_MUTED_COLOR.into()),
        ))
        .id();
    commands.entity(wrapper).add_child(precedence);

    let order_row = commands.spawn(fields_row()).id();
    commands.entity(wrapper).add_child(order_row);
    let earlier = commands
        .spawn_scene(button(
            ButtonProps::new("\u{25B2} Earlier"),
        ))
        .insert(MoveDriveButton { index, delta: -1 })
        .id();
    let later = commands
        .spawn_scene(button(
            ButtonProps::new("\u{25BC} Later"),
        ))
        .insert(MoveDriveButton { index, delta: 1 })
        .id();
    commands.entity(order_row).add_children(&[earlier, later]);

    for warning in warnings_for_selection(asset, Some(index)) {
        let alert_entity = commands
            .spawn_scene(alert(AlertVariant::Warning, vec![AlertSpan::Text(warning)]))
            .id();
        commands.entity(wrapper).add_child(alert_entity);
    }

    commands.entity(wrapper).with_children(|parent| {
        spawn_drive_row(parent, index, drive, &asset.variables);
    });
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

/// Clicking a line in the collapsed list opens it in the editor pane.
///
/// Selection is viewing state, so this must not dirty the project -- the
/// same rule `NewDriveDraft` follows, and the reason both are pinned by
/// their own tests.
fn on_drive_row_click(
    trigger: On<ButtonClickEvent>,
    rows: Query<&DriveListRow>,
    mut selected: ResMut<SelectedDrive>,
) {
    let Ok(row) = rows.get(trigger.entity) else {
        return;
    };
    if selected.0 != Some(row.0) {
        selected.0 = Some(row.0);
    }
}

fn on_add_drive_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&AddDriveButton>,
    draft: Res<NewDriveDraft>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut selected: ResMut<SelectedDrive>,
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
    // `upsert_drive` appends, so the new drive is the last one -- and it is
    // the one the author now wants open, since the pane below is the only
    // place its curve and output can be set.
    selected.0 = Some(asset.drives.len() - 1);
    dirty_state.has_unsaved_changes = true;
}

fn handle_move_drive_click(
    trigger: On<ButtonClickEvent>,
    buttons: Query<&MoveDriveButton>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut selected: ResMut<SelectedDrive>,
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
    let Some(to) = target_neighbour(&asset, mv.index, mv.delta) else {
        return;
    };
    if move_drive(&mut asset, mv.index, to) {
        selected.0 = selection_after_move(selected.0, mv.index, to);
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
        // The delete and reorder observers both keep `SelectedDrive` pointing
        // at the same DRIVE across an index shift, so both take it as a
        // `ResMut` -- an App without it would silently skip them.
        app.init_resource::<SelectedDrive>();
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

    /// Minor: the picker used to offer a bare "Tint", which reads as a colour
    /// control. It is one scalar on R/G/B and cannot shift hue.
    #[test]
    fn the_tint_prop_is_labelled_as_the_brightness_multiplier_it_is() {
        let label = prop_label(EmitterProp::Tint);
        assert_ne!(label, "Tint", "a bare `Tint` promises a colour control");
        assert!(
            label.to_lowercase().contains("bright"),
            "the label must say what it actually does: {label}"
        );
    }

    /// The qualifier must not have leaked into the generic path: every other
    /// prop still gets plain sentence case, which is what keeps `ScrollU` and
    /// `ScrollV` distinguishable.
    #[test]
    fn every_other_prop_label_is_still_plain_sentence_case() {
        assert_eq!(prop_label(EmitterProp::SizeMul), "Size mul");
        assert_eq!(prop_label(TransformProp::ScaleY), "Scale Y");
        assert_eq!(prop_label(LightProp::Intensity), "Intensity");
    }

    /// The target picker builds its option list from `EmitterProp::ALL`
    /// reflectively, so a new variant appears in the combobox with no edit
    /// here -- and with no check either. `name_to_label` runs the debug name
    /// through `to_sentence_case`, which is where a variant could come out
    /// blank or as an unreadable run-on; these three axis suffixes survive it
    /// only because "x"/"y"/"z" are in `UPPERCASE_ACRONYMS`.
    #[test]
    fn the_per_axis_props_label_legibly_in_the_picker() {
        assert_eq!(prop_label(EmitterProp::DirX), "Dir X");
        assert_eq!(prop_label(EmitterProp::DirY), "Dir Y");
        assert_eq!(prop_label(EmitterProp::DirZ), "Dir Z");
        assert_eq!(prop_label(EmitterProp::EmissionScaleX), "Emission scale X");
        assert_eq!(prop_label(EmitterProp::EmissionScaleY), "Emission scale Y");
        assert_eq!(prop_label(EmitterProp::EmissionScaleZ), "Emission scale Z");
    }

    // --- The order-legibility hazard -------------------------------------
    //
    // Grouping by variable puts two drives that fight over one target into
    // two different groups. `resolve_drives` still folds them in
    // `asset.drives` order, so the thing that decides the fight is off the
    // row the author is looking at. The three answers (printed index, row
    // note, target-scoped reorder) are pinned here; the module doc states
    // them.

    fn targets(asset: &ParticlesAsset) -> Vec<String> {
        asset
            .drives
            .iter()
            .map(|d| format!("{:?}", d.target))
            .collect()
    }

    /// The three drives every hazard test needs: two on one target with an
    /// unrelated drive wedged between them, so "the next drive in the
    /// vector" and "the next drive on this target" are different answers.
    fn scattered_asset() -> (ParticlesAsset, DriveTarget, DriveTarget) {
        let mut asset = asset_with(1, 0, 2);
        let fought_over = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::Alpha,
        };
        let unrelated = DriveTarget::Emitter {
            index: 0,
            prop: EmitterProp::SizeMul,
        };
        asset.drives = vec![
            flat_drive(0, fought_over.clone(), 2.0, DriveOp::Multiply),
            flat_drive(1, unrelated.clone(), 9.0, DriveOp::Multiply),
            flat_drive(0, fought_over.clone(), 5.0, DriveOp::Replace),
        ];
        (asset, fought_over, unrelated)
    }

    #[test]
    fn earlier_and_later_reach_past_a_drive_on_another_target() {
        let (asset, _, _) = scattered_asset();
        assert_eq!(
            target_neighbour(&asset, 2, -1),
            Some(0),
            "index 1 is on a different target, so index 0 is what index 2 fights with"
        );
        assert_eq!(target_neighbour(&asset, 0, 1), Some(2));
    }

    #[test]
    fn a_drive_alone_on_its_target_has_no_neighbour_in_either_direction() {
        let (asset, _, _) = scattered_asset();
        assert_eq!(target_neighbour(&asset, 1, -1), None);
        assert_eq!(target_neighbour(&asset, 1, 1), None);
    }

    #[test]
    fn an_out_of_range_index_has_no_neighbour() {
        let (asset, _, _) = scattered_asset();
        assert_eq!(target_neighbour(&asset, 9, -1), None);
    }

    #[test]
    fn a_target_scoped_move_swaps_the_two_and_leaves_the_bystander_put() {
        // The observable half: before the move the Replace is last and wins
        // outright (5.0); after it the Replace is first and the Multiply
        // compounds on top of it (5 * 2 = 10). The bystander at index 1 must
        // still be on the same target it started on -- a flat swap would
        // have moved it instead.
        let (mut asset, _, unrelated) = scattered_asset();
        let slot = EmitterProp::Alpha.slot().expect("Alpha is a Render prop");
        assert_eq!(
            resolve_drives(&[1.0, 1.0], &asset).emitters[0].render[slot],
            Some(5.0),
        );

        let to = target_neighbour(&asset, 2, -1).expect("the Replace has an earlier partner");
        assert!(move_drive(&mut asset, 2, to));

        assert_eq!(
            resolve_drives(&[1.0, 1.0], &asset).emitters[0].render[slot],
            Some(10.0),
            "Replace first, Multiply compounding on top of it"
        );
        assert_eq!(
            asset.drives[2].target, unrelated,
            "the drive on the other target must not have been dragged along"
        );
        assert_eq!(targets(&asset).len(), 3, "nothing was added or lost");
    }

    // --- What the list says about a fight --------------------------------

    #[test]
    fn a_drive_before_an_unmuted_later_replace_on_its_target_is_discarded() {
        let (asset, _, _) = scattered_asset();
        assert!(discarded_by_a_later_replace(&asset, 0));
        assert!(
            !discarded_by_a_later_replace(&asset, 2),
            "the Replace itself is what survives"
        );
    }

    #[test]
    fn a_muted_later_replace_discards_nothing() {
        // `sample` returns `None` for a muted drive and `resolve_drives`
        // `continue`s before `fold` ever sees it, so a muted Replace never
        // reaches the arm that discards the accumulator. Verified through
        // the real resolver as well as the predicate, because this is the
        // one case where the predicate could plausibly be wrong and still
        // look right.
        let (mut asset, _, _) = scattered_asset();
        asset.drives[2].muted = true;
        assert!(!discarded_by_a_later_replace(&asset, 0));

        let slot = EmitterProp::Alpha.slot().expect("Alpha is a Render prop");
        assert_eq!(
            resolve_drives(&[1.0, 1.0], &asset).emitters[0].render[slot],
            Some(2.0),
            "the earlier Multiply is what reaches the effect once the Replace is muted"
        );
    }

    #[test]
    fn a_replace_on_a_different_target_discards_nothing() {
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![
            flat_drive(
                0,
                DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
                2.0,
                DriveOp::Multiply,
            ),
            flat_drive(
                0,
                DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul },
                5.0,
                DriveOp::Replace,
            ),
        ];
        assert!(!discarded_by_a_later_replace(&asset, 0));
    }

    #[test]
    fn an_uncontended_live_row_has_no_note_at_all() {
        let (asset, _, _) = scattered_asset();
        assert_eq!(
            row_note(&asset, 1),
            "",
            "the ordinary case must not carry noise"
        );
    }

    #[test]
    fn a_contended_row_prints_its_rank_and_whether_it_survives() {
        let (asset, _, _) = scattered_asset();
        assert_eq!(row_note(&asset, 0), "1 of 2 \u{00b7} discarded");
        assert_eq!(row_note(&asset, 2), "2 of 2");
    }

    #[test]
    fn a_muted_row_says_so_on_top_of_everything_else() {
        let (mut asset, _, _) = scattered_asset();
        asset.drives[1].muted = true;
        assert_eq!(row_note(&asset, 1), "muted");
    }

    #[test]
    fn the_editor_panes_sentence_names_the_rank_and_the_discard() {
        let (asset, _, _) = scattered_asset();
        let text = precedence_text(&asset, 0);
        assert!(text.contains("#0"), "the apply-order number: {text}");
        assert!(text.contains("1 of 2"), "the rank: {text}");
        assert!(
            text.to_lowercase().contains("discards"),
            "and what happens to it: {text}"
        );

        let lone = precedence_text(&asset, 1);
        assert!(
            lone.contains("only drive on this target"),
            "an uncontended drive says there is no fight: {lone}"
        );
    }

    // --- The duplicate-target badge --------------------------------------

    #[test]
    fn a_scattered_double_replace_warns_exactly_once() {
        // The defect the old list had: it painted `target_warnings` under a
        // group header, and its groups were adjacency runs over the flat
        // vector, so a target whose two Replace drives were not adjacent got
        // two headers and printed the same warning twice for one problem.
        // One selection is one target, so the count is one whatever the
        // vector looks like.
        let mut asset = asset_with(1, 0, 1);
        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha };
        let other = DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul };
        asset.drives = vec![
            flat_drive(0, target.clone(), 1.0, DriveOp::Replace),
            flat_drive(0, other, 9.0, DriveOp::Multiply),
            flat_drive(0, target, 2.0, DriveOp::Replace),
        ];

        let warnings = warnings_for_selection(&asset, Some(0));
        assert_eq!(warnings.len(), 1, "one problem, one badge: {warnings:?}");
        assert_eq!(
            warnings,
            warnings_for_selection(&asset, Some(2)),
            "and selecting the other end of the same fight says the same thing once"
        );
    }

    #[test]
    fn nothing_selected_means_no_warnings_to_paint() {
        let (asset, _, _) = scattered_asset();
        assert!(warnings_for_selection(&asset, None).is_empty());
        assert!(
            warnings_for_selection(&asset, Some(99)).is_empty(),
            "an out-of-range selection must not panic or fabricate a target"
        );
    }

    // --- Grouping ---------------------------------------------------------

    #[test]
    fn groups_run_in_variable_declaration_order_not_first_use_order() {
        // Declaration order, so that reordering drives -- the one edit whose
        // purpose is to move indices -- cannot also make the groups jump.
        let mut asset = asset_with(1, 0, 3);
        let target = DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha };
        asset.drives = vec![
            flat_drive(2, target.clone(), 1.0, DriveOp::Multiply),
            flat_drive(0, target.clone(), 1.0, DriveOp::Multiply),
        ];
        let groups = group_drives_by_variable(&asset);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, vec!["v0", "v2"]);
    }

    #[test]
    fn rows_within_a_group_keep_apply_order_and_carry_their_flat_index() {
        let (asset, _, _) = scattered_asset();
        let groups = group_drives_by_variable(&asset);
        let v0 = groups.iter().find(|g| g.name == "v0").expect("v0 has drives");
        let indices: Vec<usize> = v0.rows.iter().map(|r| r.index).collect();
        assert_eq!(
            indices,
            vec![0, 2],
            "the printed numbers are indices into asset.drives, gaps and all"
        );
    }

    #[test]
    fn a_drive_naming_an_undeclared_variable_still_gets_a_group() {
        // `validate_drives` rejects this at load, but the editor holds it
        // transiently between a variable delete and its renumber. A drive
        // the author cannot see is a drive they cannot fix.
        let mut asset = asset_with(1, 0, 1);
        asset.drives = vec![flat_drive(
            7,
            DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
            1.0,
            DriveOp::Multiply,
        )];
        let groups = group_drives_by_variable(&asset);
        assert_eq!(groups.len(), 1);
        assert!(
            groups[0].name.contains('7') && groups[0].name.contains("undeclared"),
            "the group must name the id it could not resolve: {}",
            groups[0].name
        );
    }

    #[test]
    fn the_editor_header_names_the_variable_and_the_property() {
        // Two props can share a `Stage` -- `ScrollU` and `ScrollV` are both
        // `Render`, and the inspector puts a drive button for each on one
        // field -- so a header built from stage text alone cannot tell two
        // drives apart. This replaces `drive_button`'s popover-header pin,
        // which went with the popover.
        let mut asset = asset_with(1, 0, 1);
        asset.variables[0].name = "temperature".into();
        asset.drives = vec![
            flat_drive(
                0,
                DriveTarget::Emitter { index: 0, prop: EmitterProp::ScrollU },
                1.0,
                DriveOp::Multiply,
            ),
            flat_drive(
                0,
                DriveTarget::Emitter { index: 0, prop: EmitterProp::ScrollV },
                1.0,
                DriveOp::Multiply,
            ),
        ];
        let u = editing_header_text(&asset, 0).expect("drive 0 exists");
        let v = editing_header_text(&asset, 1).expect("drive 1 exists");
        assert_ne!(u, v, "two props sharing a Stage must not share a header");
        assert!(u.contains("temperature"), "the variable that moves it: {u}");
        assert!(u.to_lowercase().contains("scroll"), "and what it moves: {u}");
        assert_eq!(editing_header_text(&asset, 9), None);
    }

    // --- Selection arithmetic ---------------------------------------------

    #[test]
    fn deleting_the_selected_drive_clears_the_selection() {
        assert_eq!(selection_after_delete(Some(2), 2), None);
    }

    #[test]
    fn deleting_below_the_selection_slides_it_down_and_above_it_leaves_it() {
        assert_eq!(selection_after_delete(Some(3), 1), Some(2));
        assert_eq!(selection_after_delete(Some(1), 3), Some(1));
        assert_eq!(selection_after_delete(None, 0), None);
    }

    #[test]
    fn a_move_carries_the_selection_with_the_drive_not_the_slot() {
        assert_eq!(selection_after_move(Some(4), 4, 1), Some(1));
        // The drives the moved one stepped over shift the other way.
        assert_eq!(selection_after_move(Some(1), 4, 1), Some(2));
        assert_eq!(selection_after_move(Some(3), 1, 4), Some(2));
        // Outside the moved span, nothing changes.
        assert_eq!(selection_after_move(Some(9), 1, 4), Some(9));
        assert_eq!(selection_after_move(None, 1, 4), None);
    }

    // --- App-driven: the dock's own observers ----------------------------

    #[test]
    fn clicking_a_row_selects_it_and_never_dirties_the_project() {
        // Selection is viewing state, exactly like `NewDriveDraft`: deciding
        // to LOOK at a drive must not make the project look unsaved.
        let mut app = test_app();
        app.add_observer(on_drive_row_click);

        let (asset, _, _) = scattered_asset();
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: None,
        });

        let row = app.world_mut().spawn(DriveListRow(2)).id();
        app.world_mut().trigger(ButtonClickEvent { entity: row });

        assert_eq!(app.world().resource::<SelectedDrive>().0, Some(2));
        assert!(!app.world().resource::<DirtyState>().has_unsaved_changes);
    }

    #[test]
    fn adding_a_drive_opens_the_one_it_just_created() {
        // The editor pane is the only place a new drive's curve and output
        // can be set, so creating one and leaving the pane on something else
        // would strand it.
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

        assert_eq!(app.world().resource::<SelectedDrive>().0, Some(0));
    }

    #[test]
    fn deleting_the_selected_drive_through_the_real_observer_empties_the_pane() {
        let mut app = test_app();
        app.add_observer(drive_button::handle_drive_delete_click);

        let (asset, _, _) = scattered_asset();
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle),
            current_project_path: None,
            inspecting: None,
        });
        app.insert_resource(SelectedDrive(Some(2)));

        let delete_button = app.world_mut().spawn(DriveDeleteButton(2)).id();
        app.world_mut().trigger(ButtonClickEvent { entity: delete_button });

        assert_eq!(app.world().resource::<SelectedDrive>().0, None);
    }

    #[test]
    fn deleting_a_drive_below_the_selection_keeps_the_pane_on_the_same_drive() {
        // The stale-index hazard in its sharpest form: without the fix-up
        // the pane would keep index 2 and silently start editing whatever
        // slid into that slot.
        let mut app = test_app();
        app.add_observer(drive_button::handle_drive_delete_click);

        let (asset, _, _) = scattered_asset();
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        app.insert_resource(SelectedDrive(Some(2)));

        let delete_button = app.world_mut().spawn(DriveDeleteButton(0)).id();
        app.world_mut().trigger(ButtonClickEvent { entity: delete_button });

        assert_eq!(app.world().resource::<SelectedDrive>().0, Some(1));
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(
            asset.drives[1].output.min, 5.0,
            "index 1 is still the Replace the pane was showing"
        );
    }

    #[test]
    fn moving_the_selected_drive_keeps_the_pane_on_it() {
        let mut app = test_app();
        app.add_observer(handle_move_drive_click);

        let (asset, _, _) = scattered_asset();
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        app.insert_resource(SelectedDrive(Some(2)));

        let earlier = app.world_mut().spawn(MoveDriveButton { index: 2, delta: -1 }).id();
        app.world_mut().trigger(ButtonClickEvent { entity: earlier });

        assert!(app.world().resource::<DirtyState>().has_unsaved_changes);
        assert_eq!(app.world().resource::<SelectedDrive>().0, Some(0));
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(
            asset.drives[0].output.min, 5.0,
            "the Replace moved to index 0 and the selection followed it"
        );
    }

    #[test]
    fn a_move_click_on_the_earliest_drive_on_its_target_is_a_no_op() {
        // Not the same case as the single-drive test above: there IS an
        // earlier drive in the vector here, just not on this target. A flat
        // move would happily swap them, dirty the project, and change
        // nothing an author can see.
        let mut app = test_app();
        app.add_observer(handle_move_drive_click);

        let mut asset = asset_with(1, 0, 2);
        asset.drives = vec![
            flat_drive(
                1,
                DriveTarget::Emitter { index: 0, prop: EmitterProp::SizeMul },
                9.0,
                DriveOp::Multiply,
            ),
            flat_drive(
                0,
                DriveTarget::Emitter { index: 0, prop: EmitterProp::Alpha },
                2.0,
                DriveOp::Multiply,
            ),
        ];
        let handle = app.world_mut().resource_mut::<Assets<ParticlesAsset>>().add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });

        let earlier = app.world_mut().spawn(MoveDriveButton { index: 1, delta: -1 }).id();
        app.world_mut().trigger(ButtonClickEvent { entity: earlier });

        assert!(
            !app.world().resource::<DirtyState>().has_unsaved_changes,
            "no earlier drive on this target, so nothing to swap with"
        );
        let asset = app.world().resource::<Assets<ParticlesAsset>>().get(&handle).unwrap();
        assert_eq!(asset.drives[0].output.min, 9.0, "the vector is untouched");
    }
}
