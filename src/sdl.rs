//! SDL2, loaded at run time.
//!
//! The game talks to SDL2 through a table of function pointers that is
//! filled in when the game starts (dlopen / LoadLibrary).  That way building
//! needs no SDL development files at all: only the SDL2 runtime library has
//! to be installed (libSDL2-2.0.so.0 on Linux, SDL2.dll on Windows,
//! libSDL2-2.0.0.dylib on macOS).
#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// dynamic library loading
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod dl {
    use std::ffi::{c_char, c_int, c_void};
    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    pub unsafe fn open(name: *const c_char) -> *mut c_void {
        dlopen(name, 2 /* RTLD_NOW */)
    }
    pub unsafe fn sym(lib: *mut c_void, name: *const c_char) -> *mut c_void {
        dlsym(lib, name)
    }
}

#[cfg(windows)]
mod dl {
    use std::ffi::{c_char, c_void};
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(lib: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    pub unsafe fn open(name: *const c_char) -> *mut c_void {
        LoadLibraryA(name)
    }
    pub unsafe fn sym(lib: *mut c_void, name: *const c_char) -> *mut c_void {
        GetProcAddress(lib, name)
    }
}

/// Open the first library of `names` that can be loaded.
pub fn open_library(names: &[&str]) -> Option<*mut c_void> {
    for n in names {
        let c = CString::new(*n).ok()?;
        let h = unsafe { dl::open(c.as_ptr()) };
        if !h.is_null() {
            return Some(h);
        }
    }
    None
}

pub fn symbol(lib: *mut c_void, name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    unsafe { dl::sym(lib, c.as_ptr()) }
}

#[cfg(target_os = "windows")]
const SDL_NAMES: &[&str] = &["SDL2.dll"];
#[cfg(target_os = "macos")]
const SDL_NAMES: &[&str] = &[
    "libSDL2-2.0.0.dylib",
    "libSDL2.dylib",
    "/opt/homebrew/lib/libSDL2-2.0.0.dylib",
    "/usr/local/lib/libSDL2-2.0.0.dylib",
    "/Library/Frameworks/SDL2.framework/SDL2",
];
#[cfg(all(unix, not(target_os = "macos")))]
const SDL_NAMES: &[&str] = &["libSDL2-2.0.so.0", "libSDL2-2.0.so", "libSDL2.so"];

// ---------------------------------------------------------------------------
// types and constants
// ---------------------------------------------------------------------------

pub type Window = c_void;
pub type Renderer = c_void;
pub type Texture = c_void;
pub type Surface = c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Rect {
    pub x: c_int,
    pub y: c_int,
    pub w: c_int,
    pub h: c_int,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct FPoint {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Vertex {
    pub position: FPoint,
    pub color: Color,
    pub tex_coord: FPoint,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DisplayMode {
    pub format: u32,
    pub w: c_int,
    pub h: c_int,
    pub refresh_rate: c_int,
    pub driverdata: *mut c_void,
}

#[repr(C)]
pub struct RendererInfo {
    pub name: *const c_char,
    pub flags: u32,
    pub num_texture_formats: u32,
    pub texture_formats: [u32; 16],
    pub max_texture_width: c_int,
    pub max_texture_height: c_int,
}

impl Default for DisplayMode {
    fn default() -> Self {
        DisplayMode { format: 0, w: 0, h: 0, refresh_rate: 0, driverdata: std::ptr::null_mut() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioSpec {
    pub freq: c_int,
    pub format: u16,
    pub channels: u8,
    pub silence: u8,
    pub samples: u16,
    pub padding: u16,
    pub size: u32,
    pub callback: *mut c_void,
    pub userdata: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

/// SDL_Event is a 56 byte union; fields are read by offset.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct Event {
    pub data: [u8; 56],
}

impl Event {
    pub fn zeroed() -> Event {
        Event { data: [0; 56] }
    }
    fn u32_at(&self, o: usize) -> u32 {
        u32::from_ne_bytes([self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3]])
    }
    fn i32_at(&self, o: usize) -> i32 {
        self.u32_at(o) as i32
    }
    pub fn kind(&self) -> u32 {
        self.u32_at(0)
    }
    // keyboard
    pub fn key_repeat(&self) -> bool {
        self.data[13] != 0
    }
    pub fn key_scancode(&self) -> i32 {
        self.i32_at(16)
    }
    pub fn key_sym(&self) -> i32 {
        self.i32_at(20)
    }
    pub fn key_mod(&self) -> u16 {
        u16::from_ne_bytes([self.data[24], self.data[25]])
    }
    // mouse buttons
    pub fn button(&self) -> u8 {
        self.data[16]
    }
    pub fn button_x(&self) -> i32 {
        self.i32_at(20)
    }
    pub fn button_y(&self) -> i32 {
        self.i32_at(24)
    }
    // mouse motion
    pub fn motion_x(&self) -> i32 {
        self.i32_at(20)
    }
    pub fn motion_y(&self) -> i32 {
        self.i32_at(24)
    }
    // mouse wheel
    pub fn wheel_y(&self) -> i32 {
        let y = self.i32_at(20);
        if self.u32_at(24) == 1 {
            -y // SDL_MOUSEWHEEL_FLIPPED
        } else {
            y
        }
    }
    // text input
    pub fn text(&self) -> String {
        let bytes = &self.data[12..44];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
    /// The file dropped on the window (DROPFILE).  Call once per event: it
    /// frees SDL's copy of the name.
    pub fn take_drop_file(&self) -> Option<String> {
        let p = usize::from_ne_bytes(self.data[8..8 + std::mem::size_of::<usize>()].try_into().ok()?) as *mut c_char;
        if p.is_null() {
            return None;
        }
        let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned();
        if let Some(free) = api().SDL_free {
            unsafe { free(p as *mut c_void) };
        }
        Some(s)
    }

    pub fn window_event(&self) -> u8 {
        self.data[12]
    }
}

pub const INIT_TIMER: u32 = 0x1;
pub const INIT_AUDIO: u32 = 0x10;
pub const INIT_VIDEO: u32 = 0x20;
pub const INIT_EVENTS: u32 = 0x4000;

pub const QUIT: u32 = 0x100;
/// a file dropped on the window
pub const DROPFILE: u32 = 0x1000;
pub const WINDOWEVENT: u32 = 0x200;
pub const KEYDOWN: u32 = 0x300;
pub const KEYUP: u32 = 0x301;
pub const TEXTINPUT: u32 = 0x303;
pub const MOUSEMOTION: u32 = 0x400;
pub const MOUSEBUTTONDOWN: u32 = 0x401;
pub const MOUSEBUTTONUP: u32 = 0x402;
pub const MOUSEWHEEL: u32 = 0x403;

pub const WINDOW_FULLSCREEN_DESKTOP: u32 = 0x1001;
pub const WINDOW_SHOWN: u32 = 0x4;
pub const WINDOWPOS_CENTERED: c_int = 0x2FFF0000;

pub const RENDERER_SOFTWARE: u32 = 0x1;
pub const RENDERER_ACCELERATED: u32 = 0x2;
pub const RENDERER_PRESENTVSYNC: u32 = 0x4;

pub const BLENDMODE_NONE: c_int = 0;
pub const BLENDMODE_BLEND: c_int = 1;

pub const TEXTUREACCESS_STATIC: c_int = 0;

#[cfg(target_endian = "little")]
pub const PIXELFORMAT_RGBA32: u32 = 0x16762004; // ABGR8888
#[cfg(target_endian = "big")]
pub const PIXELFORMAT_RGBA32: u32 = 0x16462004; // RGBA8888

pub const MESSAGEBOX_ERROR: u32 = 0x10;
pub const AUDIO_S16SYS: u16 = if cfg!(target_endian = "little") { 0x8010 } else { 0x9010 };

// keys (SDL_Keycode)
const SC: i32 = 1 << 30;
pub const K_BACKSPACE: i32 = 8;
pub const K_TAB: i32 = 9;
pub const K_RETURN: i32 = 13;
pub const K_ESCAPE: i32 = 27;
pub const K_SPACE: i32 = 32;
pub const K_COMMA: i32 = 44;
pub const K_PERIOD: i32 = 46;
pub const K_0: i32 = 48;
pub const K_1: i32 = 49;
pub const K_9: i32 = 57;
pub const K_A: i32 = 97;
pub const K_B: i32 = 98;
pub const K_C: i32 = 99;
pub const K_U: i32 = 117;
pub const K_O: i32 = 111;
pub const K_S: i32 = 115;
pub const K_V: i32 = 118;
pub const K_X: i32 = 120;
pub const K_N: i32 = 110;
pub const K_J: i32 = 106;
pub const K_P: i32 = 112;
pub const K_R: i32 = 114;
pub const K_Y: i32 = 121;
pub const K_INSERT: i32 = SC | 73;
pub const K_F3: i32 = SC | 60;
pub const K_F11: i32 = SC | 68;
pub const K_HOME: i32 = SC | 74;
pub const K_KP_ENTER: i32 = SC | 88;
pub const K_RIGHT: i32 = SC | 79;
pub const K_LEFT: i32 = SC | 80;
pub const K_MINUS: i32 = 45;
pub const K_EQUALS: i32 = 61;
pub const K_KP_MINUS: i32 = SC | 86;
pub const K_KP_PLUS: i32 = SC | 87;

// scancodes (SDL_GetKeyboardState)
pub const SCAN_A: usize = 4;
pub const SCAN_D: usize = 7;
pub const SCAN_S: usize = 22;
pub const SCAN_W: usize = 26;
pub const SCAN_BACKSPACE: usize = 42;
pub const SCAN_PAGEUP: usize = 75;
pub const SCAN_PAGEDOWN: usize = 78;
pub const SCAN_RIGHT: usize = 79;
pub const SCAN_LEFT: usize = 80;
pub const SCAN_DOWN: usize = 81;
pub const SCAN_UP: usize = 82;

pub const KMOD_SHIFT: u16 = 0x1 | 0x2;
pub const KMOD_CTRL: u16 = 0x40 | 0x80;
/// the Windows / Command key
pub const KMOD_GUI: u16 = 0x400 | 0x800;

// ---------------------------------------------------------------------------
// the function table
// ---------------------------------------------------------------------------

macro_rules! sdl_api {
    (
        required { $( fn $name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)? ; )* }
        optional { $( fn $oname:ident ( $($oarg:ident : $oty:ty),* ) $(-> $oret:ty)? ; )* }
    ) => {
        pub struct Api {
            $( pub $name: unsafe extern "C" fn($($ty),*) $(-> $ret)?, )*
            $( pub $oname: Option<unsafe extern "C" fn($($oty),*) $(-> $oret)?>, )*
        }
        impl Api {
            unsafe fn load(lib: *mut c_void) -> Result<Api, String> {
                Ok(Api {
                    $( $name: {
                        let p = symbol(lib, stringify!($name));
                        if p.is_null() {
                            return Err(format!("SDL2 is too old: {} is missing", stringify!($name)));
                        }
                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($ty),*) $(-> $ret)?>(p)
                    }, )*
                    $( $oname: {
                        let p = symbol(lib, stringify!($oname));
                        if p.is_null() { None } else {
                            Some(std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($oty),*) $(-> $oret)?>(p))
                        }
                    }, )*
                })
            }
        }
    };
}

sdl_api! {
    required {
        fn SDL_Init(flags: u32) -> c_int;
        fn SDL_InitSubSystem(flags: u32) -> c_int;
        fn SDL_Quit();
        fn SDL_GetError() -> *const c_char;
        fn SDL_SetHint(name: *const c_char, value: *const c_char) -> c_int;
        fn SDL_GetVersion(v: *mut Version);
        fn SDL_GetCurrentVideoDriver() -> *const c_char;
        fn SDL_CreateWindow(title: *const c_char, x: c_int, y: c_int, w: c_int, h: c_int, flags: u32) -> *mut Window;
        fn SDL_DestroyWindow(w: *mut Window);
        fn SDL_SetWindowFullscreen(w: *mut Window, flags: u32) -> c_int;
        fn SDL_SetWindowSize(w: *mut Window, width: c_int, height: c_int);
        fn SDL_SetWindowPosition(w: *mut Window, x: c_int, y: c_int);
        fn SDL_SetWindowTitle(w: *mut Window, title: *const c_char);
        fn SDL_SetWindowIcon(w: *mut Window, icon: *mut Surface);
        fn SDL_CreateRGBSurfaceWithFormatFrom(pixels: *mut c_void, width: c_int, height: c_int, depth: c_int, pitch: c_int, format: u32) -> *mut Surface;
        fn SDL_FreeSurface(s: *mut Surface);
        fn SDL_CreateRenderer(w: *mut Window, index: c_int, flags: u32) -> *mut Renderer;
        fn SDL_DestroyRenderer(r: *mut Renderer);
        fn SDL_RenderSetLogicalSize(r: *mut Renderer, w: c_int, h: c_int) -> c_int;
        fn SDL_SetRenderDrawColor(r: *mut Renderer, red: u8, g: u8, b: u8, a: u8) -> c_int;
        fn SDL_SetRenderDrawBlendMode(r: *mut Renderer, mode: c_int) -> c_int;
        fn SDL_RenderClear(r: *mut Renderer) -> c_int;
        fn SDL_RenderFillRect(r: *mut Renderer, rect: *const Rect) -> c_int;
        fn SDL_RenderDrawLine(r: *mut Renderer, x1: c_int, y1: c_int, x2: c_int, y2: c_int) -> c_int;
        fn SDL_RenderCopy(r: *mut Renderer, t: *mut Texture, src: *const Rect, dst: *const Rect) -> c_int;
        fn SDL_RenderCopyEx(r: *mut Renderer, t: *mut Texture, src: *const Rect, dst: *const Rect, angle: f64, center: *const c_void, flip: c_int) -> c_int;
        fn SDL_RenderPresent(r: *mut Renderer);
        fn SDL_RenderSetClipRect(r: *mut Renderer, rect: *const Rect) -> c_int;
        fn SDL_RenderReadPixels(r: *mut Renderer, rect: *const Rect, format: u32, pixels: *mut c_void, pitch: c_int) -> c_int;
        fn SDL_CreateTexture(r: *mut Renderer, format: u32, access: c_int, w: c_int, h: c_int) -> *mut Texture;
        fn SDL_DestroyTexture(t: *mut Texture);
        fn SDL_UpdateTexture(t: *mut Texture, rect: *const Rect, pixels: *const c_void, pitch: c_int) -> c_int;
        fn SDL_SetTextureColorMod(t: *mut Texture, r: u8, g: u8, b: u8) -> c_int;
        fn SDL_SetTextureAlphaMod(t: *mut Texture, a: u8) -> c_int;
        fn SDL_SetTextureBlendMode(t: *mut Texture, mode: c_int) -> c_int;
        fn SDL_PollEvent(e: *mut Event) -> c_int;
        fn SDL_GetTicks() -> u32;
        fn SDL_Delay(ms: u32);
        fn SDL_GetKeyboardState(numkeys: *mut c_int) -> *const u8;
        fn SDL_GetModState() -> u16;
        fn SDL_ShowCursor(toggle: c_int) -> c_int;
        fn SDL_StartTextInput();
        fn SDL_StopTextInput();
        fn SDL_GetNumDisplayModes(display: c_int) -> c_int;
        fn SDL_GetDisplayMode(display: c_int, index: c_int, mode: *mut DisplayMode) -> c_int;
        fn SDL_GetDesktopDisplayMode(display: c_int, mode: *mut DisplayMode) -> c_int;
        fn SDL_ShowSimpleMessageBox(flags: u32, title: *const c_char, message: *const c_char, window: *mut Window) -> c_int;
        fn SDL_OpenAudioDevice(device: *const c_char, iscapture: c_int, desired: *const AudioSpec, obtained: *mut AudioSpec, allowed_changes: c_int) -> u32;
        fn SDL_CloseAudioDevice(dev: u32);
        fn SDL_PauseAudioDevice(dev: u32, pause_on: c_int);
        fn SDL_QueueAudio(dev: u32, data: *const c_void, len: u32) -> c_int;
        fn SDL_GetQueuedAudioSize(dev: u32) -> u32;
        fn SDL_ClearQueuedAudio(dev: u32);
        fn SDL_GetPerformanceCounter() -> u64;
        fn SDL_GetPerformanceFrequency() -> u64;
    }
    optional {
        fn SDL_RenderGeometry(r: *mut Renderer, t: *mut Texture, vertices: *const Vertex, num_vertices: c_int, indices: *const c_int, num_indices: c_int) -> c_int;
        fn SDL_SetTextureScaleMode(t: *mut Texture, mode: c_int) -> c_int;
        fn SDL_GetRendererOutputSize(r: *mut Renderer, w: *mut c_int, h: *mut c_int) -> c_int;
        fn SDL_GetRendererInfo(r: *mut Renderer, info: *mut RendererInfo) -> c_int;
        fn SDL_SetClipboardText(text: *const c_char) -> c_int;
        fn SDL_GetClipboardText() -> *mut c_char;
        fn SDL_free(p: *mut c_void);
    }
}

static API: OnceLock<Api> = OnceLock::new();

/// Load SDL2.  Returns a readable error if the library is missing.
/// Where to look for SDL2: a copy shipped with the game (release packages)
/// and the system's.  On Linux the system's comes first (it is built for
/// that system); elsewhere the shipped one does.  SNOWBALLZ_SDL overrides.
fn library_candidates() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("SNOWBALLZ_SDL") {
        v.push(p);
    }
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut bundled = Vec::new();
    if let Some(d) = &exe_dir {
        let at = |rel: &str| d.join(rel).to_string_lossy().into_owned();
        if cfg!(target_os = "windows") {
            bundled.push(at("SDL2.dll"));
        } else if cfg!(target_os = "macos") {
            bundled.push(at("../Frameworks/SDL2.framework/SDL2"));
            bundled.push(at("../Frameworks/libSDL2-2.0.0.dylib"));
            bundled.push(at("libSDL2-2.0.0.dylib"));
        } else {
            bundled.push(at("lib/libSDL2-2.0.so.0"));
            bundled.push(at("libSDL2-2.0.so.0"));
        }
    }
    let system = SDL_NAMES.iter().map(|s| s.to_string());
    if cfg!(all(unix, not(target_os = "macos"))) {
        v.extend(system);
        v.extend(bundled);
    } else {
        v.extend(bundled);
        v.extend(system);
    }
    v
}

pub fn load() -> Result<(), String> {
    if API.get().is_some() {
        return Ok(());
    }
    let candidates = library_candidates();
    let names: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let lib = open_library(&names).ok_or_else(|| {
        "Could not load the SDL2 library. Please install SDL2 \
         (Debian/Ubuntu: sudo apt install libsdl2-2.0-0, \
         Fedora: sudo dnf install SDL2, Arch: sudo pacman -S sdl2, \
         macOS: brew install sdl2, Windows: put SDL2.dll next to the game)."
            .to_string()
    })?;
    let api = unsafe { Api::load(lib)? };
    let _ = API.set(api);
    Ok(())
}

/// Show an error when SDL itself could not be loaded (so SDL's message box
/// is not available).  Release builds have no console, so without this the
/// game would just not start.
pub fn native_error(title: &str, text: &str) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxA(hwnd: *mut c_void, text: *const c_char, caption: *const c_char, kind: u32) -> c_int;
        }
        let t = CString::new(text.replace('\0', "")).unwrap_or_default();
        let c = CString::new(title.replace('\0', "")).unwrap_or_default();
        unsafe {
            MessageBoxA(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x10 /* MB_ICONERROR */);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("display alert {:?} message {:?} as critical", title, text);
        let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::io::IsTerminal;
        if std::io::stderr().is_terminal() {
            return; // started from a terminal: the message is printed there
        }
        let tries: [(&str, Vec<String>); 3] = [
            ("zenity", vec!["--error".into(), format!("--title={}", title), format!("--text={}", text)]),
            ("kdialog", vec!["--title".into(), title.into(), "--error".into(), text.into()]),
            ("xmessage", vec![format!("{}: {}", title, text)]),
        ];
        for (cmd, args) in tries {
            if std::process::Command::new(cmd).args(&args).status().is_ok() {
                return;
            }
        }
    }
}

/// The SDL function table (call `load` first).
pub fn api() -> &'static Api {
    API.get().expect("SDL2 is not loaded")
}

