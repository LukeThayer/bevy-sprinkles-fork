mod accelerations;
mod angle;
mod collider_properties;
mod collision;
mod colors;
mod draw_pass;
// `pub(crate)`: `components::drives` (Task 20's flat list) reuses several of
// this module's row-editing pieces verbatim (`spawn_drive_row`, its marker
// components, `stage_label`) rather than reimplementing them.
pub(crate) mod drive_button;
mod emission;
mod light;
mod material_fx;
mod particle_flags;
mod project_properties;
mod scale;
mod settings_properties;
mod sub_emitter;
mod time;
mod trail;
mod transform;
mod turbulence;
pub mod types;
pub mod utils;
mod variable;
mod velocities;
mod visibility_aabb;

pub use types::{ComboBoxOption, FieldKind, VariantField};
pub use utils::{name_to_label, path_to_label};

use bevy::prelude::*;
use bevy_sprinkles::asset::EmitterProp;
use bevy_sprinkles::prelude::*;

use crate::state::{ActiveSidebarTab, EditorState, Inspectable, SidebarTab};
use crate::ui::icons::{ICON_BOX, ICON_HASHTAG, ICON_INFORMATION, ICON_SHOWERS};
use crate::ui::tokens::{
    BORDER_COLOR, FONT_PATH, TEXT_BODY_COLOR, TEXT_MUTED_COLOR, TEXT_SIZE_LG, TEXT_SIZE_SM,
};
use crate::ui::widgets::checkbox::{CheckboxProps, checkbox};
use crate::ui::widgets::combobox::{ComboBoxOptionData, combobox_with_selected};
use crate::ui::widgets::inspector_field::{InspectorFieldProps, fields_row, spawn_inspector_field};
use crate::ui::widgets::panel::{PanelDirection, PanelProps, panel};
use crate::ui::widgets::panel_section::{PanelSectionProps, PanelSectionSize, panel_section};
use crate::ui::widgets::scroll::scrollbar;
use crate::ui::widgets::variant_edit::{VariantEditProps, spawn_field_widget, variant_edit};

use super::binding::FieldBinding;

pub fn plugin(app: &mut App) {
    app.init_resource::<InspectedEmitterTracker>()
        .init_resource::<InspectedColliderTracker>()
        .init_resource::<InspectedLightTracker>()
        .add_plugins((
            super::binding::plugin,
            time::plugin,
            emission::plugin,
            draw_pass::plugin,
            scale::plugin,
            angle::plugin,
            colors::plugin,
            velocities::plugin,
            accelerations::plugin,
            turbulence::plugin,
            trail::plugin,
            collision::plugin,
            sub_emitter::plugin,
            particle_flags::plugin,
            collider_properties::plugin,
        ))
        .add_plugins(variable::plugin)
        .add_plugins(light::plugin)
        .add_plugins(project_properties::plugin)
        .add_plugins(visibility_aabb::plugin)
        .add_plugins(drive_button::plugin)
        .add_plugins(material_fx::plugin)
        .add_systems(
            Update,
            (
                (
                    update_inspected_emitter_tracker,
                    update_inspected_collider_tracker,
                    update_inspected_light_tracker,
                ),
                (
                    cleanup_dynamic_sections,
                    setup_inspector_panel,
                    update_panel_title,
                    setup_inspector_section_fields,
                    toggle_inspector_content,
                )
                    .after(update_inspected_emitter_tracker)
                    .after(update_inspected_collider_tracker)
                    .after(update_inspected_light_tracker),
            ),
        );
}

#[derive(Resource, Default)]
pub struct InspectedEmitterTracker {
    pub current_index: Option<u8>,
}

#[derive(Resource, Default)]
pub struct InspectedColliderTracker {
    pub current_index: Option<u8>,
}

#[derive(Resource, Default)]
pub struct InspectedLightTracker {
    pub current_index: Option<u8>,
}

pub(super) fn update_inspected_emitter_tracker(
    editor_state: Res<EditorState>,
    mut tracker: ResMut<InspectedEmitterTracker>,
) {
    let new_index = editor_state
        .inspecting
        .as_ref()
        .filter(|i| i.kind == Inspectable::Emitter)
        .map(|i| i.index);

    if tracker.current_index != new_index {
        tracker.current_index = new_index;
    } else if editor_state.is_changed() {
        tracker.set_changed();
    }
}

pub(super) fn update_inspected_collider_tracker(
    editor_state: Res<EditorState>,
    mut tracker: ResMut<InspectedColliderTracker>,
) {
    let new_index = editor_state
        .inspecting
        .as_ref()
        .filter(|i| i.kind == Inspectable::Collider)
        .map(|i| i.index);

    if tracker.current_index != new_index {
        tracker.current_index = new_index;
    } else if editor_state.is_changed() {
        tracker.set_changed();
    }
}

