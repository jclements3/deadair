//! Hit resolution (SDD §5.2, FR-A3/A7/A8): raycast against head/body
//! colliders, nearest-first, plus the backstop rule.
//!
//! `dir` is the *ballistically compensated* direction — the aiming layer
//! applies [`crate::ballistics::aim_solution`] holdover before calling in,
//! so resolution itself is a straight ray. Dispersion is applied here,
//! deterministically, from the caller's [`Rng`] stream.

use da_core::{EntityId, Rng};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::ballistics::perturb_direction;
use crate::weapon::RifleConfig;

/// Maximum pellet travel considered for impacts (m).
pub const MAX_RAY_RANGE_M: f32 = 300.0;

/// Every species the sim can put in front of the muzzle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Species {
    /// Pest — low bounty, fast respawn.
    Rat,
    /// Pest — medium bounty, freezes when lit.
    Possum,
    /// Pest — high bounty, group memory.
    Raccoon,
    /// Friendly — patrols; raccoon-sized blob at distance.
    Dog,
    /// Friendly — wanders exactly where the rats are.
    Cat,
    /// Friendly — static backstop dilemma.
    Cow,
    /// Friendly — static backstop dilemma.
    Sheep,
    /// Ambient-temperature hostile; headshot-only kill, no bounty.
    Zombie,
}

impl Species {
    /// Bounty-eligible pest?
    pub fn is_pest(self) -> bool {
        matches!(self, Species::Rat | Species::Possum | Species::Raccoon)
    }

    /// Protected animal (FR-A6)?
    pub fn is_friendly(self) -> bool {
        matches!(self, Species::Dog | Species::Cat | Species::Cow | Species::Sheep)
    }

    /// The cold ones.
    pub fn is_zombie(self) -> bool {
        matches!(self, Species::Zombie)
    }
}

/// Sphere collider.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sphere {
    /// World-space center (m).
    pub center: Vec3,
    /// Radius (m).
    pub r: f32,
}

/// Which collider zone a ray struck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitZone {
    /// Instant kill zone (pests) / only kill zone (zombies).
    Head,
    /// Wound / stagger zone.
    Body,
}

/// A shootable entity: one head sphere plus a capsule-ish body approximated
/// as a chain of spheres (cheap, orientation-free, good enough for pellets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// Owning entity.
    pub id: EntityId,
    /// Species tag — drives outcome classification and penalties.
    pub species: Species,
    /// Ground position (feet), world meters.
    pub pos: Vec3,
    /// Head collider.
    pub head: Sphere,
    /// Body collider chain.
    pub body: Vec<Sphere>,
}

impl Target {
    /// Canonical collider layout per species, planted at `pos` (ground
    /// point). Offsets are authored so head and body separate cleanly for
    /// a shooter at standing eye height.
    pub fn for_species(id: EntityId, species: Species, pos: Vec3) -> Self {
        // (head_offset, head_r, [(body_offset, body_r)...])
        let (h, hr, body): (Vec3, f32, &[(Vec3, f32)]) = match species {
            Species::Rat => (
                Vec3::new(0.12, 0.10, 0.0),
                0.035,
                &[(Vec3::new(0.0, 0.04, 0.0), 0.045), (Vec3::new(-0.07, 0.04, 0.0), 0.04)],
            ),
            Species::Possum => (
                Vec3::new(0.20, 0.18, 0.0),
                0.05,
                &[(Vec3::new(0.0, 0.10, 0.0), 0.09), (Vec3::new(-0.12, 0.10, 0.0), 0.09)],
            ),
            Species::Raccoon => (
                Vec3::new(0.18, 0.32, 0.0),
                0.06,
                &[(Vec3::new(0.0, 0.16, 0.0), 0.11), (Vec3::new(-0.15, 0.14, 0.0), 0.10)],
            ),
            Species::Dog => (
                Vec3::new(0.30, 0.55, 0.0),
                0.08,
                &[(Vec3::new(0.0, 0.42, 0.0), 0.15), (Vec3::new(-0.25, 0.40, 0.0), 0.14)],
            ),
            Species::Cat => (
                Vec3::new(0.18, 0.28, 0.0),
                0.045,
                &[(Vec3::new(0.0, 0.18, 0.0), 0.08), (Vec3::new(-0.12, 0.16, 0.0), 0.07)],
            ),
            Species::Cow => (
                Vec3::new(0.90, 1.35, 0.0),
                0.16,
                &[
                    (Vec3::new(0.3, 0.95, 0.0), 0.45),
                    (Vec3::new(-0.3, 0.95, 0.0), 0.45),
                    (Vec3::new(0.0, 0.55, 0.0), 0.40),
                ],
            ),
            Species::Sheep => (
                Vec3::new(0.45, 0.75, 0.0),
                0.09,
                &[(Vec3::new(0.0, 0.55, 0.0), 0.28), (Vec3::new(-0.2, 0.50, 0.0), 0.25)],
            ),
            Species::Zombie => (
                Vec3::new(0.0, 1.62, 0.0),
                0.11,
                &[
                    (Vec3::new(0.0, 1.15, 0.0), 0.18),
                    (Vec3::new(0.0, 0.70, 0.0), 0.16),
                    (Vec3::new(0.0, 0.30, 0.0), 0.14),
                ],
            ),
        };
        Target {
            id,
            species,
            pos,
            head: Sphere { center: pos + h, r: hr },
            body: body.iter().map(|(o, r)| Sphere { center: pos + *o, r: *r }).collect(),
        }
    }
}

