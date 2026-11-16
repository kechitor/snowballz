//! Hosting a game over the internet: while the lobby is open, the game is
//! opened at the relay server (a game code, see relay.rs) and a background
//! thread asks the router (UPnP) to forward the game's port and for the
//! public address.  The lobby shows the code and the addresses friends can
//! join with.  Both are closed when the game ends.

use crate::relay::{self, RelayHost};
use crate::upnp::{self, Mapping};
use std::net::{Ipv4Addr, Ipv6Addr, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Internet {
    /// talking to the router
    Opening,
    /// the router forwards `port` (on the public side) to this computer
    Open { public: Option<Ipv4Addr>, port: u16 },
    /// automatic forwarding is turned off in the settings
    Off,
    Failed(String),
}

struct Shared {
    internet: Internet,
    mapping: Option<Mapping>,
    stopped: bool,
}

/// Every forwarding this program made, so that `close_all` can undo them
/// when the program is closed in the middle of a game.
static ACTIVE: Mutex<Vec<Arc<Mutex<Shared>>>> = Mutex::new(Vec::new());

pub struct Hosting {
    pub port: u16,
    pub lan: Option<Ipv4Addr>,
    pub ipv6: Option<Ipv6Addr>,
    shared: Arc<Mutex<Shared>>,
    relay: RelayHost,
}

impl Hosting {
    /// Start opening the game at the relay and `port` on the router (in the
    /// background).  `ipv6`: the server listens on IPv6 too.  Players who
    /// join through the relay are sent to `adopt` (the server's adopter).
    pub fn start(port: u16, ipv6: bool, adopt: Sender<TcpStream>) -> Hosting {
        let relay = RelayHost::start(adopt);
        let lan = upnp::lan_ipv4();
        let ipv6 = if ipv6 { upnp::global_ipv6() } else { None };
        let enabled = crate::settings::upnp_enabled();
        let shared = Arc::new(Mutex::new(Shared {
            internet: if enabled { Internet::Opening } else { Internet::Off },
            mapping: None,
            stopped: false,
        }));
        lock(&ACTIVE).push(shared.clone());
        if enabled {
            let sh = shared.clone();
            let spawned = std::thread::Builder::new().name("upnp".into()).spawn(move || {
                let r = upnp::open(port, &format!("Snowballz ({})", port), Duration::from_secs(3));
                let mut s = lock(&sh);
                match r {
                    Ok(m) if s.stopped => m.remove(), // the game ended meanwhile
                    Ok(m) => {
                        s.internet = Internet::Open { public: m.external_ip, port: m.external_port };
                        s.mapping = Some(m);
                    }
                    Err(e) => s.internet = Internet::Failed(e),
                }
                let line = describe(&s.internet, port);
                drop(s);
                println!("{}", line);
            });
            if spawned.is_err() {
                lock(&shared).internet = Internet::Failed("could not start".into());
            }
        }
        let h = Hosting { port, lan, ipv6, shared, relay };
        if let Some(ip) = h.lan {
            println!("Players on your network can join with: {}", crate::net::format_address(&ip.to_string(), port, crate::net::PORT));
        }
        if let Some(ip) = h.ipv6 {
            println!("Players with IPv6 can try: {}", crate::net::format_address(&ip.to_string(), port, crate::net::PORT));
        }
        h
    }

    pub fn internet(&self) -> Internet {
        lock(&self.shared).internet.clone()
    }

    pub fn relay(&self) -> relay::Status {
        self.relay.status()
    }

    /// The game code friends can join with, if there is one now.
    pub fn code(&self) -> Option<String> {
        match self.relay.status() {
            relay::Status::Ready(c) => Some(crate::relay_common::format_code(&c)),
            _ => None,
        }
    }

    /// Close the game at the relay and remove the forwarding from the
    /// router.
    pub fn stop(&self) {
        self.relay.stop();
        stop(&self.shared);
    }
}

impl Drop for Hosting {
    fn drop(&mut self) {
        self.stop();
        lock(&ACTIVE).retain(|s| !Arc::ptr_eq(s, &self.shared));
    }
}

fn stop(shared: &Arc<Mutex<Shared>>) {
    let m = {
        let mut s = lock(shared);
        s.stopped = true;
        s.mapping.take()
    };
    if let Some(m) = m {
        m.remove();
        println!("Closed port {} on the router again", m.external_port);
    }
}

/// Undo every forwarding (the program is about to exit).
pub fn close_all() {
    let all: Vec<_> = lock(&ACTIVE).clone();
    for s in &all {
        stop(s);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One line for the terminal (and the lobby's first line).
pub fn describe(i: &Internet, local_port: u16) -> String {
    match i {
        Internet::Opening => format!("Opening port {} on your router...", local_port),
        Internet::Off => "Automatic port forwarding is off ([network] upnp = 0)".into(),
        Internet::Failed(e) => format!("Could not open the port on your router automatically: {}", e),
        Internet::Open { public: Some(ip), port } if !upnp::is_private_v4(*ip) => {
            format!("Friends on the internet can join with: {}", crate::net::format_address(&ip.to_string(), *port, crate::net::PORT))
        }
        Internet::Open { public, .. } => match public {
            Some(ip) if upnp::is_cgnat_v4(*ip) => format!("The router opened the port, but its address {} is shared by your internet provider (carrier-grade NAT)", ip),
            Some(ip) => format!("The router opened the port, but it is behind another router (its address is {})", ip),
            None => "The router opened the port but did not say its public address".into(),
        },
    }
}

/// What the lobby shows for the game code: (code to copy, explanation).
pub fn code_lines(r: &relay::Status) -> (Option<String>, Vec<String>) {
    match r {
        relay::Status::Off => (None, Vec::new()),
        relay::Status::Connecting => (None, vec!["Opening the game at the relay server...".into()]),
        relay::Status::Ready(c) => (
            Some(crate::relay_common::format_code(c)),
            vec!["Friends type this code in Join a game. No router settings needed.".into()],
        ),
        relay::Status::Failed(e) => (None, vec![format!("No code right now: {}. Trying again...", e)]),
    }
}

/// What the lobby shows for the internet: (address to copy, explanation).
/// With a working game code the notes are kept short (the code is enough).
pub fn internet_lines(i: &Internet, local_port: u16, lan: Option<Ipv4Addr>, have_code: bool) -> (Option<String>, Vec<String>) {
    let (addr, notes) = internet_lines_full(i, local_port, lan);
    if have_code && addr.is_none() && !matches!(i, Internet::Opening) {
        return (None, vec!["Not reachable directly - friends can use the code above.".into()]);
    }
    (addr, notes)
}

fn internet_lines_full(i: &Internet, local_port: u16, lan: Option<Ipv4Addr>) -> (Option<String>, Vec<String>) {
    let forward_by_hand = |why: &str| {
        let to = lan.map(|ip| format!(" to {}", ip)).unwrap_or_default();
        vec![
            why.to_string(),
            format!(
                "To play over the internet, forward TCP port {}{} in your router's settings, then give friends your public address (search the web for \"what is my IP\").",
                local_port, to
            ),
        ]
    };
    match i {
        Internet::Opening => (None, vec![format!("Opening port {} on your router...", local_port)]),
        Internet::Off => (None, forward_by_hand("Automatic port opening is turned off in the settings file.")),
        Internet::Failed(e) => (None, forward_by_hand(&format!("The port could not be opened automatically: {}.", e))),
        Internet::Open { public: Some(ip), port } if !upnp::is_private_v4(*ip) => (
            Some(crate::net::format_address(&ip.to_string(), *port, crate::net::PORT)),
            vec![format!("Port {} was opened on your router automatically.", port)],
        ),
        Internet::Open { public: Some(ip), .. } if upnp::is_cgnat_v4(*ip) => (
            None,
            vec![format!(
                "The router opened the port, but your internet provider shares its public address {} between many homes (carrier-grade NAT), so friends can't reach you over IPv4. IPv6 may work, or a virtual-LAN program such as ZeroTier or Tailscale.",
                ip
            )],
        ),
        Internet::Open { public: Some(ip), port } => (
            None,
            vec![format!(
                "Your router is behind another router (its address is {}). Forward TCP port {} to {} on that one too.",
                ip, port, ip
            )],
        ),
        Internet::Open { public: None, port } => (
            None,
            vec![format!(
                "Port {} was opened on your router, but it did not say your public address. Give friends your public address (search the web for \"what is my IP\").",
                port
            )],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_texts() {
        let open = Internet::Open { public: Some(Ipv4Addr::new(203, 0, 113, 7)), port: 5555 };
        assert_eq!(internet_lines(&open, 5555, None, false).0.as_deref(), Some("203.0.113.7"));
        let moved = Internet::Open { public: Some(Ipv4Addr::new(203, 0, 113, 7)), port: 5556 };
        assert_eq!(internet_lines(&moved, 5555, None, false).0.as_deref(), Some("203.0.113.7:5556"));
        let cgnat = Internet::Open { public: Some(Ipv4Addr::new(100, 70, 1, 2)), port: 5555 };
        assert!(internet_lines(&cgnat, 5555, None, false).0.is_none());
        assert!(describe(&cgnat, 5555).contains("carrier-grade"));
        let failed = Internet::Failed("no router answered".into());
        let (addr, lines) = internet_lines(&failed, 5555, Some(Ipv4Addr::new(192, 168, 1, 20)), false);
        assert!(addr.is_none());
        assert!(lines[1].contains("port 5555 to 192.168.1.20"), "{:?}", lines);
        // with a game code the long explanation is not needed
        let (_, short) = internet_lines(&failed, 5555, None, true);
        assert_eq!(short.len(), 1);
        assert_eq!(code_lines(&relay::Status::Ready("KQ7XM4".into())).0.as_deref(), Some("KQ7-XM4"));
    }
}
