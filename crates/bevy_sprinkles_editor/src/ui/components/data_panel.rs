use bevy::input_focus::{FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy_sprinkles::asset::DriveTarget;
use bevy_sprinkles::prelude::*;

use crate::state::{DirtyState, EditorState, Inspectable, Inspecting};
use crate::ui::icons::ICON_MESH_CYLINDER;
use crate::ui::widgets::button::{
    ButtonClickEvent, ButtonProps, ButtonVariant, EditorButton, button, set_button_variant,
};
use crate::ui::widgets::combobox::{
    ComboBoxChangeEvent, ComboBoxPopover, ComboBoxTrigger, combobox_icon,
};
use crate::ui::widgets::dialog::{DialogActionEvent, EditorDialog, OpenConfirmationDialogEvent};
use crate::ui::widgets::panel::{PanelDirection, PanelProps, panel};
use crate::ui::widgets::panel_section::{
    PanelSectionProps, SecondaryButtonClickEvent, panel_section,
};
use crate::ui::widgets::scroll::scrollbar;
use crate::ui::widgets::text_edit::{
    EditorTextEdit, TextEditCommitEvent, TextEditProps, text_edit,
};
use crate::ui::widgets::utils::find_ancestor;
use crate::viewport::{RespawnCollidersEvent, RespawnEmittersEvent, RespawnLightsEvent};

const DOUBLE_CLICK_THRESHOLD: f32 = 0.3;

/// Removes the emitter at `index`, keeping the file loadable.
///
/// The emitters list is positionally addressed by drives in TWO variants, not
/// one: `DriveTarget::Emitter { index }` and `DriveTarget::Transform { index }`
/// both index into `ParticlesAsset::emitters` (a `Transform` drive targets the
/// emitter ENTITY's `Transform`, so its index is an emitter index -- see its
/// doc in `asset::drive`). Removing an entry therefore shifts both. Two things
/// follow, both required: a drive that named exactly this emitter is dropped
/// (it would otherwise address whatever slid into the slot), and every
/// surviving emitter- or transform-targeted drive above it is renumbered.
///
/// Skipping either leaves an asset `validate_drives` rejects on the very next
/// load -- and a drive naming the LAST emitter is left frankly out of range, so
/// the file the editor just wrote is one the editor cannot reopen. This is the
/// same property `variables::remove_variable` and `lights::remove_light` exist
/// to hold; the emitters list predates both and was not brought along.
pub fn remove_emitter(asset: &mut ParticlesAsset, index: usize) {
    if index >= asset.emitters.len() {
        return;
    }
    asset.emitters.remove(index);
    asset.drives.retain(|d| match &d.target {
        DriveTarget::Emitter { index: i, .. } | DriveTarget::Transform { index: i, .. } => {
            *i as usize != index
        }
        DriveTarget::Light { .. } => true,
    });
    for drive in asset.drives.iter_mut() {
        if let DriveTarget::Emitter { index: i, .. } | DriveTarget::Transform { index: i, .. } =
            &mut drive.target
        {
            if *i as usize > index {
                *i -= 1;
            }
        }
    }
}

/// Copies the emitter at `index` onto the END of the list, returning the new
/// emitter's index, or `None` if `index` names no emitter.
///
/// Appending rather than inserting beside the source is the whole point: see
/// the `"Duplicate"` arm of `on_item_menu_change` for why an insert silently
/// rewires every drive above the insertion point while leaving the file
/// perfectly loadable. `base` is the source's name with any trailing number
/// stripped, so the copy is named the same way `on_add_emitter` names a fresh
/// one.
pub fn duplicate_emitter(asset: &mut ParticlesAsset, index: usize, base: &str) -> Option<usize> {
    let source = asset.emitters.get(index)?;
    let mut new_item = source.clone();
    let existing: Vec<&str> = asset.emitters.iter().map(|e| e.name.as_str()).collect();
    new_item.name = next_unique_name(base, &existing);
    asset.emitters.push(new_item);
    Some(asset.emitters.len() - 1)
}

pub fn plugin(app: &mut App) {
    app.init_resource::<LastLoadedProject>()
        .add_observer(on_item_click)
        .add_observer(on_item_menu_change)
        .add_observer(on_rename_commit)
        .add_observer(on_delete_confirmed)
        .add_observer(on_add_emitter)
        .add_observer(on_add_mesh_fx)
        .add_observer(on_add_collider)
        .add_systems(
            Update,
            (
                setup_data_panel,
                rebuild_lists,
                update_items,
                handle_item_right_click,
                handle_item_double_click,
                focus_rename_input,
                cleanup_pending_delete,
            ),
        );
}

#[derive(Resource, Default)]
struct LastLoadedProject {
    handle: Option<AssetId<ParticlesAsset>>,
}

#[derive(Component, Default, Clone)]
pub struct EditorDataPanel;

#[derive(Component)]
struct EmittersSection;

#[derive(Component)]
struct CollidersSection;

#[derive(Component)]
struct InspectableItem {
    kind: Inspectable,
    index: u8,
}

#[derive(Component)]
struct ItemButton;

#[derive(Component)]
struct ItemMenu;

#[derive(Component)]
struct ItemsList;

#[derive(Component)]
struct RenameInput {
    item_entity: Entity,
    focused: bool,
}

#[derive(Component)]
struct Renaming;

#[derive(Resource)]
struct PendingDelete {
    kind: Inspectable,
    index: u8,
}

#[derive(Event)]
struct AddEmitterEvent;

/// Adds the "Mesh FX" preset -- see `bevy_sprinkles::asset::mesh_fx_emitter`'s
/// doc comment for why this exists rather than a separate mesh-effect object.
#[derive(Event)]
struct AddMeshFxEvent;

#[derive(Event)]
struct AddColliderEvent;

pub fn data_panel() -> impl Scene {
    bsn! {
        EditorDataPanel
        panel(
            PanelProps::new(PanelDirection::Left)
                .with_width(224)
                .with_min_width(160)
                .with_max_width(320),
        )
    }
}

fn setup_data_panel(mut commands: Commands, panels: Query<Entity, Added<EditorDataPanel>>) {
    for panel_entity in &panels {
        commands
            .entity(panel_entity)
            .with_child(scrollbar(panel_entity));

        commands
            .spawn_scene(panel_section(
                PanelSectionProps::new("Emitters")
                    .with_add_button()
                    .with_secondary_add_button(ICON_MESH_CYLINDER),
            ))
            .insert((EmittersSection, ChildOf(panel_entity)))
            .observe(on_add_emitter_click)
            .observe(on_add_mesh_fx_click);

        commands
            .spawn_scene(panel_section(
                PanelSectionProps::new("Colliders").with_add_button(),
            ))
            .insert((CollidersSection, ChildOf(panel_entity)))
            .observe(on_add_collider_click);
    }
}

fn rebuild_lists(
    mut commands: Commands,
    editor_state: Res<EditorState>,
    mut last_project: ResMut<LastLoadedProject>,
    assets: Res<Assets<ParticlesAsset>>,
    emitters_section: Query<(Entity, &Children), With<EmittersSection>>,
    colliders_section: Query<(Entity, &Children), With<CollidersSection>>,
    existing_wrappers: Query<Entity, With<ItemsList>>,
    new_sections: Query<Entity, Or<(Added<EmittersSection>, Added<CollidersSection>)>>,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };

    let Some(asset) = assets.get(handle) else {
        return;
    };

    let current_id = handle.id();
    let project_changed = last_project.handle != Some(current_id);
    let sections_added = !new_sections.is_empty();

    if !project_changed && !sections_added {
        return;
    }

    last_project.handle = Some(current_id);

    for entity in &existing_wrappers {
        commands.entity(entity).despawn();
    }

    if let Ok((section_entity, _)) = emitters_section.single() {
        spawn_items(
            &mut commands,
            section_entity,
            Inspectable::Emitter,
            asset.emitters.iter().map(|e| e.name.as_str()),
            &editor_state,
        );
    }

    if let Ok((section_entity, _)) = colliders_section.single() {
        spawn_items(
            &mut commands,
            section_entity,
            Inspectable::Collider,
            asset.colliders.iter().map(|c| c.name.as_str()),
            &editor_state,
        );
    }
}

