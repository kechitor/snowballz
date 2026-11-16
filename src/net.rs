//! Networking (IPv4 and IPv6).  The wire format is byte-for-byte the one of the Python
//! version (a type byte, then big-endian struct fields; strings are a u16
//! length + UTF-8), so the two versions can even play together.
//!
//! Every connection has a reader and a writer thread; the game polls a
//! channel of events (messages and disconnections) once per frame.

use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

pub const PORT: u16 = 5555;
pub type ConnId = u32;

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    UnitPosition { unit_id: u16, x: i16, y: i16, dir_x: i8, dir_y: i8, state_id: u8, holding_id: u8 },
    UnitAddEmblem { unit_id: u16, image_name: String, animate: bool, offset: (i16, i16) },
    UnitRemoveEmblem { unit_id: u16 },
    Warmth { unit_id: u16, warmth: i16 },
    Snowball { unit_id: u16, start_x: i32, start_y: i32, end_x: i32, end_y: i32, end_tile_x: u16, end_tile_y: u16, snowballs_left: i16, px: u16, py: u16 },
    UnitIglooChange { unit_id: u16, building_id: u16 },
    BuildingOwner { building_id: u16, player_id: u16 },
    BuildingJeopardy { building_id: u16, jeopardy: u8, player_taking_over_id: u16 },
    BuildingStorage { building_id: u16, fish: i16, crystal: i16 },
    CreateDmo { dmo_id: u16, image_name: String, position: (u16, u16), obstruction: bool, hidden: bool, minimapimage: String },
    DmoHidden { dmo_id: u16, hidden: u8 },
    ResourceQuantity { tx: i16, ty: i16, q: i16 },
    NewPlayer { player_id: u16, name: String, color: [u8; 3], loading: bool },
    WhoYouAre { player_id: u16 },
    SetJob { pos: (u16, u16), run_mode: bool, unit_ids: Vec<u16> },
    DisbanUnits { unit_ids: Vec<u16> },
    ClientFinishedLoading { player_id: u16 },
    GameStart,
    ChatMessage { msg: String, from_id: u16, to_id: u16 },
    PlayerVictory { player_id: u16, victory: Option<bool> },
    Mapname { mapname: String },
    /// before the match: a client's choice (building_id 0xffff) or, from
    /// the server, the snowballers an igloo starts with
    Loadout { building_id: u16, snowballers: u16 },
}

/// Clamp a number into an integer field (like the Python version did, a
/// value that doesn't fit is clamped instead of crashing the game).
pub fn clamp_i16(v: f64) -> i16 {
    v.clamp(i16::MIN as f64, i16::MAX as f64) as i16
}
pub fn clamp_i32(v: f64) -> i32 {
    v.clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v)
    }
    fn i8(&mut self, v: i8) {
        self.0.push(v as u8)
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_be_bytes())
    }
    fn string(&mut self, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(u16::MAX as usize);
        self.u16(n as u16);
        self.0.extend_from_slice(&b[..n]);
    }
    fn multi(&mut self, v: &[u16]) {
        self.u16(v.len().min(u16::MAX as usize) as u16);
        for x in v.iter().take(u16::MAX as usize) {
            self.u16(*x);
        }
    }
}

