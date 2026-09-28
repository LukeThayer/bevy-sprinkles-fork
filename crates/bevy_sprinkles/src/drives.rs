use std::collections::HashMap;

use bevy::prelude::*;

use crate::asset::VariableDecl;

/// The host game's per-instance knob values for one effect entity.
///
/// Attach to the same entity that holds [`Particles3d`](crate::Particles3d).
/// Two entities sharing one asset with different values here render
/// differently — that per-instance isolation is the point of the whole
/// variable system, and it is why this is a `Component` and not a `Resource`.
///
/// Keyed by name rather than by [`VariableId`](crate::asset::VariableId)
/// because the host should not have to know an effect's internal ordering, and
/// an effect swapped underneath must not silently reinterpret the host's
/// numbers as different knobs. Name resolution happens once per frame against
/// the asset's declarations, which is cheap next to everything else in a frame.
#[derive(Component, Clone, Default, Debug)]
pub struct ParticleVariables(HashMap<String, f32>);

impl ParticleVariables {
    /// Sets the value for a named variable, overwriting any previous value.
    pub fn set(&mut self, name: &str, value: f32) {
        self.0.insert(name.to_string(), value);
    }

    /// Returns the raw value set for `name`, or `None` if it was never set.
    ///
    /// This does not consult declarations or defaults — use
    /// [`resolve_values`](Self::resolve_values) for that.
    pub fn get(&self, name: &str) -> Option<f32> {
        self.0.get(name).copied()
    }

    /// Clears every value this host has set, reverting all variables to their
    /// declared defaults on the next resolve.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// One value per declaration, in declaration order, so the result indexes
    /// directly by `VariableId`.
    ///
    /// A name this effect does not declare is ignored rather than fatal: hosts
    /// legitimately hold one variable map across effect swaps. A non-finite
    /// value falls back to the default, because NaN reaching a uniform
    /// propagates through the vertex shader and can blank the whole draw —
    /// which an author reads as "my effect disappeared", with nothing pointing
    /// at the number that caused it.
    pub fn resolve_values(&self, decls: &[VariableDecl]) -> Vec<f32> {
        decls
            .iter()
            .map(|d| match self.0.get(&d.name) {
                Some(v) if v.is_finite() => *v,
                _ => d.default,
            })
            .collect()
    }

    /// Names set here that no declaration matches. Used for a warn-once
    /// diagnostic; a typo'd knob is otherwise completely silent.
    pub fn unknown_names<'a>(&'a self, decls: &[VariableDecl]) -> Vec<&'a str> {
        self.0
            .keys()
            .filter(|k| !decls.iter().any(|d| &d.name == *k))
            .map(|k| k.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{Range, VariableDecl};

    fn decls() -> Vec<VariableDecl> {
        vec![
            VariableDecl { name: "temperature".into(), default: 0.25, range: Range { min: 0.0, max: 1.0 } },
            VariableDecl { name: "charge".into(), default: 0.5, range: Range { min: 0.0, max: 1.0 } },
        ]
    }

    #[test]
    fn an_unset_variable_resolves_to_its_declared_default() {
        let v = ParticleVariables::default();
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.5]);
    }

    #[test]
    fn a_set_variable_wins_over_its_default() {
        let mut v = ParticleVariables::default();
        v.set("charge", 0.9);
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.9]);
    }

    #[test]
    fn a_name_the_effect_does_not_declare_is_ignored_not_fatal() {
        // A host that swaps an effect underneath, or fat-fingers a name, gets
        // the declared defaults and a warning -- not a panic and not silence.
        let mut v = ParticleVariables::default();
        v.set("temprature", 0.9); // typo
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.5]);
    }

    #[test]
    fn a_non_finite_host_value_falls_back_to_the_default() {
        // NaN reaching a uniform propagates through the vertex shader and can
        // blank a whole draw call, which reads as "the effect vanished".
        let mut v = ParticleVariables::default();
        v.set("temperature", f32::NAN);
        v.set("charge", f32::INFINITY);
        assert_eq!(v.resolve_values(&decls()), vec![0.25, 0.5]);
    }

    #[test]
    fn two_instances_do_not_share_state() {
        let mut a = ParticleVariables::default();
        let mut b = ParticleVariables::default();
        a.set("temperature", 1.0);
        b.set("temperature", 0.0);
        assert_eq!(a.resolve_values(&decls())[0], 1.0);
        assert_eq!(b.resolve_values(&decls())[0], 0.0);
    }
}
