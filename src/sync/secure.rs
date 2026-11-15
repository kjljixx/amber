//! Keeps sync traffic secret and safe from changes, using the Noise protocol.
//!
//! Every machine holds one [`Key`]. A new connection starts with a handshake
//! that only a machine with the same key can finish. It gives both sides keys for
//! this connection alone, so an old recording is of no use. After it, each
//! message is encrypted by a [`Channel`] and sent as one line of hex.
use std::{
    fmt,
    io::{self, BufRead, Write},
    time::{Duration, Instant},
};

use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};

use super::framing::{self, Reading};

pub const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
/// A change to the protocol changes this, so old and new builds fail the handshake.
const PROLOGUE: &[u8] = b"amber-sync/1";
const KEY_BYTES: usize = 32;
const TAG_BYTES: usize = 16;
/// Noise allows 65535 bytes in a message, and the tag is part of them.
const MAX_MESSAGE: usize = 65535;
pub const MAX_PLAINTEXT: usize = MAX_MESSAGE - TAG_BYTES;

/// The secret every machine of a group shares: 32 random bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; KEY_BYTES]);

impl fmt::Debug for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Key(hidden)")
    }
}

impl Key {
    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0_u8; KEY_BYTES];
        getrandom::fill(&mut bytes).map_err(|error| format!("Could not make a key: {error}"))?;
        Ok(Self(bytes))
    }

    /// 64 hex characters, as [`Key::to_hex`] writes them.
    pub fn parse(text: &str) -> Option<Self> {
        from_hex(text.trim())?.try_into().ok().map(Self)
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|start| u8::from_str_radix(&text[start..start + 2], 16).ok())
        .collect()
}

#[derive(Debug)]
pub enum HandshakeError {
    /// The other side said nothing in time.
    NoAnswer,
    /// The other side closed the connection or sent something that does not
    /// fit: most likely a different key, or a machine that is not Amber.
    Refused,
    Io(io::Error),
    /// The Noise library refused to start. This is a bug.
    Setup(snow::Error),
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAnswer => formatter.write_str("no answer in time"),
            Self::Refused => formatter.write_str("refused: a different key, or not Amber"),
            Self::Io(error) => write!(formatter, "network error: {error}"),
            Self::Setup(error) => write!(formatter, "could not start the handshake: {error}"),
        }
    }
}

/// One connection's encryption. Each side seals what it sends and opens what it
/// gets. Every message must be opened once, in the order it was sealed.
pub struct Channel {
    transport: TransportState,
}

impl fmt::Debug for Channel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Channel(hidden)")
    }
}

impl Channel {
    /// The message as one line of hex. A message over [`MAX_PLAINTEXT`] is refused.
    pub fn seal(&mut self, plaintext: &str) -> Result<String, String> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(format!(
                "a message of {} bytes is over the limit of {MAX_PLAINTEXT}",
                plaintext.len()
            ));
        }
        let mut sealed = vec![0_u8; plaintext.len() + TAG_BYTES];
        let length = self
            .transport
            .write_message(plaintext.as_bytes(), &mut sealed)
            .map_err(|error| format!("could not encrypt: {error}"))?;
        let mut line = to_hex(&sealed[..length]);
        line.push('\n');
        Ok(line)
    }

    /// The message inside one line from [`Channel::seal`]. A line that was
    /// changed, repeated, or made with another key is an error.
    pub fn open(&mut self, line: &str) -> Result<String, String> {
        let sealed = from_hex(line.trim_end()).ok_or("the message is not hex")?;
        let mut plaintext = vec![0_u8; sealed.len()];
        let length = self
            .transport
            .read_message(&sealed, &mut plaintext)
            .map_err(|error| format!("could not decrypt: {error}"))?;
        plaintext.truncate(length);
        String::from_utf8(plaintext).map_err(|_| "the message is not text".to_string())
    }
}