pub fn is_loaded() -> bool {
    API.get().is_some()
}

pub fn error() -> String {
    unsafe {
        let p = (api().SDL_GetError)();
        if p.is_null() {
            String::new()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

pub fn set_hint(name: &str, value: &str) {
    let n = CString::new(name).unwrap();
    let v = CString::new(value).unwrap();
    unsafe {
        (api().SDL_SetHint)(n.as_ptr(), v.as_ptr());
    }
}

/// Put text on the system clipboard.
pub fn set_clipboard(text: &str) -> bool {
    let Some(f) = api().SDL_SetClipboardText else { return false };
    let t = CString::new(text.replace('\0', "")).unwrap_or_default();
    unsafe { f(t.as_ptr()) == 0 }
}

/// The text on the system clipboard ("" if none).
pub fn clipboard() -> String {
    let (Some(get), Some(free)) = (api().SDL_GetClipboardText, api().SDL_free) else { return String::new() };
    unsafe {
        let p = get();
        if p.is_null() {
            return String::new();
        }
        let t = CStr::from_ptr(p).to_string_lossy().into_owned();
        free(p as *mut c_void);
        t
    }
}

pub fn ticks() -> u32 {
    unsafe { (api().SDL_GetTicks)() }
}

pub fn delay(ms: u32) {
    unsafe { (api().SDL_Delay)(ms) }
}

pub fn version_string() -> String {
    let mut v = Version::default();
    unsafe { (api().SDL_GetVersion)(&mut v) };
    format!("{}.{}.{}", v.major, v.minor, v.patch)
}

/// The name of a renderer ("opengl", "direct3d", "software", ...).
pub fn renderer_name(r: *mut Renderer) -> String {
    let Some(f) = api().SDL_GetRendererInfo else { return "?".into() };
    unsafe {
        let mut info: RendererInfo = std::mem::zeroed();
        if r.is_null() || f(r, &mut info) != 0 || info.name.is_null() {
            return "?".into();
        }
        CStr::from_ptr(info.name).to_string_lossy().into_owned()
    }
}

pub fn video_driver() -> String {
    unsafe {
        let p = (api().SDL_GetCurrentVideoDriver)();
        if p.is_null() {
            "none".into()
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// Keyboard state indexed by scancode.
pub fn key_pressed(scancode: usize) -> bool {
    unsafe {
        let mut n: c_int = 0;
        let p = (api().SDL_GetKeyboardState)(&mut n);
        if p.is_null() || scancode >= n as usize {
            return false;
        }
        *p.add(scancode) != 0
    }
}

pub fn mod_state() -> u16 {
    unsafe { (api().SDL_GetModState)() }
}

pub fn poll_event() -> Option<Event> {
    let mut e = Event::zeroed();
    if unsafe { (api().SDL_PollEvent)(&mut e) } != 0 {
        Some(e)
    } else {
        None
    }
}

pub fn show_cursor(show: bool) {
    unsafe {
        (api().SDL_ShowCursor)(if show { 1 } else { 0 });
    }
}

pub fn message_box(title: &str, text: &str) {
    if !is_loaded() {
        return;
    }
    let t = CString::new(title.replace('\0', "")).unwrap();
    let m = CString::new(text.replace('\0', "")).unwrap();
    unsafe {
        (api().SDL_ShowSimpleMessageBox)(MESSAGEBOX_ERROR, t.as_ptr(), m.as_ptr(), std::ptr::null_mut());
    }
}

/// Display modes of the first display, biggest first, without duplicates.
pub fn display_modes() -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    unsafe {
        let n = (api().SDL_GetNumDisplayModes)(0);
        for i in 0..n.max(0) {
            let mut m = DisplayMode::default();
            if (api().SDL_GetDisplayMode)(0, i, &mut m) == 0 && !out.contains(&(m.w, m.h)) {
                out.push((m.w, m.h));
            }
        }
    }
    out
}