impl Msg {
    pub fn type_id(&self) -> u8 {
        match self {
            Msg::UnitPosition { .. } => 100,
            Msg::UnitAddEmblem { .. } => 102,
            Msg::UnitRemoveEmblem { .. } => 103,
            Msg::Warmth { .. } => 104,
            Msg::Snowball { .. } => 105,
            Msg::UnitIglooChange { .. } => 109,
            Msg::BuildingOwner { .. } => 110,
            Msg::BuildingJeopardy { .. } => 111,
            Msg::BuildingStorage { .. } => 112,
            Msg::CreateDmo { .. } => 114,
            Msg::DmoHidden { .. } => 115,
            Msg::ResourceQuantity { .. } => 117,
            Msg::NewPlayer { .. } => 120,
            Msg::WhoYouAre { .. } => 121,
            Msg::SetJob { .. } => 122,
            Msg::DisbanUnits { .. } => 123,
            Msg::ClientFinishedLoading { .. } => 124,
            Msg::GameStart => 224,
            Msg::ChatMessage { .. } => 225,
            Msg::PlayerVictory { .. } => 226,
            Msg::Mapname { .. } => 227,
            Msg::Loadout { .. } => 228,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::with_capacity(32));
        w.u8(self.type_id());
        match self {
            Msg::UnitPosition { unit_id, x, y, dir_x, dir_y, state_id, holding_id } => {
                w.u16(*unit_id);
                w.i16(*x);
                w.i16(*y);
                w.i8(*dir_x);
                w.i8(*dir_y);
                w.u8(*state_id);
                w.u8(*holding_id);
            }
            Msg::UnitAddEmblem { unit_id, image_name, animate, offset } => {
                w.u16(*unit_id);
                w.string(image_name);
                w.u8(*animate as u8);
                w.i16(offset.0);
                w.i16(offset.1);
            }
            Msg::UnitRemoveEmblem { unit_id } => w.u16(*unit_id),
            Msg::Warmth { unit_id, warmth } => {
                w.u16(*unit_id);
                w.i16(*warmth);
            }
            Msg::Snowball { unit_id, start_x, start_y, end_x, end_y, end_tile_x, end_tile_y, snowballs_left, px, py } => {
                w.u16(*unit_id);
                w.i32(*start_x);
                w.i32(*start_y);
                w.i32(*end_x);
                w.i32(*end_y);
                w.u16(*end_tile_x);
                w.u16(*end_tile_y);
                w.i16(*snowballs_left);
                w.u16(*px);
                w.u16(*py);
            }
            Msg::UnitIglooChange { unit_id, building_id } => {
                w.u16(*unit_id);
                w.u16(*building_id);
            }
            Msg::BuildingOwner { building_id, player_id } => {
                w.u16(*building_id);
                w.u16(*player_id);
            }
            Msg::BuildingJeopardy { building_id, jeopardy, player_taking_over_id } => {
                w.u16(*building_id);
                w.u8(*jeopardy);
                w.u16(*player_taking_over_id);
            }
            Msg::BuildingStorage { building_id, fish, crystal } => {
                w.u16(*building_id);
                w.i16(*fish);
                w.i16(*crystal);
            }
            Msg::CreateDmo { dmo_id, image_name, position, obstruction, hidden, minimapimage } => {
                w.u16(*dmo_id);
                w.string(image_name);
                w.u16(position.0);
                w.u16(position.1);
                w.u8((*hidden as u8) | ((*obstruction as u8) << 1));
                w.string(minimapimage);
            }
            Msg::DmoHidden { dmo_id, hidden } => {
                w.u16(*dmo_id);
                w.u8(*hidden);
            }
            Msg::ResourceQuantity { tx, ty, q } => {
                w.i16(*tx);
                w.i16(*ty);
                w.i16(*q);
            }
            Msg::NewPlayer { player_id, name, color, loading } => {
                w.u16(*player_id);
                w.string(name);
                w.u8(color[0]);
                w.u8(color[1]);
                w.u8(color[2]);
                w.u8(*loading as u8);
            }
            Msg::WhoYouAre { player_id } => w.u16(*player_id),
            Msg::SetJob { pos, run_mode, unit_ids } => {
                w.u16(pos.0);
                w.u16(pos.1);
                w.u8(*run_mode as u8);
                w.multi(unit_ids);
            }
            Msg::DisbanUnits { unit_ids } => w.multi(unit_ids),
            Msg::ClientFinishedLoading { player_id } => w.u16(*player_id),
            Msg::GameStart => {}
            Msg::ChatMessage { msg, from_id, to_id } => {
                w.string(msg);
                w.u16(*from_id);
                w.u16(*to_id);
            }
            Msg::PlayerVictory { player_id, victory } => {
                w.u16(*player_id);
                w.u8(match victory {
                    Some(true) => b't',
                    Some(false) => b'f',
                    None => b'n',
                });
            }
            Msg::Mapname { mapname } => w.string(mapname),
            Msg::Loadout { building_id, snowballers } => {
                w.u16(*building_id);
                w.u16(*snowballers);
            }
        }
        w.0
    }

