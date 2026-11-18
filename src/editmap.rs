//! A map being edited in the map editor: what the map files describe, in a
//! form that is easy to change, plus loading and saving the map zip.
//!
//! A map is a zip with settings.ini and pictures of the same size (one
//! pixel per tile): land.png (cyan = ice), terrain.png (height in the red
//! channel), resources.png (green tree, blue ice pinnacle, cyan fish hole,
//! magenta crystal) and regions.png (one colour per territory).  The editor
//! makes one territory per igloo: every tile belongs to the nearest one.

use crate::image::Image;
use crate::ini::Ini;
use crate::png;
use crate::world::IGLOO_SIZE;
use crate::zip::{self, Zip};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Res {
    None,
    Tree,
    Pinnacle,
    Fish,
    Crystal,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Victory {
    /// take every igloo
    Conquer,
    /// be the first to bring `amount` loads of crystal / fish home
    Crystal,
    Fish,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Power {
    DoubleSnowball,
    Range,
    Defense,
}

impl Power {
    pub fn name(self) -> &'static str {
        match self {
            Power::DoubleSnowball => "DoubleSnowball",
            Power::Range => "Range",
            Power::Defense => "Defense",
        }
    }
    pub fn image(self) -> &'static str {
        match self {
            Power::DoubleSnowball => "plugins/powerup/powerup_1.png",
            Power::Range => "plugins/powerup/powerup_3.png",
            Power::Defense => "plugins/powerup/powerup_2.png",
        }
    }
}

/// The parts that change while painting (for undo).
#[derive(Clone, PartialEq)]
pub struct Ground {
    pub ice: Vec<bool>,
    pub height: Vec<u8>,
    pub res: Vec<Res>,
    /// top left tiles
    pub igloos: Vec<(i32, i32)>,
    pub powerups: Vec<(Power, i32, i32)>,
}

pub struct EditMap {
    pub w: i32,
    pub h: i32,
    pub g: Ground,
    pub name: String,
    pub description: String,
    pub victory: Victory,
    pub amount: i32,
    pub workers: u16,
    pub snowballers: u16,
    /// the file name (without .zip) once saved / when opened
    pub filename: Option<String>,
    pub music: Music,
    /// the settings of an opened map (sections the editor doesn't know are
    /// kept)
    ini: Ini,
    /// an opened map's own territories, kept while its igloos don't move
    orig_regions: Option<(Image, Vec<(i32, i32)>)>,
}

pub const MAX_IGLOOS: usize = 12;

/// The music a map plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Music {
    /// the game's usual tune
    Default,
    /// one of the game's tunes (a file in data/music)
    Track(String),
    /// the map's own music (an Ogg Vorbis file kept in the map)
    Custom { name: String, data: Vec<u8> },
}

/// The map's own music inside the map's zip ...
pub const MAP_MUSIC_FILE: &str = "music.ogg";
/// ... and how settings.ini points at it.
pub const MAP_MUSIC_REF: &str = "map:music.ogg";
/// The biggest music file a map takes.
pub const MAX_MUSIC_BYTES: usize = 25 * 1024 * 1024;

impl Music {
    pub fn label(&self) -> String {
        match self {
            Music::Default => "The game's usual music".into(),
            Music::Track(t) => tune_name(t),
            Music::Custom { name, data } => format!("Your music: {} ({:.1} MB)", name, data.len() as f64 / 1_048_576.0),
        }
    }
}

/// "llamas_y_bakunas.ogg" -> "Llamas y bakunas"
pub fn tune_name(file: &str) -> String {
    let stem = file.trim_end_matches(".ogg").trim_end_matches(".mp3").replace('_', " ");
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => stem,
    }
}

/// The game's tunes (data/music), for maps to choose from.
pub fn game_tunes() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("data/music")
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".ogg")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Read an Ogg Vorbis file for a map's own music (checked that the game
/// can play it).
pub fn read_music_file(path: &str) -> Result<Music, String> {
    let p = std::path::Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
    if !name.to_lowercase().ends_with(".ogg") {
        return Err(format!("{} is not an .ogg file (the game plays Ogg Vorbis music)", name));
    }
    let data = std::fs::read(p).map_err(|e| format!("Could not read {}: {}", name, e))?;
    if data.len() > MAX_MUSIC_BYTES {
        return Err(format!("{} is too big ({} MB at most)", name, MAX_MUSIC_BYTES / 1_048_576));
    }
    crate::vorbis::Decoder::new(&data).map_err(|e| format!("{} can't be played: {}", name, e))?;
    Ok(Music::Custom { name, data })
}

