# SnowballZ

A real-time strategy game in which you lead a colony of penguins in a
snowball fight.

![SnowballZ](wallpaper.png)

About the game
--------------
Each player starts with igloos full of penguins. **Snowballers** throw
snowballs at enemy penguins until they freeze, and take over enemy igloos.
**Workers** gather fish and crystal and build snow walls. Depending on the
map, you win by taking every enemy igloo, or by being the first to bring
home a number of loads of crystal or fish.

Play against the computer, or online with your friends.

Download and play
-----------------
Download the package for your system from the
[releases](../../releases) and unpack it:

| System | Package | Start |
|--------|---------|-------|
| Windows (64-bit) | `snowballz-<version>-windows-x64.zip` | double-click `snowballz.exe` |
| macOS (Apple Silicon and Intel, 10.13+) | `snowballz-<version>-macos.zip` | open `Snowballz.app` |
| Linux (x86-64, glibc 2.34+: Ubuntu 22.04 / Debian 12 / Fedora 35 or newer) | `snowballz-<version>-x86_64.AppImage` | make it executable (Properties, or `chmod +x`), then double-click |
| Linux, as a folder | `snowballz-<version>-linux-x64.tar.gz` | run `./snowballz` |

Everything the game needs is in the package. Each package has a `PLAY.txt`
with the details, such as the first-start warning Windows and macOS show
for unsigned programs, and adding the game to the Linux applications menu.