/// Ray/sphere intersection: nearest positive `t` along `dir` (unit).
pub fn ray_sphere(origin: Vec3, dir: Vec3, s: &Sphere) -> Option<f32> {
    let oc = origin - s.center;
    let b = oc.dot(dir);
    let c = oc.length_squared() - s.r * s.r;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t = -b - sq;
    if t >= 0.0 {
        Some(t)
    } else {
        let t2 = -b + sq;
        (t2 >= 0.0).then_some(t2)
    }
}

/// One entry along the shot ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// Distance along the ray (m).
    pub t: f32,
    /// Index into the target slice.
    pub target: usize,
    /// Zone struck.
    pub zone: HitZone,
}

/// All target intersections along the ray, one (nearest collider) per
/// target, sorted nearest-first.
pub fn ray_hits(origin: Vec3, dir: Vec3, world: &[Target]) -> Vec<RayHit> {
    let mut hits = Vec::new();
    for (i, tgt) in world.iter().enumerate() {
        let mut best: Option<(f32, HitZone)> = None;
        if let Some(t) = ray_sphere(origin, dir, &tgt.head) {
            best = Some((t, HitZone::Head));
        }
        for s in &tgt.body {
            if let Some(t) = ray_sphere(origin, dir, s) {
                if best.map_or(true, |(bt, _)| t < bt) {
                    best = Some((t, HitZone::Body));
                }
            }
        }
        if let Some((t, zone)) = best {
            if t <= MAX_RAY_RANGE_M {
                hits.push(RayHit { t, target: i, zone });
            }
        }
    }
    hits.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    hits
}

/// Pre-fire backstop check (FR-A8, SDD §5.2): true if a friendly lies in
/// the line of fire within lethal range — including *behind* the intended
/// target, since the design rule continues the ray past the target. Drive
/// the reticle warning from this.
pub fn check_backstop(origin: Vec3, dir: Vec3, world: &[Target], lethal_range_m: f32) -> bool {
    let dir = dir.normalize_or_zero();
    ray_hits(origin, dir, world)
        .iter()
        .any(|h| h.t <= lethal_range_m && world[h.target].species.is_friendly())
}

/// Outcome of a resolved shot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShotOutcome {
    /// Pest headshot inside lethal range: bounty queued (FR-A3).
    Kill {
        /// Victim.
        id: EntityId,
        /// Victim species.
        species: Species,
        /// Impact point.
        pos: Vec3,
    },
    /// Pest body hit (or headshot beyond lethal range): flees, alert pulse.
    Wound {
        /// Victim.
        id: EntityId,
        /// Victim species.
        species: Species,
        /// Impact point.
        pos: Vec3,
    },
    /// Friendly struck — directly, or via a violated backstop (the hit is
    /// converted per FR-A8). Money + reputation penalty upstream.
    FriendlyHit {
        /// Victim.
        id: EntityId,
        /// Victim species.
        species: Species,
    },
    /// Zombie headshot inside lethal range (FR-A7).
    ZombieDestroyed {
        /// Victim.
        id: EntityId,
    },
    /// Zombie body hit: brief movement pause only.
    ZombieStagger {
        /// Victim.
        id: EntityId,
    },
    /// Nothing living struck. `impact` is the pellet's terrain strike (if
    /// any) for the thermal residue decal (FR-T4).
    Miss {
        /// Ground impact point, if the ray reached terrain.
        impact: Option<Vec3>,
    },
}

