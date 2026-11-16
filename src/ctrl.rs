//! Input in the game (ctrl.py): selecting, giving orders, scrolling,
//! zooming, groups, chat.

use crate::clock;
use crate::game::Session;
use crate::gfx;
use crate::hud::{self, PanelClick};
use crate::menu;
use crate::net::Msg;
use crate::sdl;
use crate::settings;
use crate::unit;
use crate::world::*;

pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 3.0;

/// The smallest zoom that keeps the view inside the map.
pub fn min_zoom(w: &World) -> f64 {
    let needed = (w.view.w as f64 / (w.size.0 * 32) as f64).max(w.view.h as f64 / (w.size.1 * 32) as f64);
    MIN_ZOOM.max((needed * 10000.0).ceil() / 10000.0)
}

pub fn bind_screen(w: &mut World) {
    if w.view.zoom < min_zoom(w) {
        w.view.zoom = min_zoom(w);
    }
    let maxx = (w.size.0 * 32 - w.view.scale_w()) as f64;
    let maxy = (w.size.1 * 32 - w.view.scale_h()) as f64;
    w.view.x = w.view.x.min(maxx).max(0.0);
    w.view.y = w.view.y.min(maxy).max(0.0);
}

/// Centre the view on world position (x, y).
pub fn look_at(w: &mut World, x: f64, y: f64) {
    w.view.x = x - (w.view.scale_w() / 2) as f64;
    w.view.y = y - (w.view.scale_h() / 2) as f64;
    bind_screen(w);
}

pub fn change_zoom(w: &mut World, amount: f64, center: Option<(f64, f64)>) {
    let (cx, cy) = center.unwrap_or((w.view.x + w.view.scale_w() as f64 / 2.0, w.view.y + w.view.scale_h() as f64 / 2.0));
    w.view.zoom = min_zoom(w).max(MAX_ZOOM.min(w.view.zoom + amount));
    look_at(w, cx, cy);
}

pub fn look_at_home(w: &mut World) {
    if let Some(b) = w.buildings.iter().find(|b| b.player.is_some() && b.player == w.me) {
        let (x, y) = ((b.x * 32) as f64, (b.y * 32) as f64);
        look_at(w, x, y);
    }
}

fn scaled_mouse(w: &World) -> (f64, f64) {
    let (x, y) = gfx::mouse_pos();
    (x as f64 / w.view.zoom, y as f64 / w.view.zoom)
}

fn end_select(w: &mut World) {
    let mods = sdl::mod_state();
    let shift = mods & sdl::KMOD_SHIFT != 0;
    let ctrl = !shift && mods & sdl::KMOD_CTRL != 0;
    if !(shift || ctrl) {
        for &u in &w.selected {
            w.units[u].selected = false;
        }
        w.selected.clear();
    }
    let Some(start) = w.select_box_start else { return };
    let end = w.select_box_end.unwrap_or(start);
    let (sx, ex) = (start.0.min(end.0), start.0.max(end.0));
    let (sy, ey) = (start.1.min(end.1), start.1.max(end.1));
    let t = gfx::time();
    for ui in 0..w.units.len() {
        let (cx, cy) = w.units[ui].sprite_xy(t);
        if cx + 32.0 > sx && cx < ex && cy + 32.0 > sy && cy < ey && w.me.is_some() && w.igloo_player(ui) == w.me {
            if ctrl && w.selected.contains(&ui) {
                w.selected.remove(&ui);
                w.units[ui].selected = false;
            } else {
                w.selected.insert(ui);
                w.units[ui].selected = true;
            }
        }
    }
    w.select_box_start = None;
    w.select_box_end = None;
}

/// Send the selected penguins somewhere (right click; middle click runs).
fn set_job(w: &mut World, pos: (f64, f64), run_mode: bool) {
    if w.selected.is_empty() {
        return;
    }
    let (x, y) = pos;
    let tile = w.canvas_to_map(x, y);
    let sel: Vec<usize> = w.selected.iter().copied().collect();
    if w.is_server() {
        for &u in &sel {
            unit::set_job(w, u, tile, run_mode, sel.len());
        }
    } else {
        let ids = sel.iter().map(|&u| u as u16).collect();
        w.send(Msg::SetJob { pos: (tile.0 as u16, tile.1 as u16), run_mode, unit_ids: ids });
    }
    let worker = sel.iter().any(|&u| w.units[u].kind == UnitKind::Worker);
    let mx = x + w.view.x - 16.0;
    let my = y + w.view.y - 23.0;
    let color = w.me.and_then(|p| w.players.get(&p)).map(|p| p.color).unwrap_or([255; 3]);
    let now = w.now();
    let t = w.tile(tile.0, tile.1).unwrap();
    let gatherable = t.resource.map(|r| w.resources[r].kind.gatherable()).unwrap_or(false);
    if run_mode || t.building.is_some() {
        w.imprints.run_moves.add_rgb(mx, my, color, now);
    } else if gatherable && worker {
        w.imprints.gather_moves.add_rgb(mx, my, color, now);
    } else {
        w.imprints.moves.add_rgb(mx, my, color, now);
    }
}