fn spawn_items<'a>(
    commands: &mut Commands,
    section_entity: Entity,
    kind: Inspectable,
    names: impl Iterator<Item = &'a str>,
    editor_state: &EditorState,
) {
    let names: Vec<_> = names.collect();
    if names.is_empty() {
        return;
    }

    let list_entity = commands
        .spawn((
            ItemsList,
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(6.0),
                ..default()
            },
        ))
        .id();

    commands.entity(section_entity).add_child(list_entity);

    for (index, name) in names.into_iter().enumerate() {
        let index = index as u8;
        let is_active = editor_state
            .inspecting
            .map(|i| i.kind == kind && i.index == index)
            .unwrap_or(false);

        let variant = if is_active {
            ButtonVariant::Active
        } else {
            ButtonVariant::Ghost
        };

        let item_entity = commands
            .spawn((
                InspectableItem { kind, index },
                Hovered::default(),
                Interaction::None,
                Node {
                    width: percent(100),
                    ..default()
                },
            ))
            .id();

        let button_entity = commands
            .spawn_scene(button(
                ButtonProps::new(name).with_variant(variant).align_left(),
            ))
            .insert(ItemButton)
            .id();

        let menu_entity = commands
            .spawn_scene(combobox_icon(vec!["Duplicate", "Rename", "Delete"]))
            .insert(ItemMenu)
            .insert(Node {
                position_type: PositionType::Absolute,
                right: px(0.0),
                top: px(0.0),
                ..default()
            })
            .id();

        commands
            .entity(item_entity)
            .add_children(&[button_entity, menu_entity]);

        commands.entity(list_entity).add_child(item_entity);
    }
}

fn on_add_emitter_click(_event: On<ButtonClickEvent>, mut commands: Commands) {
    commands.trigger(AddEmitterEvent);
}

fn on_add_mesh_fx_click(_event: On<SecondaryButtonClickEvent>, mut commands: Commands) {
    commands.trigger(AddMeshFxEvent);
}

fn on_add_collider_click(_event: On<ButtonClickEvent>, mut commands: Commands) {
    commands.trigger(AddColliderEvent);
}

