use bevy::prelude::*;

use crate::ui::tokens::TEXT_BODY_COLOR;

const DEFAULT_ALPHA: f32 = 0.1;

#[derive(Component, Default, Clone)]
pub struct EditorSeparator;

#[derive(Clone, Copy, Default)]
pub enum SeparatorDirection {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Default)]
pub struct SeparatorProps {
    pub direction: SeparatorDirection,
    pub alpha: f32,
}

impl SeparatorProps {
    pub fn vertical() -> Self {
        Self {
            direction: SeparatorDirection::Vertical,
            alpha: DEFAULT_ALPHA,
        }
    }

    pub fn horizontal() -> Self {
        Self {
            direction: SeparatorDirection::Horizontal,
            alpha: DEFAULT_ALPHA,
        }
    }
}

pub fn separator(props: SeparatorProps) -> impl Scene {
    let (width, height) = match props.direction {
        SeparatorDirection::Vertical => (px(1), px(24)),
        SeparatorDirection::Horizontal => (percent(100), px(1)),
    };

    bsn! {
        EditorSeparator
        Node {
            width: { width },
            height: { height },
        }
        BackgroundColor({ TEXT_BODY_COLOR.with_alpha(props.alpha) })
    }
}

impl EditorSeparator {
    pub fn vertical() -> impl Scene {
        separator(SeparatorProps::vertical())
    }

    pub fn horizontal() -> impl Scene {
        separator(SeparatorProps::horizontal())
    }
}