pub fn disban_units(w: &mut World) {
    let sel: Vec<usize> = w.selected.iter().copied().collect();
    if !w.is_server() {
        let ids = sel.iter().map(|&u| u as u16).collect();
        w.send(Msg::DisbanUnits { unit_ids: ids });
    }
    for u in sel {
        unit::disban(w, u);
    }
    w.selected.clear();
}

fn scroll_screen(w: &mut World, pos: (i32, i32), dt: u32) {
    let (x, y) = pos;
    let mut speed = dt as f64 / 50.0 * 32.0;
    if w.view.zoom <= 1.0 {
        speed *= 2.0 - w.view.zoom;
    } else {
        speed /= w.view.zoom;
    }
    if x < 10 {
        w.view.x -= speed;
    } else if x > w.view.w - 10 {
        w.view.x += speed;
    }
    if y < 10 {
        w.view.y -= speed;
    } else if y > w.view.h - 10 {
        w.view.y += speed;
    }
    if !w.chat.chatting {
        if sdl::key_pressed(sdl::SCAN_RIGHT) || sdl::key_pressed(sdl::SCAN_D) {
            w.view.x += speed;
        }
        if sdl::key_pressed(sdl::SCAN_LEFT) || sdl::key_pressed(sdl::SCAN_A) {
            w.view.x -= speed;
        }
        if sdl::key_pressed(sdl::SCAN_UP) || sdl::key_pressed(sdl::SCAN_W) {
            w.view.y -= speed;
        }
        if sdl::key_pressed(sdl::SCAN_DOWN) || sdl::key_pressed(sdl::SCAN_S) {
            w.view.y += speed;
        }
    }
    bind_screen(w);
}

fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else { return String::new() };
    let mut p: Vec<char> = first.chars().collect();
    for s in &items[1..] {
        let n = p.iter().zip(s.chars()).take_while(|(a, b)| **a == *b).count();
        p.truncate(n);
    }
    p.into_iter().collect()
}

/// Ctrl (Command on a Mac) is held.
fn shortcut_held() -> bool {
    sdl::mod_state() & (sdl::KMOD_CTRL | sdl::KMOD_GUI) != 0
}

/// Ctrl+V / Cmd+V / Shift+Insert
fn is_paste(key: i32) -> bool {
    (key == sdl::K_V && shortcut_held()) || (key == sdl::K_INSERT && sdl::mod_state() & sdl::KMOD_SHIFT != 0)
}

/// Ctrl+C / Cmd+C / Ctrl+Insert
fn is_copy(key: i32) -> bool {
    (key == sdl::K_C || key == sdl::K_INSERT) && shortcut_held()
}

const COPIED: &str = "Copied to the clipboard";

/// Add the clipboard's text (its first line) to the chat line.
fn paste_into_chat(w: &mut World) {
    let text = sdl::clipboard();
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    for c in line.chars() {
        if !c.is_control() && w.chat.msg.chars().count() < 200 {
            w.chat.msg.push(c);
        }
    }
}

/// Copy the newest message on the screen (a chat line without the name).
fn copy_last_message(w: &mut World) {
    let text = w.messages.iter().rev().filter(|m| !m.force_close && !m.copy.is_empty()).map(|m| m.copy.clone()).next();
    match text {
        Some(t) if sdl::set_clipboard(&t) => w.notice_copy(&format!("{}: {}", COPIED, t), ""),
        Some(_) => w.notice_copy("Could not copy (no clipboard)", ""),
        None => w.notice_copy("No message to copy", ""),
    }
}