    /// Read one message from a stream.
    pub fn read_from(r: &mut impl Read) -> Result<Msg, String> {
        let mut rd = Rd(r);
        let t = rd.u8()?;
        Ok(match t {
            100 => Msg::UnitPosition {
                unit_id: rd.u16()?,
                x: rd.i16()?,
                y: rd.i16()?,
                dir_x: rd.u8()? as i8,
                dir_y: rd.u8()? as i8,
                state_id: rd.u8()?,
                holding_id: rd.u8()?,
            },
            102 => Msg::UnitAddEmblem { unit_id: rd.u16()?, image_name: rd.string()?, animate: rd.u8()? != 0, offset: (rd.i16()?, rd.i16()?) },
            103 => Msg::UnitRemoveEmblem { unit_id: rd.u16()? },
            104 => Msg::Warmth { unit_id: rd.u16()?, warmth: rd.i16()? },
            105 => Msg::Snowball {
                unit_id: rd.u16()?,
                start_x: rd.i32()?,
                start_y: rd.i32()?,
                end_x: rd.i32()?,
                end_y: rd.i32()?,
                end_tile_x: rd.u16()?,
                end_tile_y: rd.u16()?,
                snowballs_left: rd.i16()?,
                px: rd.u16()?,
                py: rd.u16()?,
            },
            109 => Msg::UnitIglooChange { unit_id: rd.u16()?, building_id: rd.u16()? },
            110 => Msg::BuildingOwner { building_id: rd.u16()?, player_id: rd.u16()? },
            111 => Msg::BuildingJeopardy { building_id: rd.u16()?, jeopardy: rd.u8()?, player_taking_over_id: rd.u16()? },
            112 => Msg::BuildingStorage { building_id: rd.u16()?, fish: rd.i16()?, crystal: rd.i16()? },
            114 => {
                let dmo_id = rd.u16()?;
                let image_name = rd.string()?;
                let position = (rd.u16()?, rd.u16()?);
                let v = rd.u8()?;
                let minimapimage = rd.string()?;
                Msg::CreateDmo { dmo_id, image_name, position, obstruction: v & 2 != 0, hidden: v & 1 != 0, minimapimage }
            }
            115 => Msg::DmoHidden { dmo_id: rd.u16()?, hidden: rd.u8()? },
            117 => Msg::ResourceQuantity { tx: rd.i16()?, ty: rd.i16()?, q: rd.i16()? },
            120 => Msg::NewPlayer { player_id: rd.u16()?, name: rd.string()?, color: [rd.u8()?, rd.u8()?, rd.u8()?], loading: rd.u8()? != 0 },
            121 => Msg::WhoYouAre { player_id: rd.u16()? },
            122 => {
                let pos = (rd.u16()?, rd.u16()?);
                let run_mode = rd.u8()? != 0;
                Msg::SetJob { pos, run_mode, unit_ids: rd.multi()? }
            }
            123 => Msg::DisbanUnits { unit_ids: rd.multi()? },
            124 => Msg::ClientFinishedLoading { player_id: rd.u16()? },
            224 => Msg::GameStart,
            225 => Msg::ChatMessage { msg: rd.string()?, from_id: rd.u16()?, to_id: rd.u16()? },
            226 => {
                let player_id = rd.u16()?;
                let victory = match rd.u8()? {
                    b't' => Some(true),
                    b'f' => Some(false),
                    _ => None,
                };
                Msg::PlayerVictory { player_id, victory }
            }
            227 => Msg::Mapname { mapname: rd.string()? },
            228 => Msg::Loadout { building_id: rd.u16()?, snowballers: rd.u16()? },
            t => return Err(format!("unknown message type {} (are both players running the same version?)", t)),
        })
    }
}

