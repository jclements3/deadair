//! Backend-free tests for the spatial core, subtitles, event mapping and
//! deterministic synthesis. Nothing here opens an audio device.

use da_audio::{
    attenuation, equal_power_gains, sounds_for_event, sounds_for_events, synthesize, AudioEngine,
    AudioScene, Backend, Direction8, DistanceBand, EventAudioCtx, Listener, NullBackend,
    SoundKind, SoundSource, Surface, WeatherBed,
};
use da_core::EntityId;
use da_sim::events::{DamageCause, SimEvent};
use da_sim::hit::Species;
use da_sim::noise::NoiseKind;
use glam::Vec3;

/// Listener at the origin looking down -Z (so +X is hard right).
fn listener() -> Listener {
    Listener::new(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0))
}

fn scene() -> AudioScene {
    AudioScene::default()
}

fn resolve(kind: SoundKind, pos: Vec3) -> da_audio::AudibleSource {
    scene().resolve(&listener(), &SoundSource::new(kind, pos))
}

// ---------------------------------------------------------------- panning

#[test]
fn pan_is_zero_dead_ahead() {
    let a = resolve(SoundKind::DogBark, Vec3::new(0.0, 0.0, -10.0));
    assert!(a.pan.abs() < 1e-5, "pan {} should be centred", a.pan);
    assert!(a.bearing_deg.abs() < 1e-3);
}

#[test]
fn pan_is_hard_left_and_hard_right() {
    let left = resolve(SoundKind::DogBark, Vec3::new(-10.0, 0.0, 0.0));
    let right = resolve(SoundKind::DogBark, Vec3::new(10.0, 0.0, 0.0));
    assert!((left.pan + 1.0).abs() < 1e-5, "pan {}", left.pan);
    assert!((right.pan - 1.0).abs() < 1e-5, "pan {}", right.pan);
}

#[test]
fn pan_is_continuous_and_monotone_across_the_front_arc() {
    // Sweep the source from hard left, through dead ahead, to hard right.
    let s = scene();
    let l = listener();
    let mut prev = f32::NEG_INFINITY;
    let mut prev_pan = None;
    let steps = 180;
    for i in 0..=steps {
        // bearing from -90 deg (left) to +90 deg (right)
        let deg = -90.0 + 180.0 * (i as f32 / steps as f32);
        let rad = deg.to_radians();
        // forward is -Z, right is +X
        let pos = Vec3::new(20.0 * rad.sin(), 0.0, -20.0 * rad.cos());
        let pan = s.resolve(&l, &SoundSource::new(SoundKind::DogBark, pos)).pan;
        assert!(pan >= prev - 1e-4, "pan must not go backwards at {deg} deg");
        if let Some(p) = prev_pan {
            let step: f32 = pan - p;
            assert!(step.abs() < 0.05, "pan jumped by {step} at {deg} deg");
        }
        prev = pan;
        prev_pan = Some(pan);
    }
    assert!((prev - 1.0).abs() < 1e-4);
}

#[test]
fn pan_respects_listener_facing() {
    let s = scene();
    // Facing +X: a source at +Z is now on the listener's *right*.
    let l = Listener::new(Vec3::ZERO, Vec3::X);
    let a = s.resolve(&l, &SoundSource::new(SoundKind::DogBark, Vec3::new(0.0, 0.0, 10.0)));
    assert!((a.pan - 1.0).abs() < 1e-5, "pan {}", a.pan);
    // And a source at +X is dead ahead.
    let b = s.resolve(&l, &SoundSource::new(SoundKind::DogBark, Vec3::new(10.0, 0.0, 0.0)));
    assert!(b.pan.abs() < 1e-5);
}

#[test]
fn equal_power_gains_are_unit_energy() {
    for pan in [-1.0_f32, -0.5, 0.0, 0.5, 1.0] {
        let (l, r) = equal_power_gains(pan);
        assert!((l * l + r * r - 1.0).abs() < 1e-5);
    }
    let (l, r) = equal_power_gains(-1.0);
    assert!(l > 0.99 && r < 0.01);
    let (l, r) = equal_power_gains(1.0);
    assert!(r > 0.99 && l < 0.01);
}

// ------------------------------------------------------------ attenuation