/// A key pressed while typing a chat line.
pub fn handle_chatting(w: &mut World, key: i32) {
    if is_paste(key) {
        paste_into_chat(w);
        return;
    }
    if is_copy(key) || (key == sdl::K_X && shortcut_held()) {
        if w.chat.msg.is_empty() {
            copy_last_message(w);
        } else if sdl::set_clipboard(&w.chat.msg) && key == sdl::K_X {
            w.chat.msg.clear();
        }
        return;
    }
    match key {
        sdl::K_RETURN | sdl::K_KP_ENTER => {
            if !w.chat.msg.is_empty() {
                let msg = w.chat.msg.clone();
                if let Some(me) = w.me {
                    w.player_send_msg(me, &msg);
                }
                if msg.starts_with('>') {
                    w.chat.last_chat_name = msg.split_whitespace().next().unwrap_or("").to_string();
                }
                w.chat.msg.clear();
            }
            w.chat.help.clear();
            w.chat.chatting = false;
        }
        sdl::K_ESCAPE => {
            w.chat.msg.clear();
            w.chat.chatting = false;
            w.chat.help.clear();
        }
        sdl::K_BACKSPACE => {
            w.chat.last_back_press = w.now();
            w.chat.msg.pop();
        }
        sdl::K_TAB => {
            // tab completion
            let cur = w.chat.msg.clone();
            if cur.is_empty() || cur.split_whitespace().count() > 1 {
                return;
            }
            let mut options: Vec<String> = Vec::new();
            if cur.starts_with('/') {
                options = ["/say"].iter().filter(|o| o.contains(&cur)).map(|s| s.to_string()).collect();
            }
            if cur.starts_with('>') {
                options = w
                    .players
                    .values()
                    .filter(|p| Some(p.id) != w.me)
                    .map(|p| format!(">{}", p.name))
                    .filter(|o| o.contains(&cur))
                    .collect();
            }
            if options.len() == 1 {
                w.chat.msg = format!("{} ", options[0]);
                w.chat.help.clear();
            } else if options.len() > 1 {
                w.chat.msg = common_prefix(&options);
                w.chat.help = options.join("  ");
            }
        }
        _ => {}
    }
}

/// Start typing a chat line (the key's own character is not typed).
fn start_chat(w: &mut World, text: String) {
    w.chat.chatting = true;
    w.chat.msg = text;
    w.chat.swallow_text = true;
}

/// Keys that work in the lobby and in the game (not while chatting).
/// Returns false if the key wasn't one of them.
pub fn chat_keys(w: &mut World, key: i32) -> bool {
    if is_paste(key) {
        // start a chat line with the clipboard's text
        start_chat(w, String::new());
        w.chat.swallow_text = false;
        paste_into_chat(w);
        return true;
    }
    if is_copy(key) {
        copy_last_message(w);
        return true;
    }
    match key {
        sdl::K_C => w.clear_messages(),
        sdl::K_COMMA => {
            if !w.chat.last_chat_name.is_empty() {
                let t = format!("{} ", w.chat.last_chat_name);
                start_chat(w, t);
            }
        }
        sdl::K_PERIOD => {
            if !w.chat.last_received_from.is_empty() {
                let t = format!(">{} ", w.chat.last_received_from);
                start_chat(w, t);
            }
        }
        sdl::K_RETURN | sdl::K_KP_ENTER => start_chat(w, String::new()),
        _ => return false,
    }
    true
}

/// Text typed while chatting.
pub fn chat_text(w: &mut World, text: &str) {
    if w.chat.swallow_text {
        w.chat.swallow_text = false;
        return;
    }
    if w.chat.chatting {
        for c in text.chars() {
            if !c.is_control() && w.chat.msg.chars().count() < 200 {
                w.chat.msg.push(c);
            }
        }
    }
}

/// Hold backspace to delete more.
pub fn chat_backspace_repeat(w: &mut World) {
    if w.chat.chatting && w.now().wrapping_sub(w.chat.last_back_press) > 300 && sdl::key_pressed(sdl::SCAN_BACKSPACE) {
        w.chat.msg.pop();
    }
}

