//! The game world (data.py + map.py + player.py of the Python version):
//! tiles, regions, igloos, penguins, resources and players, the map loader,
//! and the setters that also tell the network about a change (what the
//! Master* classes did).

use crate::ai::Ai;
use crate::clock;
use crate::effects::{ImprintSet, SnowParticles, Snowflakes};
use crate::gfx::{self, Font, Tex};
use crate::image::Image;
use crate::ini::Ini;
use crate::net::{self, ConnId, Msg, Net};
use crate::plugins::{self, Event, Plugins};
use crate::rng::Rng;
use crate::settings;
use crate::snowball::Snowball;
use crate::zip::Zip;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub type Pid = u16;

pub const ELEV_MULT: f64 = 4.0;
pub const IGLOO_SIZE: i32 = 4;
pub const IGLOO_RANGE: i32 = 10;
pub const TAKEOVER_TIME: u32 = 30000;
pub const SMOKE_START: (f64, f64) = (101.0, 30.0);

/// The 8 directions in sprite sheet row order.
pub const DIRECTIONS: [(i32, i32); 8] = [(0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1), (1, 0), (1, 1)];

pub fn dir_index(d: (i32, i32)) -> usize {
    DIRECTIONS.iter().position(|&x| x == d).unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// single player
    Local,
    Server,
    Client,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileKind {
    Snow,
    Ice,
}

pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub kind: TileKind,
    pub building: Option<usize>,
    pub building_root: bool,
    pub resource: Option<usize>,
    pub region: usize,
    pub elevation: f64,
}

pub struct Region {
    pub color: [u8; 3],
    pub player: Option<Pid>,
    pub tiles: Vec<usize>,
    pub igloo: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum ResKind {
    Tree,
    IcePinnacle,
    Fish,
    Crystal,
}

impl ResKind {
    pub fn name(&self) -> &'static str {
        match self {
            ResKind::Tree => "tree",
            ResKind::IcePinnacle => "icepinnacle",
            ResKind::Fish => "fish",
            ResKind::Crystal => "crystal",
        }
    }
    pub fn from_name(s: &str) -> Option<ResKind> {
        match s.trim() {
            "tree" => Some(ResKind::Tree),
            "icepinnacle" => Some(ResKind::IcePinnacle),
            "fish" => Some(ResKind::Fish),
            "crystal" => Some(ResKind::Crystal),
            _ => None,
        }
    }
    pub fn gatherable(&self) -> bool {
        matches!(self, ResKind::Fish | ResKind::Crystal)
    }
    /// index in the network's holding list [None, fish, crystal]
    pub fn holding_id(h: Option<ResKind>) -> u8 {
        match h {
            Some(ResKind::Fish) => 1,
            Some(ResKind::Crystal) => 2,
            _ => 0,
        }
    }
    pub fn from_holding_id(i: u8) -> Option<ResKind> {
        match i {
            1 => Some(ResKind::Fish),
            2 => Some(ResKind::Crystal),
            _ => None,
        }
    }
}

pub struct Resource {
    pub tile: usize,
    pub kind: ResKind,
    pub quantity: i32,
    pub image: Tex,
    pub empty_image: Option<Tex>,
    pub x: f64,
    pub y: f64,
}

impl Resource {
    pub fn texture(&self) -> Tex {
        match self.empty_image {
            Some(e) if self.quantity <= 0 => e,
            _ => self.image,
        }
    }
}

/// Dynamic map objects (the power-ups on the map).
pub struct Dmo {
    pub image_name: String,
    pub position: (i32, i32),
    pub hidden: bool,
    pub obstruction: bool,
    pub minimapimage: Option<String>,
    pub animate: bool,
    pub alpha: f64,
    pub speed: f64,
}

impl Dmo {
    pub fn new(image_name: &str, position: (i32, i32), hidden: bool, obstruction: bool, minimapimage: Option<String>, animate: bool) -> Dmo {
        Dmo {
            image_name: image_name.to_string(),
            position,
            hidden,
            obstruction,
            minimapimage,
            animate,
            alpha: if animate { 0.5 } else { 1.0 },
            speed: 0.02,
        }
    }

    pub fn doanimate(&mut self) {
        if self.animate {
            self.alpha += self.speed;
            if self.alpha > 0.8 {
                self.alpha = 0.8;
                self.speed = -self.speed;
            } else if self.alpha < 0.4 {
                self.alpha = 0.4;
                self.speed = -self.speed;
            }
        }
    }
}

