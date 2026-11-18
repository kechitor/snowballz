//! Music.  Ogg Vorbis files are decoded by the game itself (vorbis.rs)
//! and played through SDL's audio queue by a small thread.  Music is
//! optional: without an audio device the game simply runs silently.

use crate::{sdl, vorbis};
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Control {
    stop: bool,
    fade_start: Option<u32>,
    fade_ms: u32,
}

struct Playing {
    ctrl: Arc<Mutex<Control>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

static CURRENT: Mutex<Option<Playing>> = Mutex::new(None);
/// the volume sliders, in percent
static MASTER: AtomicU32 = AtomicU32::new(100);
static MUSIC: AtomicU32 = AtomicU32::new(100);

/// Set the volume (everything) and the music volume, in percent.  Takes
/// effect at once, also on the music that is playing.
pub fn set_levels(master: u8, music: u8) {
    MASTER.store(master.min(100) as u32, Ordering::Relaxed);
    MUSIC.store(music.min(100) as u32, Ordering::Relaxed);
}

/// The master slider's loudness factor (squared, like the others).
pub fn master() -> f32 {
    let m = MASTER.load(Ordering::Relaxed) as f32 / 100.0;
    m * m
}

/// The music's loudness factor.  The sliders are squared, so that 50%
/// sounds about half as loud rather than barely quieter.
fn music_gain() -> f32 {
    let m = MASTER.load(Ordering::Relaxed) as f32 / 100.0;
    let u = MUSIC.load(Ordering::Relaxed) as f32 / 100.0;
    (m * m) * (u * u)
}
static WARNED: AtomicU64 = AtomicU64::new(0);

fn warn_once(bit: u64, text: &str) {
    if WARNED.fetch_or(1 << bit, Ordering::Relaxed) & (1 << bit) == 0 {
        eprintln!("{}", text);
    }
}

pub fn enabled() -> bool {
    std::env::var("SNOWBALLZ_NO_MUSIC").is_err()
}

/// Stop the music now.
pub fn stop() {
    let old = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut p) = old {
        p.ctrl.lock().unwrap_or_else(|e| e.into_inner()).stop = true;
        if let Some(t) = p.thread.take() {
            let _ = t.join();
        }
    }
}

/// Fade the music out over `ms` milliseconds.
pub fn fade_out(ms: u32) {
    if let Some(p) = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let mut c = p.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        if c.fade_start.is_none() {
            c.fade_start = Some(sdl::ticks());
            c.fade_ms = ms.max(1);
        }
    }
}

/// Play the first of `names` (in data/music) that works; `loops` < 0
/// repeats forever.
pub fn play(names: &[String], loops: i32) -> bool {
    stop();
    if !enabled() || !sdl::is_loaded() {
        return false;
    }
    unsafe {
        if (sdl::api().SDL_InitSubSystem)(sdl::INIT_AUDIO) != 0 {
            warn_once(1, &format!("Music is off: no audio device ({})", sdl::error()));
            return false;
        }
    }
    for name in names {
        // the Python version had MP3 music; it is OGG now
        // (a whole path: a map's own music, unpacked)
        let mut path = if std::path::Path::new(name).is_absolute() { name.clone() } else { format!("data/music/{}", name) };
        if !std::path::Path::new(&path).exists() {
            if let Some(stem) = path.strip_suffix(".mp3") {
                path = format!("{}.ogg", stem);
            }
        }
        if !std::path::Path::new(&path).exists() {
            eprintln!("Could not play music {}: no such file", name);
            continue;
        }
        let dec = match vorbis::Decoder::open(&path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Could not play music {}: {}", name, e);
                continue;
            }
        };
        let (channels, rate) = (dec.channels as i32, dec.rate as i32);
        let want = sdl::AudioSpec {
            freq: rate as c_int,
            format: sdl::AUDIO_S16SYS,
            channels: channels.clamp(1, 8) as u8,
            silence: 0,
            samples: 4096,
            padding: 0,
            size: 0,
            callback: std::ptr::null_mut(),
            userdata: std::ptr::null_mut(),
        };
        let mut got = want;
        let dev = unsafe { (sdl::api().SDL_OpenAudioDevice)(std::ptr::null(), 0, &want, &mut got, 0) };
        if dev == 0 {
            warn_once(2, &format!("Music is off: could not open the audio device ({})", sdl::error()));
            return false;
        }
        let ctrl = Arc::new(Mutex::new(Control::default()));
        let c2 = ctrl.clone();
        let bytes_per_sec = rate as u32 * channels as u32 * 2;
        let thread = std::thread::Builder::new()
            .name("music".into())
            .spawn(move || {
                feed(dec, dev, bytes_per_sec, loops, c2);
                unsafe { (sdl::api().SDL_CloseAudioDevice)(dev) };
            })
            .ok();
        unsafe { (sdl::api().SDL_PauseAudioDevice)(dev, 0) };
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Playing { ctrl, thread });
        return true;
    }
    false
}

fn feed(mut dec: vorbis::Decoder, dev: u32, bytes_per_sec: u32, mut loops: i32, ctrl: Arc<Mutex<Control>>) {
    let mut pcm: Vec<f32> = Vec::with_capacity(8192);
    let mut buf: Vec<i16> = Vec::with_capacity(8192);
    let mut finished = false;
    let channels = dec.channels.max(1);
    let mut last_gain: Option<f32> = None;
    loop {
        let (stop, fade) = {
            let c = ctrl.lock().unwrap_or_else(|e| e.into_inner());
            let f = match c.fade_start {
                Some(t) => 1.0 - (sdl::ticks().wrapping_sub(t)) as f32 / c.fade_ms as f32,
                None => 1.0,
            };
            (c.stop, f)
        };
        if stop || fade <= 0.0 {
            unsafe { (sdl::api().SDL_ClearQueuedAudio)(dev) };
            return;
        }
        let queued = unsafe { (sdl::api().SDL_GetQueuedAudioSize)(dev) };
        if finished {
            if queued == 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            continue;
        }
        // keep about 1/8 s queued, so a volume change is heard quickly
        if queued > bytes_per_sec / 8 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }
        // decode about 1/20 s at a time
        pcm.clear();
        while pcm.len() < (bytes_per_sec / 40) as usize {
            match dec.next_frames(&mut pcm) {
                Some(_) => {}
                None if loops != 0 => {
                    if loops > 0 {
                        loops -= 1;
                    }
                    dec.restart();
                }
                None => {
                    finished = true;
                    break;
                }
            }
        }
        if pcm.is_empty() {
            continue;
        }
        // the volume glides from the last chunk's to this one's (no clicks
        // while a slider is dragged)
        let gain = fade.min(1.0) * music_gain();
        let from = last_gain.unwrap_or(gain);
        last_gain = Some(gain);
        let frames = (pcm.len() / channels).max(1) as f32;
        buf.clear();
        buf.extend(pcm.iter().enumerate().map(|(i, &x)| {
            let g = from + (gain - from) * ((i / channels) as f32 / frames);
            (x * g * 32767.0).clamp(-32768.0, 32767.0) as i16
        }));
        unsafe {
            (sdl::api().SDL_QueueAudio)(dev, buf.as_ptr() as *const c_void, (buf.len() * 2) as u32);
        }
    }
}
