//! The primary: owns the state and serves it to followers over TCP.
use std::{
    io::BufReader,
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, info, warn};

use super::{
    Exit, Inbound, Memory, Outbound, Settings, discovery,
    framing::{self, Reading},
    now_ms,
    wire::{Request, Shared, Standing, ToFollower, ToPrimary},
};

const HELLO_WAIT: Duration = Duration::from_secs(3);
const READ_WAIT: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(200);
const ACCEPT_POLL: Duration = Duration::from_millis(50);
const FAILED_WAIT: Duration = Duration::from_secs(2);
const MAX_FOLLOWERS: usize = 16;

struct Peer {
    connection: u64,
    lines: Sender<String>,
    stream: TcpStream,
}

#[derive(Default)]
struct Hub {
    peers: Vec<Peer>,
    state: Option<(Shared, Instant)>,
    gradient: Option<String>,
    /// Shared with the discovery thread, which tells other primaries about it.
    standing: Arc<Mutex<Standing>>,
    joined: u32,
    rejected: u32,
    requests: u32,
    interactions: u32,
    states_sent: u32,
}

impl Hub {
    fn broadcast(&mut self, line: &str) {
        self.peers
            .retain(|peer| peer.lines.send(line.to_owned()).is_ok());
    }

    fn standing(&self) -> Standing {
        *self.standing.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Someone used the app, on this machine or a follower's: note the time and
    /// tell the followers.
    fn touch(&mut self) {
        let standing = {
            let mut standing = self.standing.lock().unwrap_or_else(PoisonError::into_inner);
            standing.last_interaction_ms = standing.last_interaction_ms.max(now_ms());
            *standing
        };
        self.interactions += 1;
        self.broadcast(&framing::encode(&ToFollower::Standing(standing)));
    }

    /// What a new follower needs to catch up: the gradient, the standing, and
    /// the state as it is now.
    fn welcome(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.gradient.iter().cloned().collect();
        lines.push(framing::encode(&ToFollower::Standing(self.standing())));
        if let Some((state, at)) = &self.state {
            lines.push(framing::encode(&ToFollower::State(
                state.advanced_by(at.elapsed()),
            )));
        }
        lines
    }
}

fn locked(hub: &Mutex<Hub>) -> MutexGuard<'_, Hub> {
    hub.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Serve until another primary that outranks this one appears or the host goes away.
pub fn run(
    settings: &Settings,
    id: u64,
    outbound: &Receiver<Outbound>,
    inbound: &Sender<Inbound>,
    memory: &mut Memory,
) -> Exit {
    let listener = match TcpListener::bind(("0.0.0.0", settings.port))
        .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
    {
        Ok(listener) => listener,
        Err(error) => {
            return failed(
                inbound,
                &format!("port {} is not free", settings.port),
                &error,
            );
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let demoted = Arc::new(AtomicBool::new(false));
    let standing = Arc::new(Mutex::new(Standing {
        id,
        last_interaction_ms: memory.last_interaction_ms,
        priority: memory.priority,
    }));
    let discovery = match discovery::answer_probes(
        settings.port,
        standing.clone(),
        stop.clone(),
        demoted.clone(),
    ) {
        Ok(handle) => handle,
        Err(error) => return failed(inbound, "could not answer probes", &error),
    };
    let hub = Arc::new(Mutex::new(Hub {
        state: memory.state.clone(),
        gradient: memory.gradient.clone(),
        standing,
        ..Hub::default()
    }));
    let acceptor = {
        let (hub, inbound, stop) = (hub.clone(), inbound.clone(), stop.clone());
        let secret = settings.secret.clone();
        thread::Builder::new()
            .name("sync-accept".into())
            .spawn(move || accept_loop(listener, hub, secret, inbound, stop))
    };
    info!(
        port = settings.port,
        id,
        last_interaction_ms = memory.last_interaction_ms,
        priority = memory.priority,
        max_followers = MAX_FOLLOWERS,
        resumed_state = memory.state.is_some(),
        resumed_gradient = memory.gradient.is_some(),
        "serving as primary"
    );
    let exit = loop {
        if demoted.load(Ordering::Relaxed) {
            break Exit::Lost;
        }
        match outbound.recv_timeout(TICK) {
            Ok(Outbound::State(state)) => {
                let line = framing::encode(&ToFollower::State(state.clone()));
                let mut hub = locked(&hub);
                hub.state = Some((state, Instant::now()));
                hub.states_sent += 1;
                hub.broadcast(&line);
            }
            Ok(Outbound::Gradient(gradient)) => {
                let line = framing::encode(&ToFollower::Gradient(gradient));
                let mut hub = locked(&hub);
                hub.gradient = Some(line.clone());
                hub.broadcast(&line);
            }
            Ok(Outbound::Notice(text)) => {
                let line = framing::encode(&ToFollower::Notice(text));
                locked(&hub).broadcast(&line);
            }
            Ok(Outbound::Request(Request::Interacted)) => locked(&hub).touch(),
            Ok(Outbound::Request(request)) => {
                debug!(?request, "request ignored: this machine is the primary");
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Exit::Closed,
        }
    };
    stop.store(true, Ordering::Relaxed);
    {
        let mut hub = locked(&hub);
        for peer in hub.peers.drain(..) {
            let _ = peer.stream.shutdown(Shutdown::Both);
        }
        memory.state = hub.state.clone();
        memory.gradient = hub.gradient.clone();
        let standing = hub.standing();
        memory.last_interaction_ms = standing.last_interaction_ms;
        memory.priority = standing.priority;
        info!(
            ?exit,
            ?standing,
            followers_joined = hub.joined,
            followers_rejected = hub.rejected,
            requests_received = hub.requests,
            interactions = hub.interactions,
            states_sent = hub.states_sent,
            "primary stopped"
        );
    }
    let _ = discovery.join();
    if let Ok(acceptor) = acceptor {
        let _ = acceptor.join();
    }
    exit
}

fn failed(inbound: &Sender<Inbound>, what: &str, error: &std::io::Error) -> Exit {
    warn!(what, %error, "cannot serve as primary");
    let _ = inbound.send(Inbound::Notice(format!("Sync: {what}")));
    thread::sleep(FAILED_WAIT);
    Exit::Lost
}

fn accept_loop(
    listener: TcpListener,
    hub: Arc<Mutex<Hub>>,
    secret: String,
    inbound: Sender<Inbound>,
    stop: Arc<AtomicBool>,
) {
    let mut connection = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, address)) => {
                connection += 1;
                let (hub, secret, inbound, stop) =
                    (hub.clone(), secret.clone(), inbound.clone(), stop.clone());
                let spawned = thread::Builder::new()
                    .name("sync-follower".into())
                    .spawn(move || serve(stream, address, connection, hub, secret, inbound, stop));
                if let Err(error) = spawned {
                    warn!(%address, %error, "could not start a thread for a follower");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL);
            }
            Err(error) => {
                warn!(%error, "accepting a follower failed");
                thread::sleep(ACCEPT_POLL);
            }
        }
    }
}