struct Rd<'a, R: Read>(&'a mut R);

/// A readable reason for a failed read.
fn read_error(e: std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        UnexpectedEof => "the other side closed the connection".to_string(),
        ConnectionReset | ConnectionAborted | BrokenPipe => "the connection was cut".to_string(),
        _ => e.to_string(),
    }
}

impl<R: Read> Rd<'_, R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut b = [0u8; N];
        self.0.read_exact(&mut b).map_err(read_error)?;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.bytes()?))
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_be_bytes(self.bytes()?))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.bytes()?))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        if n > 1000 {
            return Err(format!("string length ({}) is above limit (1000)", n));
        }
        let mut b = vec![0u8; n];
        self.0.read_exact(&mut b).map_err(read_error)?;
        Ok(String::from_utf8_lossy(&b).into_owned())
    }
    fn multi(&mut self) -> Result<Vec<u16>, String> {
        let n = self.u16()? as usize;
        if n > 1000 {
            return Err(format!("multi length ({}) is above limit (1000)", n));
        }
        (0..n).map(|_| self.u16()).collect()
    }
}

/// What the network tells the game.
#[derive(Debug)]
pub enum Event {
    Msg(Msg, ConnId),
    Disconnected(ConnId, String),
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

pub struct Connection {
    pub id: ConnId,
    tx: Sender<Option<Vec<u8>>>,
    closed: Arc<AtomicBool>,
    stream: TcpStream,
}

impl Connection {
    fn new(stream: TcpStream, events: Sender<Event>) -> std::io::Result<Connection> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let _ = stream.set_nodelay(true);
        let closed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel::<Option<Vec<u8>>>();
        // writer
        let mut ws = stream.try_clone()?;
        let (c2, ev2) = (closed.clone(), events.clone());
        std::thread::Builder::new().name(format!("net-send-{}", id)).spawn(move || {
            let lost = |ws: &TcpStream, reason: String| {
                if !c2.swap(true, Ordering::SeqCst) {
                    let _ = ws.shutdown(Shutdown::Both);
                    let _ = ev2.send(Event::Disconnected(id, reason));
                }
            };
            while let Ok(item) = rx.recv() {
                match item {
                    Some(data) => {
                        if let Err(e) = ws.write_all(&data) {
                            lost(&ws, format!("send: {}", e));
                            return;
                        }
                    }
                    None => {
                        let _ = ws.flush();
                        lost(&ws, "closed".into());
                        return;
                    }
                }
            }
        })?;
        // reader
        let rs = stream.try_clone()?;
        let c3 = closed.clone();
        std::thread::Builder::new().name(format!("net-recv-{}", id)).spawn(move || {
            let mut r = BufReader::new(rs);
            loop {
                match Msg::read_from(&mut r) {
                    Ok(m) => {
                        if events.send(Event::Msg(m, id)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        if !c3.swap(true, Ordering::SeqCst) {
                            let _ = r.get_ref().shutdown(Shutdown::Both);
                            let _ = events.send(Event::Disconnected(id, e));
                        }
                        return;
                    }
                }
            }
        })?;
        Ok(Connection { id, tx, closed, stream })
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn send_bytes(&self, b: Vec<u8>) {
        if !self.is_closed() {
            let _ = self.tx.send(Some(b));
        }
    }

    pub fn send(&self, m: &Msg) {
        self.send_bytes(m.encode());
    }

    /// Close after sending what is queued.
    pub fn close(&self) {
        let _ = self.tx.send(None);
    }

    /// Close right away.
    pub fn close_now(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
    }
}

pub struct Server {
    listeners: Vec<TcpListener>,
    pub connections: Vec<Connection>,
    events_tx: Sender<Event>,
    events: Receiver<Event>,
    /// connections made elsewhere (players joining through the relay)
    adopt_tx: Sender<TcpStream>,
    adopt_rx: Receiver<TcpStream>,
}

/// Listen on `port` for IPv4 and IPv6.  Where the system allows it, one
/// IPv6 socket takes both; otherwise there is one socket for each.
fn listen(port: u16) -> std::io::Result<Vec<TcpListener>> {
    let v4 = || TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port));
    match TcpListener::bind((std::net::Ipv6Addr::UNSPECIFIED, port)) {
        Ok(v6) => {
            #[allow(deprecated)]
            let dual = matches!(v6.only_v6(), Ok(false));
            if dual {
                Ok(vec![v6])
            } else {
                Ok(vec![v4()?, v6])
            }
        }
        // no IPv6 on this computer (or the port is taken: then v4 fails too)
        Err(_) => Ok(vec![v4()?]),
    }
}