/// An igloo.
pub struct Building {
    pub x: i32,
    pub y: i32,
    pub tile: usize,
    pub region: usize,
    pub construction: i32,
    /// fish, crystal
    pub storage: [i32; 2],
    pub units: BTreeSet<usize>,
    pub units_in_building: BTreeSet<usize>,
    pub player: Option<Pid>,
    pub jeopardy: bool,
    pub jeopardy_count: u32,
    pub player_taking_over: Option<Pid>,
    pub smoke: Vec<((f64, f64), i32)>,
    pub smoke_counter: u32,
    /// the penguins this igloo started the match with (in creation order)
    pub start_units: Vec<usize>,
    /// the snowballers its owner chose for the start (None: the map's mix)
    pub loadout: Option<u16>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitKind {
    Worker,
    Snowballer,
}

impl UnitKind {
    pub fn name(&self) -> &'static str {
        match self {
            UnitKind::Worker => "worker",
            UnitKind::Snowballer => "snowballer",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Walking,
    Frozen,
    Throwing,
    Gathering,
    Entering,
    InBuilding,
    Unloading,
    GettingSnowball,
    Stray,
}

const STATES: [State; 10] = [
    State::Idle,
    State::Walking,
    State::Frozen,
    State::Throwing,
    State::Gathering,
    State::Entering,
    State::InBuilding,
    State::Unloading,
    State::GettingSnowball,
    State::Stray,
];

impl State {
    pub fn index(&self) -> u8 {
        STATES.iter().position(|s| s == self).unwrap_or(9) as u8
    }
    pub fn from_index(i: u8) -> State {
        *STATES.get(i as usize).unwrap_or(&State::Stray)
    }
}

/// What a penguin's brain answers: walk one tile, wait (try again next
/// frame) or "can't" (wait a little before trying again).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Go(i32, i32),
    Wait,
    Fail,
}

impl Step {
    pub fn is_go(&self) -> bool {
        matches!(self, Step::Go(..))
    }
}

#[derive(Clone, Debug)]
pub struct Emblem {
    pub image_name: String,
    pub animate: bool,
    pub offset: (i32, i32),
    pub alpha: f64,
}

impl Emblem {
    pub fn new(image_name: &str, animate: bool, offset: (i32, i32)) -> Emblem {
        Emblem { image_name: image_name.to_string(), animate, offset, alpha: if animate { 0.5 } else { 1.0 } }
    }
}

pub struct Unit {
    #[allow(dead_code)]
    pub id: usize,
    pub kind: UnitKind,
    pub x: i32,
    pub y: i32,
    pub draw_x: f64,
    pub draw_y: f64,
    pub current_frame: i32,
    pub anim_time: f64,
    pub group: Option<u8>,
    pub selected: bool,
    pub warmth: i32,
    pub max_warmth: i32,
    pub prv_node: Option<(i32, i32)>,
    pub igloo: usize,
    pub target_pos: Option<(i32, i32)>,
    pub stray_range: i32,
    pub(crate) gtb: Option<usize>,
    pub state: State,
    pub holding: Option<ResKind>,
    pub targeted_unit: Option<usize>,
    pub moving: bool,
    pub direction: (i32, i32),
    pub target_resource: Option<usize>,
    pub last_tiles: Vec<(i32, i32)>,
    pub counter: u32,
    pub fail_time: u32,
    pub run_mode: bool,
    pub emblems: Vec<Emblem>,
    pub sx: crate::effects::Lerp,
    pub sy: crate::effects::Lerp,
    pub snowballs: i32,
    pub going_for: Option<ResKind>,
    pub gathering_from: Option<usize>,
    pub player: Option<Pid>,
    pub frozen_since: Option<u32>,
    pub next_position: Option<((i32, i32), (i32, i32), State, Option<ResKind>)>,
    pub next_snowball: Option<(f64, f64, f64, f64, usize)>,
    pub last_state_sent: State,
}

impl Unit {
    pub fn go_to_building(&self) -> Option<usize> {
        self.gtb
    }

    /// (the old property setter: a penguin that is entering, or is inside
    /// with somewhere to go, keeps its building)
    pub fn set_go_to_building(&mut self, b: Option<usize>) {
        if self.state != State::Entering && (self.state != State::InBuilding || self.gtb.is_none()) {
            self.gtb = b;
        }
    }