/// Resolve a fired shot: apply dispersion from `rng`, raycast the world
/// nearest-first, classify per SDD §5.2, and apply the backstop rule
/// (a pest hit with a friendly behind it within lethal range becomes a
/// [`ShotOutcome::FriendlyHit`]).
///
/// Pure with respect to the world: consuming air is the caller's job
/// ([`RifleConfig::fire`]); this reads the rifle's current state for
/// dispersion and lethal range.
pub fn resolve_shot(
    origin: Vec3,
    dir: Vec3,
    world: &[Target],
    rifle: &RifleConfig,
    rng: &mut Rng,
) -> ShotOutcome {
    let aim = dir.normalize_or_zero();
    if aim == Vec3::ZERO || rifle.muzzle_energy_fpe().is_none() {
        return ShotOutcome::Miss { impact: None };
    }
    let dir = perturb_direction(aim, rifle.dispersion_mrad() / 1000.0, rng);
    let lethal = rifle.lethal_range_m();
    let hits = ray_hits(origin, dir, world);

    let Some(first) = hits.first().copied() else {
        return ShotOutcome::Miss { impact: ground_impact(origin, dir) };
    };
    let tgt = &world[first.target];
    let impact = origin + dir * first.t;

    if tgt.species.is_friendly() {
        return ShotOutcome::FriendlyHit { id: tgt.id, species: tgt.species };
    }
    if tgt.species.is_zombie() {
        return if first.zone == HitZone::Head && first.t <= lethal {
            ShotOutcome::ZombieDestroyed { id: tgt.id }
        } else {
            ShotOutcome::ZombieStagger { id: tgt.id }
        };
    }

    // Pest hit — backstop rule first: continue the ray past the target and
    // look for a friendly within lethal range (FR-A8).
    if let Some(f) = hits.iter().find(|h| {
        h.target != first.target && h.t <= lethal && world[h.target].species.is_friendly()
    }) {
        let friendly = &world[f.target];
        return ShotOutcome::FriendlyHit { id: friendly.id, species: friendly.species };
    }

    if first.zone == HitZone::Head && first.t <= lethal {
        ShotOutcome::Kill { id: tgt.id, species: tgt.species, pos: impact }
    } else {
        ShotOutcome::Wound { id: tgt.id, species: tgt.species, pos: impact }
    }
}

