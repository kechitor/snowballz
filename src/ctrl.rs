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
use crate::wall::{self, WallMode};
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

/// How close a second order must be to the first to count as "again".
const ORDER_AGAIN_TILES: i32 = 2;

/// Send the selected penguins somewhere (right click; middle click, or the
/// same order twice, runs).
fn set_job(w: &mut World, pos: (f64, f64), run_mode: bool) {
    if w.selected.is_empty() {
        return;
    }
    let (x, y) = pos;
    let mut tile = w.canvas_to_map(x, y);
    // (clicked on a wall section standing up from the tile below)
    if wall::at(w, tile.0, tile.1).is_none() {
        if let Some(wi) = wall::pick(w, tile) {
            tile = (w.walls[wi].x, w.walls[wi].y);
        }
    }
    // the same penguins sent to the same place again: they go there
    // without stopping to fight (like a middle click)
    let mut run_mode = run_mode;
    if let Some((last, who)) = &w.last_order {
        if *who == w.selected && (last.0 - tile.0).abs() <= ORDER_AGAIN_TILES && (last.1 - tile.1).abs() <= ORDER_AGAIN_TILES {
            run_mode = true;
        }
    }
    w.last_order = Some((tile, w.selected.clone()));
    let sel: Vec<usize> = w.selected.iter().copied().collect();
    if w.is_server() {
        for &u in &sel {
            unit::set_job(w, u, tile, run_mode, sel.len());
        }
    } else {
        let ids = sel.iter().map(|&u| u as u16).collect();
        w.send(Msg::SetJob { pos: (tile.0 as u16, tile.1 as u16), run_mode, unit_ids: ids });
        // (the host calls their class change off)
        for &u in &sel {
            if !matches!(w.units[u].state, State::InBuilding | State::Entering) {
                w.units[u].switch_pending = false;
            }
        }
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

/// The oldest alliance offer to me.
fn hud_offer(w: &World) -> Option<Pid> {
    let me = w.me?;
    w.offers.iter().filter(|o| o.1 == me).min_by_key(|o| o.2).map(|o| o.0)
}

fn ally_action(w: &mut World, a: hud::AllyAction) {
    match a {
        hud::AllyAction::Offer(p) => crate::alliance::request_offer(w, p),
        hud::AllyAction::Accept(p) => crate::alliance::request_answer(w, p, true),
        hud::AllyAction::Reject(p) => crate::alliance::request_answer(w, p, false),
        hud::AllyAction::Nothing => {}
    }
}

/// A client stops showing a class change the host never made after this.
const CLIENT_SWITCH_WAIT_MS: u32 = 120_000;

/// The selected penguins go home and change class (X): workers become
/// snowballers and the other way round, 1 fish each.
pub fn switch_class(w: &mut World) {
    let sel: Vec<usize> = w.selected.iter().copied().filter(|&u| !w.units[u].switch_pending).collect();
    if sel.is_empty() {
        return;
    }
    // (counted here so a client can say it too; the host decides)
    let mut fish: std::collections::HashMap<usize, i32> = Default::default();
    let (mut sent, mut poor) = (Vec::new(), 0);
    for &u in &sel {
        let ig = w.units[u].igloo;
        let promised = w.buildings[ig].units.iter().filter(|&&o| w.units[o].switch_pending).count() as i32 * unit::SWITCH_FISH;
        let left = fish.entry(ig).or_insert(w.num_storage(ig, ResKind::Fish) - promised);
        if *left >= unit::SWITCH_FISH {
            *left -= unit::SWITCH_FISH;
            sent.push(u);
        } else {
            poor += 1;
        }
    }
    let workers = sent.iter().filter(|&&u| w.units[u].kind == UnitKind::Worker).count();
    let snowballers = sent.len() - workers;
    if w.is_server() {
        let me = w.me;
        unit::order_switch(w, me, &sent);
    } else if !sent.is_empty() {
        crate::plugins::request_switch(w, &sent);
        let now = w.now();
        for &u in &sent {
            // (shown until the host says it changed, or gives up)
            w.units[u].switch_pending = true;
            w.units[u].switch_done_at = Some(now.wrapping_add(CLIENT_SWITCH_WAIT_MS));
        }
    }
    let mut parts = Vec::new();
    if snowballers > 0 {
        parts.push(format!("{} snowballer{} to become worker{}", snowballers, if snowballers == 1 { "" } else { "s" }, if snowballers == 1 { "" } else { "s" }));
    }
    if workers > 0 {
        parts.push(format!("{} worker{} to become snowballer{}", workers, if workers == 1 { "" } else { "s" }, if workers == 1 { "" } else { "s" }));
    }
    if !parts.is_empty() {
        let n = sent.len() as i32 * unit::SWITCH_FISH;
        w.notice(&format!("Sent home {} ({} fish)", parts.join(" and "), n));
    }
    if poor > 0 {
        w.notice(&format!("Not enough fish in the igloo for {} more", poor));
    }
}

fn selected_workers(w: &World) -> Vec<usize> {
    w.selected.iter().copied().filter(|&u| w.units[u].kind == UnitKind::Worker).collect()
}

/// B / U: plan a snow wall, or pick sections to turn into ice (with
/// workers selected).
pub fn wall_mode(w: &mut World, mode: WallMode) {
    if selected_workers(w).is_empty() {
        w.notice("Select some workers first: they build the walls");
        return;
    }
    w.wall_mode = if w.wall_mode == Some(mode) { None } else { Some(mode) };
}

/// A click while planning walls: put the wall piece down (B), or turn the
/// section under the mouse into ice (U).
fn wall_click(w: &mut World, tile: (i32, i32)) {
    let Some(mode) = w.wall_mode else { return };
    let workers = selected_workers(w);
    if workers.is_empty() {
        w.wall_mode = None;
        return;
    }
    match mode {
        WallMode::Build => {
            let tiles = wall::piece_tiles(tile, w.wall_rot);
            if !tiles.iter().all(|&(x, y)| wall::can_place(w, x, y)) {
                w.notice("The wall doesn't fit there (it can't go on trees, ice pinnacles, fish holes, crystal, other walls or by an igloo door): move or rotate it (R)");
                return;
            }
            let (a, b) = (tiles[0], tiles[tiles.len() - 1]);
            wall::request_plan(w, a, b, &workers);
            w.notice(&format!("Snow wall section planned ({} worker{} on it)", workers.len(), if workers.len() == 1 { "" } else { "s" }));
        }
        WallMode::Ice => {
            let me = w.me;
            let walls = wall::upgradable(w, me, &[tile]);
            if walls.is_empty() {
                w.notice("Click a finished section of your snow wall to turn it into ice");
                return;
            }
            let have = me.map(|p| wall::crystal_of(w, p)).unwrap_or(0);
            if have < wall::ICE_CRYSTAL {
                w.notice(&format!("Not enough crystal: ice costs {} a section", wall::ICE_CRYSTAL));
                return;
            }
            wall::request_ice(w, &walls, &workers);
            w.notice(&format!("Turning a section into ice ({} crystal)", wall::ICE_CRYSTAL));
        }
    }
    // (Shift keeps planning)
    if sdl::mod_state() & sdl::KMOD_SHIFT == 0 {
        w.wall_mode = None;
    }
}

/// Turn the wall piece being placed (R, or the mouse wheel).
fn rotate_wall(w: &mut World, by: i32) {
    let n = wall::ROTATIONS.len() as i32;
    w.wall_rot = (w.wall_rot as i32 + by).rem_euclid(n) as usize;
}

/// Make the selected penguins squad `n` (Ctrl+n), or add them to it
/// (Shift+n).  A penguin is in one squad at a time.
pub fn make_squad(w: &mut World, n: usize, add: bool) {
    if !add {
        for u in std::mem::take(&mut w.groups[n]) {
            w.units[u].group = None;
        }
    }
    let sel: Vec<usize> = w.selected.iter().copied().collect();
    for &u in &sel {
        if let Some(g) = w.units[u].group {
            w.groups[g as usize].remove(&u);
        }
        w.units[u].group = Some(n as u8);
        w.groups[n].insert(u);
    }
    let c = w.groups[n].len();
    w.notice(&format!("Squad {}: {} penguin{}", n, c, if c == 1 { "" } else { "s" }));
}

/// The members of squad `n` that are still mine.
pub fn squad_members(w: &World, n: usize) -> Vec<usize> {
    w.groups[n].iter().copied().filter(|&u| w.igloo_player(u) == w.me).collect()
}

/// Select squad `n` (key n or its button); a second tap soon after moves
/// the view to it.
pub fn tap_squad(w: &mut World, n: usize) {
    let members = squad_members(w, n);
    if members.is_empty() {
        return;
    }
    let now = w.now();
    let again = w.last_squad_tap.is_some_and(|(m, t)| m == n && now.wrapping_sub(t) < SQUAD_DOUBLE_TAP_MS);
    w.last_squad_tap = Some((n, now));
    for &u in &w.selected {
        w.units[u].selected = false;
    }
    w.selected = members.iter().copied().collect();
    for &u in &members {
        w.units[u].selected = true;
    }
    if again {
        // the middle of the squad (penguins inside an igloo count from there)
        let (sx, sy) = members.iter().fold((0.0, 0.0), |a, &u| (a.0 + (w.units[u].x * 32) as f64, a.1 + (w.units[u].y * 32) as f64));
        let k = members.len() as f64;
        look_at(w, sx / k, sy / k);
    }
}

/// Two taps on a squad within this long: look at it.
const SQUAD_DOUBLE_TAP_MS: u32 = 400;

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
            sdl::MOUSEWHEEL if w.wall_mode == Some(WallMode::Build) => {
                // (placing a wall: the wheel turns it)
                let y = e.wheel_y();
                if y != 0 {
                    rotate_wall(w, if y > 0 { 1 } else { -1 });
                }
            }
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
                    sdl::K_ESCAPE if w.wall_mode.is_some() => {
                        w.wall_mode = None;
                    }
                    sdl::K_R if w.wall_mode == Some(WallMode::Build) => rotate_wall(w, 1),
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
                    sdl::K_X if !w.selected.is_empty() => switch_class(w),
                    sdl::K_B => wall_mode(w, WallMode::Build),
                    sdl::K_U => wall_mode(w, WallMode::Ice),
                    sdl::K_P => w.diplomacy_open = !w.diplomacy_open,
                    sdl::K_Y | sdl::K_N if hud_offer(w).is_some() => {
                        let from = hud_offer(w).unwrap();
                        crate::alliance::request_answer(w, from, key == sdl::K_Y);
                    }
                    k => {
                        chat_keys(w, k);
                    }
                }
                // squads
                if (sdl::K_1..=sdl::K_9).contains(&key) {
                    let n = (key - sdl::K_0) as usize;
                    let m = e.key_mod();
                    if !w.selected.is_empty() && m & sdl::KMOD_CTRL != 0 {
                        make_squad(w, n, false);
                    } else if !w.selected.is_empty() && m & sdl::KMOD_SHIFT != 0 {
                        make_squad(w, n, true);
                    } else {
                        tap_squad(w, n);
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
                let squad = if e.button() == 1 && !w.draw_minimap { hud::squad_at(w, pos) } else { None };
                let ally = if !w.draw_minimap { hud::ally_click(w, pos) } else { None };
                if let Some(a) = ally {
                    if e.button() == 1 {
                        ally_action(w, a);
                    }
                } else if let Some(n) = squad {
                    tap_squad(w, n);
                } else if !hud::in_panel(w, pos) {
                    let p = scaled_mouse(w);
                    if !w.draw_minimap && w.wall_mode.is_some() {
                        // planning a wall: click to put the piece down (or to
                        // turn a section into ice); right click: take down one
                        // of my sections (in build mode), or stop
                        let tile = w.canvas_to_map(p.0, p.1);
                        let mine = wall::pick(w, tile).filter(|&wi| w.walls[wi].owner == w.me && w.me.is_some());
                        match (e.button(), mine) {
                            (1, _) => wall_click(w, tile),
                            (3, Some(wi)) if w.wall_mode == Some(WallMode::Build) => {
                                wall::request_take_down(w, wi);
                                w.notice("Wall section taken down");
                            }
                            _ => {
                                w.wall_mode = None;
                            }
                        }
                    } else if !w.draw_minimap {
                        match e.button() {
                            1 => {
                                // clicking a frozen penguin keeps its thaw countdown shown
                                w.inspected = crate::display::frozen_under_mouse(w);
                                w.select_box_start = Some((w.view.x + p.0, w.view.y + p.1));
                            }
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
    // (planning walls needs workers)
    if w.wall_mode.is_some() && selected_workers(w).is_empty() {
        w.wall_mode = None;
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
