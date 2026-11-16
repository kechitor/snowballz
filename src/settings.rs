//! Settings, kept in ~/.snowballz.ini (the same file the Python version
//! used), or %APPDATA%\SnowballZ.ini on Windows.

use crate::ini::Ini;
use std::sync::Mutex;

static CONFIG: Mutex<Option<Ini>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Ini) -> R) -> R {
    let mut g = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Some(Ini::new());
    }
    f(g.as_mut().unwrap())
}

pub fn config_path() -> String {
    if let Ok(p) = std::env::var("SNOWBALLZ_CONFIG") {
        return p;
    }
    if let Ok(home) = std::env::var("HOME") {
        return format!("{}/.snowballz.ini", home);
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return format!("{}\\SnowballZ.ini", appdata);
    }
    "snowballz.ini".to_string()
}

pub fn load() {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => with(|c| *c = Ini::parse(&text)),
        Err(_) => {
            with(|c| {
                *c = Ini::new();
                c.add_section("display");
                c.add_section("player");
                c.set("display", "snowballdetail", "3");
                c.set("player", "name", "Snowballer");
            });
            // the default resolution is the biggest one
            let n = res_available().len();
            with(|c| c.set("display", "resolution", &n.saturating_sub(1).to_string()));
            save();
        }
    }
    with(|c| {
        c.add_section("display");
        c.add_section("player");
        if c.get("player", "name").is_none() {
            c.set("player", "name", "Snowballer");
        }
    });
}

pub fn save() {
    let text = with(|c| c.write());
    if let Err(e) = std::fs::write(config_path(), text) {
        eprintln!("Could not save the settings ({}): {}", config_path(), e);
    }
}

/// A display option ("1" if it was never set).
pub fn get_option(option: &str) -> String {
    let v = with(|c| c.get("display", option).map(|s| s.to_string()));
    match v {
        Some(v) => v,
        None => {
            with(|c| c.set("display", option, "1"));
            save();
            "1".into()
        }
    }
}

pub fn option_int(option: &str) -> i32 {
    get_option(option).trim().parse().unwrap_or(1)
}

pub fn set_option(option: &str, value: &str) {
    with(|c| c.set("display", option, value));
    save();
}

/// Move `option` to the next of `options` (wrapping).
pub fn toggle(option: &str, options: &[&str]) -> String {
    let cur = get_option(option);
    let i = options.iter().position(|o| *o == cur).unwrap_or(0);
    let next = options[(i + 1) % options.len()].to_string();
    set_option(option, &next);
    next
}

/// How many snowballers the player chose for a map (None: the map's mix).
pub fn loadout(map: &str) -> Option<u16> {
    with(|c| c.get("loadout", map).and_then(|v| v.trim().parse().ok()))
}

pub fn set_loadout(map: &str, snowballers: u16) {
    with(|c| c.set("loadout", map, &snowballers.to_string()));
    save();
}

/// A sound level in percent: "volume" (everything) or "music".  100 if
/// it was never set.
pub fn sound_level(key: &str) -> u8 {
    with(|c| c.get("sound", key).and_then(|v| v.trim().parse::<u32>().ok()).map(|v| v.min(100) as u8).unwrap_or(100))
}

pub fn set_sound_level(key: &str, percent: u8) {
    with(|c| c.set("sound", key, &percent.min(100).to_string()));
    save();
}

/// The address last typed in "Join a game" (the Python version kept it
/// as [network] host too).
pub fn last_host() -> String {
    with(|c| c.get("network", "host").unwrap_or("").to_string())
}

pub fn set_last_host(host: &str) {
    with(|c| c.set("network", "host", host.trim()));
    save();
}

/// The relay server for game codes set in the settings file ([network]
/// relay), if any.
pub fn relay() -> Option<String> {
    with(|c| c.get("network", "relay").map(|v| v.trim().to_string())).filter(|v| !v.is_empty())
}

/// Open the port on the router automatically when hosting ([network]
/// upnp = 0 turns it off; so does SNOWBALLZ_NO_UPNP).
pub fn upnp_enabled() -> bool {
    std::env::var("SNOWBALLZ_NO_UPNP").is_err() && with(|c| c.get("network", "upnp").map(|v| v.trim() != "0").unwrap_or(true))
}

pub fn player_name() -> String {
    with(|c| c.get("player", "name").unwrap_or("Snowballer").to_string())
}

pub fn set_player_name(name: &str) {
    with(|c| c.set("player", "name", name));
    save();
}

/// Resolutions to choose from, smallest first, at least 640x480.
pub fn res_available() -> Vec<(i32, i32)> {
    let modes = if crate::sdl::is_loaded() { crate::sdl::display_modes() } else { Vec::new() };
    let mut out: Vec<(i32, i32)> = modes.into_iter().filter(|r| r.0 >= 640 && r.1 >= 480).collect();
    if out.is_empty() {
        // no (usable) display modes reported, e.g. a headless video driver
        out = vec![(1920, 1080), (1366, 768), (1280, 720), (1024, 768), (800, 600), (640, 480)];
    }
    out.sort();
    out.dedup();
    out
}

pub fn get_res() -> (i32, i32) {
    let avail = res_available();
    let v = with(|c| c.get("display", "resolution").map(|s| s.to_string()));
    let i = match v {
        Some(v) => v.trim().parse::<usize>().unwrap_or(0),
        None => {
            set_option("resolution", "0");
            0
        }
    };
    avail.get(i).copied().unwrap_or(*avail.last().unwrap_or(&(1024, 768)))
}

/// Frame rate limit (0 = unlimited): 1=60, 2=35 (the original), 3=none.
pub fn fps_cap() -> u32 {
    match get_option("fpscap").as_str() {
        "2" => 35,
        "3" => 0,
        _ => 60,
    }
}

/// Initial zoom of the game view: 1=Auto, 2=1x, 3=1.5x, 4=2x.
pub fn game_zoom(screen_height: i32) -> f64 {
    match get_option("gamescale").as_str() {
        "2" => 1.0,
        "3" => 1.5,
        "4" => 2.0,
        _ => ((screen_height as f64 / 768.0 * 4.0).round() / 4.0).clamp(1.0, 3.0),
    }
}
