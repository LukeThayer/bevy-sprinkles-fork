use bevy::input::mouse::MouseMotion;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use bevy::window::SystemCursorIcon;

use crate::ui::tokens::{BACKGROUND_COLOR, BORDER_COLOR};
use crate::ui::widgets::cursor::{ActiveCursor, HoverCursor};

const RESIZE_HANDLE_WIDTH: u32 = 12;

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            spawn_resize_handles,
            sync_resize_handle_positions,
            handle_resize_drag,
        ),
    );
}

#[derive(Component, Default, Clone)]
pub struct EditorPanel;

/// Which edge of the main row a panel is pinned to. Three things have to
/// agree or the resize handle detaches from the panel it resizes: which side
/// carries the 1px border, which edge the handle straddles, and which way a
/// drag widens. Every `match` on this is exhaustive with no wildcard arm, so
/// a third edge is a compile error until it has answered all three.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelDirection {
    #[default]
    Left,
    /// Pinned to the far right -- the Drives dock, and so far only it. Its
    /// handle sits on its LEFT edge, which is why [`resize_delta`] exists.
    Right,
}

/// How far a panel's width grows for a rightwards cursor motion of
/// `cursor_delta`.
///
/// A left panel's handle is on its right edge, so the two agree. A right
/// panel's handle is on its LEFT edge, so dragging that edge rightwards makes
/// the panel NARROWER and the sign flips. Pulled out of `handle_resize_drag`
/// as a pure function because the failure it guards against -- a panel that
/// shrinks when you pull it open -- is invisible to the type system and
/// unreachable by a test while the arithmetic sits inside a drag loop.
pub fn resize_delta(direction: PanelDirection, cursor_delta: f32) -> f32 {
    match direction {
        PanelDirection::Left => cursor_delta,
        PanelDirection::Right => -cursor_delta,
    }
}

/// The x of the edge a panel's resize handle straddles, given the panel's
/// centre and half-width in its parent's space. The other half of
/// [`resize_delta`]'s pairing: handle placement and drag sign must come from
/// the same `direction`.
pub fn panel_handle_edge(direction: PanelDirection, center: f32, half_width: f32) -> f32 {
    match direction {
        PanelDirection::Left => center + half_width,
        PanelDirection::Right => center - half_width,
    }
}

#[derive(Component, Default, Clone)]
pub struct PanelWidth {
    pub current: u32,
    pub min: u32,
    pub max: u32,
}

#[derive(Component)]
pub struct PanelResizeHandle {
    pub panel: Entity,
    pub direction: PanelDirection,
}

#[derive(Component, Default)]
pub struct ResizeDragState {
    pub dragging: bool,
    pub accumulated_delta: f32,
}

pub struct PanelProps {
    pub direction: PanelDirection,
    pub width: u32,
    pub min_width: u32,
    pub max_width: u32,
}

impl Default for PanelProps {
    fn default() -> Self {
        Self {
            direction: PanelDirection::default(),
            width: 250,
            min_width: 100,
            max_width: 500,
        }
    }
}

impl PanelProps {
    pub fn new(direction: PanelDirection) -> Self {
        Self {
            direction,
            ..default()
        }
    }

    pub fn with_width(mut self, width: u32) -> Self {
        self.width = width;
        self
    }

    pub fn with_min_width(mut self, min_width: u32) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn with_max_width(mut self, max_width: u32) -> Self {
        self.max_width = max_width;
        self
    }
}

pub fn panel(props: PanelProps) -> impl Scene {
    let PanelProps {
        direction,
        width,
        min_width,
        max_width,
    } = props;

    let border = match direction {
        PanelDirection::Left => UiRect::right(px(1)),
        PanelDirection::Right => UiRect::left(px(1)),
    };
    let margin = match direction {
        PanelDirection::Left | PanelDirection::Right => UiRect::ZERO,
    };

    bsn! {
        EditorPanel
        template_value(direction)
        template_value(PanelWidth {
            current: width,
            min: min_width,
            max: max_width,
        })
        Hovered
        Node {
            width: px(width),
            height: percent(100),
            min_height: px(0),
            flex_direction: { FlexDirection::Column },
            border: { border },
            margin: { margin },
            position_type: { PositionType::Relative },
            overflow: { Overflow::scroll_y() },
        }
        BackgroundColor({ BACKGROUND_COLOR })
        template_value(BorderColor::all(BORDER_COLOR))
    }
}

