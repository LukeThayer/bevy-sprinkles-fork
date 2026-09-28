use bevy::prelude::*;
use bevy_sprinkles::asset::EmitterProp;

use crate::ui::widgets::inspector_field::InspectorFieldProps;
use crate::ui::widgets::vector_edit::VectorSuffixes;

use super::{InspectorItem, InspectorSection};

pub fn plugin(_app: &mut App) {}

pub fn scale_section() -> (impl Bundle, InspectorSection) {
    (
        (),
        InspectorSection::new(
            "Scale",
            vec![
                vec![InspectorItem::Driven {
                    field: InspectorFieldProps::new("scale.range")
                        .vector(VectorSuffixes::Range)
                        .with_label("Initial scale ratio"),
                    prop: EmitterProp::SpawnSize,
                }],
                vec![InspectorItem::Driven {
                    field: InspectorFieldProps::new("scale.scale_over_lifetime").curve(),
                    prop: EmitterProp::SizeMul,
                }],
            ],
        ),
    )
}
