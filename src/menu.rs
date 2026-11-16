//! The main menu (port of menu.py).

use crate::backgrounds;
use crate::gfx;
use crate::image::Image;
use crate::ini::Ini;
use crate::sdl;
use crate::settings;
use crate::ui::{self, Action, Button, DRect, Ev, ImageW, Input, Label, Page, Panel, SelectBox, Slider, View, Widget};
use crate::zip::Zip;

pub struct MapInfo {
    pub name: String,
    pub filename: String,
    pub description: String,
    pub objectives: String,
    pub icon: gfx::Tex,
    /// the penguins each igloo starts with: (total, snowballers)
    pub penguins: (u16, u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GameType {
    Single,
    Server,
    Client(String),
}

pub struct Menu {
    pages: Vec<Page>,
    current: usize,
    maps: Vec<MapInfo>,
    cur_map: usize,
    error: Label,
    view: View,
    result: Option<(String, GameType)>,
    // where some widgets are
    play_icon: usize,
    play_name: usize,
    play_objectives: usize,
    play_description: usize,
    loadout_label: usize,
    host_input: usize,
    server_select: usize,
}

/// The maps in maps/*.zip, sorted by file name.
pub fn list_maps() -> Vec<MapInfo> {
    let mut files: Vec<String> = std::fs::read_dir("maps")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".zip"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let filename = f.split('.').next().unwrap_or("").to_string();
        let z = match Zip::open(&format!("maps/{}", f)) {
            Ok(z) => z,
            Err(e) => {
                eprintln!("Skipping map {}: {}", f, e);
                continue;
            }
        };
        let ini = match z.read("settings.ini") {
            Ok(b) => Ini::parse(&String::from_utf8_lossy(&b)),
            Err(e) => {
                eprintln!("Skipping map {}: {}", f, e);
                continue;
            }
        };
        let icon_img = z
            .read("icon.png")
            .ok()
            .and_then(|b| Image::decode(&b).ok())
            .or_else(|| Image::load("data/noicon.png").ok());
        let icon = match icon_img.and_then(|i| gfx::texture_from_image(&i).ok()) {
            Some(t) => t,
            None => gfx::texture("data/noicon.png"),
        };
        let count = |k: &str| ini.get("igloo_settings", k).and_then(|v| v.trim().parse::<u16>().ok()).unwrap_or(0);
        let (workers, snowballers) = (count("workers"), count("snowballers"));
        out.push(MapInfo {
            penguins: (workers + snowballers, snowballers),
            name: ini.get("map", "name").unwrap_or(&filename).to_string(),
            description: ini.get("map", "description").unwrap_or("").to_string(),
            objectives: ini.get("map", "objectives").unwrap_or("").to_string(),
            filename,
            icon,
        });
    }
    out
}

fn bottom_bar(page: &mut Page, buttons: Vec<Button>) {
    let mut ws: Vec<Widget> = buttons.into_iter().map(Widget::Button).collect();
    {
        let mut refs: Vec<&mut Widget> = ws.iter_mut().collect();
        ui::hbox(&mut refs, 320, Some(480 - 15), None, 20);
    }
    for w in ws {
        page.add(w);
    }
}

/// A label centered on the 640 x 480 design screen, `y` from the top.
fn centered(text: &str, size: u32, y: i32) -> Label {
    let mut l = Label::new(text, size, 0, y);
    l.set_text(text);
    l.rect.x = (320 - l.rect.w / 2).max(5);
    l
}

fn option_row(panel: &mut Panel, option: &str, values: &[&str], label: &str, labels: &[&str]) {
    let mut current = settings::get_option(option);
    if !values.contains(&current.as_str()) {
        current = values[0].to_string();
    }
    let labels: Vec<String> = labels.iter().map(|s| s.to_string()).collect();
    let mut l = Label::new(&format!("{}:", label), 16, 0, 0);
    l.min_width = 180;
    l.set_text(&format!("{}:", label));
    panel.add_row(vec![Widget::Label(l), Widget::Button(Button::switch(option, &current, values, &labels))]);
}

impl Menu {
    pub fn new() -> Menu {
        let maps = list_maps();
        let mut pages: Vec<Page> = (0..6).map(|_| Page::default()).collect();

        // main menu
        bottom_bar(
            &mut pages[0],
            vec![
                Button::new("Single Play", Action::Page(1)),
                Button::new("Online", Action::Page(3)),
                Button::new("Settings", Action::Page(2)),
                Button::new("Quit", Action::Exit),
            ],
        );

        // single play
        let panel_tex = gfx::image_texture("data/map_img_panel.png").unwrap_or_else(|_| gfx::texture("data/noicon.png"));
        pages[1].add(Widget::Image(ImageW { tex: panel_tex, rect: DRect { x: 20, y: 125, w: panel_tex.w, h: panel_tex.h } }));
        let (icon, name, obj, desc) = match maps.first() {
            Some(m) => (m.icon, m.name.clone(), m.objectives.clone(), m.description.clone()),
            None => (gfx::texture("data/noicon.png"), "No maps found".into(), String::new(), String::new()),
        };
        let play_icon = pages[1].add(Widget::Image(ImageW { tex: icon, rect: DRect { x: 25, y: 130, w: icon.w, h: icon.h } }));
        let play_name = pages[1].add(Widget::Label(Label::new(&name, 25, 300, 260)));
        let play_objectives = pages[1].add(Widget::Label(Label::new(&obj, 13, 300, 310)));
        let play_description = pages[1].add(Widget::Label(Label::new(&desc, 13, 300, 290)));
        // loadout: how many of your penguins are snowballers / workers
        pages[1].add(Widget::Label(Label::new("Your penguins:", 16, 300, 340)));
        let mut minus = Button::new("-", Action::Loadout(-1));
        minus.min_width = 34;
        minus.set_text("-");
        minus.rect.x = 300;
        minus.rect.y = 362;
        let mut plus = Button::new("+", Action::Loadout(1));
        plus.min_width = 34;
        plus.set_text("+");
        plus.rect.x = 340;
        plus.rect.y = 362;
        pages[1].add(Widget::Button(minus));
        pages[1].add(Widget::Button(plus));
        let loadout_label = pages[1].add(Widget::Label(Label::new("", 16, 384, 367)));
        bottom_bar(
            &mut pages[1],
            vec![
                Button::new("Back", Action::Page(0)),
                Button::new("Previous", Action::Seek(-1)),
                Button::new("Next", Action::Seek(1)),
                Button::new("Play", Action::SinglePlay),
            ],
        );

        // settings
        let mut panel = Panel::new(0, 2, 8);
        option_row(&mut panel, "fullscreen", &["0", "1"], "Fullscreen", &["Off", "On"]);
        let ress = settings::res_available();
        let reslabels: Vec<String> = ress.iter().map(|r| format!("{} x {}", r.0, r.1)).collect();
        let reses: Vec<String> = (0..ress.len()).map(|i| i.to_string()).collect();
        option_row(
            &mut panel,
            "resolution",
            &reses.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "Resolution",
            &reslabels.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
        option_row(&mut panel, "gamescale", &["1", "2", "3", "4"], "Game scale", &["Auto", "1x", "1.5x", "2x"]);
        option_row(&mut panel, "smooth", &["0", "1"], "Smooth scaling", &["Off", "On"]);
        option_row(&mut panel, "fpscap", &["1", "2", "3"], "Frame rate", &["60 fps", "35 fps (original)", "Unlimited"]);
        option_row(&mut panel, "daynight", &["0", "1"], "Day/Night cycles", &["Off", "On"]);
        option_row(&mut panel, "smoke", &["0", "1"], "Smoke", &["Off", "On"]);
        option_row(&mut panel, "footprints", &["0", "1"], "Footprints", &["Off", "On"]);
        option_row(&mut panel, "snowballdetail", &["1", "2", "3"], "Snowball detail", &["Low", "Medium", "High"]);
        for (key, label) in [("volume", "Volume:"), ("music", "Music:")] {
            let mut l = Label::new(label, 16, 0, 0);
            l.min_width = 180;
            l.set_text(label);
            panel.add_row(vec![Widget::Label(l), Widget::Slider(Slider::new(key, settings::sound_level(key), 150))]);
        }
        let mut name = Input::new(&settings::player_name(), 150, 16);
        name.is_name = true;
        let mut nl = Label::new("Name:", 16, 0, 0);
        nl.min_width = 180;
        nl.set_text("Name:");
        panel.add_row(vec![Widget::Label(nl), Widget::Input(name)]);
        option_row(&mut panel, "cursor", &["0", "1"], "Ice Cursor", &["Off", "On"]);
        bottom_bar(&mut pages[2], vec![Button::new("Back", Action::Page(0))]);
        pages[2].add(Widget::Panel(panel));

        // online
        bottom_bar(
            &mut pages[3],
            vec![
                Button::new("Back", Action::Page(0)),
                Button::new("Host a game", Action::Page(5)),
                Button::new("Join a game", Action::Page(4)),
            ],
        );
        for (i, line) in [
            "Host a game: your computer runs the game for everybody.",
            "The lobby shows the code (or address) to give your friends.",
            "Join a game: type or paste the host's code or address.",
        ]
        .iter()
        .enumerate()
        {
            pages[3].add(Widget::Label(centered(line, 16, 250 + 26 * i as i32)));
        }

        // join
        let mut address = Input::new(&settings::last_host(), 260, 20);
        address.max_len = 64;
        address.submit = Some(Action::Join);
        let mut row = vec![
            Widget::Label(Label::new("Code or address:", 20, 0, 0)),
            Widget::Input(address),
            Widget::Button(Button::new("Go!", Action::Join)),
        ];
        {
            let mut refs: Vec<&mut Widget> = row.iter_mut().collect();
            ui::hbox(&mut refs, 320, None, Some(240 - 15), 20);
        }
        let mut host_input = 0;
        for (i, w) in row.into_iter().enumerate() {
            let idx = pages[4].add(w);
            if i == 1 {
                host_input = idx;
            }
        }
        bottom_bar(&mut pages[4], vec![Button::new("Back", Action::Page(3))]);

        // host: choose the map
        let mut sel = SelectBox { items: Vec::new(), selected: None, scroll: 0, hover: None, rect: DRect { x: 0, y: (480 - 300) / 2, w: 200, h: 300 } };
        for m in &maps {
            sel.items.push((m.name.clone(), m.filename.clone()));
        }
        bottom_bar(&mut pages[5], vec![Button::new("Back", Action::Page(3)), Button::new("Host", Action::Host)]);
        let server_select = pages[5].add(Widget::Select(sel));

        let mut error = Label::new("", 16, 0, 0);
        error.color = [170.0 / 255.0, 0.0, 0.0, 1.0];
        error.visible = false;

        let (sw, sh) = gfx::screen_size();
        let (scale, off_x, off_y) = backgrounds::design_view(sw, sh);
        let mut menu = Menu {
            pages,
            current: 0,
            maps,
            cur_map: 0,
            error,
            view: View { scale, off_x, off_y },
            result: None,
            play_icon,
            play_name,
            play_objectives,
            play_description,
            loadout_label,
            host_input,
            server_select,
        };
        menu.show_loadout();
        menu
    }

    pub fn hide_error(&mut self) {
        self.error.visible = false;
    }

    /// Show a problem (like a failed connection) in the menu.
    pub fn show_error(&mut self, text: &str) {
        self.error.set_text(text);
        let w = self.error.rect.w;
        self.error.rect.x = (320 - w / 2).max(5);
        self.error.rect.y = 405;
        self.error.visible = true;
    }

    fn fullscreen_switch(&mut self) -> Option<&mut Button> {
        for w in self.pages[2].widgets.iter_mut() {
            if let Widget::Panel(p) = w {
                for row in p.rows.iter_mut() {
                    for e in row.iter_mut() {
                        if let Widget::Button(b) = e {
                            if b.switch.as_ref().map(|s| s.0 == "fullscreen").unwrap_or(false) {
                                return Some(b);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Show the current map's loadout (the saved choice or the map's mix).
    fn show_loadout(&mut self) {
        let text = match self.maps.get(self.cur_map) {
            Some(m) if m.penguins.0 > 0 => {
                let (total, default) = m.penguins;
                let n = settings::loadout(&m.filename).unwrap_or(default).min(total);
                format!("{} snowballers  /  {} workers", n, total - n)
            }
            _ => String::new(),
        };
        if let Widget::Label(l) = &mut self.pages[1].widgets[self.loadout_label] {
            l.set_text(&text);
        }
    }

    fn change_loadout(&mut self, d: i32) {
        let Some(m) = self.maps.get(self.cur_map) else { return };
        let (total, default) = m.penguins;
        let n = settings::loadout(&m.filename).unwrap_or(default).min(total) as i32;
        let n = (n + d).clamp(0, total as i32) as u16;
        settings::set_loadout(&m.filename.clone(), n);
        self.show_loadout();
    }

    fn seek(&mut self, dir: i32) {
        if self.maps.is_empty() {
            return;
        }
        let n = self.maps.len() as i32;
        self.cur_map = (self.cur_map as i32 + dir).rem_euclid(n) as usize;
        let m = &self.maps[self.cur_map];
        let (icon, name, obj, desc) = (m.icon, m.name.clone(), m.objectives.clone(), m.description.clone());
        let p = &mut self.pages[1];
        if let Widget::Image(i) = &mut p.widgets[self.play_icon] {
            i.tex = icon;
            i.rect.w = icon.w;
            i.rect.h = icon.h;
        }
        for (idx, text) in [(self.play_name, name), (self.play_objectives, obj), (self.play_description, desc)] {
            if let Widget::Label(l) = &mut p.widgets[idx] {
                l.set_text(&text);
            }
        }
        self.show_loadout();
    }

    fn do_action(&mut self, a: Action) {
        match a {
            Action::Page(i) => {
                self.current = i;
                // typing goes straight into the address box
                if i == 4 {
                    if let Widget::Input(inp) = &mut self.pages[4].widgets[self.host_input] {
                        inp.focused = true;
                    }
                }
            }
            Action::Exit => crate::quit(0),
            Action::Seek(d) => self.seek(d),
            Action::Loadout(d) => self.change_loadout(d),
            Action::SinglePlay => {
                if let Some(m) = self.maps.get(self.cur_map) {
                    self.result = Some((m.filename.clone(), GameType::Single));
                }
            }
            Action::Host => {
                if let Widget::Select(s) = &self.pages[5].widgets[self.server_select] {
                    if let Some(v) = s.selected_value() {
                        self.result = Some((v, GameType::Server));
                    }
                }
            }
            Action::Join => {
                let host = match &self.pages[4].widgets[self.host_input] {
                    Widget::Input(i) => i.value.trim().to_string(),
                    _ => String::new(),
                };
                if let Err(e) = crate::net::parse_address(&host, crate::net::PORT) {
                    if !host.is_empty() {
                        self.show_error(&e);
                        return;
                    }
                }
                settings::set_last_host(&host);
                let host = if host.is_empty() { "localhost".to_string() } else { host };
                let map = self.maps.get(self.cur_map).map(|m| m.filename.clone()).unwrap_or_default();
                self.result = Some((map, GameType::Client(host)));
            }
            Action::Option(opt, value) => {
                settings::set_option(&opt, &value);
                match opt.as_str() {
                    "fullscreen" => {
                        if !gfx::set_fullscreen(value == "1") {
                            settings::set_option("fullscreen", "0");
                            if let Some(b) = self.fullscreen_switch() {
                                b.set_value("0");
                            }
                        }
                    }
                    "resolution" => {
                        let (w, h) = settings::get_res();
                        gfx::set_resolution(w, h);
                    }
                    "smooth" => gfx::set_smooth(value == "1"),
                    _ => {}
                }
            }
            Action::Level(key, value, released) => {
                let (mut volume, mut music) = (settings::sound_level("volume"), settings::sound_level("music"));
                match key.as_str() {
                    "volume" => volume = value,
                    _ => music = value,
                }
                crate::audio::set_levels(volume, music);
                if released {
                    settings::set_sound_level(&key, value);
                }
            }
            Action::NameChanged(n) => {
                if !n.is_empty() {
                    settings::set_player_name(&n);
                }
            }
        }
    }

    /// F11: toggle fullscreen and update the Settings switch.
    pub fn toggle_fullscreen(&mut self) {
        let v = toggle_fullscreen();
        if let Some(b) = self.fullscreen_switch() {
            b.set_value(&v);
        }
    }

    fn to_design(&self, p: (i32, i32)) -> (i32, i32) {
        (
            ((p.0 as f64 - self.view.off_x) / self.view.scale).floor() as i32,
            ((p.1 as f64 - self.view.off_y) / self.view.scale).floor() as i32,
        )
    }

    /// Run the menu until a game is chosen: (map, game type).
    pub fn run(&mut self) -> (String, GameType) {
        self.result = None;
        let mut clock = crate::clock::Clock::new();
        sdl::show_cursor(true);
        while self.result.is_none() {
            clock.tick(35);
            let (sw, sh) = gfx::screen_size();
            let (scale, off_x, off_y) = backgrounds::design_view(sw, sh);
            self.view = View { scale, off_x, off_y };
            for e in gfx::events() {
                let ev = match e.kind() {
                    sdl::QUIT => crate::quit(0),
                    sdl::KEYDOWN if e.key_sym() == sdl::K_F11 => {
                        self.toggle_fullscreen();
                        continue;
                    }
                    sdl::KEYDOWN => Ev::Key(e.key_sym()),
                    sdl::TEXTINPUT => Ev::Text(e.text()),
                    sdl::MOUSEMOTION => Ev::Motion(self.to_design((e.motion_x(), e.motion_y()))),
                    sdl::MOUSEBUTTONDOWN => Ev::Down(e.button(), self.to_design((e.button_x(), e.button_y()))),
                    sdl::MOUSEBUTTONUP => Ev::Up(e.button(), self.to_design((e.button_x(), e.button_y()))),
                    sdl::MOUSEWHEEL => Ev::Wheel(e.wheel_y()),
                    _ => continue,
                };
                let acts = self.pages[self.current].handle(&ev);
                for a in acts {
                    self.do_action(a);
                }
            }
            self.draw();
            gfx::present();
        }
        self.result.clone().unwrap()
    }

    pub fn draw(&self) {
        gfx::clear((0, 0, 0));
        backgrounds::draw("menu", 1.0);
        self.pages[self.current].draw(&self.view);
        if self.error.visible {
            Widget::draw_label(&self.error, &self.view);
        }
    }

    /// For tests: run one menu action by name.
    pub fn press(&mut self, a: Action) {
        self.do_action(a);
    }

}

impl Widget {
    fn draw_label(l: &Label, v: &View) {
        let r = v.to_screen(&l.rect);
        let px = ((l.size as f64 * v.scale).round() as u32).max(4);
        ui::font(px).draw_screen(r.x, r.y, &l.text, l.color);
    }
}

/// F11 anywhere: switch fullscreen on/off (and remember it).
pub fn toggle_fullscreen() -> String {
    let v = settings::toggle("fullscreen", &["0", "1"]);
    if !gfx::set_fullscreen(v == "1") {
        settings::set_option("fullscreen", "0");
        return "0".into();
    }
    v
}
