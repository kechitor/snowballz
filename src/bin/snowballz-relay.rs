//! snowballz-relay: lets players join a hosted game with a room code, with
//! no port forwarding on anybody's router.  Run it on a computer that is
//! reachable from the internet (a small cloud server).  The host and the
//! players all connect *to* the relay, and it passes the game's traffic
//! between them.
//!
//!     snowballz-relay [--port 5555] [--max-rooms 500]
//!
//! (The port can also come from the PORT environment variable.)  No
//! dependencies; only the Rust standard library.

#[path = "../relay_common.rs"]
mod relay_common;

use relay_common::*;
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long a player waits for the host to answer.
const JOIN_WAIT: Duration = Duration::from_secs(15);
/// The host's control connection is dropped after this long without a
/// word (the game sends PING every 25 s).
const CONTROL_SILENCE: Duration = Duration::from_secs(80);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 4000;
const MAX_ROOMS_PER_ADDRESS: usize = 4;
const MAX_WAITING_PER_ROOM: usize = 16;

struct Waiting {
    stream: TcpStream,
    since: Instant,
}

struct Room {
    token: String,
    /// for writing JOIN lines to the host
    control: TcpStream,
    owner: IpAddr,
    waiting: HashMap<u64, Waiting>,
    next_id: u64,
}

#[derive(Default)]
struct Relay {
    rooms: HashMap<String, Room>,
    max_rooms: usize,
}

type Shared = Arc<Mutex<Relay>>;

static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Relay> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn log(text: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let _ = writeln!(io::stdout(), "{:02}:{:02}:{:02} UTC  {}", h, m, s, text);
}

fn send(mut s: &TcpStream, line: &str) -> io::Result<()> {
    s.write_all(format!("{}\n", line).as_bytes())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: snowballz-relay [--port N] [--max-rooms N]\n\nLets Snowballz players join games with a room code. Default port {}.", DEFAULT_PORT);
        return;
    }
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok());
    let port = arg("--port")
        .or_else(|| std::env::var("PORT").ok().and_then(|p| p.parse().ok()))
        .unwrap_or(DEFAULT_PORT as usize) as u16;
    let max_rooms = arg("--max-rooms").unwrap_or(500);
    let listener = match listen(port) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Could not open port {}: {}", port, e);
            std::process::exit(1);
        }
    };
    log(&format!("snowballz-relay {} listening on TCP port {}", include_str!("../../VERSION").trim(), port));
    run(listener, max_rooms);
}

/// IPv6 + IPv4 where the system allows one socket for both, else IPv4.
fn listen(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind((std::net::Ipv6Addr::UNSPECIFIED, port)).or_else(|_| TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)))
}

fn run(listener: TcpListener, max_rooms: usize) {
    let shared: Shared = Arc::new(Mutex::new(Relay { rooms: HashMap::new(), max_rooms }));
    {
        let s = shared.clone();
        thread::spawn(move || expire_waiting(s));
    }
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        if CONNECTIONS.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            let _ = send(&stream, "ERR The relay server is full, try again later.");
            continue;
        }
        let s = shared.clone();
        CONNECTIONS.fetch_add(1, Ordering::Relaxed);
        let spawned = thread::Builder::new().stack_size(64 * 1024).spawn(move || {
            let _ = handle(stream, s);
            CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
        });
        if spawned.is_err() {
            CONNECTIONS.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

fn handle(stream: TcpStream, shared: Shared) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let line = read_line(&mut &stream, 200)?;
    let Some(rest) = line.strip_prefix(HELLO) else {
        let _ = send(&stream, "ERR This is a Snowballz relay server; it needs a newer game.");
        return Ok(());
    };
    let words: Vec<&str> = rest.split_whitespace().collect();
    match words.as_slice() {
        ["HOST"] => host(stream, shared),
        ["JOIN", code] => join(stream, shared, code),
        ["ACCEPT", code, token, id] => accept(stream, shared, code, token, id),
        _ => send(&stream, "ERR Unknown request."),
    }
}

