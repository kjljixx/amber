//! Keeps the timer, the task and the gradient the same on every machine of a
//! local network.
//!
//! The first machine to start is the **primary**. It owns the state, and every
//! other machine, a **follower**, mirrors it. A follower never changes the state
//! itself: it sends a [`Request`], the primary acts on it, and the new state
//! goes out to all.
//!
//! | Part | Job |
//! |---|---|
//! | [`wire`] | The messages: [`Shared`] state, [`GradientWire`], [`Request`] |
//! | `discovery` | UDP: a follower asks, the primary answers |
//! | `primary` | TCP server: one thread to read and one to write for each follower |
//! | `follower` | TCP client: sends requests, hands the primary's messages on |
//! | `framing` | One JSON message per line, with a length limit |
//!
//! Everything runs on worker threads. The host talks to a [`Link`], which only
//! passes messages through channels, so it never waits for the network.
//!
//! If the primary goes quiet for five seconds, each follower waits a little, by a
//! different time for each, and asks again. The first to find nobody becomes
//! the primary. Should two primaries meet, one of them has to give way. These
//! decide it, in this order:
//!
//! 1. The later last interaction. Any person using any machine of a group sets
//!    it, and the group shares it. A machine that was away has an old one.
//! 2. The higher priority. The primary has a number and each follower has the
//!    number one lower. A follower that takes over keeps its own number, so an
//!    old primary that comes back wins a tie.
//! 3. The lower id. Every machine picks a random one when it starts.
mod discovery;
mod follower;
mod framing;
mod primary;
pub mod wire;

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, info};

pub use wire::{ClockWire, GradientWire, PhaseWire, Request, Shared, ViewWire};

pub const DEFAULT_PORT: u16 = 47821;
const SECRET_NAME: &str = "SYNC_SECRET";
/// How often the primary repeats its state, so that a follower can tell it is alive.
const HEARTBEAT: Duration = Duration::from_secs(2);
const STARTUP_WAIT: Duration = Duration::from_secs(5);
const REJECTED_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct Settings {
    pub port: u16,
    /// A machine to ask directly, for networks that do not pass broadcasts.
    pub peer: Option<String>,
    pub secret: String,
}

impl Settings {
    /// The secret comes from the environment, or else the dotenv file at `path`.
    /// Without one nothing is shared, because anyone on the network could join.
    pub fn from_env(path: &Path, port: u16, peer: Option<String>) -> Result<Self, String> {
        let file = match std::fs::read_to_string(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return Err("Could not read local .env configuration".into()),
        };
        let settings = crate::generation::dotenv_settings(&file, &[SECRET_NAME]);
        let secret = std::env::var(SECRET_NAME)
            .ok()
            .or_else(|| settings.get(SECRET_NAME).cloned())
            .unwrap_or_default();
        let secret = secret.trim().to_owned();
        if secret.is_empty() || secret.chars().any(char::is_control) {
            return Err(format!(
                "Sync needs {SECRET_NAME} in the environment or .env, the same on every machine"
            ));
        }
        Ok(Self { port, peer, secret })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Primary,
    Follower,
}

/// What the host hears from the network.
#[derive(Debug)]
pub enum Inbound {
    /// This machine became primary or follower. [`Link::role`] already says so.
    Role(Role),
    /// A follower asked for this (primary only).
    Request(Request),
    State(Shared),
    Gradient(GradientWire),
    /// Text for the top of the screen.
    Notice(String),
}

enum Outbound {
    State(Shared),
    Gradient(GradientWire),
    Notice(String),
    Request(Request),
}

/// Why a primary or follower stopped.
#[derive(Debug, PartialEq, Eq)]
enum Exit {
    /// The network side failed. Look for a primary again.
    Lost,
    /// The primary refused this machine.
    Rejected,
    /// The host is gone.
    Closed,
}

/// The latest state and gradient seen, so a follower that becomes primary
/// carries on from them.
#[derive(Default)]
struct Memory {
    state: Option<(Shared, Instant)>,
    gradient: Option<String>,
    /// Unix time of the last interaction anywhere in the group. 0 for none.
    last_interaction_ms: u64,
    /// This machine's priority. The first primary has 0.
    priority: i64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The host's end of the sync threads.
pub struct Link {
    outbound: Sender<Outbound>,
    inbound: Receiver<Inbound>,
    role: Role,
    last_published: Option<(Shared, Instant)>,
    published: u64,
}

impl Link {
    /// Start the threads and wait for them to settle on a role, which takes a
    /// second or two when there is no primary to find.
    pub fn start(settings: Settings) -> Result<Self, String> {
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes)
            .map_err(|error| format!("Could not make a sync id: {error}"))?;
        let id = u64::from_le_bytes(bytes);
        info!(
            port = settings.port,
            peer = settings.peer.as_deref(),
            id,
            "sync starting; the secret is set"
        );
        let (outbound, wanted) = mpsc::channel();
        let (heard, inbound) = mpsc::channel();
        thread::Builder::new()
            .name("sync".into())
            .spawn(move || manage(settings, id, wanted, heard))
            .map_err(|error| format!("Could not start sync: {error}"))?;
        let role = match inbound.recv_timeout(STARTUP_WAIT) {
            Ok(Inbound::Role(role)) => role,
            _ => return Err("Sync did not start".into()),
        };
        info!(?role, "sync started");
        Ok(Self {
            outbound,
            inbound,
            role,
            last_published: None,
            published: 0,
        })
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn is_follower(&self) -> bool {
        self.role == Role::Follower
    }

