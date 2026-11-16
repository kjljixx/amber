//! What machines say to each other. Every message is one line of JSON.
use std::{cmp::Reverse, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{Recipe, generation::GeneratedGradient};

/// The screen a machine shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewWire {
    Home,
    Task,
}

/// Which stretch of the cycle it is, as `Phase` in the `amber` binary says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseWire {
    pub work: bool,
    pub period: u8,
}

/// Time left rather than a deadline, so machines need not agree on the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockWire {
    pub paused: bool,
    pub remaining_ms: u64,
}

impl ClockWire {
    /// Whether `self` is not what `earlier` would be after `elapsed` of its own
    /// running: the cycle was reset, or the clock jumped.
    pub fn drifted_from(&self, earlier: &ClockWire, elapsed: Duration) -> bool {
        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let expected = if earlier.paused {
            earlier.remaining_ms
        } else {
            earlier.remaining_ms.saturating_sub(elapsed_ms)
        };
        self.remaining_ms.abs_diff(expected) > DRIFT_LIMIT_MS
    }
}

const DRIFT_LIMIT_MS: u64 = 500;

/// What the primary owns and every follower mirrors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shared {
    pub view: ViewWire,
    pub phase: PhaseWire,
    pub clock: ClockWire,
    /// The description shown above the clock.
    pub caption: Option<String>,
    /// The task the focus check judges the screen against.
    pub task: Option<String>,
}

impl Shared {
    /// Equal in everything but the time left.
    pub fn same_apart_from_time(&self, other: &Shared) -> bool {
        self.view == other.view
            && self.phase == other.phase
            && self.clock.paused == other.clock.paused
            && self.caption == other.caption
            && self.task == other.task
    }

    /// This state as it is `elapsed` later.
    pub fn advanced_by(&self, elapsed: Duration) -> Shared {
        let mut later = self.clone();
        if !later.clock.paused {
            let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
            later.clock.remaining_ms = later.clock.remaining_ms.saturating_sub(elapsed_ms);
        }
        later
    }
}

/// The parts of a generated gradient that a follower needs to show it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GradientWire {
    pub recipe: Recipe,
    pub radians_per_second: f64,
    pub energy: f32,
}

impl From<&GeneratedGradient> for GradientWire {
    fn from(gradient: &GeneratedGradient) -> Self {
        Self {
            recipe: gradient.recipe.clone(),
            radians_per_second: gradient.radians_per_second,
            energy: gradient.design.energy,
        }
    }
}

/// What a person asks for at a keyboard. The primary decides what happens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    TogglePause,
    ResetCycle,
    SelectTask(String),
    GoHome,
    NewLyric,
    /// A person used this machine in a way that changes nothing else, such as
    /// opening the task menu.
    Interacted,
    /// A scheduled time of day, as `HH:MM`, came round on this machine. No
    /// person acted, so it does not count as an interaction.
    ScheduledStart(String),
}

/// What settles it when two primaries meet: the later interaction wins, then
/// the higher priority, then the lower id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Standing {
    pub id: u64,
    /// Unix time of the last interaction on any machine of the group. 0 if none.
    pub last_interaction_ms: u64,
    /// The primary's number. A follower's own number is one lower.
    pub priority: i64,
}

impl Standing {
    pub fn outranks(&self, other: &Standing) -> bool {
        (self.last_interaction_ms, self.priority, Reverse(self.id))
            > (other.last_interaction_ms, other.priority, Reverse(other.id))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ToFollower {
    State(Shared),
    Standing(Standing),
    Gradient(GradientWire),
    Notice(String),
    Rejected(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ToPrimary {
    Hello { id: u64 },
    Request(Request),
}

/// What machines shout on the network to find the primary.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum Datagram {
    Probe,
    Primary { port: u16, standing: Standing },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(remaining_ms: u64, paused: bool) -> Shared {
        Shared {
            view: ViewWire::Task,
            phase: PhaseWire {
                work: true,
                period: 1,
            },
            clock: ClockWire {
                paused,
                remaining_ms,
            },
            caption: Some("write the report".into()),
            task: None,
        }
    }

    #[test]
    fn a_state_survives_a_round_trip_through_a_line() {
        let line = serde_json::to_string(&ToFollower::State(shared(90_000, false))).unwrap();
        assert!(!line.contains('\n'));
        match serde_json::from_str::<ToFollower>(&line).unwrap() {
            ToFollower::State(state) => assert_eq!(state, shared(90_000, false)),
            other => panic!("wrong message: {other:?}"),
        }
    }

    #[test]
    fn a_running_clock_runs_down_and_a_paused_one_holds() {
        let running = shared(10_000, false).advanced_by(Duration::from_secs(4));
        assert_eq!(running.clock.remaining_ms, 6_000);
        let paused = shared(10_000, true).advanced_by(Duration::from_secs(4));
        assert_eq!(paused.clock.remaining_ms, 10_000);
        let past = shared(1_000, false).advanced_by(Duration::from_secs(4));
        assert_eq!(past.clock.remaining_ms, 0);
    }

    #[test]
    fn drift_means_the_clock_jumped() {
        let before = shared(10_000, false).clock;
        let on_time = shared(6_100, false).clock;
        let reset = shared(25 * 60 * 1000, false).clock;
        assert!(!on_time.drifted_from(&before, Duration::from_secs(4)));
        assert!(reset.drifted_from(&before, Duration::from_secs(4)));
    }

    fn standing(last_interaction_ms: u64, priority: i64, id: u64) -> Standing {
        Standing {
            id,
            last_interaction_ms,
            priority,
        }
    }

    #[test]
    fn the_later_interaction_wins_then_priority_then_the_lower_id() {
        let later = standing(2_000, -5, 900);
        let earlier = standing(1_000, 5, 1);
        assert!(later.outranks(&earlier) && !earlier.outranks(&later));
        let higher = standing(1_000, 0, 900);
        let lower = standing(1_000, -1, 1);
        assert!(higher.outranks(&lower) && !lower.outranks(&higher));
        let first = standing(1_000, 0, 1);
        let second = standing(1_000, 0, 2);
        assert!(first.outranks(&second) && !second.outranks(&first));
        assert!(!first.outranks(&first));
    }

    #[test]
    fn a_machine_that_never_interacted_loses_to_one_that_did() {
        let idle = standing(0, 0, 1);
        let used = standing(1, -9, 2);
        assert!(used.outranks(&idle));
    }

    #[test]
    fn a_beacon_survives_a_round_trip() {
        let sent = Datagram::Primary {
            port: 47821,
            standing: standing(1_700_000_000_000, -2, u64::MAX),
        };
        let bytes = serde_json::to_vec(&sent).unwrap();
        match serde_json::from_slice::<Datagram>(&bytes).unwrap() {
            Datagram::Primary { port, standing } => {
                assert_eq!(port, 47821);
                assert_eq!(standing, self::standing(1_700_000_000_000, -2, u64::MAX));
            }
            other => panic!("wrong datagram: {other:?}"),
        }
    }

    #[test]
    fn states_that_differ_only_in_time_are_the_same() {
        assert!(shared(1, false).same_apart_from_time(&shared(2, false)));
        assert!(!shared(1, false).same_apart_from_time(&shared(1, true)));
    }
}