/// A host opens a room and keeps this connection for JOIN notices.
fn host(stream: TcpStream, shared: Shared) -> io::Result<()> {
    let owner = stream.peer_addr()?.ip();
    let token = new_token();
    let code = {
        let mut r = lock(&shared);
        if r.rooms.len() >= r.max_rooms {
            drop(r);
            return send(&stream, "ERR The relay server has too many games right now, try again later.");
        }
        if r.rooms.values().filter(|room| room.owner == owner).count() >= MAX_ROOMS_PER_ADDRESS {
            drop(r);
            return send(&stream, "ERR Too many games from your address.");
        }
        let mut code = new_code();
        while r.rooms.contains_key(&code) {
            code = new_code();
        }
        let control = stream.try_clone()?;
        r.rooms.insert(code.clone(), Room { token: token.clone(), control, owner, waiting: HashMap::new(), next_id: 1 });
        code
    };
    log(&format!("game {} opened ({} games)", format_code(&code), lock(&shared).rooms.len()));
    let result = control_loop(&stream, &code, &token);
    // the host left: close the room and the players still waiting
    if let Some(room) = lock(&shared).rooms.remove(&code) {
        for w in room.waiting.into_values() {
            let _ = send(&w.stream, "ERR The game was closed.");
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    log(&format!("game {} closed ({} games)", format_code(&code), lock(&shared).rooms.len()));
    result
}

fn control_loop(stream: &TcpStream, code: &str, token: &str) -> io::Result<()> {
    send(stream, &format!("OK {} {}", code, token))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut lines = LineReader::default();
    let mut last_heard = Instant::now();
    loop {
        match lines.next(&mut &*stream)? {
            Some(l) => {
                last_heard = Instant::now();
                if l == "PING" {
                    send(stream, "PONG")?;
                }
            }
            None if last_heard.elapsed() > CONTROL_SILENCE => {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "host silent"));
            }
            None => {}
        }
    }
}

/// A player asks for a room; they wait until the host accepts.
fn join(stream: TcpStream, shared: Shared, code: &str) -> io::Result<()> {
    let Some(code) = normalize_code(code) else {
        return send(&stream, &format!("ERR '{}' is not a game code.", code));
    };
    let mut r = lock(&shared);
    let Some(room) = r.rooms.get_mut(&code) else {
        drop(r);
        return send(&stream, &format!("ERR There is no game with the code {} (it may have ended).", format_code(&code)));
    };
    if room.waiting.len() >= MAX_WAITING_PER_ROOM {
        drop(r);
        return send(&stream, "ERR Too many players are joining at once, try again.");
    }
    let id = room.next_id;
    room.next_id += 1;
    if send(&room.control, &format!("JOIN {}", id)).is_err() {
        drop(r);
        return send(&stream, "ERR The host can't be reached.");
    }
    room.waiting.insert(id, Waiting { stream, since: Instant::now() });
    Ok(())
    // the host's ACCEPT (or expire_waiting) takes it from here
}

/// The host's new connection for player `id`: join the two.
fn accept(stream: TcpStream, shared: Shared, code: &str, token: &str, id: &str) -> io::Result<()> {
    let player = {
        let mut r = lock(&shared);
        let room = r.rooms.get_mut(code).filter(|room| room.token == token);
        match (room, id.parse::<u64>()) {
            (Some(room), Ok(id)) => room.waiting.remove(&id),
            _ => None,
        }
    };
    let Some(player) = player else {
        return send(&stream, "ERR That player is gone.");
    };
    send(&player.stream, "OK")?;
    send(&stream, "OK")?;
    log(&format!("player joined game {}", format_code(code)));
    pipe(stream, player.stream);
    Ok(())
}

