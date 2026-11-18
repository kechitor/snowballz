//! Sound effects: short WAV files (data/sounds, made by
//! tools/sounds/make_sounds.py) mixed together by a small thread and
//! played over the music on an audio device of their own.
//!
//! A sound can have variants (name_1, name_2, ...): one is picked at
//! random and its pitch varied a little, so repeats don't sound alike.
//! Each sound plays only so many times at once.  Without an audio device
//! the game is simply silent.

use crate::sdl;
use std::collections::HashMap;
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const RATE: u32 = 44_100;
/// Voices at once (all sounds) ...
const MAX_VOICES: usize = 24;
/// ... and of one sound.
const MAX_SAME: usize = 4;
/// A sound doesn't start again sooner than this (ms).
const MIN_GAP_MS: u32 = 35;

/// The effects slider, in percent (the master slider is audio's).
static LEVEL: AtomicU32 = AtomicU32::new(100);
static FAILED: AtomicBool = AtomicBool::new(false);

pub fn set_level(percent: u8) {
    LEVEL.store(percent.min(100) as u32, Ordering::Relaxed);
}

fn gain() -> f32 {
    let e = LEVEL.load(Ordering::Relaxed) as f32 / 100.0;
    crate::audio::master() * e * e
}

fn enabled() -> bool {
    std::env::var("SNOWBALLZ_NO_SOUND").is_err()
}

struct Voice {
    name: &'static str,
    data: Arc<Vec<f32>>,
    pos: f64,
    step: f64,
    left: f32,
    right: f32,
}

struct Mixer {
    voices: Vec<Voice>,
    last_start: HashMap<&'static str, u32>,
}

static MIXER: OnceLock<Mutex<Mixer>> = OnceLock::new();
/// A sound's variants, as samples.
type Variants = Vec<Arc<Vec<f32>>>;
static BANK: OnceLock<Mutex<HashMap<&'static str, Variants>>> = OnceLock::new();

// -- WAV files ----------------------------------------------------------------------

/// A 16-bit PCM WAV file as mono samples at `RATE`.
pub fn decode_wav(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        let body = b.get(i + 8..(i + 8 + len).min(b.len()))?;
        if id == b"fmt " && body.len() >= 16 {
            let fmt = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
            bits = u16::from_le_bytes([body[14], body[15]]);
            if fmt != 1 && fmt != 0xfffe {
                return None;
            }
        } else if id == b"data" {
            if bits != 16 || channels == 0 || rate == 0 {
                return None;
            }
            let ch = channels as usize;
            let frames = body.len() / 2 / ch;
            let mono: Vec<f32> = (0..frames)
                .map(|f| (0..ch).map(|c| i16::from_le_bytes([body[(f * ch + c) * 2], body[(f * ch + c) * 2 + 1]]) as f32).sum::<f32>() / (ch as f32 * 32768.0))
                .collect();
            if rate == RATE {
                return Some(mono);
            }
            // (another rate: resample, linearly)
            let n = (frames as u64 * RATE as u64 / rate as u64) as usize;
            let k = rate as f64 / RATE as f64;
            return Some(
                (0..n)
                    .map(|j| {
                        let p = j as f64 * k;
                        let a = p.floor() as usize;
                        let t = (p - a as f64) as f32;
                        let x0 = mono[a.min(frames - 1)];
                        let x1 = mono[(a + 1).min(frames - 1)];
                        x0 + (x1 - x0) * t
                    })
                    .collect(),
            );
        }
        i += 8 + len + (len & 1);
    }
    None
}

/// The variants of a sound (data/sounds/<name>.wav, or <name>_1.wav,
/// <name>_2.wav, ...), loaded once.
fn variants(name: &'static str) -> Variants {
    let bank = BANK.get_or_init(|| Mutex::new(HashMap::new()));
    let mut bank = bank.lock().unwrap_or_else(|e| e.into_inner());
    bank.entry(name)
        .or_insert_with(|| {
            let mut v = Vec::new();
            let load = |p: String| std::fs::read(&p).ok().and_then(|b| decode_wav(&b)).map(Arc::new);
            if let Some(s) = load(format!("data/sounds/{}.wav", name)) {
                v.push(s);
            }
            for i in 1..10 {
                match load(format!("data/sounds/{}_{}.wav", name, i)) {
                    Some(s) => v.push(s),
                    None => break,
                }
            }
            if v.is_empty() {
                eprintln!("No sound {} in data/sounds", name);
            }
            v
        })
        .clone()
}

// -- playing ---------------------------------------------------------------------