/// Handle input; false = leave the game.
pub fn handle_events(w: &mut World, s: &mut Session, dt: u32) -> bool {
    for e in gfx::events() {
        // the Escape menu takes every event while it is open
        if let Some(mut m) = s.game_menu.take() {
            if e.kind() == sdl::QUIT {
                crate::quit(0);
            }
            match m.handle(&e) {
                None => s.game_menu = Some(m),
                Some(crate::game_menu::Outcome::Resume) => clock::set_paused(false),
                Some(crate::game_menu::Outcome::Quit) => {
                    clock::set_paused(false);
                    s.quit_requested = true;
                    return false;
                }
            }
            // settings the game keeps for itself
            w.footprints = settings::get_option("footprints") != "0";
            let cursor = settings::get_option("cursor") == "1";
            if cursor != s.use_cursor {
                s.use_cursor = cursor;
                sdl::show_cursor(!cursor);
            }
            continue;
        }
        match e.kind() {
            sdl::QUIT => crate::quit(0),
            sdl::MOUSEWHEEL => {
                if !w.chat.chatting {
                    let z = w.view.zoom;
                    change_zoom(w, e.wheel_y() as f64 * 0.1 * z, None);
                }
            }
            sdl::KEYDOWN => {
                w.chat.swallow_text = false;
                if e.key_repeat() {
                    if w.chat.chatting && e.key_sym() == sdl::K_BACKSPACE {
                        // (held backspace is handled below)
                    }
                    continue;
                }
                let key = e.key_sym();
                if w.chat.chatting {
                    handle_chatting(w, key);
                    continue;
                }
                match key {
                    sdl::K_ESCAPE => {
                        let title = if w.spectating {
                            "You were defeated"
                        } else if s.single {
                            "Game paused"
                        } else {
                            "Menu"
                        };
                        let note = if s.hosting_online { Some("You are the host: quitting ends the game for everybody.") } else { None };
                        s.game_menu = Some(crate::game_menu::GameMenu::new(title, note));
                        w.select_box_start = None;
                        w.holding_mouse = false;
                        w.draw_minimap = false;
                        if s.single {
                            clock::set_paused(true);
                        }
                    }
                    sdl::K_F3 => w.show_fps = !w.show_fps,
                    sdl::K_F11 => {
                        menu::toggle_fullscreen();
                    }
                    sdl::K_HOME => look_at_home(w),
                    sdl::K_SPACE => w.draw_minimap = true,
                    sdl::K_O => disban_units(w),
                    k => {
                        chat_keys(w, k);
                    }
                }
                // groups
                if (sdl::K_1..=sdl::K_9).contains(&key) {
                    let n = (key - sdl::K_0) as usize;
                    if !w.selected.is_empty() && e.key_mod() & sdl::KMOD_CTRL != 0 {
                        for u in std::mem::take(&mut w.groups[n]) {
                            w.units[u].group = None;
                        }
                        let sel: Vec<usize> = w.selected.iter().copied().collect();
                        for &u in &sel {
                            if let Some(g) = w.units[u].group {
                                w.groups[g as usize].remove(&u);
                            }
                            w.units[u].group = Some(n as u8);
                        }
                        w.groups[n] = sel.into_iter().collect();
                    } else if !w.groups[n].is_empty() {
                        for &u in &w.selected {
                            w.units[u].selected = false;
                        }
                        w.selected = w.groups[n].clone();
                        let mut last = None;
                        for &u in &w.selected {
                            w.units[u].selected = true;
                            last = Some(u);
                        }
                        if let Some(u) = last {
                            let (x, y) = ((w.units[u].x * 32) as f64, (w.units[u].y * 32) as f64);
                            look_at(w, x, y);
                        }
                    }
                }
            }
            sdl::TEXTINPUT => chat_text(w, &e.text()),
            sdl::KEYUP => {
                if e.key_sym() == sdl::K_SPACE {
                    w.draw_minimap = false;
                }
            }
            sdl::MOUSEBUTTONDOWN => {
                let pos = gfx::mouse_pos();
                if !hud::in_panel(w, pos) {
                    let p = scaled_mouse(w);
                    if !w.draw_minimap {
                        match e.button() {
                            1 => w.select_box_start = Some((w.view.x + p.0, w.view.y + p.1)),
                            2 => set_job(w, p, true),
                            3 => set_job(w, p, false),
                            _ => {}
                        }
                    }
                    w.holding_mouse = true;
                }
            }
            sdl::MOUSEBUTTONUP => {
                let pos = gfx::mouse_pos();
                if hud::in_panel(w, pos) && e.button() == 1 {
                    match hud::panel_click(w, pos) {
                        PanelClick::Snowballers => hud::keep_kind(w, UnitKind::Snowballer),
                        PanelClick::Workers => hud::keep_kind(w, UnitKind::Worker),
                        PanelClick::Disband => disban_units(w),
                        PanelClick::Nothing => {}
                    }
                }
                if w.select_box_start.is_some() {
                    end_select(w);
                }
                w.holding_mouse = false;
            }
            _ => {}
        }
    }
    if s.game_menu.is_some() {
        return true; // no scrolling behind the menu
    }
    chat_backspace_repeat(w);

    let page_up = sdl::key_pressed(sdl::SCAN_PAGEUP);
    let page_down = sdl::key_pressed(sdl::SCAN_PAGEDOWN);
    if page_up || page_down {
        let c = (w.view.x + w.view.scale_w() as f64 / 2.0, w.view.y + w.view.scale_h() as f64 / 2.0);
        change_zoom(w, if page_up { 0.02 } else { -0.02 }, Some(c));
    }
    scroll_screen(w, gfx::mouse_pos(), dt);
    if w.draw_minimap && w.holding_mouse {
        let (mx, my) = gfx::mouse_pos();
        let (x, y) = s.minimap.pos_to_mappos(w, mx as f64, my as f64);
        w.view.x = x;
        w.view.y = y;
        bind_screen(w);
    }
    if w.select_box_start.is_some() {
        let (x, y) = scaled_mouse(w);
        w.select_box_end = Some((x + w.view.x, y + w.view.y));
    }
    true
}