impl Server {
    pub fn start(port: u16) -> Result<Server, String> {
        let listeners = listen(port).map_err(|e| {
            format!("Could not open port {} ({}). Is another Snowballz server still running?", port, e)
        })?;
        for l in &listeners {
            l.set_nonblocking(true).map_err(|e| e.to_string())?;
        }
        let (events_tx, events) = channel();
        let (adopt_tx, adopt_rx) = channel();
        Ok(Server { listeners, connections: Vec::new(), events_tx, events, adopt_tx, adopt_rx })
    }

    /// Whether IPv6 players can connect.
    pub fn has_ipv6(&self) -> bool {
        self.listeners.iter().any(|l| l.local_addr().map(|a| a.is_ipv6()).unwrap_or(false))
    }

    /// Where to send connections made elsewhere (the relay's players);
    /// `accept` takes them in like its own.
    pub fn adopter(&self) -> Sender<TcpStream> {
        self.adopt_tx.clone()
    }

    /// New connections since the last call.
    pub fn accept(&mut self) -> Vec<ConnId> {
        let mut out = Vec::new();
        while let Ok(s) = self.adopt_rx.try_recv() {
            if let Ok(c) = Connection::new(s, self.events_tx.clone()) {
                out.push(c.id);
                self.connections.push(c);
            }
        }
        for l in &self.listeners {
            // until WouldBlock: no more for now
            while let Ok((s, _addr)) = l.accept() {
                let _ = s.set_nonblocking(false);
                if let Ok(c) = Connection::new(s, self.events_tx.clone()) {
                    out.push(c.id);
                    self.connections.push(c);
                }
            }
        }
        out
    }

    pub fn poll(&mut self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn send(&mut self, m: &Msg, to: Option<&[ConnId]>, exclude: &[ConnId]) {
        let data = m.encode();
        self.connections.retain(|c| !c.is_closed());
        for c in &self.connections {
            if exclude.contains(&c.id) {
                continue;
            }
            if let Some(to) = to {
                if !to.contains(&c.id) {
                    continue;
                }
            }
            c.send_bytes(data.clone());
        }
    }

    pub fn get(&self, id: ConnId) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    pub fn drop_connection(&mut self, id: ConnId) {
        self.connections.retain(|c| c.id != id);
    }

    /// Close every connection (the port is freed when the server is dropped).
    pub fn stop(&mut self) {
        for c in &self.connections {
            c.close();
        }
        self.connections.clear();
    }
}

pub struct Client {
    pub conn: Connection,
    events: Receiver<Event>,
}

impl Client {
    /// Connect to an address as typed by the player: a name or an IPv4 /
    /// IPv6 address, optionally with a port (`host:port`, `[v6]:port`).
    pub fn connect(address: &str, default_port: u16) -> Result<Client, String> {
        let (host, port) = parse_address(address, default_port)?;
        let addrs: Vec<_> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| format!("Could not find the computer '{}'", host))?
            .collect();
        let mut last = "no address".to_string();
        for a in addrs {
            match TcpStream::connect_timeout(&a, Duration::from_secs(6)) {
                Ok(s) => {
                    let (tx, rx) = channel();
                    let conn = Connection::new(s, tx).map_err(|e| e.to_string())?;
                    return Ok(Client { conn, events: rx });
                }
                Err(e) => {
                    use std::io::ErrorKind::*;
                    last = match e.kind() {
                        ConnectionRefused => "nobody is hosting a game there".to_string(),
                        TimedOut | WouldBlock => format!(
                            "no answer (check the address; the host's router must forward port {} and their firewall must allow the game)",
                            port
                        ),
                        _ => {
                            let t = e.to_string();
                            t.split(" (os error").next().unwrap_or(&t).to_string()
                        }
                    }
                }
            }
        }
        Err(format!("Could not connect to {}: {}", address.trim(), last))
    }