fn on_add_emitter(
    _event: On<AddEmitterEvent>,
    mut commands: Commands,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut last_project: ResMut<LastLoadedProject>,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };

    let existing_names: Vec<&str> = asset.emitters.iter().map(|e| e.name.as_str()).collect();
    let name = next_unique_name("Emitter", &existing_names);

    let new_index = asset.emitters.len() as u8;
    asset.emitters.push(EmitterData {
        name,
        ..Default::default()
    });

    dirty_state.has_unsaved_changes = true;

    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Emitter,
        index: new_index,
    });

    commands.trigger(RespawnEmittersEvent);
    last_project.handle = None;
}

/// Mirrors `on_add_emitter` exactly, but pushes `mesh_fx_emitter()`'s preset
/// configuration instead of a bare default. It is a separate emitter kind
/// only at the level of "which starting values it gets" -- everything past
/// this push (inspector, drives, save/load) treats it as an ordinary
/// `EmitterData`, which is the whole point the preset's doc comment makes.
fn on_add_mesh_fx(
    _event: On<AddMeshFxEvent>,
    mut commands: Commands,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut last_project: ResMut<LastLoadedProject>,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };

    let existing_names: Vec<&str> = asset.emitters.iter().map(|e| e.name.as_str()).collect();
    let name = next_unique_name("Mesh FX", &existing_names);

    let new_index = asset.emitters.len() as u8;
    // Appending, like `on_add_emitter` -- positionally safe, no drive needs
    // renumbering (see the module's Duplicate-vs-append note elsewhere).
    asset.emitters.push(EmitterData {
        name,
        ..mesh_fx_emitter()
    });

    dirty_state.has_unsaved_changes = true;

    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Emitter,
        index: new_index,
    });

    commands.trigger(RespawnEmittersEvent);
    last_project.handle = None;
}

fn on_add_collider(
    _event: On<AddColliderEvent>,
    mut commands: Commands,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut last_project: ResMut<LastLoadedProject>,
) {
    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };

    let existing_names: Vec<&str> = asset.colliders.iter().map(|c| c.name.as_str()).collect();
    let name = next_unique_name("Collider", &existing_names);

    let new_index = asset.colliders.len() as u8;
    asset.colliders.push(ColliderData {
        name,
        ..Default::default()
    });

    dirty_state.has_unsaved_changes = true;

    editor_state.inspecting = Some(Inspecting {
        kind: Inspectable::Collider,
        index: new_index,
    });

    commands.trigger(RespawnCollidersEvent);
    last_project.handle = None;
}

fn on_item_click(
    event: On<ButtonClickEvent>,
    buttons: Query<&ChildOf, With<ItemButton>>,
    items: Query<&InspectableItem>,
    mut editor_state: ResMut<EditorState>,
) {
    let Ok(child_of) = buttons.get(event.entity) else {
        return;
    };
    let Ok(item) = items.get(child_of.parent()) else {
        return;
    };

    editor_state.inspecting = Some(Inspecting {
        kind: item.kind,
        index: item.index,
    });
}

fn on_item_menu_change(
    event: On<ComboBoxChangeEvent>,
    mut commands: Commands,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut last_project: ResMut<LastLoadedProject>,
    menus: Query<&ChildOf, With<ItemMenu>>,
    items: Query<(Entity, &InspectableItem, &Children), Without<Renaming>>,
    mut buttons: Query<&mut Node, With<ItemButton>>,
) {
    let Ok(child_of) = menus.get(event.entity) else {
        return;
    };
    let Ok((item_entity, item, children)) = items.get(child_of.parent()) else {
        return;
    };

    let item_name = get_item_name(&editor_state, &assets, item);
    let Some(item_name) = item_name else {
        return;
    };

    match event.label.as_str() {
        "Duplicate" => {
            let Some(handle) = &editor_state.current_project else {
                return;
            };
            let Some(mut asset) = assets.get_mut(handle) else {
                return;
            };

            let (base, _) = strip_trailing_number(&item_name);

            // WHERE the copy lands differs by kind, and that is a correctness
            // rule rather than a layout preference. An emitter is addressed
            // positionally by `DriveTarget::Emitter` and
            // `DriveTarget::Transform`, so inserting a copy directly after its
            // source shifts every emitter above it and silently re-points
            // every drive above it at a DIFFERENT emitter. Nothing goes out of
            // range, so the file still loads and `validate_drives` stays
            // happy -- the author's only symptom is the wrong emitter reacting
            // to a variable, with nothing to search for. Appending shifts
            // nothing, which is exactly why `on_add_emitter` and
            // `on_add_mesh_fx` append; Duplicate now matches them.
            //
            // A collider carries no positional reference anywhere in the asset
            // (no `DriveTarget` names one), so its copy may still land beside
            // its source, where an author expects it.
            let new_index = match item.kind {
                // Variables never enter this list -- they have their own, in
                // variables.rs. Kept exhaustive only because `Inspectable` is
                // a shared enum.
                Inspectable::Variable => return,
                // Same: lights have their own list, in lights.rs.
                Inspectable::Light => return,
                Inspectable::Emitter => {
                    let Some(new_index) = duplicate_emitter(&mut asset, item.index as usize, base)
                    else {
                        return;
                    };
                    new_index
                }
                Inspectable::Collider => {
                    let insert_index = item.index as usize + 1;
                    let Some(source) = asset.colliders.get(item.index as usize) else {
                        return;
                    };
                    let mut new_item = source.clone();
                    let existing: Vec<&str> =
                        asset.colliders.iter().map(|c| c.name.as_str()).collect();
                    new_item.name = next_unique_name(base, &existing);
                    asset.colliders.insert(insert_index, new_item);
                    insert_index
                }
            };

            dirty_state.has_unsaved_changes = true;
            adjust_inspecting_after_insert(&mut editor_state.inspecting, item.kind, new_index);
            trigger_respawn(&mut commands, item.kind);
            last_project.handle = None;
        }
        "Rename" => {
            let button_entity = children.iter().find(|c| buttons.get(*c).is_ok());
            if let Some(button_entity) = button_entity {
                if let Ok(mut btn_node) = buttons.get_mut(button_entity) {
                    btn_node.display = Display::None;
                }
            }
            start_rename(&mut commands, item_entity, &item_name);
        }
        "Delete" => {
            let label = match item.kind {
                Inspectable::Emitter => "Delete emitter",
                Inspectable::Collider => "Delete collider",
                // Unreachable: variables use their own list (variables.rs).
                Inspectable::Variable => "Delete variable",
                // Unreachable: lights use their own list (lights.rs).
                Inspectable::Light => "Delete light",
            };
            commands.insert_resource(PendingDelete {
                kind: item.kind,
                index: item.index,
            });
            commands.trigger(
                OpenConfirmationDialogEvent::new(label, "Delete")
                    .with_description(format!("Are you sure you want to delete {}?", item_name)),
            );
        }
        _ => {}
    }
}

