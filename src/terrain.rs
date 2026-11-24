//! Terrain: a grid of vertices (one per tile corner) moved up by the tile
//! elevation, every vertex with its own shade and ice alpha - drawn as
//! textured triangles, like the original OpenGL mesh.

use crate::gfx::{self, Tex};
use crate::rng::Rng;
use crate::sdl::{Color, FPoint, Rect, Vertex};
use crate::world::*;

pub struct Terrain {
    vw: usize,
    vh: usize,
    /// vertex y (world pixels), ice alpha 0..1, shade 0..1
    y: Vec<f32>,
    a: Vec<f32>,
    d: Vec<f32>,
    band_lo: Vec<f32>,
    band_hi: Vec<f32>,
    band_ice: Vec<bool>,
    snow: Tex,
    ice: Tex,
    verts: Vec<Vertex>,
    idx: Vec<i32>,
}

const TEX_TILE: f32 = 256.0;

impl Terrain {
    pub fn new(w: &World) -> Terrain {
        let (mw, mh) = (w.size.0 as usize, w.size.1 as usize);
        let (vw, vh) = (mw + 1, mh + 1);
        let mut rnd = Rng::new((w.size.0 * 1000 + w.size.1) as u64);
        let mut y = vec![0f32; vw * vh];
        let mut a = vec![0f32; vw * vh];
        let mut d = vec![1f32; vw * vh];
        let tile = |x: i64, yy: i64| -> Option<&Tile> {
            if x < 0 || yy < 0 {
                None
            } else {
                w.tile(x as i32, yy as i32)
            }
        };
        for vy in 0..vh {
            for vx in 0..vw {
                let (x, yy) = (vx as i64, vy as i64);
                let t1 = tile(x, yy);
                let m = t1.map(|t| t.elevation * ELEV_MULT).unwrap_or(0.0);
                y[vy * vw + vx] = (vy as f64 * 32.0 - m) as f32;

                // ice alpha: average of the tiles around the vertex
                let (mut total, mut num) = (0.0, 0.0);
                for (kx, ky) in [(x, yy), (x - 1, yy), (x - 1, yy - 1), (x, yy - 1)] {
                    if let Some(t) = tile(kx, ky) {
                        if t.kind == TileKind::Ice {
                            total += 1.0;
                        }
                        num += 1.0;
                    }
                }
                let mut alpha: f64 = if num > 0.0 { total / num } else { 0.0 };
                if alpha == 0.5 {
                    alpha += rnd.random() * 0.8 - 0.4;
                }
                a[vy * vw + vx] = alpha.clamp(0.0, 1.0) as f32;

                // elevation shadows and highlights
                let t2 = tile(x - 1, yy - 1);
                let mut dd = match (t1, t2) {
                    (Some(t1), Some(t2)) => {
                        let v = (t2.elevation - t1.elevation) / 20.0;
                        if v > 0.0 {
                            1.0 - v
                        } else {
                            1.0
                        }
                    }
                    _ => 1.0,
                };
                let t3 = tile(x + 1, yy + 1);
                if let (Some(t1), Some(t3)) = (t1, t3) {
                    dd += ((t3.elevation - t1.elevation) / 20.0).max(0.0);
                }
                dd -= 0.1;
                d[vy * vw + vx] = dd.clamp(0.0, 1.0) as f32;
            }
        }
        let mut band_lo = Vec::with_capacity(vh);
        let mut band_hi = Vec::with_capacity(vh);
        let mut band_ice = Vec::with_capacity(vh);
        for qy in 0..vh - 1 {
            let rows = &y[qy * vw..(qy + 2) * vw];
            band_lo.push(rows.iter().cloned().fold(f32::MAX, f32::min));
            band_hi.push(rows.iter().cloned().fold(f32::MIN, f32::max));
            band_ice.push(a[qy * vw..(qy + 2) * vw].iter().any(|&v| v > 0.0));
        }
        Terrain {
            vw,
            vh,
            y,
            a,
            d,
            band_lo,
            band_hi,
            band_ice,
            snow: gfx::texture("data/snow.png"),
            ice: gfx::texture("data/ice.png"),
            verts: Vec::new(),
            idx: Vec::new(),
        }
    }

