//! The embodied camp: home base as a first-person place (FPS grammar —
//! the viewport is always your perspective; menus never replace the world).
//!
//! Gear is physical: owned rifles hang on the rack, store stock stands
//! beside them with price tags, optics sit on the shelf, tins on the crate.
//! Interaction follows the game's one grammar: steer the view until the
//! object is centered, it highlights with a floating label, **right-click
//! commits** (buy / equip / mount / depart) — the same motion as taking a
//! shot. Trailhead signposts along the north fence depart for each zone.

use crate::convert;
use crate::hunt::Mounted;
use da_econ::{Accessory, Business, ItemKind, OpticModel, RifleModel};
use da_graph::prelude::*;
use da_param::ZoneExpansion;
use da_render::draw::{DrawItem, DrawList, Shape as RShape};
use glam::{Mat4, Vec3};

/// What committing (right-click) on a centered object does.
#[derive(Debug, Clone, PartialEq)]
pub enum CampAction {
    /// Buy this rifle (it moves from the store row to your rack).
    BuyRifle(RifleModel),
    /// Buy this optic.
    BuyOptic(OpticModel),
    /// Mount this owned optic for the coming night.
    MountOptic(Mounted),
    /// Buy an accessory / a tin of pellets.
    BuyAccessory(Accessory),
    /// Walk out: start the night in this zone.
    Depart(String),
}

/// One interactable object standing in the camp.
#[derive(Debug, Clone)]
pub struct CampItem {
    /// Gaze anchor (label position, pick target).
    pub pos: Vec3,
    /// First label line (name).
    pub name: String,
    /// Second label line (price / spec / travel).
    pub detail: String,
    /// What right-click does.
    pub action: CampAction,
    /// Drawn dimmer when the action is currently impossible.
    pub enabled: bool,
}

/// The camp as a live world.
pub struct CampWorld {
    expansion: ZoneExpansion,
    leaves: Vec<RenderLeaf>,
    /// Graph meshes in the camp zone, keyed by content-hash id — hand to
    /// `Renderer::register_meshes` before drawing (idempotent).
    mesh_registry: da_render::MeshRegistry,
    /// `(node, drawable) -> mesh id` cache so the per-frame draw list never
    /// rehashes mesh geometry (built alongside `mesh_registry`).
    mesh_ids: convert::MeshIds,
    /// Interactable stock, rebuilt whenever the business changes.
    pub items: Vec<CampItem>,
    /// Player feet position (eye is +1.6).
    pub player: Vec3,
}

/// Cabin-wall anchor the gear racks hang off (matches camp.zone.ron).
const RACK_ORIGIN: Vec3 = Vec3::new(22.0, 0.0, 31.5);
/// Trailhead row along the north fence gap.
const TRAIL_ORIGIN: Vec3 = Vec3::new(12.0, 0.0, 8.0);

impl CampWorld {
    /// Expand the camp source and stock it from the business.
    pub fn new(camp_source_path: &str, business: &Business, catalog: &crate::camp::ZoneCatalog) -> Result<Self, String> {
        let source = da_param::load_zone_file(camp_source_path).map_err(|e| e.to_string())?;
        let expansion = da_param::expand_zone(&source).map_err(|e| e.to_string())?;
        let center = Vec3::new(30.0, 1.6, 30.0);
        let leaves = CullVisitor::new(center).cull(&expansion.scene);
        let convert::SceneMeshes {
            registry: mesh_registry,
            ids: mesh_ids,
        } = convert::collect_meshes(&expansion.scene);
        let mut world = Self {
            expansion,
            leaves,
            mesh_registry,
            mesh_ids,
            items: Vec::new(),
            player: Vec3::new(30.0, 0.0, 14.0),
        };
        world.restock(business, catalog);
        Ok(world)
    }

    /// The camp zone's graph meshes — register with the renderer before
    /// drawing this world's draw list (idempotent, per-frame is fine).
    pub fn mesh_registry(&self) -> &da_render::MeshRegistry {
        &self.mesh_registry
    }