impl EditMap {
    pub fn new(size: i32) -> EditMap {
        let n = (size * size) as usize;
        EditMap {
            w: size,
            h: size,
            g: Ground { ice: vec![false; n], height: vec![0; n], res: vec![Res::None; n], igloos: Vec::new(), powerups: Vec::new() },
            name: "My map".into(),
            music: Music::Default,
            description: String::new(),
            victory: Victory::Conquer,
            amount: 30,
            workers: 10,
            snowballers: 12,
            filename: None,
            ini: Ini::new(),
            orig_regions: None,
        }
    }

    pub fn idx(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| (y * self.w + x) as usize)
    }

    /// Open maps/<file>.zip (or the user's own copy).
    pub fn open(file: &str) -> Result<EditMap, String> {
        let z = Zip::open(&crate::settings::map_path(file))?;
        let ini = Ini::parse(&String::from_utf8_lossy(&z.read("settings.ini")?));
        let img = |k: &str| -> Option<Image> { ini.get("map", k).and_then(|f| z.read(f.trim()).ok()).and_then(|b| Image::decode(&b).ok()) };
        let regions = img("regions").ok_or("the map has no regions picture")?;
        let (w, h) = (regions.w as i32, regions.h as i32);
        let mut m = EditMap::new(w.max(h));
        m.w = w;
        m.h = h;
        let n = (w * h) as usize;
        m.g = Ground { ice: vec![false; n], height: vec![0; n], res: vec![Res::None; n], igloos: Vec::new(), powerups: Vec::new() };
        let land = img("land");
        let terrain = img("terrain");
        let res = img("resources");
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let (ux, uy) = (x as usize, y as usize);
                if let Some(l) = &land {
                    if ux < l.w && uy < l.h && l.get(ux, uy) == [0, 255, 255, 255] {
                        m.g.ice[i] = true;
                    }
                }
                if let Some(t) = &terrain {
                    if ux < t.w && uy < t.h {
                        m.g.height[i] = t.get(ux, uy)[0];
                    }
                }
                if let Some(r) = &res {
                    if ux < r.w && uy < r.h {
                        m.g.res[i] = match r.get(ux, uy) {
                            [0, 255, 0, 255] if !m.g.ice[i] => Res::Tree,
                            [0, 0, 255, 255] if m.g.ice[i] => Res::Pinnacle,
                            [0, 255, 255, 255] => Res::Fish,
                            [255, 0, 255, 255] => Res::Crystal,
                            _ => Res::None,
                        };
                    }
                }
            }
        }
        for (_, pos) in ini.items("igloos") {
            let p: Vec<i32> = pos.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if p.len() == 2 {
                m.g.igloos.push((p[0], p[1]));
            }
        }
        for (_, params) in ini.items("powerups") {
            let parts: Vec<&str> = params.split(',').map(|s| s.trim()).collect();
            let kind = match parts.first().copied() {
                Some("DoubleSnowball") => Power::DoubleSnowball,
                Some("Range") => Power::Range,
                Some("Defense") => Power::Defense,
                _ => continue,
            };
            let x = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let y = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            m.g.powerups.push((kind, x, y));
        }
        m.name = ini.get("map", "name").unwrap_or(file).to_string();
        m.description = ini.get("map", "description").unwrap_or("").replace('\n', " ");
        let plugins = ini.get("map", "required_plugins").unwrap_or("");
        m.victory = if plugins.contains("gather_victory") {
            match ini.get("victory", "resource_type").map(|s| s.trim()) {
                Some("fish") => Victory::Fish,
                _ => Victory::Crystal,
            }
        } else {
            Victory::Conquer
        };
        m.amount = ini.get_int("victory", "amount").unwrap_or(30) as i32;
        m.workers = ini.get_int("igloo_settings", "workers").unwrap_or(10) as u16;
        m.snowballers = ini.get_int("igloo_settings", "snowballers").unwrap_or(12) as u16;
        m.music = match ini.get("map", "music").map(|s| s.trim()) {
            None | Some("") => Music::Default,
            Some(MAP_MUSIC_REF) => match z.read(MAP_MUSIC_FILE) {
                Ok(data) => Music::Custom { name: ini.get("map", "music_name").unwrap_or(MAP_MUSIC_FILE).to_string(), data },
                Err(_) => Music::Default,
            },
            Some(t) => Music::Track(t.replace(".mp3", ".ogg")),
        };
        m.filename = Some(file.to_string());
        m.orig_regions = Some((regions, m.g.igloos.clone()));
        m.ini = ini;
        Ok(m)
    }

    /// The igloo covering tile (x, y), if any.
    pub fn igloo_at(&self, x: i32, y: i32) -> Option<usize> {
        self.g.igloos.iter().position(|&(ix, iy)| x >= ix && y >= iy && x < ix + IGLOO_SIZE && y < iy + IGLOO_SIZE)
    }

    /// Room for an igloo with its top left at (x, y): inside the map and
    /// not touching another igloo.
    pub fn igloo_fits(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x + IGLOO_SIZE <= self.w
            && y + IGLOO_SIZE <= self.h
            && self.g.igloos.iter().all(|&(ix, iy)| (ix - x).abs() > IGLOO_SIZE || (iy - y).abs() > IGLOO_SIZE)
    }

    /// Put an igloo there (its tiles become snow without resources).
    pub fn add_igloo(&mut self, x: i32, y: i32) -> Result<(), String> {
        if self.g.igloos.len() >= MAX_IGLOOS {
            return Err(format!("A map can have at most {} igloos", MAX_IGLOOS));
        }
        if !self.igloo_fits(x, y) {
            return Err("No room for an igloo there".into());
        }
        for yy in y..y + IGLOO_SIZE {
            for xx in x..x + IGLOO_SIZE {
                let i = self.idx(xx, yy).unwrap();
                self.g.ice[i] = false;
                self.g.res[i] = Res::None;
            }
        }
        self.g.powerups.retain(|&(_, px, py)| !(px >= x && py >= y && px < x + IGLOO_SIZE && py < y + IGLOO_SIZE));
        self.g.igloos.push((x, y));
        Ok(())
    }

    /// The territories: for every tile, the index of the nearest igloo.
    pub fn territories(&self) -> Vec<u8> {
        let centers: Vec<(f64, f64)> = self.g.igloos.iter().map(|&(x, y)| (x as f64 + 2.0, y as f64 + 2.0)).collect();
        let mut out = vec![0u8; (self.w * self.h) as usize];
        if centers.is_empty() {
            return out;
        }
        for y in 0..self.h {
            for x in 0..self.w {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let mut best = (f64::MAX, 0);
                for (i, &(cx, cy)) in centers.iter().enumerate() {
                    let d = (px - cx).powi(2) + (py - cy).powi(2);
                    if d < best.0 {
                        best = (d, i);
                    }
                }
                out[(y * self.w + x) as usize] = best.1 as u8;
            }
        }
        out
    }

    /// What is missing before the map can be played.
    pub fn problems(&self) -> Option<String> {
        if self.g.igloos.len() < 2 {
            return Some("Place at least 2 igloos (one per player)".into());
        }
        let need = match self.victory {
            Victory::Crystal => Some((Res::Crystal, "crystals")),
            Victory::Fish => Some((Res::Fish, "fish holes")),
            Victory::Conquer => None,
        };
        if let Some((r, what)) = need {
            if !self.g.res.contains(&r) {
                return Some(format!("The victory needs {}: place some on the map", what));
            }
        }
        if self.name.trim().is_empty() {
            return Some("Give the map a name (Map settings)".into());
        }
        None
    }

    /// The objectives line the map list shows.
    pub fn objectives(&self) -> String {
        match self.victory {
            Victory::Conquer => "Take over every igloo.".into(),
            Victory::Crystal => format!("Be the first to gather {} loads of crystal into your igloo.", self.amount),
            Victory::Fish => format!("Be the first to gather {} loads of fish into your igloo.", self.amount),
        }
    }

    /// A file name from the map's name: lower case letters and digits.
    pub fn suggested_filename(&self) -> String {
        let s: String = self.name.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).take(30).collect();
        if s.is_empty() {
            "mymap".into()
        } else {
            s
        }
    }

    /// The map's pictures, settings and icon, as a zip.
    pub fn to_zip(&self, icon: &Image) -> Vec<u8> {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut land = Image::new(w, h);
        let mut terrain = Image::new(w, h);
        let mut res = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                land.set(x, y, if self.g.ice[i] { [0, 255, 255, 255] } else { [255, 255, 255, 255] });
                let v = self.g.height[i];
                terrain.set(x, y, [v, v, v, 255]);
                res.set(
                    x,
                    y,
                    match self.g.res[i] {
                        Res::None => [0, 0, 0, 0],
                        Res::Tree => [0, 255, 0, 255],
                        Res::Pinnacle => [0, 0, 255, 255],
                        Res::Fish => [0, 255, 255, 255],
                        Res::Crystal => [255, 0, 255, 255],
                    },
                );
            }
        }
        // the territories: the opened map's own while the igloos are the
        // same, else one per igloo
        let regions = match &self.orig_regions {
            Some((img, igloos)) if *igloos == self.g.igloos && img.w == w && img.h == h => img.clone(),
            _ => {
                let t = self.territories();
                let mut img = Image::new(w, h);
                for (i, &r) in t.iter().enumerate() {
                    let r = r as u32;
                    let c = [(40 + r * 17 % 200) as u8, (230 - r * 13 % 180) as u8, (r * 97 % 256) as u8, 255];
                    img.set(i % w, i / w, c);
                }
                img
            }
        };

        let mut ini = self.ini.clone();
        ini.set("map", "name", self.name.trim());
        ini.set("map", "regions", "regions.png");
        ini.set("map", "resources", "resources.png");
        ini.set("map", "land", "land.png");
        ini.set("map", "terrain", "terrain.png");
        ini.set("map", "description", self.description.trim());
        ini.set("map", "objectives", &self.objectives());
        let mut plugins = vec![];
        if self.g.powerups.iter().any(|p| p.0 == Power::Range) {
            // older games don't know the range power-up
            plugins.push("powerup-v1.3");
        } else if !self.g.powerups.is_empty() {
            plugins.push("powerup-v1.1");
        }
        if self.victory != Victory::Conquer {
            plugins.push("gather_victory-v1");
        }
        plugins.push("conquer_victory-v1");
        ini.set("map", "required_plugins", &plugins.join(", "));
        ini.set("victory", "to_win", "all");
        ini.set("victory", "to_lose", "0");
        match self.victory {
            Victory::Conquer => {}
            Victory::Crystal | Victory::Fish => {
                ini.set("victory", "resource_type", if self.victory == Victory::Fish { "fish" } else { "crystal" });
                ini.set("victory", "amount", &self.amount.to_string());
            }
        }
        // the AI gathers what the map is about, and guards the middle
        let gather = match self.victory {
            Victory::Fish => "fish",
            Victory::Crystal => "crystal",
            Victory::Conquer if self.g.res.contains(&Res::Crystal) => "crystal",
            Victory::Conquer => "fish",
        };
        if ini.get("ai", "gather").is_none() || self.victory != Victory::Conquer {
            ini.set("ai", "gather", gather);
        }
        if ini.get("ai", "guard").is_none() {
            ini.set("ai", "guard", &format!("{},{}", self.w / 2, self.h / 2));
        }
        if ini.get("ai", "gather_force").is_none() {
            ini.set("ai", "gather_force", "80");
        }
        ini.set("igloo_settings", "workers", &self.workers.to_string());
        ini.set("igloo_settings", "snowballers", &self.snowballers.to_string());
        ini.remove_section("igloos");
        for (i, (x, y)) in self.g.igloos.iter().enumerate() {
            ini.set("igloos", &(i + 1).to_string(), &format!("{},{}", x, y));
        }
        ini.remove_section("powerups");
        for (i, (p, x, y)) in self.g.powerups.iter().enumerate() {
            ini.set("powerups", &(i + 1).to_string(), &format!("{},{},{}", p.name(), x, y));
        }
        ini.remove("map", "music_name");
        match &self.music {
            Music::Default => ini.remove("map", "music"),
            Music::Track(t) => ini.set("map", "music", t),
            Music::Custom { name, .. } => {
                ini.set("map", "music", MAP_MUSIC_REF);
                ini.set("map", "music_name", name);
            }
        }
        let mut files = vec![
            ("settings.ini", ini.write().into_bytes()),
            ("regions.png", png::encode(&regions)),
            ("resources.png", png::encode(&res)),
            ("land.png", png::encode(&land)),
            ("terrain.png", png::encode(&terrain)),
            ("icon.png", png::encode(icon)),
        ];
        if let Music::Custom { data, .. } = &self.music {
            files.push((MAP_MUSIC_FILE, data.clone()));
        }
        zip::write_stored(&files)
    }

    /// Save into the user's maps folder; returns the file name.
    pub fn save(&mut self, icon: &Image) -> Result<String, String> {
        let dir = crate::settings::user_maps_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make the folder {}: {}", dir, e))?;
        let file = match &self.filename {
            Some(f) => f.clone(),
            None => {
                // a new map must not replace another one
                let base = self.suggested_filename();
                let mut f = base.clone();
                let mut n = 2;
                while std::path::Path::new(&crate::settings::map_path(&f)).exists() {
                    f = format!("{}{}", base, n);
                    n += 1;
                }
                f
            }
        };
        let path = std::path::Path::new(&dir).join(format!("{}.zip", file));
        std::fs::write(&path, self.to_zip(icon)).map_err(|e| format!("Could not save {}: {}", path.display(), e))?;
        self.filename = Some(file.clone());
        Ok(file)
    }
}