    /// Everything that has arrived since the last call.
    pub fn poll(&mut self) -> Vec<Inbound> {
        let mut messages = Vec::new();
        while let Ok(message) = self.inbound.try_recv() {
            if let Inbound::Role(role) = &message {
                info!(?role, "role changed");
                self.role = *role;
                self.last_published = None;
            }
            messages.push(message);
        }
        messages
    }

    /// Tell the followers the state, if it changed or the last word is a few
    /// seconds old. Call it as often as is convenient. Does nothing on a follower.
    pub fn publish(&mut self, state: &Shared, now: Instant) {
        if self.role != Role::Primary {
            return;
        }
        let due = match &self.last_published {
            None => true,
            Some((last, at)) => {
                let elapsed = now.saturating_duration_since(*at);
                !last.same_apart_from_time(state)
                    || state.clock.drifted_from(&last.clock, elapsed)
                    || elapsed >= HEARTBEAT
            }
        };
        if due {
            self.published += 1;
            self.last_published = Some((state.clone(), now));
            let _ = self.outbound.send(Outbound::State(state.clone()));
        }
    }

    pub fn publish_gradient(&mut self, gradient: GradientWire) {
        if self.role == Role::Primary {
            let _ = self.outbound.send(Outbound::Gradient(gradient));
        }
    }

    /// Show `text` at the top of every follower's screen.
    pub fn notice(&self, text: &str) {
        if self.role == Role::Primary {
            let _ = self.outbound.send(Outbound::Notice(text.to_owned()));
        }
    }

    /// A person used this machine. The primary notes the time and tells the
    /// followers; a follower asks the primary to do so. Call it for each action
    /// that matters to the group, not for every key.
    pub fn interacted(&self) {
        let _ = self.outbound.send(Outbound::Request(Request::Interacted));
    }