fn serve(
    stream: TcpStream,
    address: SocketAddr,
    connection: u64,
    hub: Arc<Mutex<Hub>>,
    secret: String,
    inbound: Sender<Inbound>,
    stop: Arc<AtomicBool>,
) {
    let halves = stream
        .set_nonblocking(false)
        .and_then(|()| stream.set_nodelay(true))
        .and_then(|()| stream.set_read_timeout(Some(READ_WAIT)))
        .and_then(|()| Ok((stream.try_clone()?, stream.try_clone()?)));
    let (read_half, mut write_half) = match halves {
        Ok(halves) => halves,
        Err(error) => {
            debug!(%address, %error, "connection could not be set up");
            return;
        }
    };
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let deadline = Instant::now() + HELLO_WAIT;
    let hello = loop {
        match framing::read_line(&mut reader, &mut line) {
            Ok(Reading::Line) => break framing::decode::<ToPrimary>(&line),
            Ok(Reading::Idle) if Instant::now() < deadline && !stop.load(Ordering::Relaxed) => {}
            _ => {
                debug!(%address, "no hello arrived");
                return;
            }
        }
    };
    line.clear();
    let follower_id = match hello {
        Ok(ToPrimary::Hello { id, secret: given }) if given == secret => id,
        Ok(ToPrimary::Hello { id, .. }) => {
            warn!(%address, id, "follower turned away: wrong secret");
            turn_away(&hub, &mut write_half, "wrong SYNC_SECRET");
            return;
        }
        _ => {
            warn!(%address, "the first message was not a hello; closing");
            return;
        }
    };
    let (lines, queued) = mpsc::channel::<String>();
    {
        let mut hub = locked(&hub);
        if hub.peers.len() >= MAX_FOLLOWERS {
            hub.rejected += 1;
            drop(hub);
            warn!(%address, follower_id, max = MAX_FOLLOWERS, "follower turned away: too many");
            let _ = framing::write_line(
                &mut write_half,
                &framing::encode(&ToFollower::Rejected("too many followers".into())),
            );
            return;
        }
        for welcome in hub.welcome() {
            let _ = lines.send(welcome);
        }
        hub.peers.push(Peer {
            connection,
            lines,
            stream,
        });
        hub.joined += 1;
        info!(%address, follower_id, followers = hub.peers.len(), "follower joined");
    }
    let writer = thread::Builder::new()
        .name("sync-write".into())
        .spawn(move || {
            for line in queued {
                if framing::write_line(&mut write_half, &line).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = writer {
        warn!(%address, %error, "could not start a writer thread");
    }
    while !stop.load(Ordering::Relaxed) {
        match framing::read_line(&mut reader, &mut line) {
            Ok(Reading::Line) => {
                match framing::decode::<ToPrimary>(&line) {
                    Ok(ToPrimary::Request(request)) => {
                        debug!(%address, ?request, "request received");
                        let mut hub = locked(&hub);
                        hub.requests += 1;
                        hub.touch();
                        drop(hub);
                        if request != Request::Interacted
                            && inbound.send(Inbound::Request(request)).is_err()
                        {
                            break;
                        }
                    }
                    Ok(ToPrimary::Hello { .. }) => debug!(%address, "second hello ignored"),
                    Err(error) => {
                        warn!(%address, %error, "unreadable message; dropping the follower");
                        break;
                    }
                }
                line.clear();
            }
            Ok(Reading::Idle) => {}
            Ok(Reading::Closed) => break,
            Ok(Reading::TooLong) => {
                warn!(%address, limit = framing::MAX_LINE, "message too long; dropping the follower");
                break;
            }
            Err(error) => {
                debug!(%address, %error, "read failed");
                break;
            }
        }
    }
    locked(&hub)
        .peers
        .retain(|peer| peer.connection != connection);
    info!(%address, follower_id, "follower left");
}

fn turn_away(hub: &Mutex<Hub>, writer: &mut TcpStream, reason: &str) {
    locked(hub).rejected += 1;
    let _ = framing::write_line(
        writer,
        &framing::encode(&ToFollower::Rejected(reason.into())),
    );
}