fn handle_item_right_click(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    items: Query<(&Hovered, &Children), With<InspectableItem>>,
    buttons: Query<&Hovered, With<ItemButton>>,
    menus: Query<&Children, With<ItemMenu>>,
    triggers: Query<Entity, With<ComboBoxTrigger>>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }

    for (item_hovered, item_children) in &items {
        if !item_hovered.get() {
            continue;
        }

        let mut button_hovered = false;
        let mut menu_entity = None;

        for child in item_children.iter() {
            if let Ok(btn_hovered) = buttons.get(child) {
                button_hovered = btn_hovered.get();
            }
            if menus.get(child).is_ok() {
                menu_entity = Some(child);
            }
        }

        if !button_hovered {
            continue;
        }

        let Some(menu) = menu_entity else {
            continue;
        };

        let Ok(menu_children) = menus.get(menu) else {
            continue;
        };

        for menu_child in menu_children.iter() {
            if triggers.get(menu_child).is_ok() {
                commands.trigger(ButtonClickEvent { entity: menu_child });
                return;
            }
        }
    }
}

fn next_unique_name(base_name: &str, existing: &[&str]) -> String {
    if !existing.contains(&base_name) {
        return base_name.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{} {}", base_name, n);
        if !existing.iter().any(|name| *name == candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn trigger_respawn(commands: &mut Commands, kind: Inspectable) {
    match kind {
        Inspectable::Emitter => commands.trigger(RespawnEmittersEvent),
        Inspectable::Collider => commands.trigger(RespawnCollidersEvent),
        // A variable has no world representation of its own to respawn --
        // it only reshapes drives, which are resolved fresh every frame.
        // Also unreachable from this list; see the Duplicate arm above.
        Inspectable::Variable => {}
        // Unreachable from this list (see the Duplicate arm above), but
        // implemented correctly anyway: `lights.rs`'s own add/delete flow
        // triggers this event itself, so a light DOES need a real respawn
        // (unlike a variable) if this arm is ever reached some other way.
        Inspectable::Light => commands.trigger(RespawnLightsEvent),
    }
}

fn adjust_inspecting_after_insert(
    inspecting: &mut Option<Inspecting>,
    kind: Inspectable,
    insert_index: usize,
) {
    if let Some(current) = inspecting.as_mut() {
        if current.kind == kind && current.index as usize >= insert_index {
            current.index += 1;
        }
    }
    *inspecting = Some(Inspecting {
        kind,
        index: insert_index as u8,
    });
}

fn adjust_inspecting_after_delete(
    inspecting: &mut Option<Inspecting>,
    kind: Inspectable,
    deleted_index: usize,
    new_len: usize,
) {
    if let Some(current) = inspecting.as_ref() {
        if current.kind == kind {
            if current.index as usize == deleted_index {
                *inspecting = if new_len > 0 {
                    Some(Inspecting { kind, index: 0 })
                } else {
                    None
                };
            } else if (current.index as usize) > deleted_index {
                inspecting.as_mut().unwrap().index -= 1;
            }
        }
    }
}

fn strip_trailing_number(name: &str) -> (&str, Option<u32>) {
    if let Some(pos) = name.rfind(' ') {
        let suffix = &name[pos + 1..];
        if let Ok(n) = suffix.parse::<u32>() {
            return (name[..pos].trim_end(), Some(n));
        }
    }
    (name, None)
}

fn get_item_name(
    editor_state: &EditorState,
    assets: &Assets<ParticlesAsset>,
    item: &InspectableItem,
) -> Option<String> {
    let handle = editor_state.current_project.as_ref()?;
    let asset = assets.get(handle)?;
    match item.kind {
        Inspectable::Emitter => {
            let emitter = asset.emitters.get(item.index as usize)?;
            Some(emitter.name.clone())
        }
        Inspectable::Collider => {
            let collider = asset.colliders.get(item.index as usize)?;
            Some(collider.name.clone())
        }
        Inspectable::Variable => {
            let variable = asset.variables.get(item.index as usize)?;
            Some(variable.name.clone())
        }
        Inspectable::Light => {
            let light = asset.lights.get(item.index as usize)?;
            Some(light.name.clone())
        }
    }
}

fn start_rename(commands: &mut Commands, item_entity: Entity, name: &str) {
    commands.entity(item_entity).insert(Renaming);

    let rename_entity = commands
        .spawn_scene(text_edit(TextEditProps::default().with_default_value(name)))
        .insert(RenameInput {
            item_entity,
            focused: false,
        })
        .id();

    commands.entity(item_entity).add_child(rename_entity);
}

fn handle_item_double_click(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mouse: Res<ButtonInput<MouseButton>>,
    editor_state: Res<EditorState>,
    assets: Res<Assets<ParticlesAsset>>,
    items: Query<(Entity, &InspectableItem, &Children), Without<Renaming>>,
    mut buttons: Query<(Entity, &Hovered, &mut Node), With<ItemButton>>,
    mut last_click: Local<(Option<Entity>, f32)>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    for (item_entity, item, children) in &items {
        for child in children.iter() {
            let Ok((button_entity, hovered, _)) = buttons.get(child) else {
                continue;
            };
            if !hovered.get() {
                continue;
            }

            let now = time.elapsed_secs();
            let is_double = last_click.0 == Some(button_entity)
                && (now - last_click.1) < DOUBLE_CLICK_THRESHOLD;
            *last_click = (Some(button_entity), now);

            if !is_double {
                continue;
            }

            *last_click = (None, 0.0);

            let item_name = get_item_name(&editor_state, &assets, item);
            let Some(item_name) = item_name else {
                continue;
            };

            if let Ok((_, _, mut btn_node)) = buttons.get_mut(button_entity) {
                btn_node.display = Display::None;
            }

            start_rename(&mut commands, item_entity, &item_name);
            return;
        }
    }
}

fn focus_rename_input(
    mut focus: ResMut<InputFocus>,
    mut rename_inputs: Query<(Entity, &mut RenameInput)>,
    children_query: Query<&Children>,
    text_edits: Query<Entity, With<EditorTextEdit>>,
) {
    for (entity, mut rename_input) in &mut rename_inputs {
        if rename_input.focused {
            continue;
        }
        if let Some(inner) = find_inner_text_edit(entity, &children_query, &text_edits) {
            focus.set(inner, FocusCause::Navigated);
            rename_input.focused = true;
        }
    }
}

fn find_inner_text_edit(
    entity: Entity,
    children_query: &Query<&Children>,
    text_edits: &Query<Entity, With<EditorTextEdit>>,
) -> Option<Entity> {
    if text_edits.get(entity).is_ok() {
        return Some(entity);
    }
    let Ok(children) = children_query.get(entity) else {
        return None;
    };
    for child in children.iter() {
        if let Some(found) = find_inner_text_edit(child, children_query, text_edits) {
            return Some(found);
        }
    }
    None
}

fn on_rename_commit(
    trigger: On<TextEditCommitEvent>,
    mut commands: Commands,
    rename_inputs: Query<&RenameInput>,
    parents: Query<&ChildOf>,
    items: Query<(&InspectableItem, &Children)>,
    mut buttons: Query<(Entity, &mut Node), With<ItemButton>>,
    mut button_texts: Query<&mut Text>,
    button_children: Query<&Children, With<EditorButton>>,
    editor_state: Res<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut emitter_runtimes: Query<&mut EmitterRuntime>,
) {
    let text_edit_entity = trigger.entity;

    let Some((rename_entity, rename_input)) =
        find_ancestor(text_edit_entity, &rename_inputs, &parents)
    else {
        return;
    };

    let item_entity = rename_input.item_entity;
    let new_name = trigger.text.clone();

    let Ok((item, children)) = items.get(item_entity) else {
        return;
    };

    if !new_name.is_empty() {
        if let Some(handle) = &editor_state.current_project {
            if let Some(mut asset) = assets.get_mut(handle) {
                match item.kind {
                    Inspectable::Emitter => {
                        if let Some(emitter) = asset.emitters.get_mut(item.index as usize) {
                            emitter.name = new_name.clone();
                            dirty_state.has_unsaved_changes = true;
                            for mut runtime in emitter_runtimes.iter_mut() {
                                runtime.restart(None);
                            }
                        }
                    }
                    Inspectable::Collider => {
                        if let Some(collider) = asset.colliders.get_mut(item.index as usize) {
                            collider.name = new_name.clone();
                            dirty_state.has_unsaved_changes = true;
                        }
                    }
                    // Unreachable: variables rename via their own inspector
                    // field, not this list's double-click flow. Handled
                    // correctly anyway rather than left a silent no-op.
                    Inspectable::Variable => {
                        if let Some(variable) = asset.variables.get_mut(item.index as usize) {
                            variable.name = new_name.clone();
                            dirty_state.has_unsaved_changes = true;
                        }
                    }
                    // Unreachable: lights rename via their own inspector
                    // Name field (`inspector::light`), not this list's
                    // double-click flow. Handled correctly anyway.
                    Inspectable::Light => {
                        if let Some(light) = asset.lights.get_mut(item.index as usize) {
                            light.name = new_name.clone();
                            dirty_state.has_unsaved_changes = true;
                        }
                    }
                }
            }
        }
    }

    for child in children.iter() {
        if let Ok((button_entity, mut btn_node)) = buttons.get_mut(child) {
            btn_node.display = Display::Flex;

            if !new_name.is_empty() {
                if let Ok(btn_children) = button_children.get(button_entity) {
                    for btn_child in btn_children.iter() {
                        if let Ok(mut text) = button_texts.get_mut(btn_child) {
                            **text = new_name.clone();
                        }
                    }
                }
            }
        }
    }

    commands.entity(item_entity).remove::<Renaming>();
    commands.entity(rename_entity).despawn();
}

fn on_delete_confirmed(
    _event: On<DialogActionEvent>,
    pending: Option<Res<PendingDelete>>,
    mut commands: Commands,
    mut editor_state: ResMut<EditorState>,
    mut assets: ResMut<Assets<ParticlesAsset>>,
    mut dirty_state: ResMut<DirtyState>,
    mut last_project: ResMut<LastLoadedProject>,
) {
    let Some(pending) = pending else {
        return;
    };

    let kind = pending.kind;
    let index = pending.index as usize;
    commands.remove_resource::<PendingDelete>();

    let Some(handle) = &editor_state.current_project else {
        return;
    };
    let Some(mut asset) = assets.get_mut(handle) else {
        return;
    };

    let new_len = match kind {
        Inspectable::Emitter => {
            if index >= asset.emitters.len() {
                return;
            }
            remove_emitter(&mut asset, index);
            asset.emitters.len()
        }
        Inspectable::Collider => {
            if index >= asset.colliders.len() {
                return;
            }
            asset.colliders.remove(index);
            asset.colliders.len()
        }
        // Unreachable from this list (see the Duplicate arm above), but
        // implemented correctly -- and routed through the same renumbering
        // helper the variables list itself uses -- rather than left dead.
        Inspectable::Variable => {
            if index >= asset.variables.len() {
                return;
            }
            crate::ui::components::variables::remove_variable(&mut asset, index);
            asset.variables.len()
        }
        // Same reasoning as the `Variable` arm above, routed through
        // `lights.rs`'s own renumbering helper: unreachable from this list,
        // implemented correctly anyway.
        Inspectable::Light => {
            if index >= asset.lights.len() {
                return;
            }
            crate::ui::components::lights::remove_light(&mut asset, index);
            asset.lights.len()
        }
    };

    dirty_state.has_unsaved_changes = true;
    adjust_inspecting_after_delete(&mut editor_state.inspecting, kind, index, new_len);
    trigger_respawn(&mut commands, kind);
    last_project.handle = None;
}

fn cleanup_pending_delete(
    pending: Option<Res<PendingDelete>>,
    dialogs: Query<(), With<EditorDialog>>,
    mut commands: Commands,
) {
    if pending.is_some() && dialogs.is_empty() {
        commands.remove_resource::<PendingDelete>();
    }
}

fn update_items(
    editor_state: Res<EditorState>,
    items: Query<(&InspectableItem, &Hovered, &Children, Has<Renaming>)>,
    buttons: Query<&Children, With<ItemButton>>,
    mut button_styles: Query<
        (&mut ButtonVariant, &mut BackgroundColor, &mut BorderColor),
        With<EditorButton>,
    >,
    mut menus: Query<(Entity, &mut Node, &Children), With<ItemMenu>>,
    trigger_children: Query<
        &Children,
        (
            Without<InspectableItem>,
            Without<ItemButton>,
            Without<ItemMenu>,
        ),
    >,
    mut images: Query<&mut ImageNode>,
    mut text_colors: Query<&mut TextColor>,
    popovers: Query<&ComboBoxPopover>,
) {
    for (item, hovered, children, is_renaming) in &items {
        let is_active = editor_state
            .inspecting
            .map(|i| i.kind == item.kind && i.index == item.index)
            .unwrap_or(false);

        let new_variant = if is_active {
            ButtonVariant::Active
        } else {
            ButtonVariant::Ghost
        };

        let text_color = new_variant.text_color();

        for child in children.iter() {
            if let Ok(button_children) = buttons.get(child) {
                if let Ok((mut variant, mut bg, mut border)) = button_styles.get_mut(child) {
                    if *variant != new_variant {
                        *variant = new_variant;
                        set_button_variant(new_variant, &mut bg, &mut border);

                        for button_child in button_children.iter() {
                            if let Ok(mut color) = text_colors.get_mut(button_child) {
                                color.0 = text_color.into();
                            }
                            if let Ok(mut image) = images.get_mut(button_child) {
                                image.color = text_color.into();
                            }
                        }
                    }
                }
            }
            if let Ok((menu_entity, mut node, menu_kids)) = menus.get_mut(child) {
                let has_open_popover = popovers.iter().any(|p| p.0 == menu_entity);
                let show_menu = !is_renaming && (is_active || hovered.get() || has_open_popover);

                node.display = if show_menu {
                    Display::Flex
                } else {
                    Display::None
                };
                for menu_child in menu_kids.iter() {
                    if let Ok(children) = trigger_children.get(menu_child) {
                        for trigger_child in children.iter() {
                            if let Ok(mut image) = images.get_mut(trigger_child) {
                                image.color = text_color.into();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_sprinkles::asset::drive::validate_drives;
    use bevy_sprinkles::asset::{
        Drive, DriveOp, EmitterProp, TransformProp, VariableDecl, VariableId,
    };

    fn asset_with(emitters: usize, drives: Vec<Drive>) -> ParticlesAsset {
        let mut a = ParticlesAsset::new(
            "t".into(),
            ParticlesDimension::D3,
            Default::default(),
            vec![EmitterData::default(); emitters],
            vec![],
            false,
            ParticlesAuthors::default(),
        );
        // A drive always names a variable too; declaring one keeps
        // `validate_drives` focused on the emitter-index question these tests
        // pin, rather than failing on an unrelated undeclared-variable error.
        a.variables = vec![VariableDecl {
            name: "v".into(),
            ..Default::default()
        }];
        a.drives = drives;
        a
    }

    fn drive_on(target: DriveTarget) -> Drive {
        Drive {
            variable: VariableId(0),
            target,
            curve: CurveTexture::default(),
            output: ParticleRange { min: 0.0, max: 1.0 },
            op: DriveOp::Multiply,
            muted: false,
        }
    }

    fn emitter_drive(index: u8) -> Drive {
        drive_on(DriveTarget::Emitter {
            index,
            prop: EmitterProp::Tint,
        })
    }

    fn transform_drive(index: u8) -> Drive {
        drive_on(DriveTarget::Transform {
            index,
            prop: TransformProp::ScaleY,
        })
    }

    #[test]
    fn deleting_an_emitter_removes_the_drives_that_targeted_it() {
        let mut asset = asset_with(1, vec![emitter_drive(0)]);
        remove_emitter(&mut asset, 0);
        assert!(asset.drives.is_empty());
    }

    #[test]
    fn deleting_an_emitter_also_removes_the_transform_drives_that_targeted_it() {
        // `DriveTarget::Transform`'s index is an EMITTER index, so the delete
        // must reach both variants. Dropping only `Emitter` would leave this
        // drive addressing a different emitter -- or, at the tail, nothing.
        let mut asset = asset_with(1, vec![transform_drive(0)]);
        remove_emitter(&mut asset, 0);
        assert!(asset.drives.is_empty());
    }

    #[test]
    fn deleting_an_emitter_renumbers_both_drive_variants_above_it() {
        let mut asset = asset_with(3, vec![emitter_drive(2), transform_drive(2)]);
        remove_emitter(&mut asset, 1);
        assert!(matches!(
            asset.drives[0].target,
            DriveTarget::Emitter { index: 1, .. }
        ));
        assert!(matches!(
            asset.drives[1].target,
            DriveTarget::Transform { index: 1, .. }
        ));
    }

    #[test]
    fn a_drive_on_an_emitter_below_the_deleted_one_is_left_untouched() {
        let mut asset = asset_with(3, vec![emitter_drive(0), emitter_drive(2)]);
        remove_emitter(&mut asset, 1);
        assert!(matches!(
            asset.drives[0].target,
            DriveTarget::Emitter { index: 0, .. }
        ));
        assert!(matches!(
            asset.drives[1].target,
            DriveTarget::Emitter { index: 1, .. }
        ));
    }

    /// The direct pin on the property this helper exists to protect: the
    /// editor must never write a file that fails its own load validation.
    /// Runs the delete through the REAL `validate_drives`, mirroring the
    /// equivalents in `variables.rs` and `lights.rs`.
    ///
    /// The tail drive is the sharp case: before this helper existed, deleting
    /// emitter 1 of two left `Emitter { index: 1 }` naming an emitter that no
    /// longer exists, and `validate_drives` rejects that at load -- so the
    /// editor wrote a file it could not reopen.
    #[test]
    fn a_delete_that_strands_a_tail_drive_still_leaves_the_asset_loadable() {
        let mut asset = asset_with(2, vec![emitter_drive(1), transform_drive(1)]);
        remove_emitter(&mut asset, 1);
        assert!(validate_drives(&asset).is_ok(), "{:?}", validate_drives(&asset));
    }

    #[test]
    fn a_delete_that_renumbers_still_leaves_the_asset_loadable() {
        let mut asset = asset_with(3, vec![emitter_drive(2), transform_drive(2)]);
        remove_emitter(&mut asset, 1);
        assert!(validate_drives(&asset).is_ok());
    }

    #[test]
    fn deleting_an_out_of_range_emitter_is_a_no_op() {
        let mut asset = asset_with(1, vec![emitter_drive(0)]);
        remove_emitter(&mut asset, 5);
        assert_eq!(asset.emitters.len(), 1);
        assert_eq!(asset.drives.len(), 1);
    }

    #[test]
    fn a_light_drive_survives_an_emitter_delete_untouched() {
        use bevy_sprinkles::asset::{LightData, LightProp};
        let mut asset = asset_with(2, vec![]);
        asset.lights = vec![LightData::default(); 2];
        asset.drives = vec![drive_on(DriveTarget::Light {
            index: 1,
            prop: LightProp::Intensity,
        })];
        remove_emitter(&mut asset, 0);
        assert!(matches!(
            asset.drives[0].target,
            DriveTarget::Light { index: 1, .. }
        ));
    }

    /// I1: duplicating an emitter must not move any existing emitter, because
    /// every drive addresses emitters positionally and an insert would
    /// silently re-point the ones above it -- with the file still loading
    /// clean, so nothing surfaces the rewire.
    #[test]
    fn duplicating_an_emitter_appends_and_leaves_every_drive_index_untouched() {
        let mut asset = asset_with(3, vec![emitter_drive(1), transform_drive(2)]);
        asset.emitters[0].name = "Emitter".into();
        let new_index = duplicate_emitter(&mut asset, 0, "Emitter").expect("emitter 0 exists");

        assert_eq!(new_index, 3, "the copy must land at the END of the list");
        assert_eq!(asset.emitters.len(), 4);
        assert!(matches!(
            asset.drives[0].target,
            DriveTarget::Emitter { index: 1, .. }
        ));
        assert!(matches!(
            asset.drives[1].target,
            DriveTarget::Transform { index: 2, .. }
        ));
    }

    #[test]
    fn duplicating_an_emitter_gives_the_copy_a_unique_name() {
        let mut asset = asset_with(1, vec![]);
        asset.emitters[0].name = "Emitter".into();
        duplicate_emitter(&mut asset, 0, "Emitter").expect("emitter 0 exists");
        assert_ne!(asset.emitters[0].name, asset.emitters[1].name);
    }

    #[test]
    fn duplicating_an_out_of_range_emitter_changes_nothing() {
        let mut asset = asset_with(1, vec![]);
        assert!(duplicate_emitter(&mut asset, 9, "Emitter").is_none());
        assert_eq!(asset.emitters.len(), 1);
    }

    // --- The two call sites, through the real observers ------------------
    //
    // The helper tests above prove the helpers are correct; these prove the
    // menu actually reaches them. Mutation-verified: restoring the bare
    // `asset.emitters.remove(index)` / `asset.emitters.insert(..)` this fix
    // wave replaced turns each of these red while every helper test above
    // stays green -- which is exactly the gap that let C2 ship.

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<ParticlesAsset>();
        app.init_resource::<DirtyState>();
        app.init_resource::<LastLoadedProject>();
        app
    }

    fn open(app: &mut App, asset: ParticlesAsset) -> Handle<ParticlesAsset> {
        let handle = app
            .world_mut()
            .resource_mut::<Assets<ParticlesAsset>>()
            .add(asset);
        app.insert_resource(EditorState {
            current_project: Some(handle.clone()),
            current_project_path: None,
            inspecting: None,
        });
        handle
    }

    #[test]
    fn confirming_an_emitter_delete_writes_a_file_the_loader_still_accepts() {
        let mut app = test_app();
        app.add_observer(on_delete_confirmed);

        let asset = asset_with(2, vec![emitter_drive(1), transform_drive(1)]);
        let handle = open(&mut app, asset);
        app.insert_resource(PendingDelete {
            kind: Inspectable::Emitter,
            index: 1,
        });

        let dialog = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(DialogActionEvent { entity: dialog });

        let assets = app.world().resource::<Assets<ParticlesAsset>>();
        let asset = assets.get(&handle).unwrap();
        assert_eq!(asset.emitters.len(), 1);
        assert!(
            validate_drives(asset).is_ok(),
            "the delete path must not leave a drive the loader rejects: {:?}",
            validate_drives(asset)
        );
    }

    #[test]
    fn duplicating_through_the_menu_appends_and_leaves_the_drives_pointing_where_they_did() {
        let mut app = test_app();
        app.add_observer(on_item_menu_change);

        let mut asset = asset_with(2, vec![emitter_drive(1)]);
        asset.emitters[0].name = "Emitter".into();
        asset.emitters[1].name = "Sparks".into();
        let handle = open(&mut app, asset);

        // `on_item_menu_change` reaches the item through the menu's parent and
        // requires the item to have `Children`, so the item carries a button
        // child the way a real row does.
        let item = app
            .world_mut()
            .spawn(InspectableItem {
                kind: Inspectable::Emitter,
                index: 0,
            })
            .id();
        let button = app.world_mut().spawn((ItemButton, Node::default())).id();
        app.world_mut().entity_mut(item).add_child(button);
        let menu = app.world_mut().spawn((ItemMenu, ChildOf(item))).id();

        app.world_mut().trigger(ComboBoxChangeEvent {
            entity: menu,
            selected: 0,
            label: "Duplicate".to_string(),
            value: None,
        });

        let assets = app.world().resource::<Assets<ParticlesAsset>>();
        let asset = assets.get(&handle).unwrap();
        assert_eq!(asset.emitters.len(), 3, "the copy must exist");
        assert_eq!(
            asset.emitters[1].name, "Sparks",
            "an insert would have pushed Sparks up to index 2"
        );
        assert!(
            matches!(asset.drives[0].target, DriveTarget::Emitter { index: 1, .. }),
            "the drive must still address Sparks, not the copy: {:?}",
            asset.drives[0].target
        );
    }
}