/// Pass everything between the two connections until one side closes.
fn pipe(a: TcpStream, b: TcpStream) {
    for s in [&a, &b] {
        let _ = s.set_read_timeout(None);
        let _ = s.set_nodelay(true);
    }
    let (Ok(a2), Ok(b2)) = (a.try_clone(), b.try_clone()) else { return };
    let one_way = |mut from: TcpStream, mut to: TcpStream| {
        move || {
            let _ = io::copy(&mut from, &mut to);
            let _ = from.shutdown(Shutdown::Both);
            let _ = to.shutdown(Shutdown::Both);
        }
    };
    let t = thread::Builder::new().stack_size(64 * 1024).spawn(one_way(b2, a2));
    one_way(a, b)();
    if let Ok(t) = t {
        let _ = t.join();
    }
}

/// Players the host never answered get an error after JOIN_WAIT.
fn expire_waiting(shared: Shared) {
    loop {
        thread::sleep(Duration::from_secs(1));
        let mut expired = Vec::new();
        {
            let mut r = lock(&shared);
            for room in r.rooms.values_mut() {
                let old: Vec<u64> = room.waiting.iter().filter(|(_, w)| w.since.elapsed() > JOIN_WAIT).map(|(id, _)| *id).collect();
                for id in old {
                    if let Some(w) = room.waiting.remove(&id) {
                        expired.push(w.stream);
                    }
                }
            }
        }
        for s in expired {
            let _ = send(&s, "ERR The host did not answer.");
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn start() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || run(l, 10));
        port
    }

    fn connect(port: u16, line: &str) -> TcpStream {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        send(&s, &format!("{} {}", HELLO, line)).unwrap();
        s
    }

    #[test]
    fn host_and_player_talk_through_the_relay() {
        let port = start();
        let host = connect(port, "HOST");
        let ok = read_line(&mut &host, 200).unwrap();
        let w: Vec<&str> = ok.split(' ').collect();
        assert_eq!(w[0], "OK");
        let (code, token) = (w[1].to_string(), w[2].to_string());
        // a player joins with the code typed in lower case with a dash
        let player = connect(port, &format!("JOIN {}", format_code(&code).to_lowercase()));
        assert_eq!(read_line(&mut &host, 200).unwrap(), "JOIN 1");
        // a wrong token is refused
        let bad = connect(port, &format!("ACCEPT {} {} 1", code, "0000"));
        assert!(read_line(&mut &bad, 200).unwrap().starts_with("ERR"));
        let acc = connect(port, &format!("ACCEPT {} {} 1", code, token));
        assert_eq!(read_line(&mut &acc, 200).unwrap(), "OK");
        assert_eq!(read_line(&mut &player, 200).unwrap(), "OK");
        // bytes both ways
        (&player).write_all(b"hello host").unwrap();
        let mut buf = [0u8; 10];
        (&acc).read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello host");
        (&acc).write_all(&[1, 2, 3, 0, 255]).unwrap();
        let mut b2 = [0u8; 5];
        (&player).read_exact(&mut b2).unwrap();
        assert_eq!(b2, [1, 2, 3, 0, 255]);
        // the keep-alive
        send(&host, "PING").unwrap();
        assert_eq!(read_line(&mut &host, 200).unwrap(), "PONG");
        // the player leaves: the host's side closes too
        drop(player);
        assert_eq!((&acc).read(&mut buf).unwrap_or(0), 0);
        // the host leaves: the code stops working
        drop(host);
        thread::sleep(Duration::from_millis(200));
        let late = connect(port, &format!("JOIN {}", code));
        let l = read_line(&mut &late, 200).unwrap();
        assert!(l.starts_with("ERR There is no game"), "{}", l);
    }

    #[test]
    fn unknown_codes_and_old_games() {
        let port = start();
        let p = connect(port, "JOIN ABC234");
        assert!(read_line(&mut &p, 200).unwrap().starts_with("ERR There is no game with the code ABC-234"));
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        (&s).write_all(b"\x78\x00\x01hello\n").unwrap(); // not a relay request
        assert!(read_line(&mut &s, 200).unwrap().starts_with("ERR"));
    }
}
