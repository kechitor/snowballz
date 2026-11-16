//! What the relay server and the game share: the handshake and the room
//! codes.  (The relay server includes this file with #[path].)
//!
//! Every connection to the relay starts with one line:
//!
//!   SNOWBALLZ-RELAY 1 HOST                         -> OK <code> <token>
//!       the host's control connection; the relay then sends JOIN <id>
//!       lines on it, and both sides send PING / PONG to keep it alive
//!   SNOWBALLZ-RELAY 1 JOIN <code>                  -> OK  |  ERR <why>
//!       a player who wants to join; after OK it is the game connection
//!   SNOWBALLZ-RELAY 1 ACCEPT <code> <token> <id>   -> OK  |  ERR <why>
//!       the host's answer to JOIN <id>; after OK the relay passes
//!       everything between this connection and the player's
//!
//! After the OK lines the game talks over the connections exactly as over
//! a direct one.

#![allow(dead_code)]

use std::io::{self, Read};

pub const HELLO: &str = "SNOWBALLZ-RELAY 1";
pub const DEFAULT_PORT: u16 = 5555;

/// No 0/O, 1/I/L: codes are read aloud and typed.
pub const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 6;

/// "kq7-xm4" / "KQ7 XM4" / "KQ7XM4" -> "KQ7XM4", if it is a room code: six
/// letters and digits from the alphabet, with at least one digit (so a
/// computer name like "laptop" is not taken for a code).
pub fn normalize_code(text: &str) -> Option<String> {
    let t: String = text.chars().filter(|c| !matches!(c, '-' | ' ')).map(|c| c.to_ascii_uppercase()).collect();
    let ok = t.len() == CODE_LEN
        && t.bytes().all(|b| ALPHABET.contains(&b))
        && t.bytes().any(|b| b.is_ascii_digit())
        && !text.contains('.')
        && !text.contains(':');
    ok.then_some(t)
}

/// "KQ7XM4" -> "KQ7-XM4"
pub fn format_code(code: &str) -> String {
    if code.len() == CODE_LEN {
        format!("{}-{}", &code[..3], &code[3..])
    } else {
        code.to_string()
    }
}

/// Read one line ("\n" ends it, "\r" is dropped) one byte at a time, so
/// nothing after it is consumed.  At most `max` bytes.
pub fn read_line(r: &mut impl Read, max: usize) -> io::Result<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match r.read(&mut b)? {
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed")),
            _ if b[0] == b'\n' => break,
            _ if b[0] == b'\r' => {}
            _ => {
                out.push(b[0]);
                if out.len() > max {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
                }
            }
        }
    }
    String::from_utf8(out).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not text"))
}

/// Gathers a line from a stream with a read timeout, keeping the part read
/// before a timeout.  Returns Ok(None) on a timeout.
#[derive(Default)]
pub struct LineReader {
    buf: Vec<u8>,
}

impl LineReader {
    pub fn next(&mut self, r: &mut impl Read) -> io::Result<Option<String>> {
        let mut b = [0u8; 256];
        loop {
            if let Some(i) = self.buf.iter().position(|&c| c == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                let s = String::from_utf8_lossy(&line).trim().to_string();
                return Ok(Some(s));
            }
            if self.buf.len() > 4096 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
            }
            match r.read(&mut b) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed")),
                Ok(n) => self.buf.extend_from_slice(&b[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

/// A random number (std's hash keys are seeded from the system's random
/// source).
pub fn random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish()
}

/// A new room code (with at least one digit and one letter).
pub fn new_code() -> String {
    loop {
        let mut n = random_u64();
        let code: String = (0..CODE_LEN)
            .map(|_| {
                let c = ALPHABET[(n % ALPHABET.len() as u64) as usize] as char;
                n /= ALPHABET.len() as u64;
                c
            })
            .collect();
        if code.bytes().any(|b| b.is_ascii_digit()) && code.bytes().any(|b| b.is_ascii_alphabetic()) {
            return code;
        }
    }
}

/// A secret for the host (32 hex digits).
pub fn new_token() -> String {
    format!("{:016x}{:016x}", random_u64(), random_u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(normalize_code("kq7-xm4").as_deref(), Some("KQ7XM4"));
        assert_eq!(normalize_code(" KQ7 XM4 ").as_deref(), Some("KQ7XM4"));
        assert_eq!(normalize_code("laptop"), None); // no digit
        assert_eq!(normalize_code("192.168.1.20"), None);
        assert_eq!(normalize_code("KQ7XM0"), None); // 0 is not used
        assert_eq!(normalize_code("[::1]:55"), None);
        assert_eq!(format_code("KQ7XM4"), "KQ7-XM4");
        for _ in 0..200 {
            let c = new_code();
            assert_eq!(normalize_code(&c), Some(c.clone()));
        }
        assert_ne!(new_token(), new_token());
    }

    #[test]
    fn lines() {
        let mut r: &[u8] = b"OK ABC\r\nrest";
        assert_eq!(read_line(&mut r, 100).unwrap(), "OK ABC");
        assert_eq!(r, b"rest");
        let mut lr = LineReader::default();
        let mut r2: &[u8] = b"JOIN 1\nPONG\n";
        assert_eq!(lr.next(&mut r2).unwrap().as_deref(), Some("JOIN 1"));
        assert_eq!(lr.next(&mut r2).unwrap().as_deref(), Some("PONG"));
    }
}
