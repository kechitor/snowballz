//! The menu that Escape opens during a game: Return to game / Settings /
//! Quit game.  In a single player game the game pauses while it is open.

use crate::backgrounds;
use crate::gfx;
use crate::menu;
use crate::sdl;
use crate::sdl::Rect;
use crate::ui::{Action, Button, Ev, Label, Page, Panel, View, Widget};

pub enum Outcome {
    Resume,
    Quit,
}

pub struct GameMenu {
    page: usize,
    pages: [Page; 2],
}

fn view() -> View {
    let (sw, sh) = gfx::screen_size();
    let (scale, off_x, off_y) = backgrounds::design_view(sw, sh);
    View { scale, off_x, off_y }
}

fn to_design(v: &View, p: (i32, i32)) -> (i32, i32) {
    (((p.0 as f64 - v.off_x) / v.scale).floor() as i32, ((p.1 as f64 - v.off_y) / v.scale).floor() as i32)
}

impl GameMenu {
    /// `title`: "Game paused", "You were defeated", ...;
    /// `note`: a line under the buttons (like "Quitting ends the game for
    /// everybody"), if any.
    pub fn new(title: &str, note: Option<&str>) -> GameMenu {
        // the main page: a title and three buttons, in the middle
        let mut panel = Panel::new(0, 12, 20);
        let mut t = Label::new(title, 20, 0, 0);
        t.min_width = 240;
        t.set_text(title);
        panel.add_row(vec![Widget::Label(t)]);
        for (text, action) in [("Return to game", Action::Resume), ("Settings", Action::Page(1)), ("Quit game", Action::Exit)] {
            let mut b = Button::new(text, action);
            b.min_width = 240;
            b.set_text(text);
            panel.add_row(vec![Widget::Button(b)]);
        }
        if let Some(n) = note {
            let mut l = Label::new(n, 13, 0, 0);
            l.set_text(n);
            l.color = [0.25, 0.25, 0.3, 1.0];
            panel.add_row(vec![Widget::Label(l)]);
        }
        panel.top = ((480 - panel.rect.h) / 2).max(0);
        panel.layout();
        let mut main = Page::default();
        main.add(Widget::Panel(panel));

        // the settings page
        let mut settings = Page::default();
        settings.add(Widget::Panel(menu::settings_panel(true)));
        menu::bottom_bar(&mut settings, vec![Button::new("Back", Action::Page(0))]);
        GameMenu { page: 0, pages: [main, settings] }
    }

    /// One event; Some when the menu closes.
    pub fn handle(&mut self, e: &sdl::Event) -> Option<Outcome> {
        let v = view();
        let ev = match e.kind() {
            sdl::KEYDOWN if e.key_sym() == sdl::K_ESCAPE => {
                if self.page == 1 {
                    self.page = 0;
                    return None;
                }
                return Some(Outcome::Resume);
            }
            sdl::KEYDOWN if e.key_sym() == sdl::K_F11 => {
                let value = menu::toggle_fullscreen();
                self.show_fullscreen(&value);
                return None;
            }
            sdl::KEYDOWN => Ev::Key(e.key_sym()),
            sdl::MOUSEMOTION => Ev::Motion(to_design(&v, (e.motion_x(), e.motion_y()))),
            sdl::MOUSEBUTTONDOWN => Ev::Down(e.button(), to_design(&v, (e.button_x(), e.button_y()))),
            sdl::MOUSEBUTTONUP => Ev::Up(e.button(), to_design(&v, (e.button_x(), e.button_y()))),
            sdl::MOUSEWHEEL => Ev::Wheel(e.wheel_y()),
            _ => return None,
        };
        for a in self.pages[self.page].handle(&ev) {
            match a {
                Action::Resume => return Some(Outcome::Resume),
                Action::Exit => return Some(Outcome::Quit),
                Action::Page(i) => self.page = i.min(1),
                a => {
                    if !menu::apply_setting(&a) {
                        self.show_fullscreen("0");
                    }
                }
            }
        }
        None
    }

    fn show_fullscreen(&mut self, value: &str) {
        for w in self.pages[1].widgets.iter_mut() {
            if let Widget::Panel(p) = w {
                if let Some(b) = menu::fullscreen_switch(p) {
                    b.set_value(value);
                }
            }
        }
    }

    /// Over the game: darken it, then the page.
    pub fn draw(&self) {
        let (sw, sh) = gfx::screen_size();
        gfx::fill_screen_rect(Rect::new(0, 0, sw, sh), (0, 0, 0, 140));
        let v = view();
        self.pages[self.page].draw(&v);
    }
}