#[test]
fn attenuation_falls_with_distance() {
    let k = SoundKind::DogBark;
    let mut prev = f32::INFINITY;
    for d in 0..(k.falloff_m() as u32) {
        let a = attenuation(k, d as f32);
        assert!(a <= prev + 1e-6, "attenuation rose at {d} m");
        prev = a;
    }
    assert!(attenuation(k, 1.0) > attenuation(k, 20.0));
    assert!(attenuation(k, 20.0) > attenuation(k, 60.0));
}

#[test]
fn attenuation_is_zero_at_and_beyond_falloff() {
    for k in [SoundKind::RatScurry, SoundKind::ZombieMoan, SoundKind::RifleDischarge] {
        let f = k.falloff_m();
        assert!(attenuation(k, f * 0.99) > 0.0);
        assert_eq!(attenuation(k, f), 0.0);
        assert_eq!(attenuation(k, f + 1.0), 0.0);
        assert_eq!(attenuation(k, f * 10.0), 0.0);
    }
}

#[test]
fn attenuation_is_full_inside_the_reference_distance() {
    // At the listener's feet a sound is at (nearly) full strength.
    assert!(attenuation(SoundKind::DogBark, 0.0) > 0.98);
}

#[test]
fn muffle_reduces_volume_without_moving_the_source() {
    let s = scene();
    let l = listener();
    let pos = Vec3::new(5.0, 0.0, -5.0);
    let clear = s.resolve(&l, &SoundSource::new(SoundKind::RaccoonRummage, pos));
    let blocked = s.resolve(
        &l,
        &SoundSource::new(SoundKind::RaccoonRummage, pos).with_muffle(0.75),
    );
    assert!(blocked.volume < clear.volume * 0.3);
    assert!((blocked.pan - clear.pan).abs() < 1e-6);
    assert_eq!(blocked.subtitle().direction, clear.subtitle().direction);
}

// ---------------------------------------------------------------- culling

#[test]
fn culling_drops_inaudible_sources() {
    let s = scene();
    let l = listener();
    let sources = [
        // Audible: right on top of the player.
        SoundSource::new(SoundKind::RatScurry, Vec3::new(1.0, 0.0, 0.0)),
        // Inaudible: well past the rat falloff of 8 m.
        SoundSource::new(SoundKind::RatScurry, Vec3::new(500.0, 0.0, 0.0)),
        // Inaudible: fully occluded.
        SoundSource::new(SoundKind::DogBark, Vec3::new(2.0, 0.0, 0.0)).with_muffle(1.0),
        // Inaudible: silenced by gain.
        SoundSource::new(SoundKind::DogBark, Vec3::new(2.0, 0.0, 0.0)).with_gain(0.0),
        // Audible: a moan far past where a rat would be gone.
        SoundSource::new(SoundKind::ZombieMoan, Vec3::new(0.0, 0.0, -30.0)),
    ];
    let mixed = s.mix(&l, &sources);
    assert_eq!(mixed.len(), 2);
    // Loudest first.
    assert!(mixed[0].volume >= mixed[1].volume);
    let kinds: Vec<_> = mixed.iter().map(|m| m.source.kind).collect();
    assert!(kinds.contains(&SoundKind::RatScurry));
    assert!(kinds.contains(&SoundKind::ZombieMoan));

    assert!(!s.is_audible(&l, &sources[1]));
    assert!(s.is_audible(&l, &sources[0]));
}

#[test]
fn ambient_beds_are_centred_and_never_culled() {
    let s = scene();
    let l = listener();
    let bed = SoundSource::new(SoundKind::Weather(WeatherBed::Rain), Vec3::new(9999.0, 0.0, 0.0));
    let a = s.resolve(&l, &bed);
    assert_eq!(a.pan, 0.0);
    assert!(a.volume > 0.0);
    assert_eq!(s.mix(&l, &[bed]).len(), 1);
}

// -------------------------------------------------- balance: the design bet

