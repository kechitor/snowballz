//! Game codes: hosting and joining through a relay server (see
//! src/bin/snowballz-relay.rs), so nobody has to open a port on their
//! router.  The host keeps a control connection to the relay; for every
//! player who types the code the relay asks the host to open one more
//! connection, and joins the two.  The game then runs over these
//! connections exactly as over direct ones.

use crate::net;
use crate::relay_common::*;
use crate::settings;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The relay server this copy of the game uses ("host:port"), if any:
/// SNOWBALLZ_RELAY, else [network] relay in the settings file, else the
/// RELAY file the game was built with.  "off" turns codes off.
pub fn server() -> Option<String> {
    let built_in = include_str!("../RELAY").lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("").to_string();
    let v = std::env::var("SNOWBALLZ_RELAY").ok().or_else(settings::relay).unwrap_or(built_in);
    let v = v.trim();
    (!v.is_empty() && !v.eq_ignore_ascii_case("off")).then(|| v.to_string())
}

/// If `text` is a game code and this game has a relay server: the code.
pub fn code_in(text: &str) -> Option<String> {
    server()?;
    normalize_code(text)
}

fn connect(server: &str) -> Result<TcpStream, String> {
    let (host, port) = net::parse_address(server, DEFAULT_PORT)?;
    let addrs: Vec<_> = (host.as_str(), port).to_socket_addrs().map_err(|_| format!("the relay server {} was not found", host))?.collect();
    let mut last = String::from("no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(6)) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => {
                let t = e.to_string();
                last = t.split(" (os error").next().unwrap_or(&t).to_string();
            }
        }
    }
    Err(format!("the relay server {} can't be reached ({})", server, last))
}

fn send(mut s: &TcpStream, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    s.write_all(format!("{}\n", line).as_bytes())
}

/// Join the game with `code`: a connection to its host, through the relay.
pub fn join(code: &str) -> Result<TcpStream, String> {
    let shown = format_code(code);
    let fail = |why: String| format!("Could not join game {}: {}", shown, why);
    let server = server().ok_or_else(|| fail("this copy of the game has no relay server for codes; type the host's address instead".into()))?;
    let s = connect(&server).map_err(fail)?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(20)));
    send(&s, &format!("{} JOIN {}", HELLO, code)).map_err(|e| fail(e.to_string()))?;
    let answer = read_line(&mut &s, 300).map_err(|_| fail("the relay server did not answer".into()))?;
    match answer.strip_prefix("ERR ") {
        None if answer == "OK" => {
            let _ = s.set_read_timeout(None);
            Ok(s)
        }
        Some(why) => Err(fail(why.trim_end_matches('.').to_string())),
        None => Err(fail("strange answer from the relay server".into())),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// no relay server configured
    Off,
    Connecting,
    /// friends can join with this code
    Ready(String),
    Failed(String),
}

/// A hosted game's registration at the relay (while the host plays).
pub struct RelayHost {
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    control: Arc<Mutex<Option<TcpStream>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl RelayHost {
    /// Open a game at the relay; players who join arrive on `adopt` as
    /// connections for the game's server.
    pub fn start(adopt: Sender<TcpStream>) -> RelayHost {
        let h = RelayHost {
            status: Arc::new(Mutex::new(Status::Off)),
            stop: Arc::new(AtomicBool::new(false)),
            control: Arc::new(Mutex::new(None)),
        };
        if let Some(server) = server() {
            *lock(&h.status) = Status::Connecting;
            let (status, stop, control) = (h.status.clone(), h.stop.clone(), h.control.clone());
            let _ = std::thread::Builder::new().name("relay".into()).spawn(move || run(&server, adopt, &status, &stop, &control));
        }
        h
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    /// Close the game at the relay.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(c) = lock(&self.control).take() {
            let _ = c.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for RelayHost {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Register, serve JOIN requests, and register again if the relay
/// connection is lost (until stopped).
fn run(server: &str, adopt: Sender<TcpStream>, status: &Mutex<Status>, stop: &AtomicBool, control: &Mutex<Option<TcpStream>>) {
    let mut announced = String::new();
    while !stop.load(Ordering::SeqCst) {
        let result = register(server).and_then(|(stream, code, token)| {
            *lock(control) = stream.try_clone().ok();
            if stop.load(Ordering::SeqCst) {
                let _ = stream.shutdown(Shutdown::Both);
                return Ok(());
            }
            *lock(status) = Status::Ready(code.clone());
            if announced != code {
                println!("Friends can join with the code {}", format_code(&code));
                announced = code.clone();
            }
            serve(server, &stream, &code, &token, &adopt, stop)
        });
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let why = match result {
            Ok(()) => "the relay server closed the connection".to_string(),
            Err(e) => e,
        };
        println!("Relay server: {} (trying again)", why);
        *lock(status) = Status::Failed(why);
        // try again in a few seconds (the code will be a new one)
        let until = Instant::now() + Duration::from_secs(8);
        while Instant::now() < until && !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(200));
        }
        if !stop.load(Ordering::SeqCst) {
            *lock(status) = Status::Connecting;
        }
    }
}

/// Open a game: (control connection, code, token).
fn register(server: &str) -> Result<(TcpStream, String, String), String> {
    let s = connect(server)?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    send(&s, &format!("{} HOST", HELLO)).map_err(|e| e.to_string())?;
    let answer = read_line(&mut &s, 300).map_err(|_| "the relay server did not answer".to_string())?;
    let words: Vec<&str> = answer.split_whitespace().collect();
    match words.as_slice() {
        ["OK", code, token] => Ok((s, code.to_string(), token.to_string())),
        _ => Err(answer.strip_prefix("ERR ").unwrap_or("strange answer from the relay server").trim_end_matches('.').to_string()),
    }
}

/// Answer JOIN requests until the connection ends.
fn serve(server: &str, stream: &TcpStream, code: &str, token: &str, adopt: &Sender<TcpStream>, stop: &AtomicBool) -> Result<(), String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut lines = LineReader::default();
    let (mut last_heard, mut last_ping) = (Instant::now(), Instant::now());
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        match lines.next(&mut &*stream) {
            Ok(Some(l)) => {
                last_heard = Instant::now();
                if let Some(id) = l.strip_prefix("JOIN ") {
                    let (server, code, token, id, adopt) = (server.to_string(), code.to_string(), token.to_string(), id.to_string(), adopt.clone());
                    std::thread::spawn(move || {
                        if let Ok(s) = accept(&server, &code, &token, &id) {
                            let _ = adopt.send(s);
                        }
                    });
                }
            }
            Ok(None) => {}
            Err(_) => return Err("lost the connection to the relay server".into()),
        }
        if last_heard.elapsed() > Duration::from_secs(80) {
            return Err("the relay server stopped answering".into());
        }
        if last_ping.elapsed() > Duration::from_secs(25) {
            last_ping = Instant::now();
            send(stream, "PING").map_err(|_| "lost the connection to the relay server".to_string())?;
        }
    }
}

/// The host's connection for one joining player.
fn accept(server: &str, code: &str, token: &str, id: &str) -> Result<TcpStream, String> {
    let s = connect(server)?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    send(&s, &format!("{} ACCEPT {} {} {}", HELLO, code, token, id)).map_err(|e| e.to_string())?;
    match read_line(&mut &s, 300) {
        Ok(l) if l == "OK" => {
            let _ = s.set_read_timeout(None);
            Ok(s)
        }
        Ok(l) => Err(l),
        Err(e) => Err(e.to_string()),
    }
}