/// Where a missed pellet meets the ground plane (y = 0), if it does within
/// [`MAX_RAY_RANGE_M`].
fn ground_impact(origin: Vec3, dir: Vec3) -> Option<Vec3> {
    if dir.y >= -1e-6 {
        return None;
    }
    let t = -origin.y / dir.y;
    (t > 0.0 && t <= MAX_RAY_RANGE_M).then(|| origin + dir * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eye() -> Vec3 {
        Vec3::new(0.0, 1.6, 0.0)
    }

    fn rifle() -> RifleConfig {
        let mut r = RifleConfig::tier3();
        r.matched_pellets = true;
        r
    }

    #[test]
    fn headshot_kills_pest_body_wounds() {
        let rat = Target::for_species(EntityId(1), Species::Rat, Vec3::new(8.0, 0.0, 0.0));
        let world = vec![rat.clone()];
        let mut rng = Rng::new(3);
        let head_dir = (rat.head.center - eye()).normalize();
        match resolve_shot(eye(), head_dir, &world, &rifle(), &mut rng) {
            ShotOutcome::Kill { id, species, .. } => {
                assert_eq!(id, EntityId(1));
                assert_eq!(species, Species::Rat);
            }
            other => panic!("expected Kill, got {other:?}"),
        }
        let body_dir = (rat.body[1].center - eye()).normalize();
        match resolve_shot(eye(), body_dir, &world, &rifle(), &mut rng) {
            ShotOutcome::Wound { id, .. } => assert_eq!(id, EntityId(1)),
            other => panic!("expected Wound, got {other:?}"),
        }
    }

    #[test]
    fn backstop_friendly_behind_target_warns_and_converts() {
        // Raccoon at 10 m, cow directly behind at 14 m — inside the Tier 3
        // lethal range (~37 m). Big colliders so the perturbed ray cannot
        // sneak past.
        let coon =
            Target::for_species(EntityId(1), Species::Raccoon, Vec3::new(10.0, 0.0, 0.0));
        let cow = Target {
            id: EntityId(2),
            species: Species::Cow,
            pos: Vec3::new(14.0, 0.0, 0.0),
            head: Sphere { center: Vec3::new(14.0, 2.2, 0.0), r: 0.15 },
            body: vec![Sphere { center: Vec3::new(14.0, 0.2, 0.0), r: 1.2 }],
        };
        let world = vec![coon.clone(), cow];
        let dir = (coon.head.center - eye()).normalize();
        let r = rifle();

        // Pre-fire: reticle warning.
        assert!(check_backstop(eye(), dir, &world, r.lethal_range_m()));

        // Post-fire: the pest hit is converted to a FriendlyHit penalty.
        let mut rng = Rng::new(9);
        match resolve_shot(eye(), dir, &world, &r, &mut rng) {
            ShotOutcome::FriendlyHit { id, species } => {
                assert_eq!(id, EntityId(2));
                assert_eq!(species, Species::Cow);
            }
            other => panic!("expected FriendlyHit via backstop, got {other:?}"),
        }

        // Friendly beyond lethal range is a safe backstop.
        let far_world = vec![
            coon.clone(),
            Target {
                pos: Vec3::new(120.0, 0.0, 0.0),
                head: Sphere { center: Vec3::new(120.0, 2.2, 0.0), r: 0.15 },
                body: vec![Sphere { center: Vec3::new(120.0, 0.2, 0.0), r: 1.2 }],
                ..world[1].clone()
            },
        ];
        assert!(!check_backstop(eye(), dir, &far_world, r.lethal_range_m()));
        let mut rng = Rng::new(9);
        assert!(matches!(
            resolve_shot(eye(), dir, &far_world, &r, &mut rng),
            ShotOutcome::Kill { .. }
        ));
    }

    #[test]
    fn direct_friendly_hit_is_friendly_hit() {
        let cat = Target::for_species(EntityId(5), Species::Cat, Vec3::new(9.0, 0.0, 0.0));
        let dir = (cat.body[0].center - eye()).normalize();
        let mut rng = Rng::new(1);
        assert!(matches!(
            resolve_shot(eye(), dir, &[cat], &rifle(), &mut rng),
            ShotOutcome::FriendlyHit { id: EntityId(5), species: Species::Cat }
        ));
    }

    #[test]
    fn zombie_headshot_destroys_body_staggers() {
        let z = Target::for_species(EntityId(7), Species::Zombie, Vec3::new(10.0, 0.0, 0.0));
        let world = vec![z.clone()];
        let mut rng = Rng::new(2);
        let body_dir = (z.body[0].center - eye()).normalize();
        assert!(matches!(
            resolve_shot(eye(), body_dir, &world, &rifle(), &mut rng),
            ShotOutcome::ZombieStagger { id: EntityId(7) }
        ));
        let head_dir = (z.head.center - eye()).normalize();
        assert!(matches!(
            resolve_shot(eye(), head_dir, &world, &rifle(), &mut rng),
            ShotOutcome::ZombieDestroyed { id: EntityId(7) }
        ));
    }

    #[test]
    fn pest_headshot_beyond_lethal_range_only_wounds() {
        let r = rifle(); // lethal ~41 m
        let far = r.lethal_range_m() + 10.0;
        let coon =
            Target::for_species(EntityId(3), Species::Raccoon, Vec3::new(far, 0.0, 0.0));
        let dir = (coon.head.center - eye()).normalize();
        let mut rng = Rng::new(4);
        // Fire enough deterministic shots that at least one lands on the head.
        let mut saw_wound = false;
        for _ in 0..20 {
            match resolve_shot(eye(), dir, &[coon.clone()], &r, &mut rng) {
                ShotOutcome::Wound { .. } => saw_wound = true,
                ShotOutcome::Kill { .. } => panic!("kill beyond lethal range"),
                _ => {}
            }
        }
        assert!(saw_wound);
    }

    #[test]
    fn miss_reports_ground_impact_for_heat_residue() {
        let mut rng = Rng::new(6);
        let dir = Vec3::new(1.0, -0.08, 0.0).normalize(); // hits dirt ~20 m out
        match resolve_shot(eye(), dir, &[], &rifle(), &mut rng) {
            ShotOutcome::Miss { impact: Some(p) } => {
                assert!(p.y.abs() < 1e-3 && p.x > 0.0);
            }
            other => panic!("expected ground impact, got {other:?}"),
        }
        // Shot at the sky: no residue point.
        let up = Vec3::new(0.3, 0.5, 0.0).normalize();
        assert!(matches!(
            resolve_shot(eye(), up, &[], &rifle(), &mut rng),
            ShotOutcome::Miss { impact: None }
        ));
    }
}