#[test]
fn zombie_moan_carries_farther_than_a_rat_rustle() {
    // Load-bearing: thermal cannot see zombies, so the moan must reach the
    // player long before any pest sound would.
    assert!(SoundKind::ZombieMoan.falloff_m() > SoundKind::RatScurry.falloff_m() * 3.0);
    assert!(SoundKind::ZombieMoan.loudness() > SoundKind::RatScurry.loudness());

    let s = scene();
    let l = listener();
    let far = Vec3::new(0.0, 0.0, -30.0);
    assert!(s.is_audible(&l, &SoundSource::new(SoundKind::ZombieMoan, far)));
    assert!(!s.is_audible(&l, &SoundSource::new(SoundKind::RatScurry, far)));
    // The drag-step too, though not as far as the moan.
    assert!(SoundKind::ZombieDragStep.falloff_m() > SoundKind::RatScurry.falloff_m());
    assert!(SoundKind::ZombieDragStep.falloff_m() < SoundKind::ZombieMoan.falloff_m());
}

#[test]
fn moderated_discharge_is_much_quieter_than_unmoderated() {
    let loud = SoundKind::RifleDischarge;
    let quiet = SoundKind::RifleDischargeModerated;
    // Radius cut matches da_sim's MODERATOR_FACTOR bound (>= 70% reduction).
    assert!(quiet.falloff_m() <= loud.falloff_m() * 0.31);
    assert!(quiet.loudness() < loud.loudness() * 0.2);

    let s = scene();
    let l = listener();
    let pos = Vec3::new(0.0, 0.0, -25.0);
    let a = s.resolve(&l, &SoundSource::new(loud, pos)).volume;
    let b = s.resolve(&l, &SoundSource::new(quiet, pos)).volume;
    assert!(b < a * 0.25, "moderated {b} vs unmoderated {a}");

    // At 100 m the unmoderated shot is still heard; the moderated one is gone.
    let far = Vec3::new(0.0, 0.0, -100.0);
    assert!(s.is_audible(&l, &SoundSource::new(loud, far)));
    assert!(!s.is_audible(&l, &SoundSource::new(quiet, far)));
}

#[test]
fn pest_rustles_are_class_distinguishable_by_profile() {
    let pests = [
        SoundKind::RatScurry,
        SoundKind::RabbitRustle,
        SoundKind::PossumShuffle,
        SoundKind::RaccoonRummage,
        SoundKind::HogRoot,
    ];
    // Strictly increasing loudness and reach:
    // rat < rabbit < possum < raccoon < hog.
    for w in pests.windows(2) {
        assert!(w[0].loudness() < w[1].loudness(), "{:?} vs {:?}", w[0], w[1]);
        assert!(w[0].falloff_m() < w[1].falloff_m());
    }
    // Distinct captions so the HUD can tell them apart too.
    let mut captions: Vec<_> = pests.iter().map(|p| p.caption()).collect();
    captions.sort_unstable();
    captions.dedup();
    assert_eq!(captions.len(), pests.len());
}

#[test]
fn rabbit_rustle_maps_and_sits_between_rat_and_possum() {
    use da_audio::species_sound;
    assert_eq!(
        species_sound(Species::Rabbit),
        Some(SoundKind::RabbitRustle)
    );
    assert_eq!(SoundKind::RabbitRustle.caption(), "rabbit nibbling");
    // Quieter and shorter-reaching than a possum, a touch above a rat.
    assert!(SoundKind::RabbitRustle.loudness() < SoundKind::PossumShuffle.loudness());
    assert!(SoundKind::RabbitRustle.falloff_m() < SoundKind::PossumShuffle.falloff_m());
    assert!(SoundKind::RabbitRustle.loudness() > SoundKind::RatScurry.loudness());
}

#[test]
fn gravel_footsteps_are_louder_than_grass() {
    assert!(
        SoundKind::Footstep(Surface::Gravel).loudness()
            > SoundKind::Footstep(Surface::Grass).loudness()
    );
    assert!(
        SoundKind::Footstep(Surface::Gravel).falloff_m()
            > SoundKind::Footstep(Surface::Grass).falloff_m()
    );
}

// -------------------------------------------------------------- subtitles