Run from source
---------------
You need **Rust** (from <https://rustup.rs>) and the **SDL2** library:

| System | Install |
|--------|---------|
| Debian / Ubuntu / Mint | `sudo apt install libsdl2-2.0-0` |
| Fedora | `sudo dnf install SDL2` |
| Arch | `sudo pacman -S sdl2` |
| macOS | `brew install sdl2` |
| Windows | put `SDL2.dll` (from <https://github.com/libsdl-org/SDL/releases>, an SDL **2**.x release, file `SDL2-<version>-win32-x64.zip`) in this folder |

On Windows, if `cargo` says *link.exe not found*, either install the *Build
Tools for Visual Studio* with "Desktop development with C++", or use Rust's
GNU toolchain for this folder: `rustup toolchain install
stable-x86_64-pc-windows-gnu` then `rustup override set
stable-x86_64-pc-windows-gnu`.

Then, in this folder:

    cargo run --release                          # the menu
    cargo run --release -- --single whiteforest  # skip the menu
    cargo run --release -- --server [map]        # host an online game
    cargo run --release -- --client <code or address>   # join one

How to play
-----------

### Loadout
Before a match you choose how your igloo's penguins are split between
snowballers and workers. The map decides how many penguins each igloo gets;
you decide the mix, from all workers to all snowballers.

* Single player: on the map page, with the **-** and **+** buttons under the
  map's description.
* Online: in the lobby, with the **-** / **+** buttons or the Left / Right
  arrow keys, until the host presses `S`. Every player chooses their own, and
  only sees their own.
* Your choice is remembered for each map. Each computer player gets a random
  mix.

### A match
Every match starts with a 3, 2, 1, Go! countdown: nobody moves until "Go!",
but you can look around and give orders already. Every player gets a random
team colour at the start of each match.

When one of your igloos is being taken, or your penguins are hit somewhere
you aren't looking, an alert shows at the top left with a warning sound.
Click it, or press `J`, to look there.

After the match, the winner / loser screen lists every player's numbers:
enemy penguins frozen, own penguins lost, snowballs thrown and direct hits,
loads of fish and crystal brought home, igloos taken, wall tiles built and
power-ups picked up.

### Power-ups
Every minute power-ups drop at random places (the countdown is at the top of
the screen). A snowballer picks one up by walking over it and keeps it until
it is frozen: then the thrower takes it, or it drops in the snow.

* **Rapid fire** (red diamond): makes snowballs three times as fast and keeps
  two in hand.
* **Range** (yellow circle): throws from further away, and its snowballs
  always find their target.
* **Defense** (blue shield): 4 more warmth, near misses don't hurt, direct
  hits take 1 warmth instead of 3, and it warms up by itself outside the
  igloo.

### Alliances
Press `P` in a match for the alliances panel: every other player, with an
**Offer alliance** button. An offer to you pops up at the top of the screen:
**Accept** (`Y`) or **Reject** (`N`); unanswered, it runs out after a minute.

* Accepting costs both sides 10 fish.
* Allies' penguins don't throw at each other, and allies can't take each
  other's igloos.
* Allying with a player who already has allies joins the whole alliance.
* Allies win together: on conquer maps when the alliance holds every igloo,
  on crystal maps when the alliance has the amount times its size.
* Computer players make and answer offers too, and hold grudges.

### Snow walls
Workers build walls out of snow bricks, one 8-tile **section** at a time.
Select some workers and press `B`: a section's outline follows the mouse
(green where it fits, red where it doesn't). `R` or the mouse wheel rotates
it; a click puts it down. Hold `Shift` to place several in a row; right
click (or `Escape`) stops.

* **Snow brick wall**: free. Each section takes 30 seconds of work, so two
  workers build it in 15 seconds and three in 10.
* **Ice wall**: select workers, press `U` and click a finished section of
  your snow wall. Each section costs 1 crystal and takes 15 seconds of work.
  Crystal counts towards a crystal map's goal, so ice walls are a trade-off
  there.
* A finished wall blocks the way for every penguin, yours too: leave a gap,
  or take a tile down by right-clicking it while planning with `B`. Walls
  can't go on trees, ice pinnacles, fish holes or crystal, or near an igloo
  door.
* An enemy snowball aimed at a penguin up to 2 tiles behind the wall hits
  the wall instead. A snow tile takes 8 hits, an ice tile 20. Right-click a
  battered tile with workers to patch it up. Your own side's snowballs fly
  over your walls.
* Snowballers knock down an enemy wall tile that is in their way, or the one
  you right-click with them selected.

### Computer players
Each computer player has a level: **Easy**, **Normal** or **Hard**. New games
use the level set in Settings; during a match the host (or the single
player) can change any of them in the players panel (`P`). They gather,
attack, defend their igloos and make alliances; Normal and Hard also pick up
power-ups, switch penguins between worker and snowballer, and build walls.

Controls
--------

| Input | Action |
|-------|--------|
| left drag | select penguins |
| right click | send the selected penguins there. What you click decides what they do: workers gather from a fish hole / crystal; snowballers chase an enemy penguin and throw at it until it is frozen; any penguin picks up a power-up or enters an igloo |
| middle click | same, running |
| right click the same place twice | same, running: they go there without stopping to fight |
| click a penguin type in the panel (bottom right, next to the minimap) | keep only that type selected; the arrow disbands them |
| `O` | disband the selected penguins |
| `P` | the alliances panel; `Y` / `N` accept / reject an alliance offer |
| `J` | look where the last alert happened; or click the alert |
| `B` (workers selected) | place an 8-tile snow wall section: `R` / mouse wheel rotates, click puts it down, `Shift` keeps placing; right click your wall to take it down |
| `U` (workers selected) | turn your snow wall into ice: click a section (1 crystal each) |
| right click a wall | workers: build / patch up / finish turning it into ice; snowballers: knock it down |
| `X` | change the class of the selected penguins (worker ⇄ snowballer): they go into their igloo and 10 seconds later come out as the other class; 1 fish each |
| `Ctrl`+`1`..`9` | make the selected penguins a squad |
| `Shift`+`1`..`9` | add the selected penguins to that squad |
| `1`..`9`, or click a squad's button (bottom left) | select that squad; press / click twice quickly to look at it |
| `Space` | hide / show the minimap: click or drag on it to move the view, right click to send the selected penguins there |
| mouse wheel / `Page Up` / `Page Down` | zoom |
| move the mouse to the screen edge / arrow keys / `W` `A` `S` `D` | scroll |
| `Home` | jump to your igloo |
| `Enter` | chat; `>name message` whispers to one player (`Tab` completes the name) |
| `Ctrl`+`V` (Mac: `Cmd`+`V`) | paste into the chat line |
| `Ctrl`+`C` | copy the newest chat message (while typing: copy the line you typed; `Ctrl`+`X` cuts it) |
| `.` / `,` | reply to the last whisper / whisper the last player again |
| `C` | clear the chat messages |
| `F3` | show / hide the FPS counter |
| `F11` | fullscreen on / off |
| `Escape` | the game menu: *Return to game*, *Settings*, *Quit game*; a single player game pauses while it is open |

Online games
------------
The computer that hosts a game is the server; nothing else is needed.

* **Host:** *Online -> Host a game*, pick a map. The lobby shows how friends
  can join, each with a **Copy** button:
  * **Game code** (like `KQ7-XM4`), when the game has a relay server.
    Friends type the code; nobody needs router settings.
  * **Internet**: your public address. The game asks your router to open
    TCP port **5555** for it.
  * **Same network**: for players at home / on the same Wi-Fi.
  * **IPv6**: shown when your computer has a public IPv6 address.

  Press `S` when everybody is in.
* **Join:** *Online -> Join a game*, type or paste the code or address and
  press `Enter`. Addresses can be `203.0.113.7`, `203.0.113.7:5556`,
  `[2001:db8::5]:5555` or a host name.
* **If the router can't open the port** and there is no game code, the
  lobby says so: forward TCP port 5555 to the address shown under *Same
  network* in the router's settings, and give friends your public address.
  If your internet provider puts several homes behind one public address,
  use a game code, IPv6, or a virtual-LAN program such as ZeroTier or
  Tailscale.
* The first time you host, Windows and macOS ask whether the game may accept
  connections: allow it. On Linux with a firewall (`ufw`, `firewalld`),
  allow TCP port 5555.
* A player who loses their last igloo keeps watching the game until somebody
  wins, or leaves with `Escape` -> *Quit game*.
* If a player leaves, the computer takes over their penguins and the game
  goes on. Someone joining a match that is already running gets a free igloo
  or one played by the computer.
* Everybody needs the same version of the game.
* `snowballz --server [map]` hosts without the menu (for example on a
  server); the code and addresses are printed in the terminal.

Map editor
----------
*Map editor* in the main menu makes new maps or changes existing ones.

* **New map** (80, 120 or 160 tiles square) or **Change a map**.
* Tools on the left: **Snow** / **Ice** (the ground), **Raise** / **Lower** /
  **Smooth** (hills; hold the button), **Trees**, **Pinnacles**, **Fish**,
  **Crystal**, **Eraser**, **Igloo** (one per player, at least 2), and the
  **Rapid fire** / **Range** / **Defense** power-ups. The left button paints,
  the right button removes things; `-` / `+` (or `[` `]`) change the brush.
* The mouse wheel zooms, the middle button or the arrow keys / `W` `A` `S` `D`
  move the view, `Home` shows the whole map, `Ctrl+Z` undoes, `Ctrl+S` saves.
* **Map settings**: the name and description, the victory (take every igloo,
  or be the first to gather a number of loads of crystal / fish), the
  penguins each igloo starts with, and the **Music**: the game's usual music,
  one of its tunes, or your own Ogg Vorbis (`.ogg`) file dragged onto the
  editor window (saved inside the map, up to 25 MB).
* **Import a map...** brings in a map `.zip` from your computer (or drag the
  `.zip` onto the editor's window). It is checked and copied into your maps
  folder, never replacing a map that is there.
* **X** next to one of your own maps deletes it, after you confirm.
* **Save** puts the map in your own maps folder (`~/.snowballz_maps`;
  Windows `%APPDATA%\SnowballZ\maps`): it appears in Single Play and when
  hosting.
* **Test play** saves and starts a single player game on the map.
* To play a custom map online, every player needs the map file: send it to
  your friends and they put it in their own maps folder.

Settings
--------
Settings are kept in `~/.snowballz.ini` (Windows: `%APPDATA%\SnowballZ.ini`).

* **Game scale**: `Auto` picks a zoom from your screen height; `1x`, `1.5x`,
  `2x` force one.
* **Resolution** and **Fullscreen** apply at once.
* **Volume**, **Music** and **Sound effects**: drag the sliders (or use the
  mouse wheel on them).
* **Computer players**: the level of new games' computer players.
* **Smooth scaling**, **Frame rate** (60, 35 = the original, unlimited),
  **Day/Night cycles**, **Smoke**, **Footprints**, **Snowball detail**,
  **Name**, **Ice Cursor**.

If the game crashes
-------------------
A crash writes a report to `~/.snowballz_crash.log` (Windows:
`%APPDATA%\SnowballZ_crash.log`) and shows a message box. Please send that
file with bug reports.

License
-------
GPL v2 or later, like the original game - see `LICENSE`.
