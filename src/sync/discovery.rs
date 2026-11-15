//! Finding the primary: ask on the network, and the primary answers.
use std::{
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, info, warn};

use super::{
    Settings,
    wire::{Datagram, Standing},
};

const PROBES: u32 = 3;
const PROBE_WAIT: Duration = Duration::from_millis(500);
const BEACON_EVERY: Duration = Duration::from_secs(2);
const BROADCAST: &str = "255.255.255.255";

fn decode(bytes: &[u8]) -> Option<Datagram> {
    serde_json::from_slice(bytes).ok()
}

fn encode(datagram: &Datagram) -> Vec<u8> {
    serde_json::to_vec(datagram).unwrap_or_default()
}

/// Ask for a primary, three times, and say where its server is. The broadcast
/// can miss on networks that block it, so the machine named by `--sync-peer`
/// and this machine are asked directly as well.
pub fn find_primary(settings: &Settings, own_id: u64) -> Option<SocketAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.set_broadcast(true)?;
            socket.set_read_timeout(Some(PROBE_WAIT))?;
            Ok(socket)
        })
        .inspect_err(|error| warn!(%error, "could not open the discovery socket"))
        .ok()?;
    let mut targets: Vec<SocketAddr> = Vec::new();
    for host in [BROADCAST, "127.0.0.1"]
        .into_iter()
        .chain(settings.peer.as_deref())
    {
        match (host, settings.port).to_socket_addrs() {
            Ok(found) => targets.extend(found.filter(SocketAddr::is_ipv4)),
            Err(error) => warn!(host, %error, "could not resolve a discovery target"),
        }
    }
    let probe = encode(&Datagram::Probe);
    let mut buffer = [0_u8; 512];
    for attempt in 1..=PROBES {
        for target in &targets {
            if let Err(error) = socket.send_to(&probe, target) {
                debug!(%target, %error, "probe not sent");
            }
        }
        let until = Instant::now() + PROBE_WAIT;
        while Instant::now() < until {
            let Ok((length, from)) = socket.recv_from(&mut buffer) else {
                break;
            };
            if let Some(Datagram::Primary { port, standing }) = decode(&buffer[..length]) {
                if standing.id != own_id {
                    info!(attempt, primary = %from.ip(), port, ?standing, "primary found");
                    return Some(SocketAddr::new(from.ip(), port));
                }
            }
        }
    }
    info!(probes = PROBES, "no primary answered");
    None
}

/// Answer probes for as long as `stop` is unset, and announce this primary on
/// the network every few seconds. Two primaries that hear each other settle it:
/// the one that does not outrank the other (see [`Standing`]) sets `demoted`.
/// `standing` is the primary's own, which the primary keeps up to date.
pub fn answer_probes(
    port: u16,
    standing: Arc<Mutex<Standing>>,
    stop: Arc<AtomicBool>,
    demoted: Arc<AtomicBool>,
) -> std::io::Result<thread::JoinHandle<()>> {
    let socket = UdpSocket::bind(("0.0.0.0", port))?;
    socket.set_broadcast(true)?;
    socket.set_read_timeout(Some(PROBE_WAIT))?;
    thread::Builder::new()
        .name("sync-discovery".into())
        .spawn(move || {
            let own = || *standing.lock().unwrap_or_else(PoisonError::into_inner);
            let announcement = || encode(&Datagram::Primary { port, standing: own() });
            let mut next_beacon = Instant::now();
            let mut buffer = [0_u8; 512];
            let (mut answered, mut beacons) = (0_u32, 0_u32);
            while !stop.load(Ordering::Relaxed) {
                if Instant::now() >= next_beacon {
                    next_beacon = Instant::now() + BEACON_EVERY;
                    beacons += 1;
                    if let Err(error) = socket.send_to(&announcement(), (BROADCAST, port)) {
                        debug!(%error, "beacon not sent");
                    }
                }
                let Ok((length, from)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                match decode(&buffer[..length]) {
                    Some(Datagram::Probe) => {
                        answered += 1;
                        if let Err(error) = socket.send_to(&announcement(), from) {
                            debug!(%from, %error, "answer not sent");
                        }
                    }
                    Some(Datagram::Primary { standing: other, .. })
                        if other.id != own().id && other.outranks(&own()) =>
                    {
                        warn!(%from, ?other, own = ?own(), "another primary outranks this one; stepping down");
                        demoted.store(true, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
            info!(answered, beacons, "discovery stopped");
        })
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;

    use super::*;

    fn standing(last_interaction_ms: u64, priority: i64, id: u64) -> Standing {
        Standing {
            id,
            last_interaction_ms,
            priority,
        }
    }

    fn free_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn settings(port: u16) -> Settings {
        Settings {
            port,
            peer: None,
            key: crate::sync::Key::parse(&"11".repeat(32)).unwrap(),
        }
    }

    #[test]
    fn a_primary_is_found_by_a_probe_and_not_by_its_own_id() {
        let port = free_port();
        let stop = Arc::new(AtomicBool::new(false));
        let demoted = Arc::new(AtomicBool::new(false));
        let own = Arc::new(Mutex::new(standing(0, 0, 5)));
        let handle = answer_probes(port, own, stop.clone(), demoted).unwrap();
        let found = find_primary(&settings(port), 9).expect("the primary answers");
        assert_eq!(found.port(), port);
        assert!(
            find_primary(&settings(port), 5).is_none(),
            "own answers do not count"
        );
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn a_primary_steps_down_only_for_one_that_outranks_it() {
        let port = free_port();
        let stop = Arc::new(AtomicBool::new(false));
        let demoted = Arc::new(AtomicBool::new(false));
        let own = standing(1_000, 0, 5);
        let handle = answer_probes(
            port,
            Arc::new(Mutex::new(own)),
            stop.clone(),
            demoted.clone(),
        )
        .unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        // A probe is answered after every earlier datagram has been read.
        let settled = || {
            find_primary(&settings(port), 99).expect("the primary answers");
        };
        let hear = |other: Standing| {
            let beacon = encode(&Datagram::Primary {
                port,
                standing: other,
            });
            sender.send_to(&beacon, ("127.0.0.1", port)).unwrap();
            settled();
        };

        hear(standing(999, 5, 1));
        assert!(
            !demoted.load(Ordering::Relaxed),
            "an earlier interaction loses"
        );
        hear(standing(1_000, -1, 1));
        assert!(
            !demoted.load(Ordering::Relaxed),
            "a lower priority loses a tie"
        );
        hear(own);
        assert!(
            !demoted.load(Ordering::Relaxed),
            "its own beacon does not count"
        );

        hear(standing(1_001, -3, 9));
        assert!(demoted.load(Ordering::Relaxed), "a later interaction wins");
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn a_returning_old_primary_wins_a_tie_with_the_follower_that_took_over() {
        let port = free_port();
        let stop = Arc::new(AtomicBool::new(false));
        let demoted = Arc::new(AtomicBool::new(false));
        let promoted = standing(1_000, -1, 7);
        let handle = answer_probes(
            port,
            Arc::new(Mutex::new(promoted)),
            stop.clone(),
            demoted.clone(),
        )
        .unwrap();
        let old_primary = standing(1_000, 0, 8);
        let beacon = encode(&Datagram::Primary {
            port,
            standing: old_primary,
        });
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .send_to(&beacon, ("127.0.0.1", port))
            .unwrap();
        find_primary(&settings(port), 99).expect("the primary answers");
        assert!(demoted.load(Ordering::Relaxed));
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }
}
