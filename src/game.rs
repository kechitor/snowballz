//! A game session (the Main / ServerMain / ClientMain classes of
//! snowballz.py): loading, the online lobby, the game loop, the network
//! handlers and the winner / loser screen.

use crate::ai;
use crate::audio;
use crate::backgrounds;
use crate::clock::{self, Clock};
use crate::crashlog;
use crate::ctrl;
use crate::display;
use crate::gfx::{self, Font, Tex};
use crate::hud::Minimap;
use crate::menu::GameType;
use crate::net::{self, ConnId, Event as NetEvent, Msg, Net};
use crate::online::{self, Hosting};
use crate::sdl::{self, Rect};
use crate::settings;
use crate::terrain::Terrain;
use crate::unit;
use crate::world::*;

/// Test / automation switches, read from environment variables.
#[derive(Clone, Default)]
pub struct Hooks {
    /// end the program after this many (real) milliseconds of game
    pub quit_after: Option<u32>,
    /// (milliseconds, file) screenshots
    pub shots: Vec<(u32, String)>,
    /// the local player is played by the AI
    pub autoplay: bool,
    /// a server starts the game by itself after this many milliseconds
    pub autostart: Option<u32>,
    /// print a status line every N milliseconds
    pub stats: Option<u32>,
    /// no winner/loser screen
    pub skip_end: bool,
    /// a chat line to say once the game runs
    pub say: Option<String>,
    /// (server / single player) the local player's igloos go to an AI
    /// after this many milliseconds: a quick defeat for tests
    pub lose_after: Option<u32>,
    pub chill_after: Option<u32>,
    pub give_power: Option<String>,
    pub wall_test_done: bool,
}