fn spawn_resize_handles(
    mut commands: Commands,
    panels: Query<(Entity, &PanelDirection, &ChildOf), Added<EditorPanel>>,
) {
    for (panel_entity, &direction, child_of) in &panels {
        let handle = commands
            .spawn((
                PanelResizeHandle {
                    panel: panel_entity,
                    direction,
                },
                ResizeDragState::default(),
                Hovered::default(),
                HoverCursor(SystemCursorIcon::ColResize),
                Node {
                    position_type: PositionType::Absolute,
                    width: px(RESIZE_HANDLE_WIDTH),
                    ..default()
                },
                ZIndex(10),
                Pickable {
                    should_block_lower: true,
                    is_hoverable: true,
                },
            ))
            .id();
        commands.entity(child_of.parent()).add_child(handle);
    }
}

fn sync_resize_handle_positions(
    panels: Query<(&PanelDirection, &UiGlobalTransform, &ComputedNode), With<EditorPanel>>,
    parents: Query<(&UiGlobalTransform, &ComputedNode), Without<EditorPanel>>,
    mut handles: Query<(&PanelResizeHandle, &ChildOf, &mut Node)>,
) {
    let half = (RESIZE_HANDLE_WIDTH / 2) as f32;

    for (handle, child_of, mut node) in &mut handles {
        let Ok((direction, panel_transform, panel_computed)) = panels.get(handle.panel) else {
            continue;
        };
        let Ok((parent_transform, parent_computed)) = parents.get(child_of.parent()) else {
            continue;
        };

        let scale = panel_computed.inverse_scale_factor();
        let panel_center = panel_transform.translation.x * scale;
        let parent_center = parent_transform.translation.x * scale;
        let panel_half_w = panel_computed.size().x * scale / 2.0;
        let parent_half_w = parent_computed.size().x * scale / 2.0;

        let parent_left = parent_center - parent_half_w;

        let panel_edge = panel_handle_edge(*direction, panel_center, panel_half_w);

        node.left = px(panel_edge - parent_left - half);
        node.top = px(0.0);
        node.height = percent(100);
    }
}

fn handle_resize_drag(
    mut commands: Commands,
    mut handles: Query<(Entity, &PanelResizeHandle, &mut ResizeDragState, &Hovered)>,
    mut panels: Query<(&mut Node, &mut PanelWidth), With<EditorPanel>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut mouse_motion: MessageReader<MouseMotion>,
) {
    let cursor_delta: f32 = mouse_motion.read().map(|e| e.delta.x).sum();

    for (entity, handle, mut drag_state, hovered) in &mut handles {
        if mouse.just_pressed(MouseButton::Left) && hovered.get() {
            drag_state.dragging = true;
            drag_state.accumulated_delta = 0.0;
            commands
                .entity(entity)
                .insert(ActiveCursor(SystemCursorIcon::ColResize));
        }

        if mouse.just_released(MouseButton::Left) {
            drag_state.dragging = false;
            commands.entity(entity).remove::<ActiveCursor>();
        }

        if drag_state.dragging && cursor_delta != 0.0 {
            if let Ok((mut node, mut panel_width)) = panels.get_mut(handle.panel) {
                let delta = resize_delta(handle.direction, cursor_delta);

                drag_state.accumulated_delta += delta;
                let new_width = ((panel_width.current as f32) + drag_state.accumulated_delta)
                    .clamp(panel_width.min as f32, panel_width.max as f32)
                    as u32;

                if new_width != panel_width.current {
                    drag_state.accumulated_delta = 0.0;
                    panel_width.current = new_width;
                    node.width = px(new_width);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Drives dock is the first `Right` panel, and its handle is on the
    /// opposite edge from every other panel's. A flipped sign here makes the
    /// dock shrink when the author pulls it open, which reads as "the panel
    /// is broken" rather than as an inverted axis.
    #[test]
    fn dragging_a_right_panels_handle_leftwards_widens_it() {
        assert_eq!(resize_delta(PanelDirection::Right, -10.0), 10.0);
        assert_eq!(resize_delta(PanelDirection::Left, -10.0), -10.0);
    }

    /// A right panel's handle straddles its left edge, a left panel's its
    /// right one -- the placement half of the same pairing.
    #[test]
    fn a_right_panels_handle_sits_on_its_left_edge() {
        assert_eq!(panel_handle_edge(PanelDirection::Right, 100.0, 30.0), 70.0);
        assert_eq!(panel_handle_edge(PanelDirection::Left, 100.0, 30.0), 130.0);
    }
}
