//! Snowballz - a snowball fight RTS with penguins.  Rust port of the
//! Python 3 / pygame version (itself a port of Snowballz 0.9.5.1).
//!
//!     snowballz                       the menu
//!     snowballz --single [map]        a single player game, no menu
//!     snowballz --server [map]        host an online game, no menu
//!     snowballz --client <address>    join an online game (address or game code), no menu

// Release builds on Windows are a GUI program (no console window).
#![cfg_attr(all(windows, not(debug_assertions), not(test)), windows_subsystem = "windows")]

/// println! / eprintln! that never panic (a closed terminal or pipe must
/// not crash the game).
macro_rules! println {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}
macro_rules! eprintln {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}
macro_rules! eprint {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = write!(std::io::stderr(), $($t)*);
    }};
}

mod ai;
mod audio;
mod backgrounds;
mod clock;
mod crashlog;
mod ctrl;
mod display;
mod effects;
mod game;
mod gfx;
mod hud;
mod igloo;
mod image;
mod inflate;
mod ini;
mod jpeg;
mod menu;
mod net;
mod online;
mod plugins;
mod png;
mod relay;
mod relay_common;
mod rng;
mod sdl;
mod settings;
mod snowball;
mod terrain;
mod ttf;
mod ui;
mod unit;
mod upnp;
mod vorbis;
mod world;
mod zip;

use menu::GameType;

/// Make the folder with data/ and maps/ the current directory.
fn find_root() {
    let ok = |p: &std::path::Path| p.join("data").join("walk.png").exists() && p.join("maps").exists();
    if ok(std::path::Path::new(".")) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        // a macOS app keeps its files in Contents/Resources
        if let Some(res) = exe.parent().map(|p| p.join("../Resources")) {
            if ok(&res) {
                let _ = std::env::set_current_dir(&res);
                return;
            }
        }
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            if ok(&d) {
                let _ = std::env::set_current_dir(&d);
                return;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    eprintln!("Could not find the game's data folder (data/ and maps/).");
}

/// The game chosen on the command line, if any.
fn command_line_game(args: &[String]) -> Option<(String, GameType)> {
    let pos = |flag: &str| args.iter().position(|a| a == flag);
    let map_after = |i: usize| {
        args.get(i + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "whiteforest".to_string())
    };
    if let Some(i) = pos("--single") {
        return Some((map_after(i), GameType::Single));
    }
    if let Some(i) = pos("--server") {
        return Some((map_after(i), GameType::Server));
    }
    if let Some(i) = pos("--client") {
        let host = args.get(i + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "localhost".into());
        return Some((String::new(), GameType::Client(host)));
    }
    None
}

/// Leave the program (closing the port on the router if a game is hosted).
pub fn quit(code: i32) -> ! {
    online::close_all();
    std::process::exit(code)
}

fn menu_music() {
    audio::play(&["main_menu.ogg".to_string()], -1);
}

fn main() {
    crashlog::install();
    find_root();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: snowballz [--single [map] | --server [map] | --client <game code | address[:port]>]");
        return;
    }
    if let Some(t) = std::env::var("SNOWBALLZ_TIMESCALE").ok().and_then(|v| v.parse().ok()) {
        clock::set_timescale(t);
    }
    if let Err(e) = sdl::load() {
        eprintln!("{}", e);
        sdl::native_error("Snowballz", &e);
        std::process::exit(1);
    }
    unsafe {
        if (sdl::api().SDL_Init)(sdl::INIT_VIDEO | sdl::INIT_TIMER | sdl::INIT_EVENTS) != 0 {
            let msg = format!("Could not start SDL: {}", sdl::error());
            eprintln!("{}", msg);
            sdl::message_box("Snowballz", &msg);
            std::process::exit(1);
        }
        (sdl::api().SDL_StartTextInput)();
    }
    settings::load();
    audio::set_levels(settings::sound_level("volume"), settings::sound_level("music"));
    let (w, h) = settings::get_res();
    if let Err(e) = gfx::init(w, h, settings::get_option("fullscreen") == "1", "Snowballz") {
        eprintln!("{}", e);
        sdl::message_box("Snowballz", &e);
        std::process::exit(1);
    }
    gfx::set_smooth(settings::get_option("smooth") == "1");
    println!("{}", crashlog::versions());

    let direct = command_line_game(&args);
    if let Some((map, kind)) = direct {
        if let Err(e) = game::play(kind, &map) {
            eprintln!("{}", e);
            std::process::exit(1);
        }
        return;
    }

    let mut m = menu::Menu::new();
    if let Ok(dir) = std::env::var("SNOWBALLZ_MENU_SHOTS") {
        // tests: a screenshot of every menu page
        for page in [0usize, 1, 2, 3, 4, 5] {
            m.press(ui::Action::Page(page));
            m.draw();
            gfx::screenshot(&format!("{}/menu{}.png", dir, page)).unwrap();
            gfx::present();
        }
        return;
    }
    loop {
        sdl::show_cursor(true);
        menu_music();
        let (map, kind) = m.run();
        audio::fade_out(1500);
        match game::play(kind, &map) {
            Ok(()) => m.hide_error(),
            Err(e) => {
                println!("{}", e);
                m.show_error(&e);
            }
        }
        // back to the menu: page 0, fresh view
        m.press(ui::Action::Page(0));
    }
}