pub(super) fn update_inspected_light_tracker(
    editor_state: Res<EditorState>,
    mut tracker: ResMut<InspectedLightTracker>,
) {
    let new_index = editor_state
        .inspecting
        .as_ref()
        .filter(|i| i.kind == Inspectable::Light)
        .map(|i| i.index);

    if tracker.current_index != new_index {
        tracker.current_index = new_index;
    } else if editor_state.is_changed() {
        tracker.set_changed();
    }
}

#[derive(Component, Default, Clone)]
pub struct EditorInspectorPanel;

#[derive(Component)]
struct InspectorPanelContent;

#[derive(Component, PartialEq, Eq)]
enum InspectorContentKind {
    Emitter,
    Collider,
    Variable,
    Light,
    Project,
    Settings,
    EnabledCheckbox,
}

#[derive(Component)]
struct PanelTitleText;

#[derive(Component)]
struct PanelTitleIcon;

#[derive(Component)]
pub(super) struct DynamicSectionContent;

pub fn inspector_panel() -> impl Scene {
    bsn! {
        EditorInspectorPanel
        panel(
            PanelProps::new(PanelDirection::Left)
                .with_width(320)
                .with_min_width(320)
                .with_max_width(512),
        )
    }
}

fn setup_inspector_panel(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    panels: Query<Entity, Added<EditorInspectorPanel>>,
) {
    for panel_entity in &panels {
        commands
            .entity(panel_entity)
            .with_child(scrollbar(panel_entity))
            .with_children(|parent| {
                let parent_target = parent.target_entity();
                spawn_panel_title(&mut parent.commands(), &asset_server, parent_target);

                parent
                    .spawn((
                        InspectorPanelContent,
                        Node {
                            width: percent(100),
                            flex_direction: FlexDirection::Column,
                            ..default()
                        },
                    ))
                    .with_children(|content| {
                        content
                            .spawn((
                                InspectorContentKind::Emitter,
                                Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Column,
                                    ..default()
                                },
                            ))
                            .with_children(|emitter_content| {
                                spawn_section(emitter_content, time::time_section());
                                spawn_section(emitter_content, draw_pass::draw_pass_section());
                                spawn_section(emitter_content, material_fx::scroll_section());
                                spawn_section(emitter_content, material_fx::flow_section());
                                spawn_section(emitter_content, material_fx::erosion_section());
                                spawn_section(emitter_content, material_fx::fresnel_section());
                                spawn_section(emitter_content, material_fx::soft_section());
                                spawn_section(
                                    emitter_content,
                                    material_fx::gradient_remap_section(),
                                );
                                spawn_section(
                                    emitter_content,
                                    visibility_aabb::visibility_aabb_section(),
                                );
                                spawn_section(emitter_content, emission::emission_section());
                                spawn_section(emitter_content, scale::scale_section());
                                spawn_section(emitter_content, colors::colors_section());

                                let (extra, section) = velocities::velocities_section();
                                let props =
                                    inspector_section_props(&section.title).with_add_button();
                                spawn_section_with(emitter_content, props, extra, section);

                                spawn_section(emitter_content, angle::angle_section());
                                spawn_section(
                                    emitter_content,
                                    accelerations::accelerations_section(),
                                );
                                spawn_section(emitter_content, turbulence::turbulence_section());
                                spawn_section(emitter_content, trail::trail_section());
                                spawn_section(emitter_content, collision::collision_section());
                                spawn_section(emitter_content, sub_emitter::sub_emitter_section());
                                spawn_section(
                                    emitter_content,
                                    particle_flags::particle_flags_section(),
                                );
                                spawn_section(emitter_content, transform::transform_section());
                            });

                        content
                            .spawn((
                                InspectorContentKind::Collider,
                                Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Column,
                                    display: Display::None,
                                    ..default()
                                },
                            ))
                            .with_children(|collider_content| {
                                spawn_section(
                                    collider_content,
                                    collider_properties::collider_properties_section(),
                                );
                                spawn_section(collider_content, transform::transform_section());
                            });

                        content
                            .spawn((
                                InspectorContentKind::Variable,
                                Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Column,
                                    display: Display::None,
                                    ..default()
                                },
                            ))
                            .with_children(|variable_content| {
                                spawn_section(variable_content, variable::variable_section());
                            });

                        content
                            .spawn((
                                InspectorContentKind::Light,
                                Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Column,
                                    display: Display::None,
                                    ..default()
                                },
                            ))
                            .with_children(|light_content| {
                                spawn_section(light_content, light::light_section());
                                spawn_section(light_content, light::light_transform_section());
                                spawn_section(light_content, light::light_time_section());
                                spawn_section(light_content, light::light_color_section());
                            });

                        content
                            .spawn((
                                InspectorContentKind::Project,
                                Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Column,
                                    display: Display::None,
                                    ..default()
                                },
                            ))
                            .with_children(|project_content| {
                                spawn_section(
                                    project_content,
                                    project_properties::project_properties_section(),
                                );
                                spawn_section(
                                    project_content,
                                    project_properties::project_runtime_section(),
                                );
                                spawn_section(
                                    project_content,
                                    transform::asset_transform_section(),
                                );
                            });

                        content
                            .spawn((
                                InspectorContentKind::Settings,
                                Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Column,
                                    display: Display::None,
                                    ..default()
                                },
                            ))
                            .with_children(|settings_content| {
                                let settings_target = settings_content.target_entity();
                                settings_properties::spawn_settings_properties_section(
                                    &mut settings_content.commands(),
                                    settings_target,
                                );
                            });
                    });
            });
    }
}

