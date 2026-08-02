//! Integration tests for the thermal detection model.

use deadair::{
    entity::Entity,
    thermal::ThermalOptics,
    vec::Vec3,
};

fn observer_at_origin() -> Entity {
    Entity::hunter(0, Vec3::new(0.0, 0.0, 0.0))
}

#[test]
fn human_detected_with_near_certainty_at_close_range() {
    let optics = ThermalOptics::budget();
    let human = Entity::hunter(1, Vec3::new(30.0, 0.0, 0.0));
    let p = optics.detection_probability(&observer_at_origin(), 0.0, &human, 5.0);
    // Human ΔT ≈ 28 °C — should be essentially certain at 30 m
    assert!(p > 0.99, "Expected near-certain detection of warm human, got {p:.3}");
}

#[test]
fn zombie_much_harder_to_detect_than_human() {
    let optics = ThermalOptics::budget();
    let human  = Entity::hunter(1, Vec3::new(50.0, 0.0, 0.0));
    // Zombie ΔT ≈ 0.1 °C (nearly ambient — dead body equilibrated to surroundings)
    let zombie = Entity::zombie(2, Vec3::new(50.0, 0.0, 0.0), 5.0, 0.1);
    let p_h = optics.detection_probability(&observer_at_origin(), 0.0, &human,  5.0);
    let p_z = optics.detection_probability(&observer_at_origin(), 0.0, &zombie, 5.0);
    assert!(p_h > p_z, "Human should be easier to detect than cold zombie");
    assert!(p_h / p_z.max(1e-9) > 2.0, "Zombie should be at least 2× harder to spot");
}

#[test]
fn detection_drops_with_range() {
    let optics = ThermalOptics::budget();
    // Cold zombie (0.1 °C above ambient) — SNR in the logistic transition zone
    let near_zombie = Entity::zombie(1, Vec3::new( 50.0, 0.0, 0.0), 5.0, 0.1);
    let far_zombie  = Entity::zombie(2, Vec3::new(250.0, 0.0, 0.0), 5.0, 0.1);
    let p_near = optics.detection_probability(&observer_at_origin(), 0.0, &near_zombie, 5.0);
    let p_far  = optics.detection_probability(&observer_at_origin(), 0.0, &far_zombie,  5.0);
    assert!(p_near > p_far, "Closer targets should have higher detection probability");
}

#[test]
fn mil_grade_optics_detect_zombie_better_than_budget() {
    let budget  = ThermalOptics::budget();
    let mil     = ThermalOptics::military_grade();
    // Cold zombie at mid-range: budget SNR ≈ 1.25, mil SNR ≈ 4.0
    let zombie  = Entity::zombie(1, Vec3::new(50.0, 0.0, 0.0), 5.0, 0.1);
    let p_b = budget.detection_probability(&observer_at_origin(), 0.0, &zombie, 5.0);
    let p_m = mil.detection_probability(&observer_at_origin(), 0.0, &zombie, 5.0);
    assert!(p_m > p_b, "Military-grade should detect cold zombie more reliably than budget");
}

#[test]
fn target_beyond_max_range_is_not_detected() {
    let optics = ThermalOptics::budget(); // max 300 m
    let distant = Entity::hunter(1, Vec3::new(400.0, 0.0, 0.0));
    let p = optics.detection_probability(&observer_at_origin(), 0.0, &distant, 5.0);
    assert_eq!(p, 0.0, "Target beyond max range must not be detected");
}

#[test]
fn target_outside_fov_is_not_detected() {
    let optics = ThermalOptics::budget(); // FOV = 20 °, heading = 0 °
    // Place target at 90 ° (North) — well outside the 10 ° half-FOV
    let human = Entity::hunter(1, Vec3::new(0.0, 50.0, 0.0));
    let p = optics.detection_probability(&observer_at_origin(), 0.0, &human, 5.0);
    assert_eq!(p, 0.0, "Target outside FOV must not be detected");
}

#[test]
fn snr_is_proportional_to_temperature_contrast() {
    let optics = ThermalOptics::budget();
    let obs = observer_at_origin();
    // Same position, different temperatures
    let hot  = Entity::hunter(1, Vec3::new(50.0, 0.0, 0.0)).with_temperature(40.0);
    let warm = Entity::hunter(2, Vec3::new(50.0, 0.0, 0.0)).with_temperature(20.0);
    let snr_hot  = optics.signal_to_noise(&obs, 0.0, &hot,  5.0).unwrap();
    let snr_warm = optics.signal_to_noise(&obs, 0.0, &warm, 5.0).unwrap();
    assert!(snr_hot > snr_warm, "Hotter target should have higher SNR");
}
