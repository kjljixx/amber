//! A follower: connects to the primary, hands its messages to the host and
//! sends the host's requests back.
use std::{
    io::BufReader,
    net::{SocketAddr, TcpStream},
    sync::mpsc::{Receiver, Sender, TryRecvError},
    time::{Duration, Instant},
};

use tracing::{debug, info, warn};

use super::{
    Exit, Inbound, Memory, Outbound, Settings,
    framing::{self, Reading},
    wire::{ToFollower, ToPrimary},
};

const CONNECT_WAIT: Duration = Duration::from_secs(2);
/// How often to look for requests to send between reads.
const POLL_WAIT: Duration = Duration::from_millis(50);
/// A primary that says nothing for this long is gone.
const SILENCE_LIMIT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Stats {
    states: u32,
    standings: u32,
    gradients: u32,
    notices: u32,
    requests_sent: u32,
}

pub fn run(
    address: SocketAddr,
    settings: &Settings,
    id: u64,
    outbound: &Receiver<Outbound>,
    inbound: &Sender<Inbound>,
    memory: &mut Memory,
) -> Exit {
    let stale = outbound.try_iter().count();
    if stale > 0 {
        debug!(stale, "dropped messages queued before this connection");
    }
    let connected = TcpStream::connect_timeout(&address, CONNECT_WAIT).and_then(|stream| {
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(POLL_WAIT))?;
        Ok((stream.try_clone()?, stream))
    });
    let (mut writer, stream) = match connected {
        Ok(streams) => streams,
        Err(error) => {
            warn!(%address, %error, "could not connect to the primary");
            return Exit::Lost;
        }
    };
    let hello = framing::encode(&ToPrimary::Hello {
        id,
        secret: settings.secret.clone(),
    });
    if framing::write_line(&mut writer, &hello).is_err() {
        warn!(%address, "could not say hello to the primary");
        return Exit::Lost;
    }
    info!(%address, "connected to the primary");
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut stats = Stats::default();
    let mut last_heard = Instant::now();
    let exit = loop {
        loop {
            match outbound.try_recv() {
                Ok(Outbound::Request(request)) => {
                    debug!(?request, "request sent");
                    stats.requests_sent += 1;
                    let line = framing::encode(&ToPrimary::Request(request));
                    if framing::write_line(&mut writer, &line).is_err() {
                        warn!(%address, "could not send to the primary");
                        break;
                    }
                }
                Ok(_) => debug!("message for followers ignored: this machine is a follower"),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Exit::Closed,
            }
        }
        match framing::read_line(&mut reader, &mut line) {
            Ok(Reading::Line) => {
                last_heard = Instant::now();
                match framing::decode::<ToFollower>(&line) {
                    Ok(ToFollower::State(state)) => {
                        stats.states += 1;
                        memory.state = Some((state.clone(), last_heard));
                        let _ = inbound.send(Inbound::State(state));
                    }
                    Ok(ToFollower::Standing(standing)) => {
                        stats.standings += 1;
                        memory.last_interaction_ms = standing.last_interaction_ms;
                        memory.priority = standing.priority.saturating_sub(1);
                    }
                    Ok(ToFollower::Gradient(gradient)) => {
                        stats.gradients += 1;
                        memory.gradient = Some(line.clone());
                        let _ = inbound.send(Inbound::Gradient(gradient));
                    }
                    Ok(ToFollower::Notice(text)) => {
                        stats.notices += 1;
                        let _ = inbound.send(Inbound::Notice(text));
                    }
                    Ok(ToFollower::Rejected(reason)) => {
                        warn!(%address, reason, "the primary turned this machine away");
                        let _ = inbound.send(Inbound::Notice(format!("Sync: {reason}")));
                        break Exit::Rejected;
                    }
                    Err(error) => {
                        warn!(%address, %error, "unreadable message from the primary");
                        break Exit::Lost;
                    }
                }
                line.clear();
            }
            Ok(Reading::Idle) => {}
            Ok(Reading::Closed) => {
                info!(%address, "the primary closed the connection");
                break Exit::Lost;
            }
            Ok(Reading::TooLong) => {
                warn!(%address, "message from the primary too long");
                break Exit::Lost;
            }
            Err(error) => {
                info!(%address, %error, "the connection to the primary failed");
                break Exit::Lost;
            }
        }
        if last_heard.elapsed() > SILENCE_LIMIT {
            warn!(%address, silent_secs = SILENCE_LIMIT.as_secs(), "the primary went quiet");
            break Exit::Lost;
        }
    };
    info!(
        ?exit,
        states = stats.states,
        standings = stats.standings,
        last_interaction_ms = memory.last_interaction_ms,
        priority = memory.priority,
        gradients = stats.gradients,
        notices = stats.notices,
        requests_sent = stats.requests_sent,
        "follower disconnected"
    );
    exit
}