    /// Rebuild the interactable stock (call after any purchase).
    pub fn restock(&mut self, business: &Business, catalog: &crate::camp::ZoneCatalog) {
        let mut items = Vec::new();

        // Gun rack: every rifle model, owned ones marked, unowned priced.
        for (i, model) in RifleModel::ALL.iter().enumerate() {
            let owned = business.owns(ItemKind::Rifle(*model));
            let pos = RACK_ORIGIN + Vec3::new(1.1 * i as f32, 1.1, 0.0);
            items.push(CampItem {
                pos,
                name: model.name().to_string(),
                detail: if owned {
                    "owned — carried by tier automatically".into()
                } else {
                    format!("{} — RMB buys", da_econ::fmt_dollars(model.price_cents()))
                },
                action: CampAction::BuyRifle(*model),
                enabled: !owned,
            });
        }

        // Optics shelf, above the rack.
        let mounts = [
            (OpticModel::Headlamp, Mounted::Headlamp),
            (OpticModel::NvBasic, Mounted::NvBasic),
            (OpticModel::NvPro, Mounted::NvPro),
            (OpticModel::ThermalMk1, Mounted::Thermal(1)),
            (OpticModel::ThermalMk2, Mounted::Thermal(2)),
            (OpticModel::ThermalMk3, Mounted::Thermal(3)),
        ];
        for (i, (model, mount)) in mounts.iter().enumerate() {
            let owned = *model == OpticModel::Headlamp || business.owns(ItemKind::Optic(*model));
            let pos = RACK_ORIGIN + Vec3::new(0.9 * i as f32, 1.9, 0.0);
            let (detail, action) = if owned {
                ("owned — RMB mounts for tonight".to_string(), CampAction::MountOptic(*mount))
            } else {
                let price = model
                    .price_outright_cents()
                    .or(model.upgrade_from().map(|(_, c)| c))
                    .unwrap_or(0);
                (
                    format!("{} — RMB buys — {}", da_econ::fmt_dollars(price), model.tooltip()),
                    CampAction::BuyOptic(*model),
                )
            };
            items.push(CampItem {
                pos,
                name: model.name().to_string(),
                detail,
                action,
                enabled: true,
            });
        }

        // Supply crates: pellets and field kit on the storeroom side.
        let crates = [
            Accessory::PelletTin,
            Accessory::MatchedPelletTin,
            Accessory::Moderator,
            Accessory::BatteryPack,
            Accessory::LargerTank,
            Accessory::Bicycle,
            Accessory::Rangefinder,
            Accessory::ScopeMagnification,
            Accessory::BasicScope,
            Accessory::RedFilter,
        ];
        for (i, acc) in crates.iter().enumerate() {
            let owned = !acc.is_consumable() && business.owns(ItemKind::Accessory(*acc));
            let pos = Vec3::new(34.0 + 0.8 * (i % 5) as f32, 0.5 + 0.8 * (i / 5) as f32, 31.5);
            items.push(CampItem {
                pos,
                name: acc.name().to_string(),
                detail: if owned {
                    "owned".into()
                } else {
                    format!("{} — RMB buys", da_econ::fmt_dollars(acc.price_cents()))
                },
                action: CampAction::BuyAccessory(*acc),
                enabled: !owned,
            });
        }

        // Trailheads: one signpost per zone along the north fence.
        for (i, z) in catalog.zones.iter().enumerate() {
            let pos = TRAIL_ORIGIN + Vec3::new(6.0 * i as f32, 1.4, 0.0);
            items.push(CampItem {
                pos,
                name: format!("→ {}", z.name),
                detail: format!("{} min on foot — RMB departs for the night", z.walk_min),
                action: CampAction::Depart(z.name.clone()),
                enabled: true,
            });
        }

        self.items = items;
    }

