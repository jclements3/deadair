//! Inheritable state (`StateSet`): material and thermal attributes that
//! flow down the graph, OSG-style. A field set on a child overrides the
//! same field inherited from an ancestor; unset (`None`) fields inherit.

use da_core::TempF;
use glam::{Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// Thermal properties for the 1 Hz thermal simulation (SDD §2/§10).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThermalAttach {
    /// Equilibrium temperature the object relaxes toward.
    pub base_temp: TempF,
    /// Heat capacity proxy; larger = slower temperature change.
    pub thermal_mass: f32,
    /// 0..1 fraction of sky visible (drives radiative night cooling).
    pub sky_exposure: f32,
}

/// A partial bundle of render/thermal state attachable to any node.
///
/// Every field is optional; `None` means "inherit from ancestors". The
/// effective state at a node is the override-merge of all `StateSet`s on
/// the path from the root to the node (see [`crate::Scene::effective_state`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StateSet {
    /// Base color, linear RGBA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_color: Option<Vec4>,
    /// Emissive color, linear RGB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emissive: Option<Vec3>,
    /// Metalness, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<f32>,
    /// Perceptual roughness, 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f32>,
    /// True for glass surfaces (blocks LWIR — thermal optics see glass as
    /// opaque).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<bool>,
    /// Thermal simulation attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal: Option<ThermalAttach>,
}

impl StateSet {
    /// A `StateSet` with no fields set (inherits everything).
    pub fn new() -> Self {
        Self::default()
    }

    /// True if no field is set.
    pub fn is_unset(&self) -> bool {
        *self == Self::default()
    }

    /// Returns `self` (the inherited state) with every field that is set in
    /// `over` replaced by `over`'s value — the child-overrides-parent merge.
    pub fn merged_with(&self, over: &StateSet) -> StateSet {
        StateSet {
            base_color: over.base_color.or(self.base_color),
            emissive: over.emissive.or(self.emissive),
            metallic: over.metallic.or(self.metallic),
            roughness: over.roughness.or(self.roughness),
            glass: over.glass.or(self.glass),
            thermal: over.thermal.or(self.thermal),
        }
    }

    /// Builder: set the base color.
    #[must_use]
    pub fn with_base_color(mut self, rgba: Vec4) -> Self {
        self.base_color = Some(rgba);
        self
    }

    /// Builder: set the emissive color.
    #[must_use]
    pub fn with_emissive(mut self, rgb: Vec3) -> Self {
        self.emissive = Some(rgb);
        self
    }

    /// Builder: set metalness.
    #[must_use]
    pub fn with_metallic(mut self, m: f32) -> Self {
        self.metallic = Some(m);
        self
    }

    /// Builder: set roughness.
    #[must_use]
    pub fn with_roughness(mut self, r: f32) -> Self {
        self.roughness = Some(r);
        self
    }

    /// Builder: set the glass flag.
    #[must_use]
    pub fn with_glass(mut self, glass: bool) -> Self {
        self.glass = Some(glass);
        self
    }

    /// Builder: set the thermal attachment.
    #[must_use]
    pub fn with_thermal(mut self, t: ThermalAttach) -> Self {
        self.thermal = Some(t);
        self
    }
}