fn handshake_state(key: &Key, initiator: bool) -> Result<HandshakeState, HandshakeError> {
    let params: NoiseParams = PATTERN.parse().map_err(HandshakeError::Setup)?;
    let builder = Builder::new(params)
        .psk(0, &key.0)
        .and_then(|builder| builder.prologue(PROLOGUE))
        .map_err(HandshakeError::Setup)?;
    let state = if initiator {
        builder.build_initiator()
    } else {
        builder.build_responder()
    };
    state.map_err(HandshakeError::Setup)
}

fn send(writer: &mut impl Write, bytes: &[u8]) -> Result<(), HandshakeError> {
    let mut line = to_hex(bytes);
    line.push('\n');
    framing::write_line(writer, &line).map_err(HandshakeError::Io)
}

fn receive(reader: &mut impl BufRead, deadline: Instant) -> Result<Vec<u8>, HandshakeError> {
    let mut line = String::new();
    loop {
        match framing::read_line(reader, &mut line) {
            Ok(Reading::Line) => return from_hex(line.trim_end()).ok_or(HandshakeError::Refused),
            Ok(Reading::Idle) if Instant::now() < deadline => {}
            Ok(Reading::Idle) => return Err(HandshakeError::NoAnswer),
            Ok(Reading::Closed | Reading::TooLong) => return Err(HandshakeError::Refused),
            Err(error) => return Err(HandshakeError::Io(error)),
        }
    }
}

fn finish(state: HandshakeState) -> Result<Channel, HandshakeError> {
    state
        .into_transport_mode()
        .map(|transport| Channel { transport })
        .map_err(HandshakeError::Setup)
}

/// The follower's half: speak first, then wait up to `wait` for the answer.
pub fn initiate(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    key: &Key,
    wait: Duration,
) -> Result<Channel, HandshakeError> {
    let deadline = Instant::now() + wait;
    let mut state = handshake_state(key, true)?;
    let mut buffer = vec![0_u8; MAX_MESSAGE];
    let length = state
        .write_message(&[], &mut buffer)
        .map_err(HandshakeError::Setup)?;
    send(writer, &buffer[..length])?;
    let answer = receive(reader, deadline)?;
    state
        .read_message(&answer, &mut buffer)
        .map_err(|_| HandshakeError::Refused)?;
    finish(state)
}

/// The primary's half: wait until `deadline` for the first message. A message
/// made with another key fails here, before any other work is done.
pub fn respond(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    key: &Key,
    deadline: Instant,
) -> Result<Channel, HandshakeError> {
    let mut state = handshake_state(key, false)?;
    let mut buffer = vec![0_u8; MAX_MESSAGE];
    let request = receive(reader, deadline)?;
    state
        .read_message(&request, &mut buffer)
        .map_err(|_| HandshakeError::Refused)?;
    let length = state
        .write_message(&[], &mut buffer)
        .map_err(HandshakeError::Setup)?;
    send(writer, &buffer[..length])?;
    finish(state)
}

#[cfg(test)]
mod tests {
    use std::{
        io::BufReader,
        net::{TcpListener, TcpStream},
        thread,
    };

    use super::*;

    const WAIT: Duration = Duration::from_secs(3);

    fn key_of(byte: u8) -> Key {
        Key([byte; KEY_BYTES])
    }