    /// Draw the part in view (world mode must be active).
    pub fn draw(&mut self, view_x: f64, view_y: f64, view_w: f64, view_h: f64) {
        let (tx, ty, sx, sy) = gfx::transform();
        let qx0 = ((view_x / 32.0).floor() as i64 - 1).max(0) as usize;
        let qx1 = (((view_x + view_w) / 32.0).ceil() as i64 + 1).clamp(0, self.vw as i64 - 1) as usize;
        let (y0, y1) = (view_y as f32 - 64.0, (view_y + view_h) as f32 + 64.0);
        let quads: Vec<usize> = (0..self.vh - 1).filter(|&q| self.band_hi[q] >= y0 && self.band_lo[q] <= y1).collect();
        if !gfx::is_init() {
            return;
        }
        let has_geometry = crate::sdl::api().SDL_RenderGeometry.is_some();
        if !has_geometry {
            self.draw_simple(&quads, qx0, qx1);
            return;
        }
        for pass in 0..2 {
            self.verts.clear();
            self.idx.clear();
            for &qy in &quads {
                if pass == 1 && !self.band_ice[qy] {
                    continue;
                }
                let vrow = ((qy * 32) as f32 % TEX_TILE) / TEX_TILE;
                // the texture is upside down (OpenGL's v pointed up)
                let (v_top, v_bot) = (1.0 - vrow, 1.0 - vrow - 32.0 / TEX_TILE);
                for qx in qx0..qx1 {
                    let i00 = qy * self.vw + qx;
                    let i10 = i00 + 1;
                    let i01 = i00 + self.vw;
                    let i11 = i01 + 1;
                    if pass == 1 && self.a[i00] <= 0.0 && self.a[i10] <= 0.0 && self.a[i01] <= 0.0 && self.a[i11] <= 0.0 {
                        continue;
                    }
                    let u0 = ((qx * 32) as f32 % TEX_TILE) / TEX_TILE;
                    let u1 = u0 + 32.0 / TEX_TILE;
                    let x0 = (tx + sx * (qx * 32) as f64) as f32;
                    let x1 = (tx + sx * ((qx + 1) * 32) as f64) as f32;
                    let base = self.verts.len() as i32;
                    for (i, px, u, v) in [(i00, x0, u0, v_top), (i10, x1, u1, v_top), (i01, x0, u0, v_bot), (i11, x1, u1, v_bot)] {
                        let shade = (self.d[i] * 255.0 + 0.5) as u8;
                        let alpha = if pass == 0 { 255 } else { (self.a[i] * 255.0 + 0.5) as u8 };
                        self.verts.push(Vertex {
                            position: FPoint { x: px, y: (ty + sy * self.y[i] as f64) as f32 },
                            color: Color { r: shade, g: shade, b: shade, a: alpha },
                            tex_coord: FPoint { x: u, y: v },
                        });
                    }
                    self.idx.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
                }
            }
            if !self.idx.is_empty() {
                let t = if pass == 0 { self.snow } else { self.ice };
                gfx::geometry(Some(&t), &self.verts, &self.idx);
            }
        }
    }

    /// Without SDL_RenderGeometry (SDL < 2.0.18): flat tiles.
    fn draw_simple(&self, quads: &[usize], qx0: usize, qx1: usize) {
        let (tx, ty, sx, sy) = gfx::transform();
        for &qy in quads {
            for qx in qx0..qx1 {
                let x0 = (tx + sx * (qx * 32) as f64).floor() as i32;
                let x1 = (tx + sx * ((qx + 1) * 32) as f64).floor() as i32;
                let yv = self.y[qy * self.vw + qx] as f64;
                let y0 = (ty + sy * yv).floor() as i32;
                let y1 = (ty + sy * (yv + 32.0)).floor() as i32;
                let dst = Rect::new(x0, y0, x1 - x0, y1 - y0);
                let sxp = ((qx * 32) % 256) as i32;
                let syp = ((qy * 32) % 256) as i32;
                // (each texture covers 256 world pixels, whatever its size)
                let (ks, ki) = (self.snow.w / 256, self.ice.w / 256);
                gfx::blit(&self.snow, Some(Rect::new(sxp * ks, syp * ks, 32 * ks, 32 * ks)), dst, 1.0);
                let a = self.a[qy * self.vw + qx];
                if a > 0.0 {
                    gfx::blit(&self.ice, Some(Rect::new(sxp * ki, syp * ki, 32 * ki, 32 * ki)), dst, a);
                }
            }
        }
    }
}