/// Bring a map (a Snowballz map .zip, e.g. downloaded or from another
/// computer) into the player's maps folder.  It is checked first (it must
/// open like a game map), and gets a file name of its own: an imported map
/// never replaces one that is there already.  Returns (file, map name).
pub fn import_map(path: &str) -> Result<(String, String), String> {
    let p = std::path::Path::new(path);
    let shown = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
    if !shown.to_lowercase().ends_with(".zip") {
        return Err(format!("{} is not a map (Snowballz maps are .zip files)", shown));
    }
    let data = std::fs::read(p).map_err(|e| format!("Could not read {}: {}", shown, e))?;
    let z = Zip::from_bytes(data.clone()).map_err(|e| format!("{} is not a map: {}", shown, e))?;
    let ini = Ini::parse(&String::from_utf8_lossy(&z.read("settings.ini").map_err(|_| format!("{} is not a Snowballz map (no settings.ini in it)", shown))?));
    let regions = ini.get("map", "regions").ok_or_else(|| format!("{} is not a Snowballz map (no regions picture named)", shown))?;
    let img = z.read(regions.trim()).map_err(|_| format!("{} is broken (its regions picture is missing)", shown))?;
    Image::decode(&img).map_err(|e| format!("{} is broken (its regions picture: {})", shown, e))?;
    let stem: String = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    let base = if stem.is_empty() { "imported".to_string() } else { stem };
    let mut file = base.clone();
    let mut n = 2;
    while std::path::Path::new(&crate::settings::user_maps_dir()).join(format!("{}.zip", file)).exists() || std::path::Path::new(&format!("maps/{}.zip", file)).exists() {
        file = format!("{}{}", base, n);
        n += 1;
    }
    let dir = crate::settings::user_maps_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make the folder {}: {}", dir, e))?;
    let dest = std::path::Path::new(&dir).join(format!("{}.zip", file));
    std::fs::write(&dest, data).map_err(|e| format!("Could not save {}: {}", dest.display(), e))?;
    let name = ini.get("map", "name").unwrap_or(&file).to_string();
    Ok((file, name))
}