fn mixer() -> Option<&'static Mutex<Mixer>> {
    if let Some(m) = MIXER.get() {
        return Some(m);
    }
    if FAILED.load(Ordering::Relaxed) || !enabled() || !sdl::is_loaded() {
        return None;
    }
    let dev = unsafe {
        if (sdl::api().SDL_InitSubSystem)(sdl::INIT_AUDIO) != 0 {
            FAILED.store(true, Ordering::Relaxed);
            return None;
        }
        let want = sdl::AudioSpec {
            freq: RATE as c_int,
            format: sdl::AUDIO_S16SYS,
            channels: 2,
            silence: 0,
            samples: 512,
            padding: 0,
            size: 0,
            callback: std::ptr::null_mut(),
            userdata: std::ptr::null_mut(),
        };
        let mut got = want;
        let dev = (sdl::api().SDL_OpenAudioDevice)(std::ptr::null(), 0, &want, &mut got, 0);
        if dev == 0 {
            eprintln!("Sound effects are off: could not open the audio device ({})", sdl::error());
            FAILED.store(true, Ordering::Relaxed);
            return None;
        }
        (sdl::api().SDL_PauseAudioDevice)(dev, 0);
        dev
    };
    let m = MIXER.get_or_init(|| Mutex::new(Mixer { voices: Vec::new(), last_start: HashMap::new() }));
    let _ = std::thread::Builder::new().name("sfx".into()).spawn(move || mix_loop(dev));
    Some(m)
}

/// Keep a little (about 1/30 s) mixed sound queued on the device.
fn mix_loop(dev: u32) {
    let chunk = (RATE / 100) as usize; // 10 ms
    let ahead = RATE * 4 / 30;
    let mut buf: Vec<i16> = Vec::with_capacity(chunk * 2);
    let mut acc = vec![0f32; chunk * 2];
    loop {
        let queued = unsafe { (sdl::api().SDL_GetQueuedAudioSize)(dev) };
        if queued > ahead {
            std::thread::sleep(std::time::Duration::from_millis(3));
            continue;
        }
        acc.iter_mut().for_each(|x| *x = 0.0);
        let g = gain();
        {
            let m = MIXER.get().unwrap();
            let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
            for v in m.voices.iter_mut() {
                for f in 0..chunk {
                    let p = v.pos as usize;
                    if p + 1 >= v.data.len() {
                        v.pos = v.data.len() as f64;
                        break;
                    }
                    let t = (v.pos - p as f64) as f32;
                    let s = v.data[p] + (v.data[p + 1] - v.data[p]) * t;
                    acc[f * 2] += s * v.left;
                    acc[f * 2 + 1] += s * v.right;
                    v.pos += v.step;
                }
            }
            m.voices.retain(|v| (v.pos as usize) + 1 < v.data.len());
        }
        buf.clear();
        // (a soft limit: many sounds at once don't crack)
        buf.extend(acc.iter().map(|&x| ((x * g).tanh() * 32000.0) as i16));
        unsafe { (sdl::api().SDL_QueueAudio)(dev, buf.as_ptr() as *const c_void, (buf.len() * 2) as u32) };
    }
}

/// Play a sound: `volume` 0..1, `pan` -1 (left) .. 1 (right).
pub fn play(name: &'static str, volume: f32, pan: f32) {
    if volume <= 0.01 {
        return;
    }
    let Some(m) = mixer() else { return };
    let v = variants(name);
    if v.is_empty() {
        return;
    }
    let now = sdl::ticks();
    let r = rand_u32();
    let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
    if m.last_start.get(name).is_some_and(|&t| now.wrapping_sub(t) < MIN_GAP_MS) {
        return;
    }
    if m.voices.len() >= MAX_VOICES || m.voices.iter().filter(|x| x.name == name).count() >= MAX_SAME {
        return;
    }
    m.last_start.insert(name, now);
    let data = v[r as usize % v.len()].clone();
    // a little higher or lower each time (up to about a semitone)
    let step = 1.0 + ((r >> 8) % 1000) as f64 / 1000.0 * 0.12 - 0.06;
    let pan = pan.clamp(-1.0, 1.0);
    let (left, right) = (volume * (1.0 - pan.max(0.0) * 0.6), volume * (1.0 + pan.min(0.0) * 0.6));
    m.voices.push(Voice { name, data, pos: 0.0, step, left, right });
}

fn rand_u32() -> u32 {
    static SEED: AtomicU32 = AtomicU32::new(0x9e37_79b9);
    let mut x = SEED.load(Ordering::Relaxed) ^ sdl::ticks().wrapping_mul(2_654_435_761);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    SEED.store(x, Ordering::Relaxed);
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data.len() as u32).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(channels.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * channels as u32 * 2).to_le_bytes());
        b.extend((channels * 2).to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        b
    }

    #[test]
    fn wav_files_decode() {
        let m = decode_wav(&wav(1, RATE, &[0, 16384, -16384])).unwrap();
        assert_eq!(m, vec![0.0, 0.5, -0.5]);
        // stereo is mixed down, other rates resampled
        let s = decode_wav(&wav(2, RATE, &[16384, 0, 16384, 16384])).unwrap();
        assert_eq!(s, vec![0.25, 0.5]);
        assert_eq!(decode_wav(&wav(1, 22_050, &[0; 100])).unwrap().len(), 200);
        assert!(decode_wav(b"not a wav").is_none());
        // and the game's own sounds are all there
        for name in ["alert_attack", "alert_igloo"] {
            assert!(!variants(name).is_empty(), "{}", name);
        }
    }
}
