//! First-night tutorial (NFR-2): teaches optics switching, the power
//! tradeoff, and the friendly-fire rule — by watching what the player
//! actually does, not by locking them in a corridor.
//!
//! The tutorial is a small state machine over the sim's event stream. It
//! emits prompts into the field log and retires each step once satisfied,
//! so a player who already knows the game blows through it in a minute.

use da_sim::SimEvent;

/// One teaching beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// "You're on the naked eye. Raise the rifle to scope."
    Scope,
    /// Multi-pump needs charging before every shot.
    Pump,
    /// Take the first rat.
    FirstKill,
    /// The dog is warm and raccoon-sized at distance — identify before firing.
    IdentifyFriendly,
    /// Watch the clock; get back before dawn.
    Dawn,
    /// Nothing left to teach.
    Done,
}

impl Step {
    /// The prompt shown when this step becomes active.
    pub fn prompt(self) -> &'static str {
        match self {
            Step::Scope => {
                "TUTORIAL: The wide view is your walk. Put your gaze near an \
                 animal and LEFT-CLICK to lock it — the scope rises on its \
                 own, zoomed to the range. Q lowers the rifle. The wobble \
                 you see is you: SHIFT holds your breath, briefly."
            }
            Step::Pump => {
                "TUTORIAL: The multi-pump holds no air between shots. Work the \
                 lever (Pump button) before every shot — it takes time and makes \
                 noise. That's the tier-1 tax."
            }
            Step::FirstKill => {
                "TUTORIAL: Rats feed near the barn. Lock one, settle the \
                 sway, hold high for drop and into the wind for drift, then \
                 LEFT-CLICK to fire. Head kills and pays; a body hit wounds, \
                 pays nothing, and spooks everything nearby."
            }
            Step::IdentifyFriendly => {
                "TUTORIAL: That warm blob may be the farm dog — at distance a dog \
                 reads the same size as a raccoon. Positive ID before the \
                 trigger. Hitting a friendly costs cash AND reputation, and the \
                 game refuses shots with a friendly behind the target."
            }
            Step::Dawn => {
                "TUTORIAL: Bounties only pay when you return to camp. Watch the \
                 clock — dawn ends the night wherever you're standing."
            }
            Step::Done => "",
        }
    }

    fn next(self) -> Step {
        match self {
            Step::Scope => Step::Pump,
            Step::Pump => Step::FirstKill,
            Step::FirstKill => Step::IdentifyFriendly,
            Step::IdentifyFriendly => Step::Dawn,
            Step::Dawn | Step::Done => Step::Done,
        }
    }
}

/// Tracks tutorial progress across one night.
#[derive(Debug)]
pub struct Tutorial {
    step: Step,
    announced: bool,
    /// Set once the player has scoped at least once.
    pub scoped_once: bool,
    /// Set once the rifle has been pumped to a firing charge.
    pub pumped_once: bool,
}

impl Default for Tutorial {
    fn default() -> Self {
        Self::new()
    }
}

impl Tutorial {
    pub fn new() -> Self {
        Self {
            step: Step::Scope,
            announced: false,
            scoped_once: false,
            pumped_once: false,
        }
    }

    /// Is the tutorial finished?
    pub fn is_done(&self) -> bool {
        self.step == Step::Done
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// Advance the tutorial. Call once per frame with this frame's inputs and
    /// sim events; returns any prompt that should be appended to the log.
    pub fn update(
        &mut self,
        scoped: bool,
        can_fire: bool,
        night_t: f32,
        events: &[SimEvent],
    ) -> Option<&'static str> {
        if self.step == Step::Done {
            return None;
        }
        if scoped {
            self.scoped_once = true;
        }
        if can_fire {
            self.pumped_once = true;
        }

        // Announce the current step the first time we see it.
        if !self.announced {
            self.announced = true;
            return Some(self.step.prompt());
        }

        let satisfied = match self.step {
            Step::Scope => self.scoped_once,
            Step::Pump => self.pumped_once,
            Step::FirstKill => events
                .iter()
                .any(|e| matches!(e, SimEvent::KillConfirmed { .. })),
            // Satisfied by learning the lesson the easy way (a refused or
            // survived encounter) or the hard way (hitting a friendly).
            Step::IdentifyFriendly => {
                events.iter().any(|e| matches!(e, SimEvent::FriendlyHit { .. }))
                    || night_t > 0.5
            }
            Step::Dawn => night_t > 0.8,
            Step::Done => true,
        };
        if satisfied {
            self.step = self.step.next();
            self.announced = false;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_sim::Species;
    use glam::Vec3;

    fn kill() -> SimEvent {
        SimEvent::KillConfirmed {
            id: da_core::EntityId(1),
            species: Species::Rat,
            bounty_eligible: true,
            pos: Vec3::ZERO,
        }
    }

    #[test]
    fn walks_the_player_through_every_step_in_order() {
        let mut t = Tutorial::new();
        // Each step announces once, then waits to be satisfied.
        assert!(t.update(false, false, 0.0, &[]).is_some());
        assert_eq!(t.step(), Step::Scope);
        assert!(t.update(false, false, 0.0, &[]).is_none());
        // Scoping satisfies step 1 and the next frame announces step 2.
        t.update(true, false, 0.0, &[]);
        assert_eq!(t.step(), Step::Pump);
        let p = t.update(false, false, 0.0, &[]).expect("pump prompt");
        assert!(p.contains("multi-pump"));

        t.update(false, true, 0.0, &[]); // pumped
        assert_eq!(t.step(), Step::FirstKill);
        t.update(false, true, 0.0, &[]); // announce
        t.update(false, true, 0.1, &[kill()]);
        assert_eq!(t.step(), Step::IdentifyFriendly);
        t.update(false, true, 0.1, &[]); // announce
        t.update(false, true, 0.6, &[]); // time satisfies it
        assert_eq!(t.step(), Step::Dawn);
        t.update(false, true, 0.6, &[]); // announce
        t.update(false, true, 0.9, &[]);
        assert!(t.is_done());
        assert!(t.update(false, true, 0.95, &[]).is_none());
    }

    #[test]
    fn every_live_step_has_a_prompt() {
        for s in [
            Step::Scope,
            Step::Pump,
            Step::FirstKill,
            Step::IdentifyFriendly,
            Step::Dawn,
        ] {
            assert!(!s.prompt().is_empty(), "{s:?} needs a prompt");
        }
        assert!(Step::Done.prompt().is_empty());
    }

    #[test]
    fn friendly_hit_also_satisfies_the_id_lesson() {
        let mut t = Tutorial::new();
        // Fast-forward to the ID step.
        for _ in 0..12 {
            t.update(true, true, 0.1, &[kill()]);
            if t.step() == Step::IdentifyFriendly {
                break;
            }
        }
        assert_eq!(t.step(), Step::IdentifyFriendly);
        t.update(true, true, 0.1, &[]); // announce
        t.update(
            true,
            true,
            0.1,
            &[SimEvent::FriendlyHit {
                id: da_core::EntityId(2),
                species: Species::Dog,
            }],
        );
        assert_eq!(t.step(), Step::Dawn, "learning it the hard way still counts");
    }
}
