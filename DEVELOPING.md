Developing SnowballZ
====================

How to build and run the game is in `README.md`. This file is for working on
the game itself.

Building
--------
The game has **no Rust dependencies** (no crates): SDL2 is opened at run
time, and the PNG, JPEG, zip, TrueType and Ogg Vorbis decoders are part of
the game. `cargo build` works offline and needs no `-dev` packages.

The program is `target/release/snowballz`. It looks for `data/` and `maps/`
in the current folder, then next to the program and in the folders above
it, so you can start it from anywhere inside this folder. On Windows,
release builds open no console window (debug builds do).

Making releases
---------------
* **All three systems at once (GitHub):** `.github/workflows/release.yml`
  builds and tests the game on Linux, Windows and macOS and packages each
  with SDL2. Publish a release (*Releases -> Draft a new release*, tag such
  as `v0.9.6`) and the three packages are added to it after about 10
  minutes. You can also start it by hand (*Actions -> Release -> Run
  workflow*): give it the tag of an existing release to add (or refresh)
  the packages there, or leave the tag empty and download the packages from
  the run.
* **On your own computer:**
  * Linux: `bash tools/make-release.sh linux` (bundles your system's SDL2;
    also makes the AppImage if `appimagetool` is installed)
  * macOS: `bash tools/make-release.sh macos` (downloads SDL2.framework; run
    `rustup target add aarch64-apple-darwin x86_64-apple-darwin` first for
    an app that runs on both kinds of Mac)
  * Windows: `powershell -ExecutionPolicy Bypass -File tools\make-release.ps1`
    (uses `SDL2.dll` from this folder, or downloads it)

  The package lands in `dist/`.
* The version in the package names and in the game is the one in the
  `VERSION` file (it can have four numbers, like `0.9.8.1`); use the same
  number for the release tag (`v0.9.8.1`).

Game codes: the relay server
----------------------------
Game codes go through a **relay server**, a small program
(`snowballz-relay`, in `src/bin/`) on a computer the internet can reach,
for example a small cloud server. The host and the players all connect *to*
the relay, and it passes the game's traffic between them, so no router
needs any settings.

* Each release has `snowballz-relay-<version>-linux-x64.tar.gz` and
  `-linux-arm64.tar.gz` (static programs for any Linux). Their `README.txt`
  explains the setup: copy the program, start the included systemd service,
  and allow TCP port 5555 in the server's firewall.
* Then put the server's address (`host:port`) in the **`RELAY`** file and
  make a release: every copy of that release shows a game code when hosting.
  Without an address in `RELAY` the game has no codes (addresses work as
  usual). A player can also set or override it in the settings file:
  `[network]` `relay = host:port` (`off` turns codes off).
* The host and the players must use the same relay. One small relay serves
  many games; it keeps no files and logs no addresses.
* `cargo run --release --bin snowballz-relay -- --port 5555` runs one for
  testing (`SNOWBALLZ_RELAY=127.0.0.1:5555` points the game at it).
* Automatic port opening (UPnP) can be turned off with `upnp = 0` in the
  `[network]` section of the settings file.

How the game is built
---------------------
SnowballZ 0.9.5.1 was written in Python 2 with rabbyt / OpenGL; this version
is a port to Rust. Its modules, next to the Python ones:

| Python module | Rust module |
|---------------|-------------|
| `snowballz.py` (Main / ServerMain / ClientMain) | `main.rs`, `game.rs` |
| `gfx.py`, `textures.py`, `font.py` | `gfx.rs` (SDL's GPU renderer), `ttf.rs`, `image.rs`, `png.rs`, `jpeg.rs`, `inflate.rs` |
| `terrain.py` | `terrain.rs` (textured triangles, like the original OpenGL mesh) |
| `map.py`, `data.py`, `player.py`, `plugin.py` | `world.rs`, `zip.rs`, `ini.rs` |
| `unit.py` | `unit.rs` |
| `snowball.py` | `snowball.rs`, `effects.rs` |
| `buildings/*` | `igloo.rs` |
| `ai.py` | `ai.rs` |
| `networking.py` | `net.rs` (a reader and a writer thread per connection; IPv4 + IPv6) |
| - | `upnp.rs` (router port forwarding: SSDP + SOAP), `online.rs` (hosting status), `relay.rs` + `relay_common.rs` (game codes), `bin/snowballz-relay.rs` (the relay server) |
| `ctrl.py` | `ctrl.rs` |
| `display.py` | `display.rs` |
| `selectctrl.py`, `minimap.py` | `hud.rs` |
| `events.py`, `plugins/*` | `plugins.rs` (power-ups, conquer and gather victories) |
| `menu.py`, `ui.py`, `backgrounds.py` | `menu.rs`, `ui.rs`, `backgrounds.rs` |
| `settings.py`, `crashlog.py` | `settings.rs`, `crashlog.rs` |
| pygame.mixer | `audio.rs` + `vorbis.rs` (the game's own Ogg Vorbis decoder, SDL audio) |

* Drawing uses SDL's accelerated renderer (OpenGL / Direct3D / Metal). With
  SDL older than 2.0.18 the terrain is drawn as flat tiles (no elevation
  shading).
* The settings file is the one the Python version used. The network
  messages are the Python version's plus one for loadouts, so this version
  can't play with the Python one.
* The warning sounds are made by `tools/sounds/make_sounds.py` into
  `data/sounds/` (16-bit WAV files: they can be replaced).
* A crash report has the game, system, SDL version, video driver, renderer
  and the full backtrace. An error inside a single frame is logged and the
  game carries on; only a frame that keeps failing ends it.

Pictures
--------
The pictures are rendered from the original Blender files, which are from
Blender 2.42-2.45 and need Blender 2.79 (newer versions lose their
animations and materials). BLENDS_DIR is the folder with those files;
`crystalempty.blend`, `snowballerfrozen.blend`, `smallcrystalrush.blend` and
`whiteforest.blend` are made there from the originals when missing.

* The penguin sprite sheets, trees, ice pinnacles, igloo, crystals, frozen
  penguins (with their outfits in the team colour), markers and snow walls,
  at 4 times the old size; the fish holes, which have no Blender files, are
  painted pictures (`tools/sprites/painted/`):

      python3 tools/sprites/make_sprites.py BLENDS_DIR path/to/blender-2.79

* The full screen backgrounds (menu, loading, winner, loser), wide enough
  for 21:9, and the maps' pictures, into the map files:

      python3 tools/sprites/make_backgrounds.py BLENDS_DIR path/to/blender-2.79

* The game's icon (`icon.png` and the packages' icons in `tools/release/`):

      python3 tools/sprites/make_icon.py BLENDS_DIR path/to/blender-2.79

* The wallpaper at the top of `README.md` (`wallpaper.png`):

      python3 tools/sprites/make_wallpaper.py BLENDS_DIR path/to/blender-2.79

* The menus' and panels' pictures (buttons, text fields, the selection
  panel, the mouse cursor, the easel under a map's picture, the lobby's
  igloo):

      python3 tools/sprites/make_ui.py BLENDS_DIR path/to/blender-2.79

* Drawn without Blender: the ground's snow and ice (`make_terrain.py`), and
  the small effects - snowballs, imprints, footprints, smoke, snowflakes,
  the selection box, the frozen penguin's flag (`make_effects.py`):

      python3 tools/sprites/make_terrain.py
      python3 tools/sprites/make_effects.py

  and the power-ups (`make_powerups.py`, into `plugins/powerup/`):

      python3 tools/sprites/make_powerups.py

  and the pieces the in-game HUD's panels are drawn with (`make_hud.py`,
  into `data/hud/`; the drawing is in `src/glass.rs`):

      python3 tools/sprites/make_hud.py

  Run `make_terrain.py` before `make_ui.py` (the selection panel uses the
  ice).

The game draws each picture at its old size and uses halved copies when
zoomed out, so a picture at the old size still works. `SNOWBALLZ_LOOK=fx,fy[,zoom]`
points the view at a spot of the map (fractions of its size), and
`SNOWBALLZ_SELECT=1` selects your penguins (to see the command bar), for
screenshots with `SNOWBALLZ_SHOTS`.

Tests
-----

    cargo test --release

checks the decoders against every game image, map and music track (the Ogg
Vorbis decoder matches libvorbis to within float rounding), the font, that
the network messages are byte-for-byte the Python version's (plus the
loadout message), the router port forwarding against a pretend router,
IPv4 / IPv6 hosting, and the relay server (a host and a player talking
through it).

For automated play the game reads a few environment variables:
`SNOWBALLZ_AUTOPLAY=1` (the AI plays for you), `SNOWBALLZ_TIMESCALE=1000`
(10x game speed), `SNOWBALLZ_QUIT_AFTER=<ms>`, `SNOWBALLZ_STATS=<ms>`,
`SNOWBALLZ_SHOTS=<ms>:<file.png>,...`, `SNOWBALLZ_SKIP_END=1`,
`SNOWBALLZ_AUTOSTART=<ms>` (a server starts on its own),
`SNOWBALLZ_NO_MUSIC=1`, `SNOWBALLZ_CONFIG=<ini file>`, `SNOWBALLZ_NO_UPNP=1`,
`SNOWBALLZ_SSDP_ADDR=<ip:port>` / `SNOWBALLZ_UPNP_LOCATION=<url>` (talk to a
test router instead of the real one), `SNOWBALLZ_RELAY=<host:port>`.