    /// A client over a connection made elsewhere (through the relay).
    pub fn from_stream(s: TcpStream) -> Result<Client, String> {
        let (tx, rx) = channel();
        let conn = Connection::new(s, tx).map_err(|e| e.to_string())?;
        Ok(Client { conn, events: rx })
    }

    pub fn poll(&mut self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

/// Split "host", "host:port", "[v6]", "[v6]:port" or a bare IPv6 address
/// into (host, port).  Spaces and a pasted "http://" or trailing "/" are
/// ignored.
pub fn parse_address(text: &str, default_port: u16) -> Result<(String, u16), String> {
    let mut t: &str = text.trim();
    for prefix in ["snowballz://", "http://", "https://"] {
        if t.len() >= prefix.len() && t[..prefix.len()].eq_ignore_ascii_case(prefix) {
            t = &t[prefix.len()..];
        }
    }
    let t = t.trim_end_matches('/').trim();
    if t.is_empty() {
        return Err("Type the host's address first".into());
    }
    let bad_port = |p: &str| format!("'{}' is not a port number (1-65535)", p);
    let port_of = |p: &str| p.parse::<u16>().ok().filter(|&n| n > 0).ok_or_else(|| bad_port(p));
    if let Some(rest) = t.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or("A ']' is missing in the address")?;
        let port = match after.strip_prefix(':') {
            Some(p) => port_of(p)?,
            None if after.is_empty() => default_port,
            None => return Err(format!("Strange text after the address: '{}'", after)),
        };
        return Ok((h.to_string(), port));
    }
    match t.matches(':').count() {
        0 => Ok((t.to_string(), default_port)),
        1 => {
            let (h, p) = t.split_once(':').unwrap();
            Ok((h.to_string(), port_of(p)?))
        }
        _ => Ok((t.to_string(), default_port)), // a bare IPv6 address
    }
}

/// The address to give to other players: `host` or `host:port`, with
/// brackets around IPv6 addresses that have a port.
pub fn format_address(host: &str, port: u16, default_port: u16) -> String {
    match (port == default_port, host.contains(':')) {
        (true, _) => host.to_string(),
        (false, true) => format!("[{}]:{}", host, port),
        (false, false) => format!("{}:{}", host, port),
    }
}

/// The game's network: none, a server or a client.
pub enum Net {
    Server(Server),
    Client(Client),
}

impl Net {
    pub fn poll(&mut self) -> Vec<Event> {
        match self {
            Net::Server(s) => s.poll(),
            Net::Client(c) => c.poll(),
        }
    }

    /// Send to everybody (the server) / to the server (a client).
    pub fn send(&mut self, m: &Msg) {
        match self {
            Net::Server(s) => s.send(m, None, &[]),
            Net::Client(c) => c.conn.send(m),
        }
    }