fn toggle_inspector_content(
    editor_state: Res<EditorState>,
    active_tab: Res<ActiveSidebarTab>,
    mut content: Query<(&mut Node, &InspectorContentKind)>,
) {
    if !editor_state.is_changed() && !active_tab.is_changed() {
        return;
    }

    let inspecting_kind = if active_tab.0 == SidebarTab::Outliner {
        editor_state.inspecting.as_ref().map(|i| i.kind)
    } else {
        None
    };

    for (mut node, kind) in &mut content {
        let visible = match kind {
            InspectorContentKind::Emitter => inspecting_kind == Some(Inspectable::Emitter),
            InspectorContentKind::Collider => inspecting_kind == Some(Inspectable::Collider),
            InspectorContentKind::Variable => inspecting_kind == Some(Inspectable::Variable),
            InspectorContentKind::Light => inspecting_kind == Some(Inspectable::Light),
            InspectorContentKind::Project => {
                active_tab.0 == SidebarTab::Project && editor_state.current_project.is_some()
            }
            InspectorContentKind::Settings => active_tab.0 == SidebarTab::Settings,
            InspectorContentKind::EnabledCheckbox => inspecting_kind.is_some(),
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

pub(crate) fn set_display_visible(node: &mut Node, visible: bool) {
    let display = if visible {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
}

fn spawn_panel_title(commands: &mut Commands, asset_server: &AssetServer, parent: Entity) {
    let font: Handle<Font> = asset_server.load(FONT_PATH);

    let title = commands
        .spawn((
            Node {
                width: percent(100),
                align_items: AlignItems::Center,
                column_gap: px(12.0),
                padding: UiRect::axes(px(24.0), px(20.0)),
                border: UiRect::bottom(px(1.0)),
                ..default()
            },
            BorderColor::all(BORDER_COLOR),
            ChildOf(parent),
        ))
        .id();

    let left = commands
        .spawn((
            Node {
                align_items: AlignItems::Center,
                column_gap: px(6.0),
                flex_grow: 1.0,
                ..default()
            },
            ChildOf(title),
        ))
        .id();

    commands.spawn((
        PanelTitleIcon,
        ImageNode::new(asset_server.load(ICON_SHOWERS)).with_color(Color::Srgba(TEXT_BODY_COLOR)),
        Node {
            width: px(16.0),
            height: px(16.0),
            ..default()
        },
        ChildOf(left),
    ));
    commands.spawn((
        PanelTitleText,
        Text::new(""),
        TextFont {
            font: font.into(),
            font_size: TEXT_SIZE_LG.into(),
            weight: FontWeight::SEMIBOLD,
            ..default()
        },
        TextColor(TEXT_BODY_COLOR.into()),
        ChildOf(left),
    ));

    commands
        .spawn_scene(checkbox(CheckboxProps::new("Enabled").checked(true)))
        .insert((
            InspectorContentKind::EnabledCheckbox,
            FieldBinding::emitter("enabled", FieldKind::Bool),
        ))
        .insert(ChildOf(title));
}

pub enum InspectorItem {
    Field(InspectorFieldProps),
    Variant {
        path: String,
        props: VariantEditProps,
    },
    /// A numeric field paired with the drive affordance for the
    /// `EmitterProp` it authors -- see `drive_button`'s module doc. Kept as
    /// its own variant (rather than a builder method on `InspectorFieldProps`)
    /// so the widgets crate does not need to know about `EmitterProp`.
    Driven {
        field: InspectorFieldProps,
        prop: EmitterProp,
    },
    /// A field reached through the enclosing draw-pass material's own
    /// `VariantField` accessor rather than a bare emitter path -- Task 22's
    /// `FxSettings` fields, which live at `draw_pass.material.fx.<name>`,
    /// two hops inside the `StandardParticleMaterial` a
    /// `DrawPassMaterial::Standard` tuple variant carries. Dispatches
    /// through `variant_edit::spawn_field_widget` (Task 19/20's per-
    /// `FieldKind` widget picker), the same dispatch every OTHER variant-
    /// carried field in this inspector already goes through, rather than
    /// re-deriving a widget per `FieldKind` here. `drives` lists the
    /// `EmitterProp`s (zero, one, or -- for a vector field whose components
    /// are driven independently, like `fx.scroll`'s `ScrollU`/`ScrollV` --
    /// two) that get a drive button appended after the field.
    MaterialField {
        base_path: &'static str,
        field: VariantField,
        drives: Vec<EmitterProp>,
    },
}

impl From<InspectorFieldProps> for InspectorItem {
    fn from(props: InspectorFieldProps) -> Self {
        Self::Field(props)
    }
}

#[derive(Component)]
pub struct InspectorSection {
    pub title: String,
    pub rows: Vec<Vec<InspectorItem>>,
    initialized: bool,
}

impl InspectorSection {
    pub fn new(title: impl Into<String>, rows: Vec<Vec<InspectorItem>>) -> Self {
        Self {
            title: title.into(),
            rows,
            initialized: false,
        }
    }

    /// Creates a section where each field occupies its own row.
    pub fn from_fields(title: impl Into<String>, fields: Vec<InspectorItem>) -> Self {
        Self {
            title: title.into(),
            rows: fields.into_iter().map(|f| vec![f]).collect(),
            initialized: false,
        }
    }
}

pub(super) fn section_needs_setup<S: Component, C: Component>(
    sections: &Query<(Entity, &InspectorSection), With<S>>,
    existing: &Query<Entity, With<C>>,
) -> Option<Entity> {
    let Ok((entity, section)) = sections.single() else {
        return None;
    };
    if !section.initialized || !existing.is_empty() {
        return None;
    }
    Some(entity)
}

fn inspector_section_props(title: &str) -> PanelSectionProps {
    PanelSectionProps::new(title)
        .collapsible()
        .with_size(PanelSectionSize::XL)
}

pub(super) fn spawn_section(
    content: &mut ChildSpawnerCommands,
    parts: (impl Bundle, InspectorSection),
) {
    let (extra, section) = parts;
    spawn_section_with(
        content,
        inspector_section_props(&section.title),
        extra,
        section,
    );
}

pub(super) fn spawn_section_with(
    content: &mut ChildSpawnerCommands,
    props: PanelSectionProps,
    extra: impl Bundle,
    section: InspectorSection,
) {
    let target = content.target_entity();
    content
        .commands()
        .spawn_scene(panel_section(props))
        .insert(extra)
        .insert(section)
        .insert(ChildOf(target));
}

fn setup_inspector_section_fields(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut sections: Query<(Entity, &mut InspectorSection)>,
) {
    for (entity, mut section) in &mut sections {
        if section.initialized {
            continue;
        }
        section.initialized = true;

        let rows = std::mem::take(&mut section.rows);

        commands.entity(entity).with_children(|parent| {
            for row_items in rows {
                parent.spawn(fields_row()).with_children(|row| {
                    for item in row_items {
                        match item {
                            InspectorItem::Field(props) => {
                                spawn_inspector_field(row, props, &asset_server);
                            }
                            InspectorItem::Variant { path, props } => {
                                let row_target = row.target_entity();
                                row.commands()
                                    .spawn_scene(variant_edit(props))
                                    .insert(FieldBinding::emitter(&path, FieldKind::default()))
                                    .insert(ChildOf(row_target));
                            }
                            InspectorItem::Driven { field, prop } => {
                                spawn_inspector_field(row, field, &asset_server);
                                let row_target = row.target_entity();
                                row.commands()
                                    .spawn_scene(drive_button::drive_button(
                                        drive_button::DriveButtonProps::new(prop),
                                    ))
                                    .insert(ChildOf(row_target));
                            }
                            InspectorItem::MaterialField {
                                base_path,
                                field,
                                drives,
                            } => {
                                let row_target = row.target_entity();
                                let binding = FieldBinding::emitter_variant_field(
                                    base_path,
                                    &field.name,
                                    field.kind.clone(),
                                );
                                let label = path_to_label(&field.name);
                                let mut cmds = row.commands();
                                let widget_entity = spawn_field_widget(
                                    &mut cmds,
                                    &asset_server,
                                    &field,
                                    label,
                                    binding,
                                );
                                cmds.entity(widget_entity).insert(ChildOf(row_target));
                                for prop in drives {
                                    cmds.spawn_scene(drive_button::drive_button(
                                        drive_button::DriveButtonProps::new(prop),
                                    ))
                                    .insert(ChildOf(row_target));
                                }
                            }
                        }
                    }
                });
            }
        });
    }
}

fn get_outliner_title(
    editor_state: &EditorState,
    assets: &Assets<ParticlesAsset>,
) -> Option<(String, &'static str)> {
    let inspecting = editor_state.inspecting.as_ref()?;
    let handle = editor_state.current_project.as_ref()?;
    let asset = assets.get(handle)?;
    Some(match inspecting.kind {
        Inspectable::Emitter => {
            let emitter = asset.emitters.get(inspecting.index as usize);
            let name = emitter.map(|e| e.name.clone()).unwrap_or_default();
            (name, ICON_SHOWERS)
        }
        Inspectable::Collider => {
            let collider = asset.colliders.get(inspecting.index as usize);
            let name = collider.map(|c| c.name.clone()).unwrap_or_default();
            (name, ICON_BOX)
        }
        Inspectable::Variable => {
            let variable = asset.variables.get(inspecting.index as usize);
            let name = variable.map(|v| v.name.clone()).unwrap_or_default();
            (name, ICON_HASHTAG)
        }
        Inspectable::Light => {
            let light = asset.lights.get(inspecting.index as usize);
            let name = light.map(|l| l.name.clone()).unwrap_or_default();
            // No dedicated light-bulb icon asset exists yet; the info glyph
            // is a placeholder rather than a claim this is the right icon.
            (name, ICON_INFORMATION)
        }
    })
}

fn update_panel_title(
    editor_state: Res<EditorState>,
    active_tab: Res<ActiveSidebarTab>,
    assets: Res<Assets<ParticlesAsset>>,
    mut title_text: Query<&mut Text, With<PanelTitleText>>,
    mut title_icon: Query<&mut ImageNode, With<PanelTitleIcon>>,
    asset_server: Res<AssetServer>,
    new_titles: Query<Entity, Added<PanelTitleText>>,
) {
    let should_update =
        editor_state.is_changed() || active_tab.is_changed() || !new_titles.is_empty();
    if !should_update {
        return;
    }

    let (name, icon_path) = match active_tab.0 {
        SidebarTab::Outliner => get_outliner_title(&editor_state, &assets).unwrap_or_else(|| {
            (
                SidebarTab::Outliner.label().to_string(),
                SidebarTab::Outliner.icon(),
            )
        }),
        tab => (tab.label().to_string(), tab.icon()),
    };

    for mut text in &mut title_text {
        **text = name.clone();
    }

    for mut icon in &mut title_icon {
        icon.image = asset_server.load(icon_path);
    }
}

fn cleanup_dynamic_sections(
    mut commands: Commands,
    emitter_tracker: Res<InspectedEmitterTracker>,
    collider_tracker: Res<InspectedColliderTracker>,
    light_tracker: Res<InspectedLightTracker>,
    existing: Query<Entity, With<DynamicSectionContent>>,
) {
    if !emitter_tracker.is_changed() && !collider_tracker.is_changed() && !light_tracker.is_changed()
    {
        return;
    }

    for entity in &existing {
        commands.entity(entity).try_despawn();
    }
}

pub(super) fn spawn_labeled_combobox(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    label: &str,
    options: Vec<ComboBoxOptionData>,
    selected: usize,
    marker: impl Bundle,
) {
    parent.spawn(fields_row()).with_children(|row| {
        row.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            flex_grow: 1.0,
            flex_shrink: 1.0,
            flex_basis: Val::Px(0.0),
            ..default()
        })
        .with_children(|wrapper| {
            wrapper.spawn((
                Text::new(label),
                TextFont {
                    font: font.clone().into(),
                    font_size: TEXT_SIZE_SM.into(),
                    weight: FontWeight::MEDIUM,
                    ..default()
                },
                TextColor(TEXT_MUTED_COLOR.into()),
            ));
            let wrapper_target = wrapper.target_entity();
            wrapper
                .commands()
                .spawn_scene(combobox_with_selected(options, selected))
                .insert(marker)
                .insert(ChildOf(wrapper_target));
        });
    });
}