    /// Walk. Same WASD feel as the field; clamped to the camp clearing.
    pub fn walk(&mut self, move_dir: Vec3, dt: f32) {
        self.player += move_dir * dt;
        self.player.x = self.player.x.clamp(7.0, 53.0);
        self.player.z = self.player.z.clamp(7.0, 53.0);
        self.player.y = 0.0;
    }

    pub fn eye(&self) -> Vec3 {
        self.player + Vec3::Y * 1.6
    }

    /// The item under the gaze: nearest to the view axis, within reach.
    pub fn gaze_item(&self, axis: Vec3) -> Option<usize> {
        const REACH_M: f32 = 8.0;
        const MAX_OFF_MIL: f32 = 120.0;
        let eye = self.eye();
        let candidates: Vec<(usize, Vec3)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.pos.distance(eye) <= REACH_M)
            .map(|(i, it)| (i, it.pos))
            .collect();
        crate::aim::pick_nearest_axis(eye, axis, &candidates, MAX_OFF_MIL)
    }

    /// Assemble the frame. Camp sits at late dusk — bright enough to see,
    /// dark enough that the lantern bloom and a mounted optic mean something.
    pub fn draw_list(&self, business: &Business) -> DrawList {
        let ambient = 62.0;
        let mut items: Vec<DrawItem> = Vec::with_capacity(self.leaves.len() + 96);
        // Horizon apron below the zone's own ground slab (see hunt.rs).
        items.push(DrawItem {
            shape: RShape::GroundPatch { half: 90.0 },
            world: Mat4::from_translation(Vec3::new(30.0, -0.15, 30.0)),
            albedo: [0.22, 0.28, 0.16],
            emissive: 0.0,
            temp_f: ambient - 2.0,
            glass: false,
            coat_f: 0.0,
        });
        for leaf in &self.leaves {
            if let Some(item) =
                convert::leaf_to_item(leaf, &self.expansion.scene, &self.mesh_ids, ambient)
            {
                items.push(item);
            }
        }

        // Physical stock. Owned gear reads warm-toned; store stock cool.
        for it in &self.items {
            let owned_tint = !it.enabled || matches!(it.action, CampAction::MountOptic(_));
            let albedo = if owned_tint {
                [0.42, 0.32, 0.2] // walnut and worn bluing
            } else {
                [0.3, 0.33, 0.38] // shrink-wrapped store stock
            };
            match &it.action {
                CampAction::BuyRifle(_) => {
                    // A rifle: stock box + barrel cylinder leaning on the rack.
                    items.push(DrawItem {
                        shape: RShape::Box { half: Vec3::new(0.05, 0.35, 0.09) },
                        world: Mat4::from_translation(it.pos - Vec3::Y * 0.45),
                        albedo,
                        emissive: 0.0,
                        temp_f: ambient,
                        glass: false,
                        coat_f: 0.0,
                    });
                    items.push(DrawItem {
                        shape: RShape::Cylinder { radius: 0.02, height: 0.6 },
                        world: Mat4::from_translation(it.pos - Vec3::Y * 0.1),
                        albedo: [0.15, 0.15, 0.17],
                        emissive: 0.0,
                        temp_f: ambient,
                        glass: false,
                        coat_f: 0.0,
                    });
                }
                CampAction::BuyOptic(_) | CampAction::MountOptic(_) => {
                    items.push(DrawItem {
                        shape: RShape::Cylinder { radius: 0.045, height: 0.22 },
                        world: Mat4::from_translation(it.pos)
                            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2),
                        albedo,
                        emissive: 0.0,
                        temp_f: ambient,
                        glass: false,
                        coat_f: 0.0,
                    });
                }
                CampAction::BuyAccessory(_) => {
                    items.push(DrawItem {
                        shape: RShape::Box { half: Vec3::splat(0.16) },
                        world: Mat4::from_translation(it.pos),
                        albedo,
                        emissive: 0.0,
                        temp_f: ambient,
                        glass: false,
                        coat_f: 0.0,
                    });
                }
                CampAction::Depart(_) => {
                    // Signpost: post + plank.
                    items.push(DrawItem {
                        shape: RShape::Cylinder { radius: 0.05, height: 1.5 },
                        world: Mat4::from_translation(Vec3::new(it.pos.x, 0.0, it.pos.z)),
                        albedo: [0.32, 0.26, 0.18],
                        emissive: 0.0,
                        temp_f: ambient,
                        glass: false,
                        coat_f: 0.0,
                    });
                    items.push(DrawItem {
                        shape: RShape::Box { half: Vec3::new(0.4, 0.1, 0.02) },
                        world: Mat4::from_translation(it.pos),
                        albedo: [0.5, 0.42, 0.3],
                        emissive: 0.0,
                        temp_f: ambient,
                        glass: false,
                        coat_f: 0.0,
                    });
                }
            }
        }
        let _ = business;

        DrawList {
            items,
            ambient_f: ambient,
            sky_temp_f: ambient - 40.0,
            moonlight: 0.95, // dusk: the yard is readable to the naked eye
            heat_decals: vec![],
            eyeshine: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camp::ZoneCatalog;

    fn world() -> CampWorld {
        let camp = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/camp.zone.ron");
        let zones = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/zones");
        CampWorld::new(camp, &Business::new(), &ZoneCatalog::load(zones).expect("catalog"))
            .expect("camp world")
    }

    #[test]
    fn camp_expands_and_stocks_everything() {
        let w = world();
        // 5 rifles + 6 optics + 10 accessories + 6 trailheads.
        assert_eq!(w.items.len(), 5 + 6 + 10 + 6);
        assert!(w.draw_list(&Business::new()).items.len() > 60);
        // A fresh business owns nothing: every rifle is buyable.
        assert!(w
            .items
            .iter()
            .filter(|i| matches!(i.action, CampAction::BuyRifle(_)))
            .all(|i| i.enabled));
    }

    #[test]
    fn gaze_picks_the_centered_item_within_reach() {
        let mut w = world();
        // Stand in front of the rack, look at the first rifle.
        w.player = Vec3::new(RACK_ORIGIN.x, 0.0, RACK_ORIGIN.z - 3.0);
        let target = w.items[0].pos;
        let axis = (target - w.eye()).normalize();
        let picked = w.gaze_item(axis).expect("centered item picks");
        assert_eq!(w.items[picked].pos, target);
        // From the far side of camp the rack is out of reach.
        w.player = Vec3::new(30.0, 0.0, 8.0);
        assert!(w.gaze_item(axis).is_none() || w.items[w.gaze_item(axis).expect("i")].pos.distance(w.eye()) <= 8.0);
    }

    #[test]
    fn buying_a_rifle_moves_it_from_store_to_owned() {
        let mut w = world();
        let mut biz = Business::new();
        biz.buy_rifle(RifleModel::MultiPump).expect("afford");
        w.restock(&biz, &ZoneCatalog::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/zones")).expect("catalog"));
        let rack: Vec<_> = w
            .items
            .iter()
            .filter(|i| matches!(i.action, CampAction::BuyRifle(_)))
            .collect();
        assert!(!rack[0].enabled, "owned rifle no longer buyable");
        assert!(rack[1].enabled, "unowned still for sale");
    }

    #[test]
    fn every_zone_has_a_trailhead() {
        let w = world();
        let trailheads: Vec<_> = w
            .items
            .iter()
            .filter_map(|i| match &i.action {
                CampAction::Depart(z) => Some(z.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(trailheads.len(), 6);
        assert!(trailheads.contains(&"Home Farm".to_string()));
        assert!(trailheads.contains(&"Main Street".to_string()));
    }

    #[test]
    fn walking_is_clamped_to_the_clearing() {
        let mut w = world();
        w.walk(Vec3::new(-100.0, 0.0, -100.0), 10.0);
        assert!(w.player.x >= 7.0 && w.player.z >= 7.0);
        w.walk(Vec3::new(100.0, 0.0, 100.0), 10.0);
        assert!(w.player.x <= 53.0 && w.player.z <= 53.0);
    }
}