/// Ask the computer's own "open file" window for a map (.zip); `Ok(None)`:
/// cancelled, `Err`: there is no such window to be had (then maps can be
/// dragged onto the game's window instead).
pub fn pick_map_file() -> Result<Option<String>, String> {
    use std::process::Command;
    let tries: Vec<(&str, Vec<&str>)> = if cfg!(target_os = "windows") {
        vec![(
            "powershell",
            vec!["-NoProfile", "-STA", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Title = 'Import a Snowballz map'; $f.Filter = 'Snowballz maps (*.zip)|*.zip'; if ($f.ShowDialog() -eq 'OK') { $f.FileName }"],
        )]
    } else if cfg!(target_os = "macos") {
        vec![("osascript", vec!["-e", "POSIX path of (choose file of type {\"zip\", \"public.zip-archive\"} with prompt \"Import a Snowballz map\")"])]
    } else {
        vec![
            ("zenity", vec!["--file-selection", "--title=Import a Snowballz map", "--file-filter=Snowballz maps (zip) | *.zip *.ZIP"]),
            ("kdialog", vec!["--title", "Import a Snowballz map", "--getopenfilename", ".", "*.zip"]),
            ("yad", vec!["--file", "--title=Import a Snowballz map", "--file-filter=*.zip"]),
        ]
    };
    for (cmd, args) in tries {
        match Command::new(cmd).args(&args).output() {
            Ok(out) => {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                return Ok((out.status.success() && !path.is_empty()).then_some(path));
            }
            Err(_) => continue, // (not installed: the next)
        }
    }
    Err("No file chooser found: drag the map's .zip onto this window instead".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (tests that point SNOWBALLZ_MAPS somewhere take turns)
    static MAPS_DIR: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn maps_import_from_the_computer_without_replacing_any() {
        let _turn = MAPS_DIR.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("snowballz-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SNOWBALLZ_MAPS", dir.join("maps"));
        let src = dir.join("WhiteForest.zip");
        std::fs::copy("maps/whiteforest.zip", &src).unwrap();
        // (the game has a whiteforest of its own: this one gets a new name)
        let (f1, name) = import_map(src.to_str().unwrap()).unwrap();
        assert_eq!((f1.as_str(), name.as_str()), ("whiteforest2", "White Forest"));
        let (f2, _) = import_map(src.to_str().unwrap()).unwrap();
        assert_eq!(f2, "whiteforest3");
        assert!(EditMap::open(&f2).is_ok());
        // not maps
        let bad = dir.join("notes.zip");
        std::fs::write(&bad, b"hello").unwrap();
        assert!(import_map(bad.to_str().unwrap()).is_err());
        let other = dir.join("tune.ogg");
        std::fs::write(&other, b"x").unwrap();
        assert!(import_map(other.to_str().unwrap()).unwrap_err().contains("not a map"));
        let empty = dir.join("empty.zip");
        std::fs::write(&empty, zip::write_stored(&[("readme.txt", b"hi".to_vec())])).unwrap();
        assert!(import_map(empty.to_str().unwrap()).unwrap_err().contains("no settings.ini"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_map_saves_and_loads_like_a_game_map() {
        let _turn = MAPS_DIR.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("snowballz-maps-{}", std::process::id()));
        std::env::set_var("SNOWBALLZ_MAPS", &dir);
        let mut m = EditMap::new(40);
        assert!(m.problems().is_some());
        m.add_igloo(3, 3).unwrap();
        assert!(m.add_igloo(5, 5).is_err()); // too close
        m.add_igloo(30, 30).unwrap();
        let i = m.idx(20, 5).unwrap();
        m.g.res[i] = Res::Crystal;
        let (a, b) = (m.idx(10, 10).unwrap(), m.idx(11, 10).unwrap());
        m.g.ice[a] = true;
        m.g.height[b] = 200;
        m.g.powerups.push((Power::Range, 20, 20));
        m.name = "Test Island!".into();
        m.victory = Victory::Crystal;
        m.amount = 25;
        assert_eq!(m.problems(), None);
        let file = m.save(&Image::new(8, 8)).unwrap();
        assert_eq!(file, "testisland");
        let back = EditMap::open(&file).unwrap();
        assert_eq!(back.g.igloos, vec![(3, 3), (30, 30)]);
        assert_eq!(back.g.res[i], Res::Crystal);
        assert!(back.g.ice[m.idx(10, 10).unwrap()]);
        assert_eq!(back.g.height[m.idx(11, 10).unwrap()], 200);
        assert_eq!(back.g.powerups, vec![(Power::Range, 20, 20)]);
        assert_eq!(back.victory, Victory::Crystal);
        assert_eq!(back.amount, 25);
        assert_eq!(back.name, "Test Island!");
        assert_eq!(back.music, Music::Default);
        // its own music goes into the map and comes back out
        let mut back = back;
        assert!(read_music_file("README.md").is_err());
        back.music = read_music_file("data/music/llamas_y_bakunas.ogg").unwrap();
        let file2 = back.save(&Image::new(8, 8)).unwrap();
        let again = EditMap::open(&file2).unwrap();
        assert_eq!(again.music, back.music);
        assert_eq!(again.music.label().split(" (").next(), Some("Your music: llamas_y_bakunas.ogg"));
        let mut t = again;
        t.music = Music::Track("snowballz2.ogg".into());
        let f3 = t.save(&Image::new(8, 8)).unwrap();
        assert_eq!(EditMap::open(&f3).unwrap().music, Music::Track("snowballz2.ogg".into()));
        // the territories: two, split between the igloos
        let t = back.territories();
        assert_eq!(t[back.idx(0, 0).unwrap()], 0);
        assert_eq!(t[back.idx(39, 39).unwrap()], 1);
        let _ = std::fs::remove_dir_all(&dir);

        // (in the same test: SNOWBALLZ_MAPS is shared by all tests)
        std::env::set_var("SNOWBALLZ_MAPS", "/nonexistent-snowballz-maps");
        for m in ["crystalcrises", "iceland", "smallcrystalrush", "whiteforest"] {
            let e = EditMap::open(m).unwrap_or_else(|e| panic!("{}: {}", m, e));
            assert!(e.g.igloos.len() >= 2, "{}", m);
            assert!(e.g.res.iter().any(|&r| r != Res::None), "{}", m);
        }
    }
}
