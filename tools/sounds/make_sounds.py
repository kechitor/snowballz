"""The game's sound effects, made from scratch (noise, tones and sweeps;
no recordings).  Writes data/sounds/*.wav (16-bit mono, 44.1 kHz).

    python3 tools/sounds/make_sounds.py

Needs numpy.  Any of the files can be replaced with your own (same name, a 16-bit PCM
WAV file, mono or stereo, any sample rate)."""
import os, wave
import numpy as np

SR = 44100
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'data', 'sounds')


def t_of(dur):
    return np.arange(int(SR * dur)) / SR


def env(t, attack, decay):
    """A quick rise, then an exponential fall."""
    a = np.clip(t / max(attack, 1e-4), 0, 1)
    return a * np.exp(-np.maximum(t - attack, 0) / decay)


def one_pole_lp(x, cutoff):
    a = np.exp(-2 * np.pi * cutoff / SR)
    y = np.empty_like(x)
    s = 0.0
    for i, v in enumerate(x):
        s = (1 - a) * v + a * s
        y[i] = s
    return y


def lp(x, cutoff, order=2):
    for _ in range(order):
        x = one_pole_lp(x, cutoff)
    return x


def hp(x, cutoff):
    return x - lp(x, cutoff, 1)


def bp(x, lo, hi):
    return lp(hp(x, lo), hi, 2)


def sweep(t, f0, f1, k=1.0):
    """A sine gliding from f0 to f1 (exponentially, speed k)."""
    f = f1 + (f0 - f1) * np.exp(-t * k * 30)
    return np.sin(2 * np.pi * np.cumsum(f) / SR)


def bell(t, f, decay, partials=((1, 1.0), (2.76, 0.35), (5.4, 0.18), (8.93, 0.08))):
    """A glassy, bell-like note (inharmonic partials; higher ones fade faster)."""
    y = np.zeros_like(t)
    for m, a in partials:
        y += a * np.sin(2 * np.pi * f * m * t) * np.exp(-t * m / (decay * 1.0) * 0.6)
    return y * np.clip(t / 0.002, 0, 1)


def place(dst, src, at):
    # (a short fade at the end: no click where a note is cut off)
    src = src.copy()
    k = min(len(src), int(0.012 * SR))
    if k:
        src[-k:] *= np.linspace(1, 0, k)
    i = int(at * SR)
    n = min(len(src), len(dst) - i)
    if n > 0:
        dst[i:i + n] += src[:n]


def finish(x, peak=0.85, fade=0.01):
    n = int(fade * SR)
    if n and len(x) > n:
        x[-n:] *= np.linspace(1, 0, n)
    m = np.max(np.abs(x)) or 1
    return x / m * peak


def save(name, x):
    os.makedirs(OUT, exist_ok=True)
    data = (np.clip(x, -1, 1) * 32767).astype('<i2').tobytes()
    with wave.open(os.path.join(OUT, name + '.wav'), 'wb') as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(SR)
        f.writeframes(data)
    print(name, '%.2fs' % (len(x) / SR))


# -- the sounds ---------------------------------------------------------------------

def alert_attack():
    """Your penguins are under attack: two quick falling tones."""
    out = np.zeros(int(0.42 * SR))
    for k, f in enumerate([880, 659]):
        tt = t_of(0.19)
        tone = (np.sin(2 * np.pi * f * tt) + 0.25 * np.sin(2 * np.pi * 3 * f * tt) + 0.1 * np.sin(2 * np.pi * 5 * f * tt)) * env(tt, 0.004, 0.09)
        place(out, tone, k * 0.16)
    return finish(out, 0.7, 0.03)


def alert_igloo():
    """Your igloo is being taken: three urgent horn-like pulses."""
    out = np.zeros(int(0.75 * SR))
    for k, f in enumerate([988, 988, 740]):
        tt = t_of(0.22)
        vib = 1 + 0.006 * np.sin(2 * np.pi * 7 * tt)
        ph = 2 * np.pi * np.cumsum(f * vib) / SR
        tone = (np.sin(ph) + 0.5 * np.sin(2 * ph) + 0.3 * np.sin(3 * ph) + 0.15 * np.sin(4 * ph))
        e = np.clip(tt / 0.01, 0, 1) * np.clip((0.2 - tt) / 0.04, 0, 1)
        place(out, lp(tone * e, 3500, 1), k * 0.17 + (0.08 if k == 2 else 0))
    return finish(out, 0.7, 0.03)


if __name__ == '__main__':
    save('alert_attack', alert_attack())
    save('alert_igloo', alert_igloo())