    pub fn sprite_xy(&self, t: u32) -> (f64, f64) {
        (self.sx.get(t), self.sy.get(t))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayerKind {
    Human,
    Ai,
}

pub struct Player {
    pub id: Pid,
    pub name: String,
    pub kind: PlayerKind,
    pub color: [u8; 3],
    pub glcolor: [f32; 3],
    pub connection: Option<ConnId>,
    pub loading: bool,
    /// tile -> how long it stays interesting (in AI ticks), insertion order
    pub alert_tiles: Vec<((i32, i32), i32)>,
    pub snowballer_alert_range: f64,
    pub open_resources: BTreeSet<usize>,
    pub victory: Option<bool>,
    pub ai: Ai,
    /// how many of the starting penguins should be snowballers (the rest
    /// are workers); None keeps the map's mix
    pub loadout: Option<u16>,
}

impl Player {
    pub fn new(id: Pid, name: &str, kind: PlayerKind, color: [u8; 3]) -> Player {
        // (components of exactly 0 or 1 were taken as already being 0..1)
        let glcolor = color.map(|c| if c > 1 { c as f32 / 255.0 } else { c as f32 });
        Player {
            id,
            name: name.to_string(),
            kind,
            color,
            glcolor,
            connection: None,
            loading: false,
            alert_tiles: Vec::new(),
            snowballer_alert_range: 40.0,
            open_resources: BTreeSet::new(),
            victory: None,
            ai: Ai::new(id),
            loadout: None,
        }
    }

    pub fn set_alert(&mut self, pos: (i32, i32), v: i32) {
        match self.alert_tiles.iter_mut().find(|(p, _)| *p == pos) {
            Some(e) => e.1 = v,
            None => self.alert_tiles.push((pos, v)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub x: f64,
    pub y: f64,
    pub w: i32,
    pub h: i32,
    pub zoom: f64,
}

impl View {
    pub fn scale_w(&self) -> i32 {
        (self.w as f64 / self.zoom).floor() as i32
    }
    pub fn scale_h(&self) -> i32 {
        (self.h as f64 / self.zoom).floor() as i32
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UnitCfg {
    pub max_warmth: i32,
    pub gather_time: u32,
    pub create_snowball_time: u32,
}

impl Default for UnitCfg {
    fn default() -> Self {
        UnitCfg { max_warmth: 10, gather_time: 20000, create_snowball_time: 3000 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AiCfg {
    pub gather: Option<ResKind>,
    pub guard_pos: Option<(i32, i32)>,
    pub gather_force: i32,
}

pub struct Penguins {
    pub cells: Vec<[Tex; 11]>,
}

impl Penguins {
    fn load(path: &str) -> Penguins {
        let big = gfx::texture(path);
        let mut cells = Vec::new();
        for dc in 0..8 {
            let mut row = [big; 11];
            for (i, c) in row.iter_mut().enumerate() {
                *c = big.sub(32 * i as i32, (256 - 32) - 32 * dc, 32, 32);
            }
            cells.push(row);
        }
        Penguins { cells }
    }

    /// frame 0-9, or 10 for "idle"
    pub fn get(&self, frame: usize, dir: (i32, i32)) -> Tex {
        self.cells[dir_index(dir)][frame.min(10)]
    }
}

pub struct Textures {
    pub penguins: HashMap<&'static str, Penguins>,
    pub frozen: Tex,
    pub trees: Vec<Tex>,
    pub icepinnacle: Vec<Tex>,
    pub fish: Tex,
    pub fish_empty: Tex,
    pub crystal: Tex,
    pub crystal_empty: Tex,
    pub snowball: [Tex; 2],
    pub snowparticle: Tex,
    pub smoke: Tex,
    pub selected: Tex,
    pub marker: Tex,
    pub jeopardy_marker: Tex,
    pub igloo: Tex,
}

impl Textures {
    pub fn load() -> Textures {
        let mut penguins = HashMap::new();
        for (k, f) in [
            ("walk", "walk"),
            ("workerwalk", "workerwalk"),
            ("snowballerwalk", "snowballerwalk"),
            ("throwing", "throw"),
            ("snowballerthrowing", "snowballerthrow"),
            ("getting_snowball", "gathersnow"),
            ("snowballergetting_snowball", "snowballergathersnow"),
            ("unloadfish", "unloadfish"),
            ("workerunloadfish", "workerunloadfish"),
            ("unloadcrystal", "unloadcrystal"),
            ("workerunloadcrystal", "workerunloadcrystal"),
            ("gathering", "gatherfish"),
            ("workergathering", "workergatherfish"),
            ("walkfish", "walkfish"),
            ("workerwalkfish", "workerwalkfish"),
            ("walkcrystal", "walkcrystal"),
            ("workerwalkcrystal", "workerwalkfish"),
        ] {
            penguins.insert(k, Penguins::load(&format!("data/{}.png", f)));
        }
        let tt = gfx::texture("data/trees.png");
        let it = gfx::texture("data/icepinnacles.png");
        let ss = gfx::texture("data/snowball.png");
        Textures {
            penguins,
            frozen: gfx::texture("data/frozen.png"),
            trees: vec![tt.sub(0, 0, 64, 64), tt.sub(64, 0, 64, 64), tt.sub(0, 64, 64, 64), tt.sub(64, 64, 64, 64)],
            icepinnacle: vec![it.sub(0, 0, 64, 64), it.sub(64, 0, 64, 64), it.sub(0, 64, 64, 64)],
            fish: gfx::texture("data/fish.png"),
            fish_empty: gfx::texture("data/fishempty.png"),
            crystal: gfx::texture("data/crystal.png"),
            crystal_empty: gfx::texture("data/crystalempty.png"),
            snowball: [ss.sub(0, 0, 8, 8), ss.sub(8, 0, 8, 8)],
            snowparticle: gfx::texture("data/snowparticle.png"),
            smoke: gfx::texture("data/smoke.png"),
            selected: gfx::texture("data/selected.png"),
            marker: gfx::texture("data/marker.png"),
            jeopardy_marker: gfx::texture("data/jeopardy_marker.png"),
            igloo: gfx::texture("data/igloo.png"),
        }
    }
}

pub struct Message {
    pub time: u32,
    pub msg: String,
    /// what Ctrl+C copies ("" = nothing): a chat line without the name
    pub copy: String,
    pub color: [f32; 3],
    pub force_close: bool,
    pub num_lines: usize,
    pub active: bool,
    pub alpha: f64,
    pub speed: f64,
}

impl Message {
    fn new(time: u32, msg: String, color: [f32; 3], pulsate: bool) -> Message {
        let num_lines = msg.split('\n').count();
        let copy = msg.clone();
        Message { time, msg, copy, color, force_close: false, num_lines, active: pulsate, alpha: 1.0, speed: 0.05 }
    }

    pub fn animate(&mut self) {
        if self.active {
            self.alpha += self.speed;
            if self.alpha > 1.0 {
                self.alpha = 1.0;
                self.speed = -self.speed;
            } else if self.alpha < 0.6 {
                self.alpha = 0.6;
                self.speed = -self.speed;
            }
        }
    }
}

/// The chat line being typed.
#[derive(Default)]
pub struct Chat {
    pub chatting: bool,
    pub msg: String,
    pub help: String,
    pub last_back_press: u32,
    pub last_chat_name: String,
    pub last_received_from: String,
    /// ignore the text event of the key that opened the chat
    pub swallow_text: bool,
}

pub struct Imprints {
    pub snowball: ImprintSet,
    pub snowball_ice: ImprintSet,
    pub foot: ImprintSet,
    pub moves: ImprintSet,
    pub run_moves: ImprintSet,
    pub gather_moves: ImprintSet,
}

/// Something to draw in the depth-sorted draw buffer.
pub enum DrawItem {
    Tex { tex: Tex, x: f64, y: f64, color: [f32; 4] },
}

pub struct Rendered {
    pub key: (i32, f64, f64),
    pub item: DrawItem,
}

impl Rendered {
    pub fn tex(tex: Tex, x: f64, y: f64, z: i32, color: [f32; 4]) -> Rendered {
        Rendered { key: (z, y + tex.h as f64, -x), item: DrawItem::Tex { tex, x, y, color } }
    }
}

pub struct World {
    #[allow(dead_code)]
    pub map_name: String,
    pub config: Ini,
    pub size: (i32, i32),
    pub tiles: Vec<Tile>,
    pub regions: Vec<Region>,
    pub units: Vec<Unit>,
    pub buildings: Vec<Building>,
    pub resources: Vec<Resource>,
    pub dmos: Vec<Dmo>,
    pub unit_nodes: HashMap<(i32, i32), BTreeSet<usize>>,
    pub players: BTreeMap<Pid, Player>,
    pub me: Option<Pid>,
    /// set when the match starts (loadouts can be chosen until then)
    pub started: bool,
    pub role: Role,
    pub net: Option<Net>,
    pub snowballs: Vec<Snowball>,
    pub particles: Vec<SnowParticles>,
    pub selected: BTreeSet<usize>,
    pub select_box_start: Option<(f64, f64)>,
    pub select_box_end: Option<(f64, f64)>,
    pub draw_minimap: bool,
    pub show_fps: bool,
    pub view: View,
    pub messages: Vec<Message>,
    pub chat: Chat,
    pub groups: [BTreeSet<usize>; 10],
    pub imprints: Imprints,
    pub flakes: Snowflakes,
    pub plugins: Plugins,
    pub ai_cfg: AiCfg,
    pub worker_cfg: UnitCfg,
    pub snowballer_cfg: UnitCfg,
    pub footprints: bool,
    pub rng: Rng,
    pub tex: Textures,
    pub chat_font: Font,
    pub stat_font: Font,
    pub draw_buffer: Vec<Rendered>,
    /// the local player was defeated and watches the rest of the game
    pub spectating: bool,
    pub holding_mouse: bool,
}

pub const UNIT_NODE: i32 = 7;

impl World {
    pub fn now(&self) -> u32 {
        clock::now()
    }

    pub fn is_server(&self) -> bool {
        self.role != Role::Client
    }

    pub fn tile_index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.size.0 || y >= self.size.1 {
            None
        } else {
            Some((y * self.size.0 + x) as usize)
        }
    }

    pub fn tile(&self, x: i32, y: i32) -> Option<&Tile> {
        self.tile_index(x, y).map(|i| &self.tiles[i])
    }

    pub fn elevation(&self, x: i32, y: i32) -> f64 {
        self.tile(x, y).map(|t| t.elevation).unwrap_or(0.0)
    }

    pub fn unit_cfg(&self, k: UnitKind) -> UnitCfg {
        match k {
            UnitKind::Worker => self.worker_cfg,
            UnitKind::Snowballer => self.snowballer_cfg,
        }
    }

    // -- the map ------------------------------------------------------------

    /// Load maps/<name>.zip.  Plugins are only started on a server (or in
    /// single player); clients get their state from the server.
    pub fn load(name: &str, role: Role, net: Option<Net>, view: View, chat_font: Font, stat_font: Font) -> Result<World, String> {
        let z = Zip::open(&format!("maps/{}.zip", name))?;
        let config = Ini::parse(&String::from_utf8_lossy(&z.read("settings.ini")?));
        let get = |k: &str| config.get("map", k).map(|s| s.to_string());
        let img = |k: &str| -> Result<Option<Image>, String> {
            match get(k) {
                Some(f) => Ok(Some(Image::decode(&z.read(&f)?).map_err(|e| format!("{}: {}", f, e))?)),
                None => Ok(None),
            }
        };
        let simage = img("resources")?.ok_or("the map has no resources image")?;
        let rimage = img("regions")?.ok_or("the map has no regions image")?;
        let timage = img("terrain")?;
        let land = img("land")?;
        let size = (rimage.w as i32, rimage.h as i32);

        let mut w = World {
            map_name: name.to_string(),
            config: config.clone(),
            size,
            tiles: Vec::new(),
            regions: Vec::new(),
            units: Vec::new(),
            buildings: Vec::new(),
            resources: Vec::new(),
            dmos: Vec::new(),
            unit_nodes: HashMap::new(),
            players: BTreeMap::new(),
            me: None,
            started: false,
            role,
            net,
            snowballs: Vec::new(),
            particles: Vec::new(),
            selected: BTreeSet::new(),
            select_box_start: None,
            select_box_end: None,
            draw_minimap: false,
            show_fps: false,
            view,
            messages: Vec::new(),
            chat: Chat::default(),
            groups: Default::default(),
            imprints: Imprints {
                snowball: ImprintSet::new("data/snowballimprent.png", 2.0),
                snowball_ice: ImprintSet::new("data/snowball_ice_imprent.png", 2.0),
                foot: ImprintSet::new("data/footprint.png", 4.0),
                moves: ImprintSet::new("data/marker.png", 0.3),
                run_moves: ImprintSet::new("data/marker_run.png", 0.3),
                gather_moves: ImprintSet::new("data/marker_gather.png", 0.3),
            },
            flakes: Snowflakes::new(),
            plugins: Plugins::default(),
            ai_cfg: AiCfg::default(),
            worker_cfg: UnitCfg::default(),
            snowballer_cfg: UnitCfg { max_warmth: 10, ..Default::default() },
            footprints: settings::get_option("footprints") != "0",
            rng: Rng::from_time(),
            tex: Textures::load(),
            chat_font,
            stat_font,
            draw_buffer: Vec::new(),
            spectating: false,
            holding_mouse: false,
        };

        // regions and terrain (regions in the order the Python version made
        // them: column by column)
        let mut region_of: HashMap<[u8; 3], usize> = HashMap::new();
        let mut tiles: Vec<Option<Tile>> = (0..size.0 * size.1).map(|_| None).collect();
        for x in 0..size.0 {
            for y in 0..size.1 {
                let p = rimage.get(x as usize, y as usize);
                let rn = [p[0], p[1], p[2]];
                let ri = *region_of.entry(rn).or_insert_with(|| {
                    w.regions.push(Region { color: rn, player: None, tiles: Vec::new(), igloo: None });
                    w.regions.len() - 1
                });
                let mut kind = TileKind::Snow;
                if let Some(l) = &land {
                    if (x as usize) < l.w && (y as usize) < l.h && l.get(x as usize, y as usize) == [0, 255, 255, 255] {
                        kind = TileKind::Ice;
                    }
                }
                let mut elevation = 0.0;
                if let Some(t) = &timage {
                    if (x as usize) < t.w && (y as usize) < t.h {
                        elevation = t.get(x as usize, y as usize)[0] as f64 / 255.0 * 30.0;
                    }
                }
                let idx = (y * size.0 + x) as usize;
                w.regions[ri].tiles.push(idx);
                tiles[idx] = Some(Tile { x, y, kind, building: None, building_root: false, resource: None, region: ri, elevation });
            }
        }
        w.tiles = tiles.into_iter().map(|t| t.unwrap()).collect();

        // unit settings
        for (sec, cfg) in [("snowballers", &mut w.snowballer_cfg), ("workers", &mut w.worker_cfg)] {
            for (k, v) in config.items(sec) {
                let v: i64 = v.trim().parse().unwrap_or(0);
                match k.as_str() {
                    "max_warmth" => cfg.max_warmth = v as i32,
                    "gather_time" => cfg.gather_time = v as u32,
                    "create_snowball_time" => cfg.create_snowball_time = v as u32,
                    _ => {}
                }
            }
        }

        // AI settings
        if let Some(g) = config.get("ai", "gather") {
            w.ai_cfg.gather = ResKind::from_name(g);
        }
        if let Some(g) = config.get("ai", "guard") {
            let p: Vec<i32> = g.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if p.len() == 2 {
                w.ai_cfg.guard_pos = Some((p[0], p[1]));
            }
        }
        if let Some(f) = config.get_int("ai", "gather_force") {
            w.ai_cfg.gather_force = f as i32;
        }

        // igloos (and their penguins)
        for (_, pos) in config.items("igloos") {
            let p: Vec<i32> = pos.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if p.len() != 2 {
                return Err(format!("bad igloo position '{}'", pos));
            }
            w.add_igloo(p[0], p[1])?;
        }

        // resources
        for x in 0..size.0 {
            for y in 0..size.1 {
                let ti = w.tile_index(x, y).unwrap();
                if w.tiles[ti].building.is_some() || w.tiles[ti].resource.is_some() {
                    continue;
                }
                let c = if (x as usize) < simage.w && (y as usize) < simage.h { simage.get(x as usize, y as usize) } else { [0; 4] };
                let kind = w.tiles[ti].kind;
                let r = if c == [0, 255, 0, 255] && kind == TileKind::Snow {
                    let i = w.rng.randint(0, 3) as usize;
                    Some((ResKind::Tree, 10, w.tex.trees[i], None))
                } else if c == [0, 0, 255, 255] && kind == TileKind::Ice {
                    let i = w.rng.randint(0, 2) as usize;
                    Some((ResKind::IcePinnacle, 10, w.tex.icepinnacle[i], None))
                } else if c == [0, 255, 255, 255] {
                    Some((ResKind::Fish, 7, w.tex.fish, Some(w.tex.fish_empty)))
                } else if c == [255, 0, 255, 255] {
                    Some((ResKind::Crystal, 7, w.tex.crystal, Some(w.tex.crystal_empty)))
                } else {
                    None
                };
                if let Some((kind, q, image, empty)) = r {
                    let t = &w.tiles[ti];
                    let rx = (t.x * 32) as f64;
                    let ry = (t.y * 32) as f64 - image.h as f64 + 32.0 - t.elevation * ELEV_MULT;
                    w.resources.push(Resource { tile: ti, kind, quantity: q, image, empty_image: empty, x: rx, y: ry });
                    w.tiles[ti].resource = Some(w.resources.len() - 1);
                }
            }
        }

        // plugins
        if w.is_server() {
            w.plugins = plugins::init(&mut w)?;
        }
        Ok(w)
    }

    fn add_igloo(&mut self, x: i32, y: i32) -> Result<(), String> {
        let root = self.tile_index(x, y).ok_or(format!("igloo at {},{} is outside the map", x, y))?;
        let region = self.tiles[root].region;
        if self.regions[region].igloo.is_some() {
            return Err("You can only have 1 igloo per region!".into());
        }
        let mut tiles = Vec::new();
        for xx in 0..IGLOO_SIZE {
            for yy in 0..IGLOO_SIZE {
                let ti = self.tile_index(x + xx, y + yy).ok_or("igloo outside the map")?;
                let t = &self.tiles[ti];
                if t.building.is_none() && t.kind == TileKind::Snow && t.resource.is_none() {
                    tiles.push(ti);
                } else {
                    return Err(format!("building already there! ({},{})", x, y));
                }
            }
        }
        let bi = self.buildings.len();
        self.buildings.push(Building {
            x,
            y,
            tile: root,
            region,
            construction: 100,
            storage: [0, 0],
            units: BTreeSet::new(),
            units_in_building: BTreeSet::new(),
            player: None,
            jeopardy: false,
            jeopardy_count: 0,
            player_taking_over: None,
            smoke: vec![(SMOKE_START, 40)],
            smoke_counter: self.now(),
            start_units: Vec::new(),
            loadout: None,
        });
        self.regions[region].igloo = Some(bi);
        self.tiles[root].building_root = true;
        for ti in tiles {
            self.tiles[ti].building = Some(bi);
        }
        let workers = self.config.get_int("igloo_settings", "workers").unwrap_or(0);
        let snowballers = self.config.get_int("igloo_settings", "snowballers").unwrap_or(0);
        for _ in 0..workers {
            self.new_unit(UnitKind::Worker, (x + 1, y + 1), bi);
        }
        for _ in 0..snowballers {
            self.new_unit(UnitKind::Snowballer, (x + 1, y + 1), bi);
        }
        self.buildings[bi].start_units = ((self.units.len() - (workers + snowballers).max(0) as usize)..self.units.len()).collect();
        Ok(())
    }

    /// The penguins every igloo starts with on this map:
    /// (total, snowballers the map gives by default).
    pub fn igloo_penguins(&self) -> (u16, u16) {
        let workers = self.config.get_int("igloo_settings", "workers").unwrap_or(0).max(0) as u16;
        let snowballers = self.config.get_int("igloo_settings", "snowballers").unwrap_or(0).max(0) as u16;
        (workers + snowballers, snowballers)
    }

    /// Give an igloo's starting penguins the chosen mix: the last
    /// `snowballers` of them become snowballers, the others workers (so the
    /// map's own mix changes nothing).  Every computer in an online game applies
    /// the same loadouts to the same penguins.
    pub fn apply_loadout(&mut self, b: usize, snowballers: u16) {
        let Some(bld) = self.buildings.get_mut(b) else { return };
        let n = (snowballers as usize).min(bld.start_units.len());
        bld.loadout = Some(n as u16);
        let start = bld.start_units.clone();
        let first_snowballer = start.len() - n;
        for (i, &u) in start.iter().enumerate() {
            let kind = if i >= first_snowballer { UnitKind::Snowballer } else { UnitKind::Worker };
            self.set_unit_kind(u, kind);
        }
    }

    /// Turn a penguin (still in its igloo, at the start) into another kind.
    fn set_unit_kind(&mut self, u: usize, kind: UnitKind) {
        if u >= self.units.len() || self.units[u].kind == kind {
            return;
        }
        let cfg = self.unit_cfg(kind);
        let unit = &mut self.units[u];
        unit.kind = kind;
        unit.max_warmth = cfg.max_warmth;
        unit.warmth = cfg.max_warmth;
        unit.holding = None;
        unit.snowballs = if kind == UnitKind::Snowballer { 1 } else { 0 };
        unit.going_for = if kind == UnitKind::Worker { Some(ResKind::Fish) } else { None };
    }

    fn new_unit(&mut self, kind: UnitKind, pos: (i32, i32), igloo: usize) {
        let id = self.units.len();
        let cfg = self.unit_cfg(kind);
        let e = self.elevation(pos.0, pos.1);
        let (px, py) = ((pos.0 * 32) as f64, (pos.1 * 32) as f64 - e * ELEV_MULT);
        let frame = self.rng.randint(0, 9);
        self.units.push(Unit {
            id,
            kind,
            x: pos.0,
            y: pos.1,
            draw_x: 0.0,
            draw_y: 0.0,
            current_frame: frame,
            anim_time: 0.0,
            group: None,
            selected: false,
            warmth: cfg.max_warmth,
            max_warmth: cfg.max_warmth,
            prv_node: None,
            igloo,
            target_pos: None,
            stray_range: 5,
            gtb: Some(igloo),
            state: State::InBuilding,
            holding: None,
            targeted_unit: None,
            moving: false,
            direction: (0, -1),
            target_resource: None,
            last_tiles: Vec::new(),
            counter: 0,
            fail_time: 0,
            run_mode: false,
            emblems: Vec::new(),
            sx: crate::effects::Lerp::fixed(px),
            sy: crate::effects::Lerp::fixed(py),
            snowballs: if kind == UnitKind::Snowballer { 1 } else { 0 },
            going_for: if kind == UnitKind::Worker { Some(ResKind::Fish) } else { None },
            gathering_from: None,
            player: None,
            frozen_since: None,
            next_position: None,
            next_snowball: None,
            last_state_sent: State::InBuilding,
        });
        self.buildings[igloo].units.insert(id);
        self.buildings[igloo].units_in_building.insert(id);
    }

    // -- network --------------------------------------------------------------

    /// Send to everybody (server) / to the server (client); nothing in
    /// single player.
    pub fn send(&mut self, m: Msg) {
        if let Some(n) = self.net.as_mut() {
            n.send(&m);
        }
    }

    pub fn send_to(&mut self, m: Msg, to: &[ConnId], exclude: &[ConnId]) {
        if let Some(Net::Server(s)) = self.net.as_mut() {
            s.send(&m, Some(to), exclude);
        }
    }

    pub fn send_except(&mut self, m: Msg, exclude: &[ConnId]) {
        if let Some(Net::Server(s)) = self.net.as_mut() {
            s.send(&m, None, exclude);
        }
    }

    // -- setters that also tell the clients ----------------------------------

    pub fn set_warmth(&mut self, ui: usize, v: i32) {
        self.units[ui].warmth = v;
        if self.role == Role::Server {
            self.send(Msg::Warmth { unit_id: ui as u16, warmth: net::clamp_i16(v as f64) });
        }
    }

    pub fn set_quantity(&mut self, ri: usize, q: i32) {
        self.resources[ri].quantity = q;
        if self.role == Role::Server {
            let t = &self.tiles[self.resources[ri].tile];
            let m = Msg::ResourceQuantity { tx: t.x as i16, ty: t.y as i16, q: net::clamp_i16(q as f64) };
            self.send(m);
        }
    }

    /// The owner of an igloo changes (its penguins change sides too).
    pub fn set_building_player(&mut self, bi: usize, p: Option<Pid>) {
        let Some(p) = p else { return };
        if self.buildings[bi].player == Some(p) {
            return;
        }
        self.buildings[bi].player = Some(p);
        let units: Vec<usize> = self.buildings[bi].units.iter().copied().collect();
        for u in units {
            self.units[u].player = Some(p);
        }
        if self.role == Role::Server {
            self.send(Msg::BuildingOwner { building_id: bi as u16, player_id: p });
        }
    }

    pub fn set_jeopardy(&mut self, bi: usize, j: bool) {
        self.buildings[bi].jeopardy = j;
        if self.role == Role::Server {
            let p = self.buildings[bi].player_taking_over.unwrap_or(0);
            self.send(Msg::BuildingJeopardy { building_id: bi as u16, jeopardy: j as u8, player_taking_over_id: p });
        }
    }

    pub fn storage_changed(&mut self, bi: usize) {
        if self.role == Role::Server {
            let s = self.buildings[bi].storage;
            self.send(Msg::BuildingStorage { building_id: bi as u16, fish: net::clamp_i16(s[0] as f64), crystal: net::clamp_i16(s[1] as f64) });
        }
    }

    pub fn add_storage(&mut self, bi: usize, kind: ResKind, n: i32) {
        let i = if kind == ResKind::Crystal { 1 } else { 0 };
        self.buildings[bi].storage[i] += n;
        self.storage_changed(bi);
    }

    pub fn num_storage(&self, bi: usize, kind: ResKind) -> i32 {
        self.buildings[bi].storage[if kind == ResKind::Crystal { 1 } else { 0 }]
    }

    pub fn set_dmo_hidden(&mut self, di: usize, h: bool) {
        if self.dmos[di].hidden != h {
            self.dmos[di].hidden = h;
            if self.role == Role::Server {
                self.send(Msg::DmoHidden { dmo_id: di as u16, hidden: h as u8 });
            }
        }
    }

    pub fn set_victory(&mut self, pid: Pid, v: Option<bool>) {
        let Some(p) = self.players.get_mut(&pid) else { return };
        if p.victory != v {
            p.victory = v;
            if self.role == Role::Server {
                self.send(Msg::PlayerVictory { player_id: pid, victory: v });
            }
        }
    }

    pub fn add_emblem(&mut self, ui: usize, e: Emblem) {
        if self.role == Role::Server {
            let m = Msg::UnitAddEmblem {
                unit_id: ui as u16,
                image_name: e.image_name.clone(),
                animate: e.animate,
                offset: (e.offset.0 as i16, e.offset.1 as i16),
            };
            self.send(m);
        }
        if !self.units[ui].emblems.iter().any(|x| x.image_name == e.image_name) {
            self.units[ui].emblems.push(e);
        }
    }

    pub fn remove_emblem(&mut self, ui: usize, image_name: &str) {
        if self.units[ui].emblems.iter().any(|x| x.image_name == image_name) {
            if self.role == Role::Server {
                self.send(Msg::UnitRemoveEmblem { unit_id: ui as u16 });
            }
            self.units[ui].emblems.retain(|x| x.image_name != image_name);
        }
    }

    /// A region changes hands (with its igloo and resources).
    pub fn set_region_player(&mut self, ri: usize, p: Option<Pid>) {
        let old = self.regions[ri].player;
        self.regions[ri].player = p;
        plugins::fire(self, Event::RegionTakeover(ri, old));
        let tiles = self.regions[ri].tiles.clone();
        for ti in tiles {
            if let Some(res) = self.tiles[ti].resource {
                if let Some(o) = old {
                    if let Some(op) = self.players.get_mut(&o) {
                        op.open_resources.remove(&res);
                    }
                }
                if let Some(np) = p.and_then(|p| self.players.get_mut(&p)) {
                    np.open_resources.insert(res);
                }
            }
            let t = &self.tiles[ti];
            if t.building_root {
                if let Some(b) = t.building {
                    self.set_building_player(b, p);
                }
            }
        }
        let ais: Vec<Pid> = self.players.values().filter(|pl| pl.kind == PlayerKind::Ai).map(|pl| pl.id).collect();
        for a in ais {
            crate::ai::setup(self, a);
        }
    }

    pub fn is_alive(&self, pid: Pid) -> bool {
        self.regions.iter().any(|r| r.player == Some(pid))
    }

    pub fn glcolor(&self, pid: Option<Pid>) -> [f32; 3] {
        pid.and_then(|p| self.players.get(&p)).map(|p| p.glcolor).unwrap_or([1.0; 3])
    }

    pub fn me_color(&self) -> [f32; 3] {
        self.glcolor(self.me)
    }

    pub fn igloo_player(&self, ui: usize) -> Option<Pid> {
        self.buildings[self.units[ui].igloo].player
    }

    pub fn player_name(&self, pid: Pid) -> String {
        self.players.get(&pid).map(|p| p.name.clone()).unwrap_or_default()
    }

    // -- chat ----------------------------------------------------------------

    /// A message only shown on this computer.
    pub fn notice(&mut self, msg: &str) {
        if self.me.is_some() {
            let c = self.me_color();
            self.messages.push(Message::new(self.now(), msg.to_string(), c, false));
        }
    }

    /// A notice whose Ctrl+C copies `copy` ("" = not copyable).
    pub fn notice_copy(&mut self, msg: &str, copy: &str) {
        self.notice(msg);
        if self.me.is_some() {
            if let Some(m) = self.messages.last_mut() {
                m.copy = copy.to_string();
            }
        }
    }

    pub fn clear_messages(&mut self) {
        for m in self.messages.iter_mut() {
            m.force_close = true;
        }
    }

    /// Show a chat message here.
    pub fn display_chat(&mut self, msg: &str, from: Pid, to: Option<Pid>) {
        let now = self.now();
        let fc = self.glcolor(Some(from));
        let from_name = self.player_name(from);
        match to {
            Some(to) => {
                let to_name = self.player_name(to);
                if Some(from) == self.me {
                    self.messages.push(Message::new(now, format!(">{}: {}", to_name, msg), fc, false));
                    self.messages.last_mut().unwrap().copy = msg.to_string();
                }
                if Some(to) == self.me {
                    self.messages.push(Message::new(now, format!("{} > {}", from_name, msg), fc, true));
                    self.messages.last_mut().unwrap().copy = msg.to_string();
                    self.chat.last_received_from = from_name.clone();
                }
                let to_ai = self.players.get(&to).map(|p| p.kind == PlayerKind::Ai).unwrap_or(false);
                let from_human = self.players.get(&from).map(|p| p.kind == PlayerKind::Human).unwrap_or(false);
                if to_ai && from_human {
                    let reply = crate::ai::reply(self, to, msg, &from_name);
                    if let Some(p) = self.players.get_mut(&to) {
                        if !p.ai.send_msgs.contains(&reply) {
                            p.ai.send_msgs.push(reply);
                        }
                    }
                }
            }
            None => {
                self.messages.push(Message::new(now, format!("{}: {}", from_name, msg), fc, false));
                self.messages.last_mut().unwrap().copy = msg.to_string();
            }
        }
    }

    /// Show a chat message and send it on.
    pub fn send_chat(&mut self, msg: &str, from: Pid, to: Option<Pid>) {
        self.display_chat(msg, from, to);
        let m = Msg::ChatMessage { msg: msg.to_string(), from_id: from, to_id: to.unwrap_or(0xffff) };
        if self.is_server() {
            match to {
                Some(t) => {
                    if let Some(c) = self.players.get(&t).and_then(|p| p.connection) {
                        self.send_to(m, &[c], &[]);
                    }
                }
                None => {
                    let ex: Vec<ConnId> = self.players.get(&from).and_then(|p| p.connection).into_iter().collect();
                    self.send_except(m, &ex);
                }
            }
        } else {
            self.send(m);
        }
    }

    /// A player types a line: "/say text", ">name text" or just text.
    pub fn player_send_msg(&mut self, pid: Pid, msg: &str) {
        if msg.is_empty() {
            return;
        }
        let (cmd, text) = if msg.starts_with('/') || msg.starts_with('>') {
            let mut parts = msg.split_whitespace();
            let cmd = parts.next().unwrap_or("").to_string();
            (cmd, parts.collect::<Vec<_>>().join(" "))
        } else {
            ("/say".to_string(), msg.to_string())
        };
        if cmd == "/say" {
            self.send_chat(&text, pid, None);
        } else if let Some(name) = cmd.strip_prefix('>') {
            let to = self.players.values().find(|p| p.name == name).map(|p| p.id);
            if let Some(to) = to {
                self.send_chat(&text, pid, Some(to));
            }
        }
    }

    /// Map tile under a point of the screen (taking elevation into account).
    pub fn canvas_to_map(&self, x: f64, y: f64) -> (i32, i32) {
        let mx = self.view.x + x;
        let my = self.view.y + y;
        let tx = ((mx / 32.0).floor() as i32).clamp(0, self.size.0 - 1);
        let ty = ((my / 32.0).floor() as i32).clamp(0, self.size.1 - 1);
        let mut m = (tx, ty);
        for i in 1..4 {
            if let Some(t) = self.tile(tx, ty + i) {
                let py = (t.y * 32) as f64 - t.elevation * ELEV_MULT;
                if my > py {
                    m = (t.x, t.y);
                }
            }
        }
        m
    }

    /// Units around a tile (the 3x3 nodes of 7x7 tiles around it).
    pub fn units_around(&self, x: i32, y: i32) -> Vec<usize> {
        let nx = x.div_euclid(UNIT_NODE);
        let ny = y.div_euclid(UNIT_NODE);
        let mut out = Vec::new();
        for (dx, dy) in [(0, 0), (-1, 0), (1, 0), (0, 1), (-1, 1), (1, 1), (0, -1), (-1, -1), (1, -1)] {
            if let Some(n) = self.unit_nodes.get(&(nx + dx, ny + dy)) {
                out.extend(n.iter().copied());
            }
        }
        out
    }

    pub fn shutdown_net(&mut self) {
        if let Some(mut n) = self.net.take() {
            n.shutdown();
        }
    }
}
