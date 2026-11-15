//! One JSON message per line, with a limit on how long a line may be.
use std::io::{self, BufRead, ErrorKind, Read, Write};

use serde::{Serialize, de::DeserializeOwned};

/// A gradient recipe is a few kilobytes; anything near this is not Amber.
pub const MAX_LINE: usize = 1 << 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    /// `line` holds one whole message, with its newline.
    Line,
    /// The read timed out. `line` keeps what arrived so far.
    Idle,
    Closed,
    TooLong,
}

/// Add to `line` what the stream has, up to the next newline. Keep calling
/// with the same `line` after [`Reading::Idle`], and clear it after
/// [`Reading::Line`].
pub fn read_line(reader: &mut impl BufRead, line: &mut String) -> io::Result<Reading> {
    let room = MAX_LINE.saturating_sub(line.len());
    if room == 0 {
        return Ok(Reading::TooLong);
    }
    match reader.by_ref().take(room as u64).read_line(line) {
        Ok(0) => Ok(Reading::Closed),
        Ok(_) if line.ends_with('\n') => Ok(Reading::Line),
        Ok(_) if line.len() >= MAX_LINE => Ok(Reading::TooLong),
        Ok(_) => Ok(Reading::Closed),
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Ok(Reading::Idle)
        }
        Err(error) => Err(error),
    }
}

pub fn encode(message: &impl Serialize) -> String {
    let mut line = serde_json::to_string(message).unwrap_or_default();
    line.push('\n');
    line
}

pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, String> {
    serde_json::from_str(line.trim_end()).map_err(|error| error.to_string())
}

pub fn write_line(writer: &mut impl Write, line: &str) -> io::Result<()> {
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn lines_come_out_one_at_a_time() {
        let mut reader = Cursor::new(b"one\ntwo\n".to_vec());
        let mut line = String::new();
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), Reading::Line);
        assert_eq!(line, "one\n");
        line.clear();
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), Reading::Line);
        assert_eq!(line, "two\n");
        line.clear();
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), Reading::Closed);
    }

    #[test]
    fn a_line_with_no_end_is_cut_off_at_the_limit() {
        let mut reader = Cursor::new(vec![b'x'; MAX_LINE + 10]);
        let mut line = String::new();
        assert_eq!(read_line(&mut reader, &mut line).unwrap(), Reading::TooLong);
        assert_eq!(line.len(), MAX_LINE);
    }

    #[test]
    fn a_message_goes_out_as_one_line_and_comes_back() {
        let line = encode(&vec!["a", "b"]);
        assert_eq!(line, "[\"a\",\"b\"]\n");
        assert_eq!(decode::<Vec<String>>(&line).unwrap(), ["a", "b"]);
        assert!(decode::<Vec<String>>("{").is_err());
    }
}
