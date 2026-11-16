//! Game time and the frame limiter.

use crate::sdl;
use std::sync::atomic::{AtomicU32, Ordering};

/// Game speed in percent (100 = normal).  Only changed by tests
/// (SNOWBALLZ_TIMESCALE) to play whole games quickly.
static SCALE: AtomicU32 = AtomicU32::new(100);

pub fn set_timescale(percent: u32) {
    SCALE.store(percent.max(1), Ordering::Relaxed);
}

/// Milliseconds of game time (pygame.time.get_ticks()).
pub fn now() -> u32 {
    let s = SCALE.load(Ordering::Relaxed);
    if s == 100 {
        sdl::ticks()
    } else {
        (sdl::ticks() as u64 * s as u64 / 100) as u32
    }
}

/// Frame limiter (pygame.time.Clock).
pub struct Clock {
    last: u32,
    times: std::collections::VecDeque<u32>,
}

impl Clock {
    pub fn new() -> Clock {
        Clock { last: sdl::ticks(), times: std::collections::VecDeque::new() }
    }

    /// Wait so that at most `fps` frames per second run (0: no limit);
    /// returns the (real) milliseconds since the previous tick.
    pub fn tick(&mut self, fps: u32) -> u32 {
        if fps > 0 {
            let frame = 1000 / fps;
            let elapsed = sdl::ticks().wrapping_sub(self.last);
            if elapsed < frame {
                sdl::delay(frame - elapsed);
            }
        }
        let now = sdl::ticks();
        let dt = now.wrapping_sub(self.last);
        self.last = now;
        self.times.push_back(dt);
        if self.times.len() > 30 {
            self.times.pop_front();
        }
        dt
    }

    pub fn fps(&self) -> f64 {
        let total: u32 = self.times.iter().sum();
        if total == 0 {
            0.0
        } else {
            self.times.len() as f64 * 1000.0 / total as f64
        }
    }
}
