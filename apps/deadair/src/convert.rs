//! Bridges between the crates: scene graph leaves → renderer draw items,
//! thermal attaches → full thermal profiles, species name mappings.

use da_graph::prelude::*;
use da_render::draw::{DrawItem, Shape as RShape};
use da_thermal::ThermalProfile;

/// Map a scene-graph shape to a renderer shape. `None` = not drawable yet.
pub fn map_shape(shape: &da_graph::Shape) -> Option<RShape> {
    Some(match *shape {
        da_graph::Shape::Box { half_extents } => RShape::Box { half: half_extents },
        da_graph::Shape::Cylinder { radius, height } => RShape::Cylinder { radius, height },
        da_graph::Shape::Sphere { radius } => RShape::Sphere { radius },
        // Renderer has no capsule primitive yet; a cylinder is close enough
        // for silhouettes.
        da_graph::Shape::Capsule { radius, height } => RShape::Cylinder {
            radius,
            height: height + 2.0 * radius,
        },
        da_graph::Shape::Mesh { .. } => return None,
    })
}

/// Convert one culled leaf into a draw item; `temp_f` comes from the
/// thermal sim (or ambient when the node carries no profile).
pub fn leaf_to_item(leaf: &RenderLeaf, scene: &Scene, temp_f: f32) -> Option<DrawItem> {
    let node = scene.node(leaf.node)?;
    let geode = match node.kind() {
        NodeKind::Geode(g) => g,
        _ => return None,
    };
    let drawable = geode.drawables.get(leaf.drawable)?;
    let shape = map_shape(&drawable.shape)?;
    let color = leaf
        .state
        .base_color
        .unwrap_or(glam::Vec4::new(0.5, 0.5, 0.5, 1.0));
    let emissive = leaf
        .state
        .emissive
        .map(|e| e.length() / 3.0f32.sqrt())
        .unwrap_or(0.0);
    Some(DrawItem {
        shape,
        world: leaf.world,
        albedo: [color.x, color.y, color.z],
        emissive,
        temp_f,
        glass: leaf.state.glass.unwrap_or(false),
    })
}

/// Recover a full thermal profile from the graph's slimmer `ThermalAttach`
/// by matching the nearest da-thermal preset on (thermal_mass, sky_exposure)
/// — the attach doesn't carry solar gain, the preset supplies it.
pub fn profile_from_attach(attach: &ThermalAttach) -> ThermalProfile {
    let presets = [
        ThermalProfile::metal_roof(),
        ThermalProfile::rock(),
        ThermalProfile::grass(),
        ThermalProfile::water(),
        ThermalProfile::tree(),
        ThermalProfile::building_wall(),
        ThermalProfile::glass(),
    ];
    let dist = |p: &ThermalProfile| {
        let dm = (p.thermal_mass.ln() - attach.thermal_mass.max(0.1).ln()).abs();
        let ds = (p.sky_exposure - attach.sky_exposure).abs() * 3.0;
        dm + ds
    };
    let mut best = presets[0].clone();
    let mut best_d = dist(&best);
    for p in &presets[1..] {
        let d = dist(p);
        if d < best_d {
            best_d = d;
            best = p.clone();
        }
    }
    // Keep the attach's authored numbers; the preset only contributes the
    // solar-gain estimate.
    ThermalProfile {
        thermal_mass: attach.thermal_mass,
        sky_exposure: attach.sky_exposure,
        ..best
    }
}

/// da-param species names → da-sim species.
pub fn sim_species(name: &str) -> Option<da_sim::Species> {
    use da_sim::Species::*;
    Some(match name {
        "Rat" => Rat,
        "Possum" => Possum,
        "Raccoon" => Raccoon,
        "Dog" => Dog,
        "Cat" => Cat,
        "Cow" => Cow,
        "Sheep" => Sheep,
        "Zombie" => Zombie,
        "Groundhog" => Groundhog,
        "Beaver" => Beaver,
        "JuvenileFeralHog" => JuvenileFeralHog,
        _ => return None,
    })
}

/// da-sim species → da-econ species (for the ledger).
pub fn econ_species(s: da_sim::Species) -> da_econ::Species {
    use da_sim::Species as S;
    match s {
        S::Rat => da_econ::Species::Rat,
        S::Possum => da_econ::Species::Possum,
        S::Raccoon => da_econ::Species::Raccoon,
        S::Groundhog => da_econ::Species::Groundhog,
        S::Beaver => da_econ::Species::Beaver,
        S::JuvenileFeralHog => da_econ::Species::JuvenileFeralHog,
        S::Dog => da_econ::Species::Dog,
        S::Cat => da_econ::Species::Cat,
        S::Cow => da_econ::Species::Cow,
        S::Sheep => da_econ::Species::Sheep,
        S::Zombie => da_econ::Species::Zombie,
    }
}

/// da-param hazard kinds → da-sim hazard kinds.
pub fn sim_hazard_kind(k: da_param::HazardKind) -> da_sim::HazardKind {
    use da_param::HazardKind as P;
    use da_sim::HazardKind as S;
    match k {
        P::Wire => S::Wire,
        P::Hole => S::Hole,
        P::CreekBank => S::CreekBank,
        P::Water => S::Water,
        P::Limb => S::Limb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_core::TempF;

    #[test]
    fn metal_roof_attach_recovers_solar_gain() {
        let attach = ThermalAttach {
            base_temp: TempF(68.0),
            thermal_mass: 100.0,
            sky_exposure: 1.0,
        };
        let p = profile_from_attach(&attach);
        assert!(p.initial_solar_gain_f > 15.0, "metal roof gain: {p:?}");
        assert_eq!(p.sky_exposure, 1.0);
    }

    #[test]
    fn grass_attach_maps_to_low_gain() {
        let attach = ThermalAttach {
            base_temp: TempF(68.0),
            thermal_mass: 60.0,
            sky_exposure: 0.9,
        };
        let p = profile_from_attach(&attach);
        assert!(p.initial_solar_gain_f < 5.0, "grass gain: {p:?}");
    }

    #[test]
    fn species_round_trip() {
        assert_eq!(sim_species("Rat"), Some(da_sim::Species::Rat));
        assert_eq!(
            sim_species("JuvenileFeralHog"),
            Some(da_sim::Species::JuvenileFeralHog)
        );
        assert_eq!(
            econ_species(da_sim::Species::Beaver),
            da_econ::Species::Beaver
        );
        assert_eq!(econ_species(da_sim::Species::Cat), da_econ::Species::Cat);
    }
}