#[test]
fn subtitle_directions_at_known_geometries() {
    // forward = -Z, right = +X
    let cases = [
        (Vec3::new(0.0, 0.0, -10.0), Direction8::N),
        (Vec3::new(7.0, 0.0, -7.0), Direction8::NE),
        (Vec3::new(10.0, 0.0, 0.0), Direction8::E),
        (Vec3::new(7.0, 0.0, 7.0), Direction8::SE),
        (Vec3::new(0.0, 0.0, 10.0), Direction8::S),
        (Vec3::new(-7.0, 0.0, 7.0), Direction8::SW),
        (Vec3::new(-10.0, 0.0, 0.0), Direction8::W),
        (Vec3::new(-7.0, 0.0, -7.0), Direction8::NW),
    ];
    for (pos, want) in cases {
        let sub = resolve(SoundKind::DogBark, pos).subtitle();
        assert_eq!(sub.direction, want, "at {pos:?}");
    }
}

#[test]
fn a_source_behind_the_listener_pans_correctly_and_is_captioned_south() {
    let s = scene();
    let l = listener();
    // Directly behind: pan is centred (front/back is ambiguous in stereo),
    // and the caption is the only thing that disambiguates it.
    let back = s.resolve(&l, &SoundSource::new(SoundKind::ZombieMoan, Vec3::new(0.0, 0.0, 20.0)));
    assert!(back.pan.abs() < 1e-5);
    assert_eq!(back.subtitle().direction, Direction8::S);
    assert!(back.subtitle().direction.is_behind());

    // Behind-left pans left; behind-right pans right.
    let bl = s.resolve(&l, &SoundSource::new(SoundKind::ZombieMoan, Vec3::new(-14.0, 0.0, 14.0)));
    assert!(bl.pan < -0.6, "pan {}", bl.pan);
    assert_eq!(bl.subtitle().direction, Direction8::SW);
    assert!(bl.subtitle().direction.is_behind());

    let br = s.resolve(&l, &SoundSource::new(SoundKind::ZombieMoan, Vec3::new(14.0, 0.0, 14.0)));
    assert!(br.pan > 0.6, "pan {}", br.pan);
    assert_eq!(br.subtitle().direction, Direction8::SE);

    // Ahead is not "behind".
    assert!(!resolve(SoundKind::ZombieMoan, Vec3::new(0.0, 0.0, -20.0))
        .subtitle()
        .direction
        .is_behind());
}

#[test]
fn distance_bands_are_relative_to_the_kinds_own_falloff() {
    let k = SoundKind::ZombieMoan; // falloff 60 m
    let f = k.falloff_m();
    let band = |frac: f32| {
        resolve(k, Vec3::new(0.0, 0.0, -f * frac))
            .subtitle()
            .distance_band
    };
    assert_eq!(band(0.1), DistanceBand::Close);
    assert_eq!(band(0.24), DistanceBand::Close);
    assert_eq!(band(0.4), DistanceBand::Near);
    assert_eq!(band(0.59), DistanceBand::Near);
    assert_eq!(band(0.8), DistanceBand::Far);

    // 10 m is "far" for a rat (falloff 8 -> culled) but "close" for a moan.
    assert_eq!(
        resolve(SoundKind::ZombieMoan, Vec3::new(0.0, 0.0, -10.0))
            .subtitle()
            .distance_band,
        DistanceBand::Close
    );
    assert_eq!(
        resolve(SoundKind::RaccoonRummage, Vec3::new(0.0, 0.0, -12.0))
            .subtitle()
            .distance_band,
        DistanceBand::Far
    );
}

#[test]
fn subtitle_renders_the_hud_line() {
    // The line from the design brief.
    let sub = resolve(SoundKind::RaccoonRummage, Vec3::new(-6.0, 0.0, 0.0)).subtitle();
    assert_eq!(sub.to_line(), "raccoon rummaging — near, left");
}

#[test]
fn direction8_bearing_buckets_wrap() {
    assert_eq!(Direction8::from_bearing_deg(0.0), Direction8::N);
    assert_eq!(Direction8::from_bearing_deg(360.0), Direction8::N);
    assert_eq!(Direction8::from_bearing_deg(-45.0), Direction8::NW);
    assert_eq!(Direction8::from_bearing_deg(-180.0), Direction8::S);
    assert_eq!(Direction8::from_bearing_deg(179.0), Direction8::S);
}

// ----------------------------------------------------------- event mapping

fn ctx() -> EventAudioCtx {
    EventAudioCtx {
        player_pos: Vec3::new(1.0, 0.0, 2.0),
        moderated: false,
        surface: Surface::Gravel,
    }
}