    /// Two ends of one connection, with the short read timeouts sync uses.
    fn connection() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        for stream in [&client, &server] {
            stream
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
        }
        (client, server)
    }

    /// A follower with `follower_key` and a primary with `primary_key` meet.
    fn meet(
        follower_key: &Key,
        primary_key: &Key,
    ) -> (
        Result<Channel, HandshakeError>,
        Result<Channel, HandshakeError>,
    ) {
        let (client, server) = connection();
        let primary_key = primary_key.clone();
        let primary = thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut writer = server;
            respond(
                &mut reader,
                &mut writer,
                &primary_key,
                Instant::now() + WAIT,
            )
        });
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut writer = client;
        let follower = initiate(&mut reader, &mut writer, follower_key, WAIT);
        (follower, primary.join().unwrap())
    }

    #[test]
    fn a_key_is_64_hex_characters() {
        let key = Key::generate().unwrap();
        assert_eq!(key.to_hex().len(), 64);
        assert_eq!(Key::parse(&key.to_hex()), Some(key.clone()));
        assert_eq!(
            Key::parse(&format!("  {}\n", key.to_hex().to_uppercase())),
            Some(key.clone())
        );
        let bad_keys = [
            String::new(),
            "abcd".to_string(),
            "a".repeat(63),
            "a".repeat(65),
            "g".repeat(64),
            format!("+{}", "a".repeat(63)),
            "pomodoro".repeat(8),
        ];
        for bad in &bad_keys {
            assert_eq!(Key::parse(bad), None, "{bad:?}");
        }
        assert_eq!(format!("{key:?}"), "Key(hidden)");
        assert_ne!(key, Key::generate().unwrap(), "keys are random");
    }

    #[test]
    fn the_same_key_opens_what_it_sealed_in_both_directions() {
        let key = key_of(7);
        let (follower, primary) = meet(&key, &key);
        let (mut follower, mut primary) = (follower.unwrap(), primary.unwrap());
        for round in 0..3 {
            let line = follower.seal(&format!("request {round}")).unwrap();
            assert!(!line.contains("request"), "the line is not readable");
            assert_eq!(primary.open(&line).unwrap(), format!("request {round}"));
            let line = primary.seal(&format!("state {round}")).unwrap();
            assert_eq!(follower.open(&line).unwrap(), format!("state {round}"));
        }
    }

    #[test]
    fn a_different_key_fails_the_handshake_on_both_sides() {
        let (follower, primary) = meet(&key_of(1), &key_of(2));
        assert!(
            matches!(primary, Err(HandshakeError::Refused)),
            "{primary:?}"
        );
        assert!(
            matches!(follower, Err(HandshakeError::Refused)),
            "{follower:?}"
        );
    }

    #[test]
    fn a_changed_message_is_refused() {
        let key = key_of(3);
        let (follower, primary) = meet(&key, &key);
        let (mut follower, mut primary) = (follower.unwrap(), primary.unwrap());
        let line = follower.seal("pause").unwrap();
        let first = line.chars().next().unwrap();
        let changed = line.replacen(first, if first == '0' { "1" } else { "0" }, 1);
        assert!(primary.open(&changed).is_err());
        assert!(primary.open("not hex at all\n").is_err());
    }

    #[test]
    fn a_replayed_message_is_refused() {
        let key = key_of(4);
        let (follower, primary) = meet(&key, &key);
        let (mut follower, mut primary) = (follower.unwrap(), primary.unwrap());
        let line = follower.seal("pause").unwrap();
        assert_eq!(primary.open(&line).unwrap(), "pause");
        assert!(primary.open(&line).is_err());
    }

    #[test]
    fn a_message_over_the_limit_is_refused() {
        let key = key_of(5);
        let (follower, primary) = meet(&key, &key);
        let (mut follower, mut primary) = (follower.unwrap(), primary.unwrap());
        assert!(follower.seal(&"x".repeat(MAX_PLAINTEXT + 1)).is_err());
        let line = follower.seal(&"x".repeat(MAX_PLAINTEXT)).unwrap();
        assert_eq!(primary.open(&line).unwrap().len(), MAX_PLAINTEXT);
    }

    #[test]
    fn a_plain_text_line_is_not_a_handshake() {
        let (mut client, server) = connection();
        let mut reader = BufReader::new(server.try_clone().unwrap());
        let mut writer = server;
        client
            .write_all(b"{\"Hello\":{\"id\":1,\"secret\":\"x\"}}\n")
            .unwrap();
        let result = respond(&mut reader, &mut writer, &key_of(1), Instant::now() + WAIT);
        assert!(matches!(result, Err(HandshakeError::Refused)), "{result:?}");
    }
}