    /// Ask the primary for something. Does nothing on the primary itself.
    pub fn request(&self, request: Request) {
        if self.role == Role::Follower {
            let _ = self.outbound.send(Outbound::Request(request));
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        info!(role = ?self.role, states_published = self.published, "sync stopped");
    }
}

/// How long a follower waits before it looks for a primary again. It differs
/// from machine to machine, so they do not all decide to be primary at once.
fn election_delay(id: u64) -> Duration {
    Duration::from_millis(500 + id % 1000)
}

fn manage(settings: Settings, id: u64, outbound: Receiver<Outbound>, inbound: Sender<Inbound>) {
    let mut memory = Memory::default();
    let mut role = None;
    loop {
        let found = discovery::find_primary(&settings, id);
        let now = if found.is_some() {
            Role::Follower
        } else {
            Role::Primary
        };
        if role != Some(now) {
            role = Some(now);
            if inbound.send(Inbound::Role(now)).is_err() {
                return;
            }
        }
        let exit = match found {
            Some(address) => {
                follower::run(address, &settings, id, &outbound, &inbound, &mut memory)
            }
            None => primary::run(&settings, id, &outbound, &inbound, &mut memory),
        };
        match exit {
            Exit::Closed => return,
            Exit::Rejected => thread::sleep(REJECTED_WAIT),
            Exit::Lost if now == Role::Follower => {
                let wait = election_delay(id);
                debug!(
                    wait_ms = wait.as_millis() as u64,
                    "waiting before looking for a primary"
                );
                thread::sleep(wait);
            }
            Exit::Lost => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;

    use super::*;
    use wire::{ClockWire, PhaseWire, ViewWire};

    fn free_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn settings(port: u16, secret: &str) -> Settings {
        Settings {
            port,
            peer: None,
            secret: secret.into(),
        }
    }

    fn state(remaining_ms: u64, caption: &str) -> Shared {
        Shared {
            view: ViewWire::Task,
            phase: PhaseWire {
                work: true,
                period: 2,
            },
            clock: ClockWire {
                paused: false,
                remaining_ms,
            },
            caption: Some(caption.into()),
            task: Some("report".into()),
        }
    }

    fn wait_for<T>(link: &mut Link, mut found: impl FnMut(Inbound) -> Option<T>) -> Option<T> {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            for message in link.poll() {
                if let Some(result) = found(message) {
                    return Some(result);
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn a_follower_mirrors_the_primary_and_asks_it_for_things() {
        let port = free_port();
        let mut primary = Link::start(settings(port, "shared")).unwrap();
        assert_eq!(primary.role(), Role::Primary);
        let mut follower = Link::start(settings(port, "shared")).unwrap();
        assert_eq!(follower.role(), Role::Follower);

        primary.publish(&state(60_000, "write the report"), Instant::now());
        let got = wait_for(&mut follower, |message| match message {
            Inbound::State(state) => Some(state),
            _ => None,
        })
        .expect("the follower hears the state");
        assert_eq!(got.caption.as_deref(), Some("write the report"));
        assert!(got.clock.remaining_ms <= 60_000);

        primary.notice("Scheduled work: starting");
        let notice = wait_for(&mut follower, |message| match message {
            Inbound::Notice(text) => Some(text),
            _ => None,
        });
        assert_eq!(notice.as_deref(), Some("Scheduled work: starting"));

        follower.request(Request::TogglePause);
        let request = wait_for(&mut primary, |message| match message {
            Inbound::Request(request) => Some(request),
            _ => None,
        });
        assert_eq!(request, Some(Request::TogglePause));
    }

    /// Ask the primary on `port` for its standing. A primary binds its socket a
    /// moment after it says it has started, so the first probes may go unanswered.
    fn standing_of(port: u16) -> wire::Standing {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let probe = serde_json::to_vec(&wire::Datagram::Probe).unwrap();
        let mut buffer = [0_u8; 512];
        for _ in 0..25 {
            let answered = socket
                .send_to(&probe, ("127.0.0.1", port))
                .and_then(|_| socket.recv_from(&mut buffer));
            if let Ok((length, _)) = answered {
                if let Ok(wire::Datagram::Primary { standing, .. }) =
                    serde_json::from_slice(&buffer[..length])
                {
                    return standing;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        panic!("no primary answered on port {port}");
    }

    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if done() {
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn an_interaction_on_either_machine_sets_the_groups_time() {
        let port = free_port();
        let primary = Link::start(settings(port, "shared")).unwrap();
        let follower = Link::start(settings(port, "shared")).unwrap();
        assert_eq!(standing_of(port).last_interaction_ms, 0, "nobody has acted");

        primary.interacted();
        assert!(wait_until(|| standing_of(port).last_interaction_ms > 0));
        let first = standing_of(port).last_interaction_ms;

        thread::sleep(Duration::from_millis(20));
        follower.interacted();
        assert!(
            wait_until(|| standing_of(port).last_interaction_ms > first),
            "a follower's action counts too"
        );
    }

    #[test]
    fn a_follower_that_takes_over_keeps_the_time_and_has_a_lower_priority() {
        let port = free_port();
        let mut primary = Link::start(settings(port, "shared")).unwrap();
        primary.interacted();
        primary.publish(&state(60_000, "write the report"), Instant::now());
        assert!(wait_until(|| standing_of(port).last_interaction_ms > 0));
        let before = standing_of(port);
        assert_eq!(before.priority, 0, "the first primary starts at 0");

        let mut follower = Link::start(settings(port, "shared")).unwrap();
        wait_for(&mut follower, |message| match message {
            Inbound::State(_) => Some(()),
            _ => None,
        })
        .expect("the follower has caught up");

        drop(primary);
        wait_for(&mut follower, |message| match message {
            Inbound::Role(Role::Primary) => Some(()),
            _ => None,
        })
        .expect("the follower takes over");
        let after = standing_of(port);
        assert_eq!(after.last_interaction_ms, before.last_interaction_ms);
        assert_eq!(after.priority, before.priority - 1);
        assert_ne!(after.id, before.id);
    }

    #[test]
    fn a_machine_with_the_wrong_secret_is_turned_away() {
        let port = free_port();
        let _primary = Link::start(settings(port, "shared")).unwrap();
        let mut stranger = Link::start(settings(port, "guess")).unwrap();
        let notice = wait_for(&mut stranger, |message| match message {
            Inbound::Notice(text) => Some(text),
            _ => None,
        })
        .expect("the stranger is told");
        assert!(notice.contains("wrong"), "{notice}");
    }

    #[test]
    fn a_missing_secret_stops_sync_before_it_starts() {
        let path = std::env::temp_dir().join("amber-sync-test-no-such.env");
        let error = Settings::from_env(&path, DEFAULT_PORT, None);
        if std::env::var(SECRET_NAME).is_err() {
            assert!(error.unwrap_err().contains(SECRET_NAME));
        }
    }

    #[test]
    fn publishing_waits_for_a_change_or_the_heartbeat() {
        let (outbound, wanted) = mpsc::channel();
        let (_heard, inbound) = mpsc::channel();
        let mut link = Link {
            outbound,
            inbound,
            role: Role::Primary,
            last_published: None,
            published: 0,
        };
        let start = Instant::now();
        link.publish(&state(60_000, "a"), start);
        link.publish(&state(59_000, "a"), start + Duration::from_secs(1));
        assert_eq!(wanted.try_iter().count(), 1, "a quiet second sends nothing");
        link.publish(&state(58_000, "b"), start + Duration::from_secs(2));
        assert_eq!(
            wanted.try_iter().count(),
            1,
            "a new caption is sent at once"
        );
        link.publish(&state(25 * 60_000, "b"), start + Duration::from_secs(3));
        assert_eq!(wanted.try_iter().count(), 1, "a reset is sent at once");
        link.publish(
            &state(25 * 60_000 - 2_000, "b"),
            start + Duration::from_secs(5),
        );
        assert_eq!(wanted.try_iter().count(), 1, "the heartbeat repeats it");
        assert_eq!(link.published, 4);
    }
}