fn kinds_of(ev: &SimEvent, c: &EventAudioCtx) -> Vec<SoundKind> {
    sounds_for_event(ev, c, |_| None)
        .into_iter()
        .map(|s| s.kind)
        .collect()
}

#[test]
fn discharge_maps_to_moderated_variant_when_moderated() {
    let ev = SimEvent::NoiseMade {
        pos: Vec3::new(0.0, 0.0, -3.0),
        radius_m: 60.0,
        kind: NoiseKind::Discharge,
    };
    assert_eq!(kinds_of(&ev, &ctx()), vec![SoundKind::RifleDischarge]);
    let mut m = ctx();
    m.moderated = true;
    assert_eq!(kinds_of(&ev, &m), vec![SoundKind::RifleDischargeModerated]);
}

#[test]
fn noise_kinds_map_to_their_sounds() {
    let at = Vec3::new(4.0, 0.0, 4.0);
    let mk = |k| SimEvent::NoiseMade {
        pos: at,
        radius_m: 10.0,
        kind: k,
    };
    assert_eq!(kinds_of(&mk(NoiseKind::PumpStroke), &ctx()), vec![SoundKind::PumpStroke]);
    assert_eq!(
        kinds_of(&mk(NoiseKind::Other), &ctx()),
        vec![SoundKind::Footstep(Surface::Gravel)]
    );
    // Position is carried through from the event.
    let srcs = sounds_for_event(&mk(NoiseKind::PumpStroke), &ctx(), |_| None);
    assert_eq!(srcs[0].pos, at);
}

#[test]
fn player_damage_maps_to_a_grunt_and_zombie_contact_adds_a_moan() {
    let hazard = SimEvent::PlayerDamaged {
        amount: 10.0,
        cause: DamageCause::Hazard(da_sim::hazard::HazardKind::CreekBank),
    };
    assert_eq!(kinds_of(&hazard, &ctx()), vec![SoundKind::PlayerGrunt]);

    let zed = SimEvent::PlayerDamaged {
        amount: 30.0,
        cause: DamageCause::ZombieContact,
    };
    assert_eq!(
        kinds_of(&zed, &ctx()),
        vec![SoundKind::PlayerGrunt, SoundKind::ZombieMoan]
    );

    // Harder hits grunt louder, and land on the player.
    let soft = sounds_for_event(&hazard, &ctx(), |_| None);
    let hard = sounds_for_event(&zed, &ctx(), |_| None);
    assert!(hard[0].gain > soft[0].gain);
    assert_eq!(soft[0].pos, ctx().player_pos);
}

#[test]
fn hits_and_kills_map_to_impact_plus_species_voice() {
    let pos = Vec3::new(-5.0, 0.0, -5.0);
    let wounded_hog = SimEvent::Wounded {
        id: EntityId(7),
        species: Species::JuvenileFeralHog,
        pos,
    };
    assert_eq!(
        kinds_of(&wounded_hog, &ctx()),
        vec![SoundKind::PelletImpact, SoundKind::HogGrunt]
    );

    // A rat is silent apart from the impact.
    let dead_rat = SimEvent::KillConfirmed {
        id: EntityId(1),
        species: Species::Rat,
        bounty_eligible: true,
        pos,
    };
    assert_eq!(kinds_of(&dead_rat, &ctx()), vec![SoundKind::PelletImpact]);

    // Zombies staggered by a body shot moan; destroyed ones do not.
    assert_eq!(
        kinds_of(&SimEvent::ZombieStaggered { id: EntityId(3) }, &ctx()),
        vec![SoundKind::PelletImpact, SoundKind::ZombieMoan]
    );
    assert_eq!(
        kinds_of(&SimEvent::ZombieDestroyed { id: EntityId(3) }, &ctx()),
        vec![SoundKind::PelletImpact]
    );
}

#[test]
fn friendly_hit_uses_the_entity_locator() {
    let where_the_dog_is = Vec3::new(20.0, 0.0, -20.0);
    let ev = SimEvent::FriendlyHit {
        id: EntityId(42),
        species: Species::Dog,
    };
    let srcs = sounds_for_event(&ev, &ctx(), |id| {
        (id == EntityId(42)).then_some(where_the_dog_is)
    });
    assert_eq!(
        srcs.iter().map(|s| s.kind).collect::<Vec<_>>(),
        vec![SoundKind::PelletImpact, SoundKind::DogBark]
    );
    assert!(srcs.iter().all(|s| s.pos == where_the_dog_is));

    // Without a locator it falls back to the player.
    let fallback = sounds_for_event(&ev, &ctx(), |_| None);
    assert_eq!(fallback[0].pos, ctx().player_pos);
}