impl Hooks {
    pub fn from_env() -> Hooks {
        let num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u32>().ok());
        let shots = std::env::var("SNOWBALLZ_SHOTS")
            .ok()
            .map(|v| {
                v.split(',')
                    .filter_map(|s| s.split_once(':'))
                    .filter_map(|(t, p)| t.trim().parse().ok().map(|t| (t, p.trim().to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Hooks {
            quit_after: num("SNOWBALLZ_QUIT_AFTER"),
            shots,
            autoplay: std::env::var("SNOWBALLZ_AUTOPLAY").is_ok(),
            autostart: num("SNOWBALLZ_AUTOSTART"),
            stats: num("SNOWBALLZ_STATS"),
            skip_end: std::env::var("SNOWBALLZ_SKIP_END").is_ok(),
            say: std::env::var("SNOWBALLZ_SAY").ok(),
            lose_after: num("SNOWBALLZ_LOSE_AFTER"),
            chill_after: num("SNOWBALLZ_CHILL_AFTER"),
            give_power: std::env::var("SNOWBALLZ_GIVE_POWER").ok(),
            wall_test_done: false,
        }
    }
}

/// The parts of a running game that aren't the world.
pub struct Session {
    /// time of day: 0 = midnight, 44 = noon, wraps at 88
    pub time: i32,
    pub last_time_tick: u32,
    pub terrain: Terrain,
    pub loading_alpha: f32,
    pub fps: f64,
    pub last_ai_tick: u32,
    pub last_alert_tick: u32,
    pub cursor: Tex,
    pub use_cursor: bool,
    pub minimap: Minimap,
    /// the Escape menu, while it is open
    pub game_menu: Option<crate::game_menu::GameMenu>,
    /// a single player game (the Escape menu pauses it)
    pub single: bool,
    /// hosting an online game (quitting ends it for everybody)
    pub hosting_online: bool,
    /// the player chose Quit game
    pub quit_requested: bool,
    /// The start countdown: game time stands still until this (real)
    /// time, then "Go!" shows until `go_until`.
    pub countdown_until: Option<u32>,
    pub go_until: Option<u32>,
}

/// The countdown before a match (3, 2, 1) ...
pub const COUNTDOWN_MS: u32 = 3000;

/// A map's own music, written out of the map's zip to a temporary file
/// (its path), so that it can be played.
fn unpack_map_music(map: &str) -> Option<String> {
    let z = crate::zip::Zip::open(&crate::settings::map_path(map)).ok()?;
    let data = z.read(crate::editmap::MAP_MUSIC_FILE).ok()?;
    let safe: String = map.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let p = std::env::temp_dir().join(format!("snowballz-music-{}.ogg", safe));
    std::fs::write(&p, data).ok()?;
    Some(p.to_string_lossy().into_owned())
}
/// ... and how long "Go!" stays.
pub const GO_MS: u32 = 900;

pub struct Game {
    pub kind: GameType,
    pub mapname: String,
    pub world: Option<World>,
    pub session: Option<Session>,
    pending_net: Option<Net>,
    running: bool,
    lobbying: bool,
    disconnected_at: Option<u32>,
    clock: Clock,
    lobbybg: Tex,
    snow: Tex,
    cursor: Tex,
    hooks: Hooks,
    frame_errors: u32,
    pub error: Option<String>,
    started: u32,
    said: bool,
    last_stats: u32,
    /// the local player's choice of snowballers (None: the map's mix)
    loadout: Option<u16>,
    /// the lobby's - and + buttons (screen pixels)
    loadout_buttons: [Rect; 2],
    /// hosting: the router's port forwarding and the addresses to share
    hosting: Option<Hosting>,
    /// the lobby's Copy buttons: (button, address)
    copy_buttons: Vec<(Rect, String)>,
    /// the address copied last, and when
    copied: Option<(String, u32)>,
}

fn fonts() -> (Font, Font) {
    let f = gfx::load_font("data/TSCu_Comic.ttf", 20).expect("font");
    (f, f.with_size(16))
}

/// Play one game (from the lobby to the winner / loser screen).
pub fn play(kind: GameType, mapname: &str) -> Result<(), String> {
    let kind_is_client = matches!(kind, GameType::Client(_));
    let mut hosting = None;
    let net = match &kind {
        GameType::Single => None,
        GameType::Server => {
            let s = net::Server::start(net::PORT)?;
            println!("Waiting for a connection");
            hosting = Some(Hosting::start(net::PORT, s.has_ipv6(), s.adopter()));
            Some(Net::Server(s))
        }
        GameType::Client(host) => {
            // a game code goes through the relay server, anything else is
            // an address
            let c = match crate::relay::code_in(host) {
                Some(code) => net::Client::from_stream(crate::relay::join(&code)?)?,
                None => net::Client::connect(host, net::PORT)?,
            };
            c.conn.send(&Msg::NewPlayer { player_id: 0xffff, name: settings::player_name(), color: [0, 0, 0], loading: true });
            Some(Net::Client(c))
        }
    };
    let mut g = Game {
        kind,
        mapname: mapname.to_string(),
        world: None,
        session: None,
        pending_net: net,
        running: true,
        lobbying: false,
        disconnected_at: None,
        clock: Clock::new(),
        lobbybg: gfx::texture_sized("data/lobbybg.png", 256),
        snow: gfx::texture("data/snow.png"),
        cursor: gfx::texture_sized("data/mouse_cursor.png", 64),
        hooks: Hooks::from_env(),
        frame_errors: 0,
        error: None,
        started: sdl::ticks(),
        said: false,
        last_stats: 0,
        loadout: match kind_is_client {
            true => None,
            false => settings::loadout(mapname),
        },
        loadout_buttons: [Rect::new(0, 0, 0, 0); 2],
        hosting,
        copy_buttons: Vec::new(),
        copied: None,
    };
    let r = g.lobby();
    g.shutdown();
    r?;
    match g.error.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Game {
    fn is_server(&self) -> bool {
        self.kind != GameType::Client(String::new()) && !matches!(self.kind, GameType::Client(_))
    }

    fn role(&self) -> Role {
        match self.kind {
            GameType::Single => Role::Local,
            GameType::Server => Role::Server,
            GameType::Client(_) => Role::Client,
        }
    }

    fn net_mut(&mut self) -> Option<&mut Net> {
        match self.world.as_mut() {
            Some(w) => w.net.as_mut(),
            None => self.pending_net.as_mut(),
        }
    }

    fn shutdown(&mut self) {
        if let Some(w) = self.world.as_mut() {
            w.shutdown_net();
        }
        if let Some(mut n) = self.pending_net.take() {
            n.shutdown();
        }
        // close the port on the router again
        self.hosting.take();
    }

    fn loading_progress(&self, fraction: f32) {
        let _ = gfx::events();
        let (sw, sh) = gfx::screen_size();
        gfx::clear((0, 0, 0));
        backgrounds::draw("loading", 1.0);
        let (bw, bh) = ((sw as f32 * 0.4) as i32, (sh / 70).max(10));
        let (x, y) = ((sw - bw) / 2, (sh as f32 * 0.9) as i32);
        gfx::fill_screen_rect(Rect::new(x - 3, y - 3, bw + 6, bh + 6), (255, 255, 255, 255));
        gfx::fill_screen_rect(Rect::new(x, y, bw, bh), (40, 40, 60, 255));
        gfx::fill_screen_rect(Rect::new(x, y, (bw as f32 * fraction) as i32, bh), (220, 30, 30, 255));
        gfx::present();
    }

    fn load_data(&mut self, mapname: &str) -> Result<(), String> {
        self.loading_progress(0.1);
        let (sw, sh) = gfx::screen_size();
        let view = View { x: 0.0, y: 0.0, w: sw, h: sh, zoom: settings::game_zoom(sh) };
        let (chat, stat) = fonts();
        let net = self.pending_net.take();
        let w = World::load(mapname, self.role(), net, view, chat, stat)?;
        self.loading_progress(0.7);
        let terrain = Terrain::new(&w);
        let minimap = Minimap::new(&w);
        self.loading_progress(1.0);
        let use_cursor = settings::get_option("cursor") == "1";
        sdl::show_cursor(!use_cursor);
        self.session = Some(Session {
            time: 45,
            last_time_tick: clock::now(),
            terrain,
            loading_alpha: 1.0,
            fps: 0.0,
            last_ai_tick: 0,
            last_alert_tick: clock::now(),
            cursor: self.cursor,
            use_cursor,
            minimap,
            game_menu: None,
            single: self.kind == GameType::Single,
            hosting_online: self.kind == GameType::Server,
            quit_requested: false,
            countdown_until: None,
            go_until: None,
        });
        self.mapname = mapname.to_string();
        self.world = Some(w);
        Ok(())
    }

    fn lobby(&mut self) -> Result<(), String> {
        self.running = true;
        backgrounds::preload("loading");
        self.loading_progress(0.0);
        if self.is_server() {
            let m = self.mapname.clone();
            self.load_data(&m)?;
            let name = settings::player_name();
            let w = self.world.as_mut().unwrap();
            let me = add_player(w, &name, None).ok_or("the map has no igloo for you")?;
            w.me = Some(me);
            w.players.get_mut(&me).unwrap().loading = false;
            w.players.get_mut(&me).unwrap().loadout = self.loadout;
            if self.hooks.autoplay {
                w.players.get_mut(&me).unwrap().kind = PlayerKind::Ai;
            }
            w.send(Msg::ClientFinishedLoading { player_id: me });
        }
        if self.net_mut().is_some() {
            self.lobbying = true;
            while self.lobbying && self.running {
                self.lobby_loop();
            }
        }
        if self.running && self.world.is_some() {
            self.run();
        }
        Ok(())
    }

    fn lobby_loop(&mut self) {
        self.clock.tick(35);
        self.guarded(|g| g.handle_network());
        self.check_connection();
        if let (Some(t), true) = (self.hooks.autostart, self.is_server()) {
            if sdl::ticks().wrapping_sub(self.started) > t {
                self.lobbying = false;
            }
        }
        for e in gfx::events() {
            match e.kind() {
                sdl::QUIT => crate::quit(0),
                sdl::KEYDOWN => {
                    if e.key_repeat() {
                        continue;
                    }
                    let k = e.key_sym();
                    let chatting = self.world.as_ref().map(|w| w.chat.chatting).unwrap_or(false);
                    if let Some(w) = self.world.as_mut() {
                        w.chat.swallow_text = false;
                    }
                    if !chatting {
                        match k {
                            sdl::K_ESCAPE => {
                                self.running = false;
                                self.lobbying = false;
                            }
                            sdl::K_S => self.lobbying = false,
                            sdl::K_LEFT | sdl::K_MINUS | sdl::K_KP_MINUS => self.change_loadout(-1),
                            sdl::K_RIGHT | sdl::K_EQUALS | sdl::K_KP_PLUS => self.change_loadout(1),
                            _ => {
                                if let Some(w) = self.world.as_mut() {
                                    ctrl::chat_keys(w, k);
                                }
                            }
                        }
                    } else if let Some(w) = self.world.as_mut() {
                        ctrl::handle_chatting(w, k);
                    }
                }
                sdl::TEXTINPUT => {
                    if let Some(w) = self.world.as_mut() {
                        ctrl::chat_text(w, &e.text());
                    }
                }
                sdl::MOUSEBUTTONDOWN if e.button() == 1 => {
                    let (mx, my) = gfx::mouse_pos();
                    let hit = |r: &Rect| mx >= r.x && mx < r.x + r.w && my >= r.y && my < r.y + r.h;
                    if hit(&self.loadout_buttons[0]) {
                        self.change_loadout(-1);
                    } else if hit(&self.loadout_buttons[1]) {
                        self.change_loadout(1);
                    } else if let Some((_, addr)) = self.copy_buttons.iter().find(|(r, _)| hit(r)) {
                        if sdl::set_clipboard(addr) {
                            self.copied = Some((addr.clone(), sdl::ticks()));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(w) = self.world.as_mut() {
            ctrl::chat_backspace_repeat(w);
        }
        self.draw_lobby();
    }

    /// The local player's loadout: (snowballers, total), once the map is known.
    fn current_loadout(&self) -> Option<(u16, u16)> {
        let w = self.world.as_ref()?;
        let (total, default) = w.igloo_penguins();
        if total == 0 {
            return None;
        }
        Some((self.loadout.unwrap_or(default).min(total), total))
    }

    /// One more (+1) or one less (-1) snowballer for the local player.
    fn change_loadout(&mut self, d: i32) {
        let Some((cur, total)) = self.current_loadout() else { return };
        let n = (cur as i32 + d).clamp(0, total as i32) as u16;
        self.loadout = Some(n);
        settings::set_loadout(&self.mapname, n);
        let server = self.is_server();
        let w = self.world.as_mut().unwrap();
        if server {
            if let Some(p) = w.me.and_then(|me| w.players.get_mut(&me)) {
                p.loadout = Some(n);
            }
        } else if w.me.is_some() {
            w.send(Msg::Loadout { building_id: 0xffff, snowballers: n });
        }
    }

    /// The loadout chooser at the top of the lobby (screen pixels, y down).
    fn draw_loadout(&mut self, sw: i32, sh: i32) {
        let Some((n, total)) = self.current_loadout() else {
            self.loadout_buttons = [Rect::new(0, 0, 0, 0); 2];
            return;
        };
        let (font, small) = fonts();
        let cx = sw / 2;
        // text at a top-left screen position (the UI is y up)
        let text = |f: &Font, x: i32, top: i32, t: &str, c: [f32; 4]| {
            let (_, th) = f.size_of(t);
            f.render(x as f64, (sh - top - th) as f64, t, c);
        };
        let black = [0.0, 0.0, 0.0, 1.0];
        let title = "Your penguins for this match:";
        let (tw, _) = font.size_of(title);
        text(&font, cx - tw / 2, 30, title, black);
        let mix = format!("{} snowballers  /  {} workers", n, total - n);
        let (mw, _) = font.size_of(&mix);
        let (bw, bh, top) = (40, 34, 66);
        let minus = Rect::new(cx - mw / 2 - 20 - bw, top, bw, bh);
        let plus = Rect::new(cx + mw / 2 + 20, top, bw, bh);
        let (mx, my) = gfx::mouse_pos();
        for (r, label, enabled) in [(minus, "-", n > 0), (plus, "+", n < total)] {
            let hover = mx >= r.x && mx < r.x + r.w && my >= r.y && my < r.y + r.h;
            let c = match (enabled, hover) {
                (false, _) => (120, 130, 160, 200),
                (true, true) => (70, 110, 230, 255),
                (true, false) => (40, 70, 190, 255),
            };
            gfx::fill_screen_rect(Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4), (20, 30, 80, 255));
            gfx::fill_screen_rect(r, c);
            let (lw, lh) = font.size_of(label);
            text(&font, r.x + (r.w - lw) / 2, r.y + (r.h - lh) / 2, label, [1.0; 4]);
        }
        self.loadout_buttons = [minus, plus];
        text(&font, cx - mw / 2, top + 4, &mix, black);
        let hint = "Left / Right arrows or the - + buttons";
        let (hw, _) = small.size_of(hint);
        text(&small, cx - hw / 2, top + bh + 10, hint, [0.3, 0.3, 0.35, 1.0]);
    }

    /// The host's addresses for the other players (screen pixels, y down),
    /// between the loadout chooser and the player list (which starts at
    /// `bottom`).
    fn draw_online(&mut self, sw: i32, sh: i32, bottom: i32) {
        self.copy_buttons.clear();
        let Some(h) = self.hosting.as_ref() else { return };
        let (font, small) = fonts();
        let text = |f: &Font, x: i32, top: i32, t: &str, c: [f32; 4]| {
            let (_, th) = f.size_of(t);
            f.render(x as f64, (sh - top - th) as f64, t, c);
        };
        let black = [0.0, 0.0, 0.0, 1.0];
        let grey = [0.25, 0.25, 0.3, 1.0];
        let pw = (sw - 40).min(760);
        let x0 = (sw - pw) / 2;
        let col = x0 + 160;
        let wrap_w = pw - 172;
        let relay = h.relay();
        let (code, code_notes) = online::code_lines(&relay);
        let (public, notes) = online::internet_lines(&h.internet(), h.port, h.lan, code.is_some());
        let fmt = |ip: String| net::format_address(&ip, h.port, net::PORT);
        let mut rows: Vec<(&str, Option<String>, Vec<String>)> = Vec::new();
        if relay != crate::relay::Status::Off {
            rows.push(("Game code:", code, code_notes));
        }
        rows.push(("Internet:", public, notes));
        if let Some(ip) = h.lan {
            rows.push(("Same network:", Some(fmt(ip.to_string())), Vec::new()));
        }
        if let Some(ip) = h.ipv6 {
            let note = "Works if both of you have IPv6 and the router lets the game in.".to_string();
            rows.push(("IPv6:", Some(fmt(ip.to_string())), vec![note]));
        }
        let wrapped: Vec<Vec<String>> = rows.iter().map(|r| r.2.iter().flat_map(|n| wrap(&small, n, wrap_w)).collect()).collect();
        let (row_h, note_h) = (37, 19);
        let row_height = |addr: bool, n: usize| {
            let n = n as i32;
            if addr {
                row_h + note_h * n
            } else {
                row_h.max(7 + note_h * n + 6)
            }
        };
        let total = |notes: bool| {
            44 + rows
                .iter()
                .zip(&wrapped)
                .map(|(r, w)| row_height(r.1.is_some(), if notes { w.len() } else { (r.1.is_none() && !w.is_empty()) as usize }))
                .sum::<i32>()
        };
        let top = 140;
        // too little room (small window, many players): notes shortened
        let full = top + total(true) <= bottom;
        let height = total(full);
        if top + height > bottom + 60 {
            return; // a very small window: the terminal shows the addresses
        }
        gfx::fill_screen_rect(Rect::new(x0 - 2, top - 2, pw + 4, height + 4), (90, 110, 160, 160));
        gfx::fill_screen_rect(Rect::new(x0, top, pw, height), (250, 252, 255, 215));
        let title = if relay != crate::relay::Status::Off {
            "Online game - friends join with the code (or an address):"
        } else {
            "Online game - friends join with one of these addresses:"
        };
        text(&font, x0 + 12, top + 6, title, black);
        let mut y = top + 38;
        let (mx, my) = gfx::mouse_pos();
        let now = sdl::ticks();
        for ((label, addr, _), notes) in rows.iter().zip(&wrapped) {
            let notes: &[String] = if full { notes } else { &notes[..notes.len().min(addr.is_none() as usize)] };
            text(&font, x0 + 12, y + 4, label, black);
            match addr {
                Some(a) => {
                    text(&font, col, y + 4, a, [0.05, 0.15, 0.55, 1.0]);
                    let r = Rect::new(x0 + pw - 100, y + 2, 88, 28);
                    let hover = mx >= r.x && mx < r.x + r.w && my >= r.y && my < r.y + r.h;
                    gfx::fill_screen_rect(Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4), (20, 30, 80, 255));
                    gfx::fill_screen_rect(r, if hover { (70, 110, 230, 255) } else { (40, 70, 190, 255) });
                    let done = matches!(&self.copied, Some((c, t)) if c == a && now.wrapping_sub(*t) < 2000);
                    let caption = if done { "Copied!" } else { "Copy" };
                    let (cw, ch) = small.size_of(caption);
                    text(&small, r.x + (r.w - cw) / 2, r.y + (r.h - ch) / 2, caption, [1.0; 4]);
                    self.copy_buttons.push((r, a.clone()));
                    for (i, n) in notes.iter().enumerate() {
                        text(&small, col, y + row_h - 4 + note_h * i as i32, n, grey);
                    }
                }
                None => {
                    for (i, n) in notes.iter().enumerate() {
                        text(&small, col, y + 7 + note_h * i as i32, n, grey);
                    }
                }
            }
            y += row_height(addr.is_some(), notes.len());
        }
    }

    fn draw_lobby(&mut self) {
        let (sw, sh) = gfx::screen_size();
        gfx::clear((0, 0, 0));
        gfx::begin_ui();
        // snow everywhere (the texture at 2x)
        let mut y = 0;
        while y < sh {
            let mut x = 0;
            while x < sw {
                gfx::blit(&self.snow, None, Rect::new(x, y, 512, 512), 1.0);
                x += 512;
            }
            y += 512;
        }
        self.lobbybg.draw(0.0, 60.0, [1.0; 4], 1.0);
        let (chat_font, _) = fonts();
        let black = [0.0, 0.0, 0.0, 1.0];
        if let Some(w) = self.world.as_mut() {
            if w.chat.chatting {
                gfx::fill_rect(0.0, 0.0, sw as f64, 60.0, Some([0.0, 0.0, 0.0, 0.75]));
                if !w.chat.help.is_empty() {
                    chat_font.render(10.0, 30.0, &w.chat.help, [0.65, 0.65, 0.65, 1.0]);
                }
                chat_font.render(10.0, 0.0, &format!("{}_", w.chat.msg), [1.0; 4]);
            }
            display::draw_messages(w);
        }
        let cx = sw as f64 / 2.0;
        if self.is_server() {
            chat_font.render(cx - 200.0, 20.0, "Waiting for connections - Press 'S' to start", black);
        } else {
            chat_font.render(cx - 230.0, 20.0, "Waiting for more players to join - Press 'Enter' to chat", black);
        }
        let mut i = 0.0;
        if let Some(w) = self.world.as_ref() {
            for p in w.players.values() {
                let mut t = p.name.clone();
                let a = if p.loading {
                    t.push_str(" (loading)");
                    0.35
                } else {
                    1.0
                };
                chat_font.render(cx - 60.0, 100.0 + 23.0 * i, &t, [p.glcolor[0], p.glcolor[1], p.glcolor[2], a]);
                i += 1.0;
            }
        }
        chat_font.render(cx - 80.0, 100.0 + 23.0 * i, "Players:", black);
        self.draw_loadout(sw, sh);
        // the online panel ends above "Players:" (screen pixels, y down)
        let players_top = sh - (100 + 23 * i as i32) - 30;
        self.draw_online(sw, sh, players_top);
        if settings::get_option("cursor") == "1" {
            let (mx, my) = gfx::mouse_pos();
            self.cursor.draw(mx as f64, (sh - 64 - my) as f64, [1.0; 4], 1.0);
        }
        gfx::present();
    }

    fn run(&mut self) {
        {
            let w = self.world.as_mut().unwrap();
            let mut names = Vec::new();
            match w.config.get("map", "music").map(|m| m.trim()) {
                // the map's own music: out of the map into a temporary file
                Some(crate::editmap::MAP_MUSIC_REF) => {
                    if let Some(p) = unpack_map_music(&w.map_name) {
                        names.push(p);
                    }
                }
                Some(m) if !m.is_empty() => names.push(m.to_string()),
                _ => {}
            }
            names.push("snowballz2.ogg".into());
            names.push("main_menu.ogg".into());
            audio::play(&names, -1);
            let now = clock::now();
            let s = self.session.as_mut().unwrap();
            s.time = 45;
            s.last_time_tick = now;
            s.last_alert_tick = now;
        }
        self.started = sdl::ticks();
        if self.is_server() {
            apply_loadouts(self.world.as_mut().unwrap());
            add_map_players(self.world.as_mut().unwrap());
            crate::ai::send_levels(self.world.as_mut().unwrap());
        }
        {
            let w = self.world.as_mut().unwrap();
            w.started = true;
            w.started_at = clock::now();
        }
        look_at_starting_igloo(self.world.as_mut().unwrap());
        test_look(self.world.as_mut().unwrap());
        // 3, 2, 1, go! (game time stands still meanwhile: the view can
        // move and orders can be given, nobody moves yet)
        if std::env::var("SNOWBALLZ_NO_COUNTDOWN").is_err() {
            let s = self.session.as_mut().unwrap();
            s.countdown_until = Some(sdl::ticks().wrapping_add(COUNTDOWN_MS));
            clock::set_paused(true);
        }
        if self.kind == GameType::Server {
            self.world.as_mut().unwrap().send(Msg::GameStart);
            // a reminder of the address, for friends who are late
            if let Some(h) = &self.hosting {
                let w = self.world.as_mut().unwrap();
                if let Some(code) = h.code() {
                    w.notice_copy(&format!("Friends can still join during the game with the code {}", code), &code);
                } else {
                    let addr = online::internet_lines(&h.internet(), h.port, h.lan, false).0.or_else(|| h.lan.map(|ip| net::format_address(&ip.to_string(), h.port, net::PORT)));
                    if let Some(a) = addr {
                        w.notice_copy(&format!("Friends can still join during the game: {}", a), &a);
                    }
                }
            }
        }
        while self.running {
            let fps_cap = settings::fps_cap();
            if self.is_server() {
                self.guarded(|g| g.ai_loop());
            }
            if self.kind != GameType::Single {
                self.guarded(|g| g.handle_network());
                self.check_connection();
            }
            self.frame(fps_cap);
            // the game is over when somebody has won; a defeated player
            // keeps watching (and the host keeps the game going)
            let w = self.world.as_mut().unwrap();
            if let Some(me) = w.me {
                let mine = w.players.get(&me).and_then(|p| p.victory);
                let someone_won = w.players.values().any(|p| p.victory == Some(true));
                if mine == Some(true) || someone_won {
                    self.running = false;
                } else if mine == Some(false) && !w.spectating {
                    w.spectating = true;
                    for &u in &w.selected {
                        w.units[u].selected = false;
                    }
                    w.selected.clear();
                    w.notice("You were defeated. You can keep watching the game; press Esc for the menu.");
                }
            }
        }
        clock::set_paused(false);
        audio::fade_out(1000);
        let quit = self.session.as_ref().map(|s| s.quit_requested).unwrap_or(false);
        if self.disconnected_at.is_none() && !quit {
            self.end_screen();
        }
    }

    /// Run a step; a panic in it is logged and the game carries on (only a
    /// step that keeps failing ends the game).
    fn guarded(&mut self, f: impl FnOnce(&mut Game)) {
        crashlog::set_guarded(true);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        crashlog::set_guarded(false);
        match r {
            Ok(()) => self.frame_errors = 0,
            Err(e) => {
                self.frame_errors += 1;
                if crashlog::guarded_errors() <= 5 {
                    if let Some(w) = self.world.as_mut() {
                        w.notice("An error happened (see the crash log)");
                    }
                }
                if self.frame_errors > 30 {
                    std::panic::resume_unwind(e);
                }
            }
        }
    }

    fn frame(&mut self, fps_cap: u32) {
        // the start countdown
        if let Some(s) = self.session.as_mut() {
            if let Some(t) = s.countdown_until {
                let now = sdl::ticks();
                if (now.wrapping_sub(t) as i32) >= 0 {
                    s.countdown_until = None;
                    s.go_until = Some(now.wrapping_add(GO_MS));
                    if s.game_menu.is_none() {
                        clock::set_paused(false);
                    }
                } else if !clock::is_paused() {
                    // (the Escape menu was closed meanwhile)
                    clock::set_paused(true);
                }
            }
        }
        let mut dt = self.clock.tick(fps_cap).min(144);
        // a paused single player game stands still
        if clock::is_paused() {
            dt = 0;
        }
        let fps = self.clock.fps();
        self.hooks_tick();
        self.guarded(|g| {
            let w = g.world.as_mut().unwrap();
            let s = g.session.as_mut().unwrap();
            s.fps = fps;
            if !ctrl::handle_events(w, s, dt) {
                g.running = false;
            } else {
                display::draw(s, w, dt);
            }
        });
    }

    fn hooks_tick(&mut self) {
        let elapsed = sdl::ticks().wrapping_sub(self.started);
        if let Some(text) = self.hooks.say.clone() {
            if !self.said && elapsed > 1500 {
                if let Some(w) = self.world.as_mut() {
                    if let Some(me) = w.me {
                        self.said = true;
                        w.player_send_msg(me, &text);
                    }
                }
            }
        }
        if let Some(i) = self.hooks.shots.iter().position(|(t, _)| elapsed >= *t) {
            let (_, path) = self.hooks.shots.remove(i);
            // draw the frame again and save it before it is shown
            if let (Some(w), Some(s)) = (self.world.as_mut(), self.session.as_mut()) {
                s.loading_alpha = 0.0;
                test_look(w);
                crate::gfx::set_present_hook(Some(path));
                display::draw(s, w, 0);
            }
        }
        if let Some(every) = self.hooks.stats {
            if elapsed.wrapping_sub(self.last_stats) >= every {
                self.last_stats = elapsed;
                if let Some(w) = self.world.as_ref() {
                    println!("STATS t={} {}", elapsed, stats_line(w));
                }
            }
        }
        if elapsed > 4000 && std::env::var("SNOWBALLZ_WALL_TEST").is_ok() && !self.hooks.wall_test_done {
            // test hook: (hosting) crystal for me, finished walls around
            // the other igloos, and my snowballers sent at the nearest one;
            // (a player) my workers sent to build a wall by my igloo
            self.hooks.wall_test_done = true;
            let server = self.is_server();
            if let Some(w) = self.world.as_mut() {
                if server {
                    crate::wall::test_setup(w);
                } else {
                    crate::wall::test_request(w);
                }
            }
        }
        if std::env::var("SNOWBALLZ_PATH_TEST").is_ok() && self.is_server() {
            // test hook: a wall (a cup, open the far side) between my
            // penguins and a spot they are sent to; how many get there
            if let Some(w) = self.world.as_mut() {
                crate::path::test_hook(w, elapsed);
            }
        }
        if std::env::var("SNOWBALLZ_HUNT_TEST").is_ok() && self.is_server() {
            // test hook: my snowballers sent after the nearest enemy penguin
            if let Some(w) = self.world.as_mut() {
                crate::unit::hunt_test(w, elapsed);
            }
        }
        if self.hooks.give_power.is_some() && elapsed > 2000 && self.is_server() {
            // test hook: power-ups for my snowballers
            let kind = self.hooks.give_power.take().unwrap();
            if let Some(w) = self.world.as_mut() {
                let me = w.me;
                crate::plugins::test_give(w, &kind, me);
            }
        }
        if let Some(t) = self.hooks.chill_after {
            // test hook: chill every penguin of mine (half frozen, half cold)
            if elapsed > t && self.is_server() {
                self.hooks.chill_after = None;
                if let Some(w) = self.world.as_mut() {
                    let mine: Vec<usize> = (0..w.units.len())
                        .filter(|&u| w.igloo_player(u) == w.me && !matches!(w.units[u].state, State::InBuilding | State::Entering))
                        .collect();
                    for (i, u) in mine.into_iter().enumerate() {
                        // (SNOWBALLZ_CHILL_TO=n: everybody to n instead)
                        let to = std::env::var("SNOWBALLZ_CHILL_TO").ok().and_then(|v| v.parse().ok());
                        w.set_warmth(u, to.unwrap_or(if i % 2 == 0 { 0 } else { 3 }));
                    }
                }
            }
        }
        if let Some(t) = self.hooks.lose_after {
            if elapsed > t && self.is_server() {
                self.hooks.lose_after = None;
                if let Some(w) = self.world.as_mut() {
                    let ai = w.players.values().filter(|p| p.kind == PlayerKind::Ai).map(|p| p.id).min();
                    let mine: Vec<usize> = (0..w.regions.len()).filter(|&r| w.regions[r].player.is_some() && w.regions[r].player == w.me).collect();
                    for r in mine {
                        w.set_region_player(r, ai);
                    }
                }
            }
        }
        if let Some(q) = self.hooks.quit_after {
            if elapsed > q {
                if let Some(w) = self.world.as_ref() {
                    println!("SUMMARY {}", stats_line(w));
                }
                self.shutdown();
                crate::quit(0);
            }
        }
    }

    fn ai_loop(&mut self) {
        let w = self.world.as_mut().unwrap();
        let s = self.session.as_mut().unwrap();
        let now = clock::now();
        crate::plugins::tick(w);
        crate::alliance::tick(w);
        crate::wall::tick(w);
        // (each computer player thinks at its level's pace: see ai::run)
        if now > s.last_ai_tick.wrapping_add(250) {
            s.last_ai_tick = now;
            let ids: Vec<Pid> = w.players.values().filter(|p| p.kind == PlayerKind::Ai).map(|p| p.id).collect();
            for p in ids {
                if w.is_alive(p) {
                    ai::run(w, p);
                }
            }
        }
        // alert tiles fade
        if now > s.last_alert_tick.wrapping_add(3000) {
            s.last_alert_tick = now;
            for p in w.players.values_mut() {
                for e in p.alert_tiles.iter_mut() {
                    if e.1 != 0 {
                        e.1 -= 1;
                    }
                }
                p.alert_tiles.retain(|e| e.1 != 0);
            }
        }
    }

    fn check_connection(&mut self) {
        if let Some(t) = self.disconnected_at {
            if clock::now().wrapping_sub(t) > 3000 {
                self.running = false;
                self.lobbying = false;
            }
        }
    }

    fn end_screen(&mut self) {
        if self.hooks.skip_end {
            if let Some(w) = self.world.as_ref() {
                println!("SUMMARY {}", stats_line(w));
            }
            return;
        }
        let w = self.world.as_ref().unwrap();
        let won = w.me.and_then(|p| w.players.get(&p)).and_then(|p| p.victory).unwrap_or(false);
        let name = if won { "winner" } else { "loser" };
        sdl::show_cursor(true);
        let _ = gfx::events();
        let mut alpha = 0.0f32;
        let shown = sdl::ticks();
        // (screenshot hook: SNOWBALLZ_END_SHOT=path)
        let mut shot = std::env::var("SNOWBALLZ_END_SHOT").ok();
        loop {
            self.clock.tick(35);
            // (a moment before a key or click counts, so the last order of
            // the match doesn't skip the screen)
            let ready = sdl::ticks().wrapping_sub(shown) > 1200;
            let mut done = false;
            for e in gfx::events() {
                match e.kind() {
                    sdl::QUIT => crate::quit(0),
                    sdl::KEYUP | sdl::MOUSEBUTTONUP if ready => done = true,
                    _ => {}
                }
            }
            if done {
                break;
            }
            alpha = (alpha + 0.05).min(1.0);
            gfx::clear((0, 0, 0));
            backgrounds::draw(name, alpha);
            let w = self.world.as_ref().unwrap();
            crate::stats::draw_screen(w, won, ((alpha - 0.3) / 0.7).max(0.0));
            if ready && alpha >= 1.0 {
                if let Some(p) = shot.take() {
                    crate::gfx::set_present_hook(Some(p));
                    gfx::present();
                    crate::quit(0);
                }
            }
            gfx::present();
        }
    }

    // -- network ---------------------------------------------------------------

    fn handle_network(&mut self) {
        let mut new_conns = Vec::new();
        let events = match self.net_mut() {
            Some(Net::Server(s)) => {
                new_conns = s.accept();
                s.poll()
            }
            Some(n) => n.poll(),
            None => return,
        };
        for c in new_conns {
            let m = Msg::Mapname { mapname: self.mapname.clone() };
            self.world.as_mut().unwrap().send_to(m, &[c], &[]);
        }
        for e in events {
            match e {
                NetEvent::Disconnected(c, reason) => self.handle_disconnected(c, &reason),
                NetEvent::Msg(m, c) => {
                    if self.is_server() {
                        if let Some(w) = self.world.as_mut() {
                            server_message(w, m, c);
                        }
                    } else {
                        self.client_message(m);
                    }
                }
            }
        }
    }

    fn handle_disconnected(&mut self, c: ConnId, reason: &str) {
        if self.is_server() {
            let w = self.world.as_mut().unwrap();
            if let Some(Net::Server(s)) = w.net.as_mut() {
                s.drop_connection(c);
            }
            let ids: Vec<Pid> = w.players.values().filter(|p| p.connection == Some(c)).map(|p| p.id).collect();
            for id in ids {
                let p = w.players.get_mut(&id).unwrap();
                p.connection = None;
                p.loading = false;
                if p.kind == PlayerKind::Human {
                    p.kind = PlayerKind::Ai;
                    let name = p.name.clone();
                    println!("{} left the game ({}) - the AI takes over", name, reason);
                    w.notice(&format!("{} left the game - the AI takes over", name));
                    ai::setup(w, id);
                }
            }
        } else if self.disconnected_at.is_none() {
            self.disconnected_at = Some(clock::now());
            println!("Lost the connection to the server: {}", reason);
            if let Some(w) = self.world.as_mut() {
                w.notice("Lost the connection to the server");
            }
            // shown in the menu afterwards
            self.error = Some(format!("Lost the connection to the server: {}", reason));
        }
    }

    fn client_message(&mut self, m: Msg) {
        if let Msg::Mapname { mapname } = &m {
            if self.world.is_none() {
                if let Err(e) = self.load_data(mapname) {
                    self.error = Some(format!("Could not load the map '{}': {}", mapname, e));
                    self.running = false;
                    self.lobbying = false;
                }
                self.loadout = settings::loadout(mapname);
            }
            return;
        }
        if let Msg::GameStart = m {
            self.lobbying = false;
            return;
        }
        let Some(w) = self.world.as_mut() else { return };
        if let Msg::WhoYouAre { player_id } = m {
            if w.players.contains_key(&player_id) {
                w.me = Some(player_id);
                look_at_starting_igloo(w);
                w.send(Msg::ClientFinishedLoading { player_id });
                if let Some(n) = self.loadout {
                    w.send(Msg::Loadout { building_id: 0xffff, snowballers: n });
                }
            }
            return;
        }
        client_message(w, m);
    }
}

/// One status line for tests.
/// Split `text` into lines no wider than `width` pixels.
fn wrap(font: &Font, text: &str, width: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{} {}", cur, word) };
        if !cur.is_empty() && font.size_of(&candidate).0 > width {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        } else {
            cur = candidate;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn stats_line(w: &World) -> String {
    let mut parts = Vec::new();
    for p in w.players.values() {
        let regions = w.regions.iter().filter(|r| r.player == Some(p.id) && r.igloo.is_some()).count();
        let units = (0..w.units.len()).filter(|&u| w.igloo_player(u) == Some(p.id)).count();
        let sb = (0..w.units.len()).filter(|&u| w.igloo_player(u) == Some(p.id) && w.units[u].kind == UnitKind::Snowballer).count();
        let v = match p.victory {
            Some(true) => "won",
            Some(false) => "lost",
            None => "-",
        };
        let storage: i32 = w.buildings.iter().filter(|b| b.player == Some(p.id)).map(|b| b.storage[1]).sum();
        parts.push(format!("{}[{}]:igloos={},units={}({}sb/{}wk),crystal={},{}", p.name, p.id, regions, units, sb, units - sb, storage, v));
    }
    let frozen = w.units.iter().filter(|u| u.state == State::Frozen).count();
    let inside = w.units.iter().filter(|u| u.state == State::InBuilding).count();
    let cold = w.units.iter().filter(|u| u.warmth > 0 && u.warmth < 5 && !matches!(u.state, State::InBuilding | State::Entering)).count();
    let me = w.me.map(|m| m.to_string()).unwrap_or("-".into());
    // my snowballers out on the snow: "power:snowballs in hand" counts
    let mut held: std::collections::BTreeMap<String, usize> = Default::default();
    for u in 0..w.units.len() {
        if w.igloo_player(u) == w.me && w.units[u].kind == UnitKind::Snowballer && !matches!(w.units[u].state, State::InBuilding | State::Entering | State::Frozen) {
            let p = match crate::plugins::power_of(w, u) {
                Some(k) => format!("{:?}", k),
                None => "none".into(),
            };
            *held.entry(format!("{}:{}", p, w.units[u].snowballs)).or_default() += 1;
        }
    }
    let held: Vec<String> = held.iter().map(|(k, v)| format!("{}x{}", k, v)).collect();
    let dmos = w.dmos.iter().filter(|d| !d.hidden).count();
    let mine: Vec<usize> = (0..w.units.len()).filter(|&u| w.igloo_player(u) == w.me).collect();
    let my_workers = mine.iter().filter(|&&u| w.units[u].kind == UnitKind::Worker).count();
    let my_pending = mine.iter().filter(|&&u| w.units[u].switch_pending).count();
    let my_fish: i32 = (0..w.buildings.len()).filter(|&b| w.buildings[b].player == w.me).map(|b| w.num_storage(b, crate::world::ResKind::Fish)).sum();
    format!("mine: workers={} snowballers={} pending={} fish={} || me={} frozen={} inside={} cold={} snowballs={} held=[{}] dmos_shown={} | {} | powerups {}", my_workers, mine.len() - my_workers, my_pending, my_fish, me, frozen, inside, cold, w.snowballs.len(), held.join(" "), dmos, parts.join(" "), crate::plugins::summary(w))
}

/// Team colours: every player gets one of these at random (not the colour
/// of their igloo's region in the map).
const TEAM_COLORS: [[u8; 3]; 12] = [
    [220, 40, 40],   // red
    [40, 90, 230],   // blue
    [40, 170, 60],   // green
    [240, 200, 30],  // yellow
    [150, 60, 200],  // purple
    [250, 130, 20],  // orange
    [30, 190, 200],  // cyan
    [235, 90, 170],  // pink
    [120, 70, 30],   // brown
    [140, 220, 40],  // lime
    [20, 40, 120],   // navy
    [130, 130, 30],  // olive
];

/// A team colour nobody in the match has yet.
fn random_team_color(w: &mut World, region_color: [u8; 3]) -> [u8; 3] {
    let free: Vec<[u8; 3]> = TEAM_COLORS.iter().copied().filter(|c| !w.players.values().any(|p| p.color == *c)).collect();
    if free.is_empty() {
        return region_color;
    }
    free[w.rng.randint(0, free.len() as i32 - 1) as usize]
}

fn new_player_id(w: &World) -> Pid {
    let mut id = 1;
    while w.players.contains_key(&id) {
        id += 1;
    }
    id
}

/// A human player joins: give them the igloo of `color`, or any free one
/// (or one played by the AI - never another human's).
pub fn add_player(w: &mut World, name: &str, color: Option<[u8; 3]>) -> Option<Pid> {
    let ri = match color.and_then(|c| w.regions.iter().position(|r| r.color == c)) {
        Some(ri) => ri,
        None => {
            let mut idx: Vec<usize> = (0..w.regions.len()).collect();
            w.rng.shuffle(&mut idx);
            idx.into_iter().find(|&ri| {
                let r = &w.regions[ri];
                r.igloo.is_some() && r.player.map(|p| w.players.get(&p).map(|p| p.kind == PlayerKind::Ai).unwrap_or(true)).unwrap_or(true)
            })?
        }
    };
    if let Some(old) = w.regions[ri].player {
        if !w.is_alive(old) {
            w.players.remove(&old);
        }
    }
    let rc = w.regions[ri].color;
    let color = random_team_color(w, rc);
    let id = new_player_id(w);
    let mut p = Player::new(id, name, PlayerKind::Human, color);
    p.loading = true;
    w.players.insert(id, p);
    w.send(Msg::NewPlayer { player_id: id, name: name.to_string(), color, loading: true });
    w.set_region_player(ri, Some(id));
    Some(id)
}

/// At the start: give every player's igloo the penguins they chose, and
/// tell the clients (server / single player).
fn apply_loadouts(w: &mut World) {
    let chosen: Vec<(Pid, u16)> = w.players.values().filter_map(|p| p.loadout.map(|n| (p.id, n))).collect();
    for (pid, n) in chosen {
        let igloos: Vec<usize> = (0..w.buildings.len()).filter(|&b| w.buildings[b].player == Some(pid)).collect();
        for b in igloos {
            w.apply_loadout(b, n);
            let n = w.buildings[b].loadout.unwrap_or(n);
            w.send(Msg::Loadout { building_id: b as u16, snowballers: n });
        }
    }
}

/// The AI takes every igloo nobody plays.
pub fn add_map_players(w: &mut World) {
    for ri in 0..w.regions.len() {
        if w.regions[ri].player.is_some() || w.regions[ri].igloo.is_none() {
            continue;
        }
        let id = new_player_id(w);
        let name = format!("ai{}", w.players.len());
        let rc = w.regions[ri].color;
        let color = random_team_color(w, rc);
        let mut pl = Player::new(id, &name, PlayerKind::Ai, color);
        pl.ai_level = ai::default_level();
        w.players.insert(id, pl);
        w.send(Msg::NewPlayer { player_id: id, name, color, loading: false });
        w.set_region_player(ri, Some(id));
        // each AI its own mix of snowballers and workers (from a quarter
        // to four fifths snowballers), told to everybody like a player's
        if let Some(b) = w.regions[ri].igloo {
            let total = w.buildings[b].start_units.len() as i32;
            if total > 0 {
                let lo = (total as f64 * 0.25).round() as i32;
                let hi = ((total as f64 * 0.8).round() as i32).max(lo);
                let n = w.rng.randint(lo, hi) as u16;
                w.apply_loadout(b, n);
                w.send(Msg::Loadout { building_id: b as u16, snowballers: n });
                ai::setup(w, id);
            }
        }
    }
}

pub fn look_at_starting_igloo(w: &mut World) {
    ctrl::look_at_home(w);
}

/// Test hook: SNOWBALLZ_LOOK=fx,fy[,zoom] looks at a spot (fractions of the
/// map), zoomed (at the start, and again for each screenshot).
fn test_look(w: &mut World) {
    // (SNOWBALLZ_SELECT=1: my penguins out on the map are selected, to see
    // the command bar)
    if std::env::var("SNOWBALLZ_SELECT").is_ok_and(|v| v == "1") && w.me.is_some() {
        for u in 0..w.units.len() {
            if w.igloo_player(u) == w.me && w.units[u].state != crate::world::State::InBuilding {
                w.units[u].selected = true;
                w.selected.insert(u);
            }
        }
    }
    let Ok(look) = std::env::var("SNOWBALLZ_LOOK") else { return };
    let f: Vec<f64> = look.split(',').filter_map(|v| v.parse().ok()).collect();
    if f.len() < 2 {
        return;
    }
    if let Some(&z) = f.get(2) {
        ctrl::change_zoom(w, z - w.view.zoom, None);
    }
    ctrl::look_at(w, f[0] * (w.size.0 * 32) as f64, f[1] * (w.size.1 * 32) as f64);
}

/// The units of a client's order that really belong to that client.
fn units_of_sender(w: &World, ids: &[u16], c: ConnId) -> Vec<usize> {
    ids.iter()
        .map(|&i| i as usize)
        .filter(|&i| i < w.units.len())
        .filter(|&i| w.igloo_player(i).and_then(|p| w.players.get(&p)).map(|p| p.connection == Some(c)).unwrap_or(false))
        .collect()
}

fn server_message(w: &mut World, m: Msg, c: ConnId) {
    match m {
        Msg::NewPlayer { name, color, loading, .. } => {
            // a client asks to join: find a name nobody has
            let mut n = name.clone();
            let mut k = 0;
            while w.players.values().any(|p| p.name == n) {
                k += 1;
                n = format!("{}{}", name, k);
            }
            let Some(id) = add_player(w, &n, Some(color)) else {
                println!("A player tried to join, but every igloo is taken.");
                w.notice(&format!("{} could not join: the game is full", n));
                if let Some(Net::Server(s)) = w.net.as_mut() {
                    if let Some(conn) = s.get(c) {
                        conn.close();
                    }
                }
                return;
            };
            {
                let p = w.players.get_mut(&id).unwrap();
                p.loading = loading;
                p.connection = Some(c);
            }
            // tell the client about all players, igloo owners, who they are
            // and the power-ups
            let players: Vec<Msg> = w
                .players
                .values()
                .map(|p| Msg::NewPlayer { player_id: p.id, name: p.name.clone(), color: p.color, loading: p.loading })
                .collect();
            for pm in players {
                w.send_to(pm, &[c], &[]);
            }
            let owners: Vec<Msg> = w
                .buildings
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.player.map(|p| Msg::BuildingOwner { building_id: i as u16, player_id: p }))
                .collect();
            for om in owners {
                w.send_to(om, &[c], &[]);
            }
            let loadouts: Vec<Msg> = w
                .buildings
                .iter()
                .enumerate()
                .filter_map(|(i, b)| b.loadout.map(|n| Msg::Loadout { building_id: i as u16, snowballers: n }))
                .collect();
            for lm in loadouts {
                w.send_to(lm, &[c], &[]);
            }
            w.send_to(Msg::WhoYouAre { player_id: id }, &[c], &[]);
            let dmos: Vec<Msg> = w
                .dmos
                .iter()
                .enumerate()
                .map(|(i, d)| Msg::CreateDmo {
                    dmo_id: i as u16,
                    image_name: d.image_name.clone(),
                    position: (d.position.0 as u16, d.position.1 as u16),
                    obstruction: d.obstruction,
                    hidden: d.hidden,
                    minimapimage: d.minimapimage.clone().unwrap_or_default(),
                })
                .collect();
            for dm in dmos {
                w.send_to(dm, &[c], &[]);
            }
            if let Some(ms) = crate::plugins::next_drop_in(w) {
                crate::plugins::announce(w, ms, false, Some(c));
            }
            crate::alliance::send_state_to(w, c);
            crate::wall::send_state_to(w, c);
            crate::ai::send_levels_to(w, c);
            // joining a match that already runs: straight into the game
            // (the Python version left such players waiting in the lobby)
            if w.started {
                w.send_to(Msg::GameStart, &[c], &[]);
            }
        }
        Msg::ClientFinishedLoading { player_id } => {
            if let Some(p) = w.players.get_mut(&player_id) {
                p.loading = false;
                w.send(Msg::ClientFinishedLoading { player_id });
            }
        }
        Msg::Loadout { snowballers, .. } => {
            // only before the match starts, and only for the sender's player
            if !w.started {
                for p in w.players.values_mut().filter(|p| p.connection == Some(c)) {
                    p.loadout = Some(snowballers);
                }
            }
        }
        Msg::SetJob { pos, run_mode, unit_ids } => {
            let pos = (pos.0 as i32, pos.1 as i32);
            if w.tile_index(pos.0, pos.1).is_none() {
                return;
            }
            let us = units_of_sender(w, &unit_ids, c);
            let n = us.len();
            for u in us {
                unit::set_job(w, u, pos, run_mode, n);
            }
        }
        Msg::DisbanUnits { unit_ids } => {
            for u in units_of_sender(w, &unit_ids, c) {
                unit::disban(w, u);
            }
        }
        Msg::ChatMessage { msg, from_id, to_id } => {
            let sender = w.players.iter().find(|(_, p)| p.connection == Some(c)).map(|(&id, _)| id);
            if crate::plugins::hidden_request(w, &msg, from_id, sender) {
                return;
            }
            let Some(from) = w.players.get(&from_id) else { return };
            if from.connection == Some(c) {
                let to = if to_id == 0xffff { None } else { Some(to_id) };
                if to.map(|t| w.players.contains_key(&t)).unwrap_or(true) {
                    w.send_chat(&msg, from_id, to);
                }
            }
        }
        _ => {}
    }
}

fn client_message(w: &mut World, m: Msg) {
    match m {
        Msg::NewPlayer { player_id, name, color, loading } => {
            let mut p = Player::new(player_id, &name, PlayerKind::Human, color);
            p.loading = loading;
            w.players.insert(player_id, p);
        }
        Msg::ClientFinishedLoading { player_id } => {
            if let Some(p) = w.players.get_mut(&player_id) {
                p.loading = false;
            }
        }
        Msg::Loadout { building_id, snowballers } => w.apply_loadout(building_id as usize, snowballers),
        Msg::CreateDmo { dmo_id, image_name, position, obstruction, hidden, minimapimage } => {
            if dmo_id as usize == w.dmos.len() {
                let mm = if minimapimage.is_empty() { None } else { Some(minimapimage) };
                w.dmos.push(Dmo::new(&image_name, (position.0 as i32, position.1 as i32), hidden, obstruction, mm, false));
            }
        }
        Msg::ChatMessage { msg, from_id, to_id } => {
            if crate::plugins::timer_message(w, &msg, from_id) {
                return;
            }
            if w.players.contains_key(&from_id) {
                let to = if to_id == 0xffff { None } else { Some(to_id).filter(|t| w.players.contains_key(t)) };
                w.display_chat(&msg, from_id, to);
            }
        }
        Msg::ResourceQuantity { tx, ty, q } => {
            if let Some(r) = w.tile(tx as i32, ty as i32).and_then(|t| t.resource) {
                w.resources[r].quantity = q as i32;
            }
        }
        Msg::UnitPosition { unit_id, x, y, dir_x, dir_y, state_id, holding_id } => {
            let ui = unit_id as usize;
            if ui < w.units.len() && w.tile_index(x as i32, y as i32).is_some() {
                unit::handle_position(w, ui, x as i32, y as i32, (dir_x as i32, dir_y as i32), State::from_index(state_id), ResKind::from_holding_id(holding_id));
            }
        }
        Msg::Snowball { unit_id, .. } => {
            let ui = unit_id as usize;
            if ui < w.units.len() {
                unit::handle_snowball(w, ui, &m);
            }
        }
        Msg::Warmth { unit_id, warmth } => {
            if let Some(u) = w.units.get_mut(unit_id as usize) {
                u.warmth = warmth as i32;
            }
        }
        Msg::UnitIglooChange { unit_id, building_id } => {
            if (building_id as usize) < w.buildings.len() {
                if let Some(u) = w.units.get_mut(unit_id as usize) {
                    u.igloo = building_id as usize;
                }
            }
        }
        Msg::UnitAddEmblem { unit_id, image_name, animate, offset } => {
            let ui = unit_id as usize;
            if ui < w.units.len() {
                w.add_emblem(ui, Emblem::new(&image_name, animate, (offset.0 as i32, offset.1 as i32)));
            }
        }
        Msg::UnitRemoveEmblem { unit_id } => {
            if let Some(u) = w.units.get_mut(unit_id as usize) {
                u.emblems.clear();
            }
        }
        Msg::BuildingOwner { building_id, player_id } => {
            let bi = building_id as usize;
            if bi < w.buildings.len() && w.players.contains_key(&player_id) && w.buildings[bi].player != Some(player_id) {
                w.set_building_player(bi, Some(player_id));
                let r = w.buildings[bi].region;
                w.set_region_player(r, Some(player_id));
            }
        }
        Msg::BuildingJeopardy { building_id, jeopardy, player_taking_over_id } => {
            let now = w.now();
            if let Some(b) = w.buildings.get_mut(building_id as usize) {
                b.jeopardy = jeopardy != 0;
                b.player_taking_over = Some(player_taking_over_id);
                b.jeopardy_count = now;
            }
        }
        Msg::BuildingStorage { building_id, fish, crystal } => {
            if let Some(b) = w.buildings.get_mut(building_id as usize) {
                b.storage = [fish as i32, crystal as i32];
            }
        }
        Msg::DmoHidden { dmo_id, hidden } => {
            if let Some(d) = w.dmos.get_mut(dmo_id as usize) {
                d.hidden = hidden != 0;
            }
        }
        Msg::PlayerVictory { player_id, victory } => {
            if let Some(p) = w.players.get_mut(&player_id) {
                println!("{:?}", victory);
                p.victory = victory;
            }
        }
        _ => {}
    }
}
