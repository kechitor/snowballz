//! Animated values and the little visual effects: footprints and other
//! imprints, move markers, falling snow and snow particles.

use crate::gfx::{self, Tex};
use crate::rng::Rng;

/// Linear interpolation from `a` to `b` between times `t0` and `t1`
/// (rabbyt's lerp).
#[derive(Clone, Copy, Debug)]
pub struct Lerp {
    pub a: f64,
    pub b: f64,
    pub t0: u32,
    pub t1: u32,
}

impl Lerp {
    pub fn fixed(v: f64) -> Lerp {
        Lerp { a: v, b: v, t0: 0, t1: 0 }
    }

    pub fn new(a: f64, b: f64, start: u32, dt: f64) -> Lerp {
        Lerp { a, b, t0: start, t1: start.wrapping_add(dt.max(0.0) as u32) }
    }

    pub fn between(a: f64, b: f64, t0: u32, t1: u32) -> Lerp {
        Lerp { a, b, t0, t1 }
    }

    pub fn get(&self, t: u32) -> f64 {
        if t <= self.t0 {
            return self.a;
        }
        if t >= self.t1 || self.t1 == self.t0 {
            return self.b;
        }
        self.a + (self.b - self.a) * ((t - self.t0) as f64 / (self.t1 - self.t0) as f64)
    }
}

/// `v` wrapped into [low, high) (rabbyt's wrap).
pub fn wrap(low: f64, high: f64, v: f64) -> f64 {
    let span = high - low;
    if span <= 0.0 {
        return low;
    }
    low + (v - low).rem_euclid(span)
}

struct Imprint {
    x: f64,
    y: f64,
    color: [f32; 3],
    t0: u32,
}

/// Fading marks on the ground (footprints, snowball marks, move markers).
pub struct ImprintSet {
    tex: Tex,
    length: u32,
    items: Vec<Imprint>,
}

impl ImprintSet {
    pub fn new(path: &str, seconds: f64) -> ImprintSet {
        ImprintSet { tex: gfx::texture(path), length: (seconds * 1000.0) as u32, items: Vec::new() }
    }

    pub fn add(&mut self, x: f64, y: f64, color: [f32; 3], now: u32) {
        self.items.push(Imprint { x, y, color, t0: now });
    }

    /// Colours of 0-255 are turned into 0-1 (like the Python version).
    pub fn add_rgb(&mut self, x: f64, y: f64, color: [u8; 3], now: u32) {
        let c = color.map(|c| if c > 1 { c as f32 / 255.0 } else { c as f32 });
        self.add(x, y, c, now);
    }

    pub fn draw(&mut self, now: u32, view: (f64, f64, f64, f64)) {
        let len = self.length;
        self.items.retain(|i| now.wrapping_sub(i.t0) < len || now < i.t0);
        let (vx, vy, vw, vh) = view;
        for i in &self.items {
            if i.x + 32.0 < vx || i.y + 32.0 < vy || i.x > vx + vw || i.y > vy + vh {
                continue;
            }
            let a = Lerp::new(1.0, 0.0, i.t0, len as f64).get(now) as f32;
            self.tex.draw(i.x, i.y, [i.color[0], i.color[1], i.color[2], a], 1.0);
        }
    }

}

struct Flake {
    tn: usize,
    x0: f64,
    y0: f64,
    life: f64,
    t0: u32,
    lifetime: f64,
}

/// Falling snow at night.
pub struct Snowflakes {
    tex: [Tex; 4],
    flakes: Vec<Flake>,
    max: usize,
    bounds: [[f64; 2]; 2],
}

impl Snowflakes {
    pub fn new() -> Snowflakes {
        let st = gfx::texture("data/snowparticles.png");
        Snowflakes {
            tex: [st.sub(0, 0, 8, 8), st.sub(8, 0, 8, 8), st.sub(0, 8, 8, 8), st.sub(8, 8, 8, 8)],
            flakes: Vec::new(),
            max: 100,
            bounds: [[0.0; 2]; 2],
        }
    }

    pub fn run(&mut self, falling: bool, now: u32, rng: &mut Rng, view: (f64, f64, f64, f64)) {
        self.flakes.retain(|f| (now.wrapping_sub(f.t0) as f64) < f.lifetime);
        let (vx, vy, sw, sh) = view;
        if falling && self.flakes.len() < self.max {
            let tn = rng.randint(0, 3) as usize;
            let x0 = rng.random() * sw;
            let y0 = rng.random() * sh;
            let speed = rng.random() * 0.05 + 0.05;
            let life = rng.random() * 200.0 + 300.0;
            self.flakes.push(Flake { tn, x0, y0, life, t0: now, lifetime: life / speed });
        }
        self.bounds = [[vx, vx + sw], [vy, vy + sh]];
        for f in &self.flakes {
            let x = wrap(self.bounds[0][0], self.bounds[0][1], f.x0);
            let y = wrap(self.bounds[1][0], self.bounds[1][1], Lerp::new(f.y0, f.y0 + f.life, f.t0, f.lifetime).get(now));
            let scale = Lerp::new(1.0, 0.0, f.t0, f.lifetime).get(now);
            self.tex[f.tn].draw(x, y, [1.0; 4], scale);
        }
    }
}

/// A burst of snow where a snowball lands.
pub struct SnowParticles {
    parts: Vec<(Lerp, Lerp)>,
    start: u32,
    end: u32,
}

impl SnowParticles {
    pub fn new(x: f64, y: f64, dir: (f64, f64), n: i32, now: u32, rng: &mut Rng) -> SnowParticles {
        let dt = 400.0;
        let parts = (0..n)
            .map(|_| {
                let dx = dir.0 + (rng.random() - 1.0) * 30.0;
                let dy = dir.1 + (rng.random() - 1.0) * 30.0;
                (Lerp::new(x, x + dx, now, dt), Lerp::new(y, y + dy, now, dt))
            })
            .collect();
        SnowParticles { parts, start: now, end: now + dt as u32 }
    }

    /// Draw; false once finished.
    pub fn run(&self, now: u32, tex: &Tex) -> bool {
        if now >= self.end {
            return false;
        }
        let a = Lerp::between(1.0, 0.0, self.start + 200, self.end).get(now) as f32;
        for (x, y) in &self.parts {
            tex.draw(x.get(now), y.get(now), [1.0, 1.0, 1.0, a], 1.0);
        }
        true
    }
}