#[test]
fn misc_events_map_as_expected() {
    assert_eq!(kinds_of(&SimEvent::DryFire, &ctx()), vec![SoundKind::PumpStroke]);
    assert!(kinds_of(&SimEvent::Missed { impact: None }, &ctx()).is_empty());
    assert_eq!(
        kinds_of(&SimEvent::Missed { impact: Some(Vec3::ZERO) }, &ctx()),
        vec![SoundKind::PelletImpact]
    );
    assert!(kinds_of(
        &SimEvent::HeatResidue {
            kind: da_sim::events::HeatKind::Barrel,
            pos: Vec3::ZERO
        },
        &ctx()
    )
    .is_empty());
    assert_eq!(
        kinds_of(
            &SimEvent::PestFled {
                id: EntityId(5),
                species: Species::Raccoon
            },
            &ctx()
        ),
        vec![SoundKind::RaccoonRummage]
    );
}

#[test]
fn a_whole_tick_maps_in_order() {
    let events = [
        SimEvent::NoiseMade {
            pos: Vec3::ZERO,
            radius_m: 60.0,
            kind: NoiseKind::Discharge,
        },
        SimEvent::Missed { impact: Some(Vec3::new(0.0, 0.0, -20.0)) },
        SimEvent::PestFled { id: EntityId(2), species: Species::Rat },
    ];
    let kinds: Vec<_> = sounds_for_events(&events, &ctx(), |_| None)
        .into_iter()
        .map(|s| s.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            SoundKind::RifleDischarge,
            SoundKind::PelletImpact,
            SoundKind::RatScurry
        ]
    );
}

// ------------------------------------------------------ synthesis + engine

#[test]
fn synthesis_is_deterministic_for_a_given_seed() {
    for kind in [
        SoundKind::RatScurry,
        SoundKind::ZombieMoan,
        SoundKind::RifleDischarge,
        SoundKind::Footstep(Surface::Mud),
        SoundKind::Weather(WeatherBed::Wind),
    ] {
        let a = synthesize(kind, 0xDEAD_A12);
        let b = synthesize(kind, 0xDEAD_A12);
        assert_eq!(a.samples, b.samples, "{kind:?} not reproducible");
        // Different seeds give variation, not a loop.
        let c = synthesize(kind, 0xDEAD_A13);
        assert_ne!(a.samples, c.samples, "{kind:?} ignores its seed");
    }
}

#[test]
fn synthesized_voices_are_sane_audio() {
    for kind in [
        SoundKind::RatScurry,
        SoundKind::PossumShuffle,
        SoundKind::RaccoonRummage,
        SoundKind::HogRoot,
        SoundKind::HogGrunt,
        SoundKind::ZombieMoan,
        SoundKind::ZombieDragStep,
        SoundKind::DogBark,
        SoundKind::CowLow,
        SoundKind::SheepBleat,
        SoundKind::CreekAmbience,
        SoundKind::RifleDischarge,
        SoundKind::RifleDischargeModerated,
        SoundKind::PumpStroke,
        SoundKind::PelletImpact,
        SoundKind::PlayerGrunt,
        SoundKind::Footstep(Surface::Gravel),
        SoundKind::Weather(WeatherBed::Rain),
    ] {
        let v = synthesize(kind, 7);
        assert!(!v.samples.is_empty(), "{kind:?} rendered nothing");
        assert!(v.samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{kind:?} clipped");
        // Not silence.
        let peak = v.samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "{kind:?} peak {peak}");
        // Declicked at both ends.
        assert!(v.samples[0].abs() < 1e-3, "{kind:?} clicks in");
        assert!(v.samples[v.samples.len() - 1].abs() < 1e-3, "{kind:?} clicks out");
        assert!((v.duration_s() - kind.profile().duration_s).abs() < 0.01);
    }
}

