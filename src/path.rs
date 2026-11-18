//! Finding the way round walls.
//!
//! Penguins normally just head for where they are going (with a little
//! randomness, stepping round trees and other penguins as they meet them).
//! That gets them stuck behind a wall: they walk along it, turn back, and
//! walk along it again.  So when a finished wall section lies between a
//! penguin and where it is going, it follows a path (A*, 8 directions)
//! round the walls, trees, ice pinnacles and igloos instead.  The path is
//! kept until the goal or the walls change, or the penguin gets pushed off
//! it.

use crate::world::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Searching gives up after this many tiles (then the penguin heads for
/// the closest tile it found).
const MAX_NODES: usize = 12_000;

const DIRS: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Is there a finished wall anywhere near the way from `a` to `b`?
pub fn walls_in_the_way(w: &World, a: (i32, i32), b: (i32, i32)) -> bool {
    let (x0, x1) = (a.0.min(b.0) - 3, a.0.max(b.0) + 3);
    let (y0, y1) = (a.1.min(b.1) - 3, a.1.max(b.1) + 3);
    w.walls.iter().any(|s| s.built() && s.x >= x0 && s.x <= x1 && s.y >= y0 && s.y <= y1)
}

/// Does a finished wall tile lie on the straight line from `a` to `b`?
pub fn wall_between(w: &World, a: (i32, i32), b: (i32, i32)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let n = dx.abs().max(dy.abs());
    if n == 0 || w.walls.is_empty() {
        return false;
    }
    (1..=n).any(|i| {
        let p = (a.0 + (dx as f64 * i as f64 / n as f64).round() as i32, a.1 + (dy as f64 * i as f64 / n as f64).round() as i32);
        solid_at(w, p)
    })
}

fn blocked(w: &World, p: (i32, i32), goal: (i32, i32)) -> bool {
    let Some(ti) = w.tile_index(p.0, p.1) else { return true };
    if p == goal {
        return false;
    }
    let t = &w.tiles[ti];
    if t.resource.is_some() || crate::wall::solid(w, ti) {
        return true;
    }
    // (penguins can walk on the top row of an igloo, and into its door)
    t.building.is_some_and(|b| w.buildings[b].y != p.1)
}

fn solid_at(w: &World, p: (i32, i32)) -> bool {
    w.tile_index(p.0, p.1).is_some_and(|i| crate::wall::solid(w, i))
}

/// The tiles from `from` (not included) to `goal` round the obstacles, or
/// to the reachable tile closest to it.
pub fn find(w: &World, from: (i32, i32), goal: (i32, i32)) -> Vec<(i32, i32)> {
    let (sw, sh) = w.size;
    let idx = |p: (i32, i32)| (p.1 * sw + p.0) as usize;
    let n = (sw * sh) as usize;
    let mut cost = vec![u32::MAX; n];
    let mut came: Vec<u32> = vec![u32::MAX; n];
    let h = |p: (i32, i32)| {
        let (dx, dy) = ((p.0 - goal.0).unsigned_abs(), (p.1 - goal.1).unsigned_abs());
        10 * dx.max(dy) + 4 * dx.min(dy)
    };
    let mut open = BinaryHeap::new();
    cost[idx(from)] = 0;
    open.push(Reverse((h(from), 0u32, from)));
    let mut best = (h(from), from);
    let mut seen = 0;
    while let Some(Reverse((_, g, p))) = open.pop() {
        if g > cost[idx(p)] {
            continue;
        }
        if p == goal {
            best = (0, p);
            break;
        }
        seen += 1;
        if seen > MAX_NODES {
            break;
        }
        for d in DIRS {
            let q = (p.0 + d.0, p.1 + d.1);
            if q.0 < 0 || q.1 < 0 || q.0 >= sw || q.1 >= sh || blocked(w, q, goal) {
                continue;
            }
            // no slipping between the ends of a diagonal wall
            if d.0 != 0 && d.1 != 0 && solid_at(w, (p.0 + d.0, p.1)) && solid_at(w, (p.0, p.1 + d.1)) {
                continue;
            }
            let ng = g + if d.0 != 0 && d.1 != 0 { 14 } else { 10 };
            let qi = idx(q);
            if ng < cost[qi] {
                cost[qi] = ng;
                came[qi] = idx(p) as u32;
                let hq = h(q);
                if hq < best.0 {
                    best = (hq, q);
                }
                open.push(Reverse((ng + hq, ng, q)));
            }
        }
    }
    let mut path = Vec::new();
    let mut p = best.1;
    while p != from {
        path.push(p);
        let c = came[idx(p)];
        if c == u32::MAX {
            return Vec::new();
        }
        p = ((c as i32) % sw, (c as i32) / sw);
    }
    path.reverse();
    path
}

/// The next step for a penguin going to `goal` round the walls (`None`:
/// no walls in the way, or nowhere to go).
pub fn step(w: &mut World, ui: usize, goal: (i32, i32)) -> Option<Step> {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *OFF.get_or_init(|| std::env::var("SNOWBALLZ_NO_PATHS").is_ok()) {
        return None; // (test hook: the old way, to compare)
    }
    let pos = (w.units[ui].x, w.units[ui].y);
    if pos == goal || !walls_in_the_way(w, pos, goal) {
        w.units[ui].path.clear();
        return None;
    }
    let u = &mut w.units[ui];
    while u.path.first() == Some(&pos) {
        u.path.remove(0);
    }
    let adjacent = |q: &(i32, i32)| (q.0 - pos.0).abs() <= 1 && (q.1 - pos.1).abs() <= 1;
    let stale = u.path_goal != Some(goal) || u.path_version != w.wall_version || !u.path.first().is_some_and(adjacent);
    if stale {
        let path = find(w, pos, goal);
        let u = &mut w.units[ui];
        u.path = path;
        u.path_goal = Some(goal);
        u.path_version = w.wall_version;
        // (an old wander back and forth shouldn't stop it now)
        u.last_tiles.clear();
    }
    let next = *w.units[ui].path.first()?;
    Some(Step::Go(next.0 - pos.0, next.1 - pos.1))
}

/// (test hook) At 4 s: a wall 6 tiles below my igloo, 17 tiles wide with
/// its ends bent down 5 tiles (a cup, open at the bottom), and all my
/// penguins sent to the middle under it.  Then every 5 s: how many got
/// there.
pub fn test_hook(w: &mut World, elapsed: u32) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(4000);
    static GOAL: AtomicU32 = AtomicU32::new(0);
    if elapsed < NEXT.load(Ordering::Relaxed) {
        return;
    }
    NEXT.store(elapsed + 5000, Ordering::Relaxed);
    let me = w.me;
    let mine: Vec<usize> = (0..w.units.len()).filter(|&u| w.igloo_player(u) == me).collect();
    if GOAL.load(Ordering::Relaxed) == 0 {
        let Some((x, y)) = w.buildings.iter().find(|b| b.player == me && me.is_some()).map(|b| (b.x, b.y)) else { return };
        let wy = y + 6;
        let mut tiles: Vec<(i32, i32)> = (x - 8..=x + 8).map(|i| (i, wy)).collect();
        for k in 1..=5 {
            tiles.push((x - 8, wy + k));
            tiles.push((x + 8, wy + k));
        }
        let n = crate::wall::test_build(w, me, &tiles);
        let goal = (x, wy + 3);
        GOAL.store(((goal.0 as u32) << 16) | goal.1 as u32, Ordering::Relaxed);
        let n_units = mine.len();
        for &u in &mine {
            crate::unit::set_job(w, u, goal, false, n_units);
            w.units[u].run_mode = true;
        }
        println!("PATHTEST wall {} tiles, goal {:?}, {} penguins sent", n, goal, n_units);
        return;
    }
    let g = GOAL.load(Ordering::Relaxed);
    let goal = ((g >> 16) as i32, (g & 0xffff) as i32);
    let near = mine.iter().filter(|&&u| (w.units[u].x - goal.0).abs().max((w.units[u].y - goal.1).abs()) <= 3).count();
    println!("PATHTEST t={} {} of {} within 3 tiles of the goal", elapsed, near, mine.len());
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagonal_cost_is_more_than_straight() {
        // (the heuristic: octile distance, 10 a straight step, 14 diagonal)
        let h = |dx: u32, dy: u32| 10 * dx.max(dy) + 4 * dx.min(dy);
        assert_eq!(h(3, 0), 30);
        assert_eq!(h(3, 3), 42);
    }
}
