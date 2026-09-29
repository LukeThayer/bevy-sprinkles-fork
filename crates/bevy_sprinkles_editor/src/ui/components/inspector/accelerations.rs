use bevy::prelude::*;
use bevy_sprinkles::asset::EmitterProp;

use crate::ui::widgets::inspector_field::InspectorFieldProps;
use crate::ui::widgets::vector_edit::VectorSuffixes;

use super::{InspectorItem, InspectorSection};

pub fn plugin(_app: &mut App) {}

pub fn accelerations_section() -> (impl Bundle, InspectorSection) {
    (
        (),
        InspectorSection::new(
            "Accelerations",
            vec![vec![InspectorItem::Driven {
                field: InspectorFieldProps::new("accelerations.gravity")
                    .vector(VectorSuffixes::XYZ),
                prop: EmitterProp::Gravity,
            }]],
        ),
    )
}