    pub fn shutdown(&mut self) {
        match self {
            Net::Server(s) => s.stop(),
            Net::Client(c) => c.conn.close_now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(m: Msg) {
        let b = m.encode();
        let back = Msg::read_from(&mut &b[..]).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn messages_roundtrip() {
        roundtrip(Msg::UnitPosition { unit_id: 7, x: -3, y: 140, dir_x: -1, dir_y: 1, state_id: 9, holding_id: 2 });
        roundtrip(Msg::Snowball { unit_id: 1, start_x: -20, start_y: 30, end_x: 4000, end_y: -5, end_tile_x: 3, end_tile_y: 4, snowballs_left: 1, px: 3, py: 3 });
        roundtrip(Msg::NewPlayer { player_id: 0xffff, name: "Snowballer".into(), color: [0, 0, 0], loading: true });
        roundtrip(Msg::SetJob { pos: (4, 5), run_mode: true, unit_ids: vec![1, 2, 3] });
        roundtrip(Msg::CreateDmo { dmo_id: 2, image_name: "plugins/powerup/powerup_1.png".into(), position: (30, 84), obstruction: false, hidden: true, minimapimage: "m.png".into() });
        roundtrip(Msg::PlayerVictory { player_id: 2, victory: None });
        roundtrip(Msg::ChatMessage { msg: "héllo".into(), from_id: 1, to_id: 0xffff });
        roundtrip(Msg::GameStart);
        roundtrip(Msg::Loadout { building_id: 0xffff, snowballers: 12 });
    }

    #[test]
    fn same_bytes_as_python() {
        // struct.pack("!B", 104) + struct.pack("!Hh", 3, -1)
        assert_eq!(Msg::Warmth { unit_id: 3, warmth: -1 }.encode(), vec![104, 0, 3, 0xff, 0xff]);
        // MNewPlayer(1, "ab", (255, 0, 1), False)
        assert_eq!(
            Msg::NewPlayer { player_id: 1, name: "ab".into(), color: [255, 0, 1], loading: false }.encode(),
            vec![120, 0, 1, 0, 2, b'a', b'b', 255, 0, 1, 0]
        );
        // MPlayerVictory(5, True) -> "!B" + "!Hc"
        assert_eq!(Msg::PlayerVictory { player_id: 5, victory: Some(true) }.encode(), vec![226, 0, 5, b't']);
    }

    #[test]
    fn addresses() {
        let p = |t: &str| parse_address(t, 5555);
        assert_eq!(p("203.0.113.7").unwrap(), ("203.0.113.7".into(), 5555));
        assert_eq!(p(" 203.0.113.7:5556 ").unwrap(), ("203.0.113.7".into(), 5556));
        assert_eq!(p("myhost.example.org").unwrap(), ("myhost.example.org".into(), 5555));
        assert_eq!(p("2001:db8::5").unwrap(), ("2001:db8::5".into(), 5555));
        assert_eq!(p("[2001:db8::5]:6000").unwrap(), ("2001:db8::5".into(), 6000));
        assert_eq!(p("[2001:db8::5]").unwrap(), ("2001:db8::5".into(), 5555));
        assert_eq!(p("http://1.2.3.4:7/").unwrap(), ("1.2.3.4".into(), 7));
        assert!(p("1.2.3.4:abc").is_err());
        assert!(p("1.2.3.4:0").is_err());
        assert!(p("").is_err());
        assert_eq!(format_address("203.0.113.7", 5555, 5555), "203.0.113.7");
        assert_eq!(format_address("203.0.113.7", 5556, 5555), "203.0.113.7:5556");
        assert_eq!(format_address("2001:db8::5", 5556, 5555), "[2001:db8::5]:5556");
    }

    #[test]
    fn server_takes_ipv4_and_ipv6() {
        // a free port
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut s = Server::start(port).unwrap();
        let v4 = Client::connect(&format!("127.0.0.1:{}", port), 1);
        assert!(v4.is_ok(), "{:?}", v4.err());
        let v6 = (s.has_ipv6() && TcpListener::bind("[::1]:0").is_ok()).then(|| Client::connect(&format!("[::1]:{}", port), 1));
        if let Some(v6) = &v6 {
            assert!(v6.is_ok(), "{:?}", v6.as_ref().err());
        }
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(s.accept().len(), 1 + v6.is_some() as usize);
        // the port is taken now (macOS lets an IPv4 socket share a port
        // with a dual-stack one, so it is not checked there)
        if !cfg!(target_os = "macos") {
            assert!(Server::start(port).is_err());
        }
        drop(s);
        let err = Client::connect(&format!("127.0.0.1:{}", port), 1).err().unwrap();
        assert!(err.contains("nobody is hosting"), "{}", err);
    }
}