#[test]
fn synth_ids_are_unique_across_kinds() {
    let all = [
        SoundKind::RatScurry,
        SoundKind::PossumShuffle,
        SoundKind::RaccoonRummage,
        SoundKind::HogRoot,
        SoundKind::HogGrunt,
        SoundKind::GroundhogScrabble,
        SoundKind::BeaverSlap,
        SoundKind::ZombieMoan,
        SoundKind::ZombieDragStep,
        SoundKind::DogBark,
        SoundKind::CowLow,
        SoundKind::SheepBleat,
        SoundKind::CatMeow,
        SoundKind::CreekAmbience,
        SoundKind::RifleDischarge,
        SoundKind::RifleDischargeModerated,
        SoundKind::PumpStroke,
        SoundKind::PelletImpact,
        SoundKind::PlayerGrunt,
        SoundKind::Footstep(Surface::Grass),
        SoundKind::Footstep(Surface::Gravel),
        SoundKind::Footstep(Surface::Mud),
        SoundKind::Footstep(Surface::Wood),
        SoundKind::Footstep(Surface::Water),
        SoundKind::Weather(WeatherBed::Wind),
        SoundKind::Weather(WeatherBed::Rain),
    ];
    let mut ids: Vec<u64> = all.iter().map(|k| k.synth_id()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all.len());
}

#[test]
fn engine_drives_the_null_backend_and_returns_captions() {
    let mut engine = AudioEngine::new(NullBackend::new(), AudioScene::default(), 99);
    let l = listener();
    let sources = [
        SoundSource::new(SoundKind::ZombieMoan, Vec3::new(10.0, 0.0, 0.0)),
        // culled
        SoundSource::new(SoundKind::RatScurry, Vec3::new(0.0, 0.0, -400.0)),
    ];
    let captions = engine.play_frame(&l, &sources).expect("null backend cannot fail");
    assert_eq!(captions.len(), 1);
    assert_eq!(captions[0].to_line(), "zombie moaning — close, right");
    assert_eq!(engine.backend.kinds(), vec![SoundKind::ZombieMoan]);
    assert_eq!(engine.backend.listener, Some(l));
    assert!(engine.backend.cues[0].pan > 0.99);
    assert!(engine.backend.cues[0].volume > 0.0);

    engine.backend.stop_all();
    assert_eq!(engine.backend.stops, 1);
    engine.backend.clear();
    assert!(engine.backend.cues.is_empty());
}

#[test]
fn engine_replays_identically_from_the_same_seed() {
    let l = listener();
    let sources = [
        SoundSource::new(SoundKind::RaccoonRummage, Vec3::new(3.0, 0.0, -3.0)),
        SoundSource::new(SoundKind::RatScurry, Vec3::new(-2.0, 0.0, 1.0)),
    ];
    let run = || {
        let mut e = AudioEngine::new(NullBackend::new(), AudioScene::default(), 2024);
        e.play_frame(&l, &sources).expect("null backend cannot fail");
        e.backend.cues
    };
    assert_eq!(run(), run());
}

#[test]
fn engine_maps_and_plays_sim_events_end_to_end() {
    let mut engine = AudioEngine::new(NullBackend::new(), AudioScene::default(), 5);
    let l = listener();
    let events = [SimEvent::NoiseMade {
        pos: Vec3::new(0.0, 0.0, -5.0),
        radius_m: 60.0,
        kind: NoiseKind::Discharge,
    }];
    let captions = engine
        .play_events(&l, &events, &EventAudioCtx::default(), |_| None)
        .expect("null backend cannot fail");
    assert_eq!(captions[0].text, "gunshot");
    assert_eq!(captions[0].direction, Direction8::N);
    assert_eq!(engine.backend.kinds(), vec![SoundKind::RifleDischarge]);
}

#[test]
fn master_gain_scales_everything_and_can_silence_the_mix() {
    let l = listener();
    let src = [SoundSource::new(SoundKind::DogBark, Vec3::new(0.0, 0.0, -5.0))];
    let full = AudioScene::new(1.0).mix(&l, &src)[0].volume;
    let half = AudioScene::new(0.5).mix(&l, &src)[0].volume;
    assert!((full * 0.5 - half).abs() < 1e-6);
    assert!(AudioScene::new(0.0).mix(&l, &src).is_empty());
}
