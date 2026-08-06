//! Canned [`StateSet`]s the generators attach: base color + thermal attach
//! in one bundle. Thermal numbers come from the [`da_thermal::ThermalProfile`]
//! presets wherever one exists — those presets are the source of truth for
//! how scenery reads through the thermal optic.

use da_core::TempF;
use da_graph::{StateSet, ThermalAttach};
use da_thermal::ThermalProfile;
use glam::{Vec3, Vec4};

use crate::source::{Biome, PropThermal, TreeKind};

/// Equilibrium temperature used for ambient-coupled scenery (profiles with
/// `base_temp: None`); the thermal sim relaxes objects toward ambient, this
/// is just the neutral starting point.
const EQ: TempF = TempF(50.0);

/// Convert a static [`ThermalProfile`] preset into the simpler per-node
/// [`ThermalAttach`] the scene graph carries.
fn attach(p: ThermalProfile) -> ThermalAttach {
    ThermalAttach {
        base_temp: p.base_temp.unwrap_or(EQ),
        thermal_mass: p.thermal_mass,
        sky_exposure: p.sky_exposure,
    }
}

/// Hand-rolled attach for materials without a preset (shingle, dry wood).
fn custom(thermal_mass: f32, sky_exposure: f32) -> ThermalAttach {
    ThermalAttach {
        base_temp: EQ,
        thermal_mass,
        sky_exposure,
    }
}

fn rgba(r: f32, g: f32, b: f32) -> Vec4 {
    Vec4::new(r, g, b, 1.0)
}

/// Thin metal roof panel: full sky exposure, low mass — reads below ambient
/// on clear nights.
pub(crate) fn metal_roof() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.62, 0.63, 0.66))
        .with_metallic(0.9)
        .with_roughness(0.45)
        .with_thermal(attach(ThermalProfile::metal_roof()))
}

/// General sheet-metal surface (silo barrel, dumpster, mast).
pub(crate) fn metal_surface() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.55, 0.55, 0.58))
        .with_metallic(0.85)
        .with_roughness(0.5)
        .with_thermal(attach(ThermalProfile::metal_roof()))
}

/// Building wall (masonry/wood) with the given tint.
pub(crate) fn building_wall(r: f32, g: f32, b: f32) -> StateSet {
    StateSet::new()
        .with_base_color(rgba(r, g, b))
        .with_metallic(0.0)
        .with_roughness(0.9)
        .with_thermal(attach(ThermalProfile::building_wall()))
}

/// Asphalt-shingle roof: sky-facing but slower to cool than metal.
pub(crate) fn shingle_roof() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.25, 0.22, 0.2))
        .with_roughness(0.95)
        .with_thermal(custom(1200.0, 0.85))
}

/// Weathered dry lumber (fence posts/rails, dock timbers, dam logs).
pub(crate) fn wood() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.45, 0.36, 0.26))
        .with_roughness(0.95)
        .with_thermal(custom(700.0, 0.5))
}

/// Live tree trunk.
pub(crate) fn tree_trunk() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.35, 0.27, 0.18))
        .with_roughness(1.0)
        .with_thermal(attach(ThermalProfile::tree()))
}

/// Tree canopy, tinted per species.
pub(crate) fn tree_canopy(kind: TreeKind) -> StateSet {
    let c = match kind {
        TreeKind::Oak => rgba(0.2, 0.35, 0.12),
        TreeKind::Pine => rgba(0.12, 0.28, 0.14),
        TreeKind::Sycamore => rgba(0.24, 0.38, 0.18),
        TreeKind::Apple => rgba(0.26, 0.42, 0.16),
        TreeKind::Maple => rgba(0.3, 0.4, 0.12),
    };
    StateSet::new()
        .with_base_color(c)
        .with_roughness(1.0)
        .with_thermal(attach(ThermalProfile::tree()))
}

/// Open water surface — enormous thermal mass.
pub(crate) fn water() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.1, 0.16, 0.2))
        .with_metallic(0.0)
        .with_roughness(0.1)
        .with_thermal(attach(ThermalProfile::water()))
}

/// Creek bank / bare earth strip (rock-like store of daytime heat).
pub(crate) fn bank_earth() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.32, 0.26, 0.2))
        .with_roughness(1.0)
        .with_thermal(attach(ThermalProfile::rock()))
}

/// Poured concrete / stone (dock, steps, headstones).
pub(crate) fn concrete() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.6, 0.6, 0.58))
        .with_roughness(0.85)
        .with_thermal(attach(ThermalProfile::rock()))
}

/// Low crop / lawn vegetation.
pub(crate) fn vegetation() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.24, 0.4, 0.14))
        .with_roughness(1.0)
        .with_thermal(attach(ThermalProfile::grass()))
}

/// Bare dirt mound (burrows, hog rooting).
pub(crate) fn dirt() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.4, 0.31, 0.22))
        .with_roughness(1.0)
        .with_thermal(attach(ThermalProfile::rock()))
}

/// Glass pane: `glass: true` (LWIR-opaque) plus the glass thermal preset.
pub(crate) fn glass_pane() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.4, 0.5, 0.55))
        .with_metallic(0.0)
        .with_roughness(0.05)
        .with_glass(true)
        .with_thermal(attach(ThermalProfile::glass()))
}

/// Streetlight head: warm emissive (NV bloom source), light glass housing.
pub(crate) fn lamp_head() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.9, 0.85, 0.7))
        .with_emissive(Vec3::new(1.0, 0.85, 0.55))
        .with_thermal(attach(ThermalProfile::glass()))
}

/// Red aviation beacon on masts.
pub(crate) fn beacon() -> StateSet {
    StateSet::new()
        .with_base_color(rgba(0.8, 0.1, 0.1))
        .with_emissive(Vec3::new(1.0, 0.05, 0.05))
        .with_thermal(attach(ThermalProfile::glass()))
}

/// State for a `.vim`-authored prop mesh: each [`PropThermal`] preset maps
/// onto one of the canned StateSets above, so props read through the
/// thermal/NV optics exactly like the built-in generators' geometry.
pub(crate) fn prop(kind: PropThermal) -> StateSet {
    match kind {
        PropThermal::Metal => metal_surface(),
        PropThermal::MetalRoof => metal_roof(),
        PropThermal::Wood => wood(),
        PropThermal::Concrete => concrete(),
        PropThermal::BuildingWall => building_wall(0.6, 0.55, 0.48),
        PropThermal::Glass => glass_pane(),
    }
}

/// Ground plane state for a biome.
pub(crate) fn ground(biome: Biome) -> StateSet {
    match biome {
        Biome::Grass => StateSet::new()
            .with_base_color(rgba(0.22, 0.36, 0.13))
            .with_roughness(1.0)
            .with_thermal(attach(ThermalProfile::grass())),
        Biome::Gravel => StateSet::new()
            .with_base_color(rgba(0.5, 0.48, 0.45))
            .with_roughness(1.0)
            .with_thermal(attach(ThermalProfile::rock())),
        Biome::Mud => StateSet::new()
            .with_base_color(rgba(0.3, 0.24, 0.18))
            .with_roughness(1.0)
            .with_thermal(ThermalAttach {
                base_temp: EQ,
                thermal_mass: ThermalProfile::rock().thermal_mass,
                sky_exposure: 0.4, // canopy-shaded creek bottom
            }),
        Biome::Asphalt => StateSet::new()
            .with_base_color(rgba(0.16, 0.16, 0.17))
            .with_roughness(0.9)
            .with_thermal(attach(ThermalProfile::rock())),
    }
}
