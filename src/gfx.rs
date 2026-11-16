//! The drawing layer (what gfx.py was in the Python version), on top of
//! SDL's accelerated 2D renderer.
//!
//! * a drawing state: translate / scale stack, a current colour and two
//!   coordinate modes
//!     - world mode: y grows downwards, camera + zoom (`begin_world`)
//!     - ui mode:    y grows upwards from the bottom left (`begin_ui`)
//! * textures and sub-textures (sub-texture `y` is measured from the
//!   *bottom* of the image, like the old OpenGL code), tinted and faded by
//!   the GPU
//! * text, drawn from glyphs rasterised by `ttf`
//!
//! Everything runs on the main thread; the state lives in a thread local.

// The drawing API is kept complete (like gfx.py); not every helper is used.
#![allow(dead_code)]

use crate::image::Image;
use crate::sdl::{self, Rect};
use crate::ttf;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_void, CString};

/// A texture or a part of one.  `w`/`h` are its size; (`sx`, `sy`) its top
/// left corner inside the SDL texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tex {
    pub id: u32,
    pub sx: i32,
    pub sy: i32,
    pub w: i32,
    pub h: i32,
}

impl Tex {
    pub fn width(&self) -> f64 {
        self.w as f64
    }
    pub fn height(&self) -> f64 {
        self.h as f64
    }
    /// Sub-texture; `y` counts from the bottom of this texture.
    pub fn sub(&self, x: i32, y: i32, w: i32, h: i32) -> Tex {
        let top = self.h - y - h;
        Tex { id: self.id, sx: self.sx + x, sy: self.sy + top, w, h }
    }
    /// Draw with the current colour (top-left corner in world mode,
    /// bottom-left in ui mode).
    pub fn render(&self, x: f64, y: f64) {
        let c = get_color();
        draw_tex(self, x, y, c, 1.0);
    }
    pub fn draw(&self, x: f64, y: f64, color: [f32; 4], scale: f64) {
        draw_tex(self, x, y, color, scale);
    }
}

struct TexData {
    ptr: *mut sdl::Texture,
    w: i32,
    h: i32,
}

struct GlyphTex {
    tex: *mut sdl::Texture,
    w: i32,
    h: i32,
    left: i32,
    top: i32,
    advance: i32,
}

pub struct State {
    pub window: *mut sdl::Window,
    pub renderer: *mut sdl::Renderer,
    /// Logical screen size.
    pub w: i32,
    pub h: i32,
    yup: bool,
    tx: f64,
    ty: f64,
    sx: f64,
    sy: f64,
    stack: Vec<(f64, f64, f64, f64)>,
    color: [f32; 4],
    smooth: bool,
    pub time: u32,
    textures: Vec<TexData>,
    by_path: HashMap<String, Tex>,
    fonts: Vec<ttf::Font>,
    font_paths: HashMap<String, usize>,
    glyphs: HashMap<(usize, u32, char), GlyphTex>,
    pub mouse: (i32, i32),
    pub has_geometry: bool,
    pub fullscreen: bool,
}

thread_local! {
    static G: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    G.with(|g| f(g.borrow_mut().as_mut().expect("graphics are not initialised")))
}

pub fn is_init() -> bool {
    G.with(|g| g.borrow().is_some())
}

// ---------------------------------------------------------------------------
// window
// ---------------------------------------------------------------------------

/// Open the window (logical size `w` x `h`).
pub fn init(w: i32, h: i32, fullscreen: bool, title: &str) -> Result<(), String> {
    let api = sdl::api();
    sdl::set_hint("SDL_RENDER_SCALE_QUALITY", "1");
    let t = CString::new(title).unwrap();
    let mut flags = sdl::WINDOW_SHOWN;
    if fullscreen {
        flags |= sdl::WINDOW_FULLSCREEN_DESKTOP;
    }
    let window = unsafe {
        (api.SDL_CreateWindow)(t.as_ptr(), sdl::WINDOWPOS_CENTERED, sdl::WINDOWPOS_CENTERED, w, h, flags)
    };
    let window = if window.is_null() && fullscreen {
        // could not go fullscreen: use a window
        unsafe {
            (api.SDL_CreateWindow)(t.as_ptr(), sdl::WINDOWPOS_CENTERED, sdl::WINDOWPOS_CENTERED, w, h, sdl::WINDOW_SHOWN)
        }
    } else {
        window
    };
    if window.is_null() {
        return Err(format!("Could not open a window: {}", sdl::error()));
    }
    let mut renderer = unsafe { (api.SDL_CreateRenderer)(window, -1, sdl::RENDERER_ACCELERATED) };
    if renderer.is_null() {
        renderer = unsafe { (api.SDL_CreateRenderer)(window, -1, sdl::RENDERER_SOFTWARE) };
    }
    if renderer.is_null() {
        return Err(format!("Could not create a renderer: {}", sdl::error()));
    }
    unsafe {
        (api.SDL_RenderSetLogicalSize)(renderer, w, h);
    }
    let st = State {
        window,
        renderer,
        w,
        h,
        yup: true,
        tx: 0.0,
        ty: 0.0,
        sx: 1.0,
        sy: 1.0,
        stack: Vec::new(),
        color: [1.0; 4],
        smooth: true,
        time: 0,
        textures: Vec::new(),
        by_path: HashMap::new(),
        fonts: Vec::new(),
        font_paths: HashMap::new(),
        glyphs: HashMap::new(),
        mouse: (w / 2, h / 2),
        has_geometry: api.SDL_RenderGeometry.is_some(),
        fullscreen,
    };
    G.with(|g| *g.borrow_mut() = Some(st));
    if let Ok(icon) = Image::load("icon.png") {
        set_icon(&icon);
    }
    Ok(())
}

fn set_icon(img: &Image) {
    with(|s| unsafe {
        let mut px = img.px.clone();
        let surf = (sdl::api().SDL_CreateRGBSurfaceWithFormatFrom)(
            px.as_mut_ptr() as *mut c_void,
            img.w as i32,
            img.h as i32,
            32,
            (img.w * 4) as i32,
            sdl::PIXELFORMAT_RGBA32,
        );
        if !surf.is_null() {
            (sdl::api().SDL_SetWindowIcon)(s.window, surf);
            (sdl::api().SDL_FreeSurface)(surf);
        }
    })
}

/// The SDL renderer in use (for bug reports).
pub fn renderer_name() -> String {
    if !is_init() {
        return "none".into();
    }
    with(|s| sdl::renderer_name(s.renderer))
}

pub fn screen_size() -> (i32, i32) {
    with(|s| (s.w, s.h))
}

pub fn set_fullscreen(on: bool) -> bool {
    with(|s| unsafe {
        let ok = (sdl::api().SDL_SetWindowFullscreen)(s.window, if on { sdl::WINDOW_FULLSCREEN_DESKTOP } else { 0 }) == 0;
        if ok {
            s.fullscreen = on;
            if !on {
                (sdl::api().SDL_SetWindowSize)(s.window, s.w, s.h);
                (sdl::api().SDL_SetWindowPosition)(s.window, sdl::WINDOWPOS_CENTERED, sdl::WINDOWPOS_CENTERED);
            }
        }
        ok
    })
}

/// Change the game's resolution (the window size, or the size that is
/// scaled to the monitor in fullscreen).
pub fn set_resolution(w: i32, h: i32) {
    with(|s| unsafe {
        s.w = w;
        s.h = h;
        if !s.fullscreen {
            (sdl::api().SDL_SetWindowSize)(s.window, w, h);
            (sdl::api().SDL_SetWindowPosition)(s.window, sdl::WINDOWPOS_CENTERED, sdl::WINDOWPOS_CENTERED);
        }
        (sdl::api().SDL_RenderSetLogicalSize)(s.renderer, w, h);
    })
}

pub fn set_title(title: &str) {
    let t = CString::new(title).unwrap();
    with(|s| unsafe { (sdl::api().SDL_SetWindowTitle)(s.window, t.as_ptr()) })
}

static PRESENT_HOOK: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Save the next frame to this PNG file just before it is shown (tests).
pub fn set_present_hook(path: Option<String>) {
    *PRESENT_HOOK.lock().unwrap() = path;
}

pub fn present() {
    let hook = PRESENT_HOOK.lock().unwrap().take();
    if let Some(path) = hook {
        match screenshot(&path) {
            Ok(()) => println!("screenshot {}", path),
            Err(e) => eprintln!("screenshot {}: {}", path, e),
        }
    }
    with(|s| unsafe { (sdl::api().SDL_RenderPresent)(s.renderer) })
}

pub fn mouse_pos() -> (i32, i32) {
    with(|s| s.mouse)
}

/// Poll every pending event (and keep track of the mouse position, in
/// logical coordinates).
pub fn events() -> Vec<sdl::Event> {
    let mut out = Vec::new();
    while let Some(e) = sdl::poll_event() {
        match e.kind() {
            sdl::MOUSEMOTION => with(|s| s.mouse = (e.motion_x(), e.motion_y())),
            sdl::MOUSEBUTTONDOWN | sdl::MOUSEBUTTONUP => with(|s| s.mouse = (e.button_x(), e.button_y())),
            _ => {}
        }
        out.push(e);
    }
    out
}

/// Save what has been drawn so far as a PNG (for tests and bug reports).
pub fn screenshot(path: &str) -> Result<(), String> {
    let (w, h) = with(|s| unsafe {
        let mut ow = 0;
        let mut oh = 0;
        get_output_size(s, &mut ow, &mut oh);
        (ow, oh)
    });
    let mut img = Image::new(w as usize, h as usize);
    let r = with(|s| unsafe {
        (sdl::api().SDL_RenderReadPixels)(
            s.renderer,
            std::ptr::null(),
            sdl::PIXELFORMAT_RGBA32,
            img.px.as_mut_ptr() as *mut c_void,
            w * 4,
        )
    });
    if r != 0 {
        return Err(sdl::error());
    }
    for p in img.px.chunks_mut(4) {
        p[3] = 255;
    }
    std::fs::write(path, crate::png::encode(&img)).map_err(|e| e.to_string())
}

unsafe fn get_output_size(s: &State, w: &mut i32, h: &mut i32) {
    // The read-back is in output pixels: in fullscreen (or on a HiDPI
    // screen) that is the monitor's size, not the game's logical size.
    *w = s.w;
    *h = s.h;
    if let Some(f) = sdl::api().SDL_GetRendererOutputSize {
        let (mut ow, mut oh) = (0, 0);
        if f(s.renderer, &mut ow, &mut oh) == 0 && ow > 0 && oh > 0 {
            *w = ow;
            *h = oh;
        }
    }
}

// ---------------------------------------------------------------------------
// drawing state
// ---------------------------------------------------------------------------

pub fn set_time(t: u32) {
    with(|s| s.time = t)
}

pub fn time() -> u32 {
    with(|s| s.time)
}

pub fn begin_world(cam_x: f64, cam_y: f64, zoom: f64) {
    with(|s| {
        s.yup = false;
        s.tx = -cam_x * zoom;
        s.ty = -cam_y * zoom;
        s.sx = zoom;
        s.sy = zoom;
        s.stack.clear();
    })
}

pub fn begin_ui() {
    with(|s| {
        s.yup = true;
        s.tx = 0.0;
        s.ty = 0.0;
        s.sx = 1.0;
        s.sy = 1.0;
        s.stack.clear();
    })
}

pub fn world_to_screen_x(x: f64) -> i32 {
    with(|s| (s.tx + s.sx * x + 0.5).floor() as i32)
}

pub fn world_to_screen_y(y: f64) -> i32 {
    with(|s| (s.ty + s.sy * y + 0.5).floor() as i32)
}

pub fn push() {
    with(|s| {
        let t = (s.tx, s.ty, s.sx, s.sy);
        s.stack.push(t)
    })
}

pub fn pop() {
    with(|s| {
        if let Some((a, b, c, d)) = s.stack.pop() {
            s.tx = a;
            s.ty = b;
            s.sx = c;
            s.sy = d;
        }
    })
}

pub fn translate(x: f64, y: f64) {
    with(|s| {
        s.tx += s.sx * x;
        s.ty += s.sy * y;
    })
}

pub fn scale(sx: f64, sy: f64) {
    with(|s| {
        s.sx *= sx;
        s.sy *= sy;
    })
}

pub fn color4f(r: f32, g: f32, b: f32, a: f32) {
    with(|s| s.color = [r, g, b, a])
}

pub fn color3f(r: f32, g: f32, b: f32) {
    color4f(r, g, b, 1.0)
}

pub fn get_color() -> [f32; 4] {
    with(|s| s.color)
}

pub fn set_smooth(flag: bool) {
    with(|s| {
        s.smooth = flag;
        if let Some(f) = sdl::api().SDL_SetTextureScaleMode {
            for t in &s.textures {
                unsafe {
                    f(t.ptr, if flag { 1 } else { 0 });
                }
            }
        }
    })
}

pub fn c8(c: f32) -> u8 {
    (c * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

pub fn clear(rgb: (u8, u8, u8)) {
    with(|s| unsafe {
        (sdl::api().SDL_SetRenderDrawColor)(s.renderer, rgb.0, rgb.1, rgb.2, 255);
        (sdl::api().SDL_RenderClear)(s.renderer);
    })
}

/// A rect in the current space -> screen pixels (left, top, w, h).  The
/// size is rounded on its own so a moving sprite keeps a constant size.
fn rect_of(s: &State, x: f64, y: f64, w: f64, h: f64) -> Rect {
    let ww = ((w * s.sx.abs()) + 0.5).floor().max(0.0) as i32;
    let hh = ((h * s.sy.abs()) + 0.5).floor().max(0.0) as i32;
    let left = s.tx + s.sx * x;
    let gy = s.ty + s.sy * y;
    let l = (left + 0.5).floor() as i32;
    let t = if s.yup { (s.h as f64 - gy + 0.5).floor() as i32 - hh } else { (gy + 0.5).floor() as i32 };
    Rect::new(l, t, ww, hh)
}

fn point_of(s: &State, x: f64, y: f64) -> (i32, i32) {
    let px = s.tx + s.sx * x;
    let mut py = s.ty + s.sy * y;
    if s.yup {
        py = s.h as f64 - py;
    }
    ((px + 0.5).floor() as i32, (py + 0.5).floor() as i32)
}

/// Fill a rectangle with `color` (or the current colour); negative sizes
/// are allowed.
pub fn fill_rect(mut x: f64, mut y: f64, mut w: f64, mut h: f64, color: Option<[f32; 4]>) {
    if w < 0.0 {
        x += w;
        w = -w;
    }
    if h < 0.0 {
        y += h;
        h = -h;
    }
    with(|s| {
        let c = color.unwrap_or(s.color);
        if c[3] <= 0.0 {
            return;
        }
        let r = rect_of(s, x, y, w, h);
        if r.w <= 0 || r.h <= 0 {
            return;
        }
        unsafe {
            (sdl::api().SDL_SetRenderDrawBlendMode)(s.renderer, sdl::BLENDMODE_BLEND);
            (sdl::api().SDL_SetRenderDrawColor)(s.renderer, c8(c[0]), c8(c[1]), c8(c[2]), c8(c[3].min(1.0)));
            (sdl::api().SDL_RenderFillRect)(s.renderer, &r);
        }
    })
}

pub fn draw_box(x: f64, y: f64, w: f64, h: f64) {
    fill_rect(x, y, w, h, None)
}

/// Fill a rectangle given in screen pixels (y down).
pub fn fill_screen_rect(r: Rect, rgba: (u8, u8, u8, u8)) {
    with(|s| unsafe {
        (sdl::api().SDL_SetRenderDrawBlendMode)(s.renderer, sdl::BLENDMODE_BLEND);
        (sdl::api().SDL_SetRenderDrawColor)(s.renderer, rgba.0, rgba.1, rgba.2, rgba.3);
        (sdl::api().SDL_RenderFillRect)(s.renderer, &r);
    })
}

pub fn draw_line(x1: f64, y1: f64, x2: f64, y2: f64, color: Option<[f32; 4]>) {
    with(|s| {
        let c = color.unwrap_or(s.color);
        let (ax, ay) = point_of(s, x1, y1);
        let (bx, by) = point_of(s, x2, y2);
        unsafe {
            (sdl::api().SDL_SetRenderDrawBlendMode)(s.renderer, sdl::BLENDMODE_BLEND);
            (sdl::api().SDL_SetRenderDrawColor)(s.renderer, c8(c[0]), c8(c[1]), c8(c[2]), c8(c[3]));
            (sdl::api().SDL_RenderDrawLine)(s.renderer, ax, ay, bx, by);
        }
    })
}

/// Square dots of `size` pixels centred on each point.
pub fn draw_points(points: &[(f64, f64)], color: [f32; 4], size: i32) {
    with(|s| unsafe {
        (sdl::api().SDL_SetRenderDrawBlendMode)(s.renderer, sdl::BLENDMODE_BLEND);
        (sdl::api().SDL_SetRenderDrawColor)(s.renderer, c8(color[0]), c8(color[1]), c8(color[2]), c8(color[3]));
        for &(x, y) in points {
            let (px, py) = point_of(s, x, y);
            let r = Rect::new(px - size / 2, py - size / 2, size, size);
            (sdl::api().SDL_RenderFillRect)(s.renderer, &r);
        }
    })
}

/// Limit drawing to a screen rectangle (None: no limit).
pub fn set_clip(r: Option<Rect>) {
    with(|s| unsafe {
        match r {
            Some(r) => (sdl::api().SDL_RenderSetClipRect)(s.renderer, &r),
            None => (sdl::api().SDL_RenderSetClipRect)(s.renderer, std::ptr::null()),
        };
    })
}

// ---------------------------------------------------------------------------
// textures
// ---------------------------------------------------------------------------

fn create_texture(s: &mut State, img: &Image) -> Result<u32, String> {
    let api = sdl::api();
    unsafe {
        let t = (api.SDL_CreateTexture)(
            s.renderer,
            sdl::PIXELFORMAT_RGBA32,
            sdl::TEXTUREACCESS_STATIC,
            img.w as i32,
            img.h as i32,
        );
        if t.is_null() {
            return Err(format!("could not create a {}x{} texture: {}", img.w, img.h, sdl::error()));
        }
        (api.SDL_UpdateTexture)(t, std::ptr::null(), img.px.as_ptr() as *const c_void, (img.w * 4) as i32);
        (api.SDL_SetTextureBlendMode)(t, sdl::BLENDMODE_BLEND);
        if let Some(f) = api.SDL_SetTextureScaleMode {
            f(t, if s.smooth { 1 } else { 0 });
        }
        s.textures.push(TexData { ptr: t, w: img.w as i32, h: img.h as i32 });
    }
    Ok((s.textures.len() - 1) as u32)
}

/// Make a texture from an image (exact size).
pub fn texture_from_image(img: &Image) -> Result<Tex, String> {
    with(|s| {
        let id = create_texture(s, img)?;
        Ok(Tex { id, sx: 0, sy: 0, w: img.w as i32, h: img.h as i32 })
    })
}

/// Replace the pixels of a whole texture (same size).
pub fn update_texture(t: &Tex, img: &Image) {
    with(|s| unsafe {
        let d = &s.textures[t.id as usize];
        if d.w == img.w as i32 && d.h == img.h as i32 {
            (sdl::api().SDL_UpdateTexture)(d.ptr, std::ptr::null(), img.px.as_ptr() as *const c_void, (img.w * 4) as i32);
        }
    })
}

/// Load (once) a game texture.  Like the old OpenGL textures it is padded
/// to a power of two size, with the picture in the bottom left corner.
pub fn texture(path: &str) -> Tex {
    if let Some(t) = with(|s| s.by_path.get(path).copied()) {
        return t;
    }
    let img = match Image::load(path) {
        Ok(i) => i.pad_pow2(),
        Err(e) => {
            eprintln!("Could not load {}: {}", path, e);
            let mut i = Image::new(32, 32);
            for p in i.px.chunks_mut(4) {
                p.copy_from_slice(&[255, 0, 255, 255]);
            }
            i
        }
    };
    let t = texture_from_image(&img).expect("texture");
    with(|s| s.by_path.insert(path.to_string(), t));
    t
}

/// Load (once) an image at its own size (menu pictures, backgrounds).
pub fn image_texture(path: &str) -> Result<Tex, String> {
    let key = format!("raw:{}", path);
    if let Some(t) = with(|s| s.by_path.get(&key).copied()) {
        return Ok(t);
    }
    let img = Image::load(path)?;
    let t = texture_from_image(&img)?;
    with(|s| s.by_path.insert(key, t));
    Ok(t)
}

pub fn free_texture(t: &Tex) {
    with(|s| {
        let d = &mut s.textures[t.id as usize];
        if !d.ptr.is_null() {
            unsafe { (sdl::api().SDL_DestroyTexture)(d.ptr) };
            d.ptr = std::ptr::null_mut();
        }
        s.by_path.retain(|_, v| v.id != t.id);
    })
}

fn tex_ptr(s: &State, t: &Tex) -> *mut sdl::Texture {
    s.textures.get(t.id as usize).map(|d| d.ptr).unwrap_or(std::ptr::null_mut())
}

/// Draw `t` with its top-left (world) / bottom-left (ui) corner at x, y;
/// `color` multiplies the image.
pub fn draw_tex(t: &Tex, x: f64, y: f64, color: [f32; 4], scale: f64) {
    if color[3] <= 0.0 {
        return;
    }
    with(|s| {
        let r = rect_of(s, x, y, t.w as f64 * scale, t.h as f64 * scale);
        if r.w <= 0 || r.h <= 0 || r.x > s.w || r.y > s.h || r.x + r.w < 0 || r.y + r.h < 0 {
            return;
        }
        let p = tex_ptr(s, t);
        if p.is_null() {
            return;
        }
        let src = Rect::new(t.sx, t.sy, t.w, t.h);
        unsafe {
            let api = sdl::api();
            (api.SDL_SetTextureColorMod)(p, c8(color[0]), c8(color[1]), c8(color[2]));
            (api.SDL_SetTextureAlphaMod)(p, c8(color[3].min(1.0)));
            (api.SDL_RenderCopy)(s.renderer, p, &src, &r);
        }
    })
}

/// Draw a texture into a screen rectangle (y down), no transform.
pub fn blit(t: &Tex, src: Option<Rect>, dst: Rect, alpha: f32) {
    with(|s| {
        let p = tex_ptr(s, t);
        if p.is_null() || dst.w <= 0 || dst.h <= 0 {
            return;
        }
        let src = src.unwrap_or(Rect::new(t.sx, t.sy, t.w, t.h));
        unsafe {
            let api = sdl::api();
            (api.SDL_SetTextureColorMod)(p, 255, 255, 255);
            (api.SDL_SetTextureAlphaMod)(p, c8(alpha));
            (api.SDL_RenderCopy)(s.renderer, p, &src, &dst);
        }
    })
}

/// Textured triangles (screen coordinates); returns false if the SDL
/// version can't draw them.
pub fn geometry(t: Option<&Tex>, verts: &[sdl::Vertex], indices: &[i32]) -> bool {
    with(|s| {
        let Some(f) = sdl::api().SDL_RenderGeometry else { return false };
        let p = t.map(|t| tex_ptr(s, t)).unwrap_or(std::ptr::null_mut());
        if let Some(_t) = t {
            unsafe {
                (sdl::api().SDL_SetTextureColorMod)(p, 255, 255, 255);
                (sdl::api().SDL_SetTextureAlphaMod)(p, 255);
            }
        }
        unsafe {
            f(s.renderer, p, verts.as_ptr(), verts.len() as i32, indices.as_ptr(), indices.len() as i32) == 0
        }
    })
}

/// Current transform (tx, ty, sx, sy) - for code that does its own maths.
pub fn transform() -> (f64, f64, f64, f64) {
    with(|s| (s.tx, s.ty, s.sx, s.sy))
}

pub fn texture_size(t: &Tex) -> (i32, i32) {
    with(|s| s.textures.get(t.id as usize).map(|d| (d.w, d.h)).unwrap_or((0, 0)))
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/// A font at one pixel size (the old `Font(facename, size)`).
#[derive(Clone, Copy, Debug)]
pub struct Font {
    pub face: usize,
    pub size: u32,
}

pub fn load_font(path: &str, size: u32) -> Result<Font, String> {
    if let Some(&face) = with(|s| s.font_paths.get(path).copied()).as_ref() {
        return Ok(Font { face, size });
    }
    let f = ttf::Font::load(path)?;
    let face = with(|s| {
        s.fonts.push(f);
        s.font_paths.insert(path.to_string(), s.fonts.len() - 1);
        s.fonts.len() - 1
    });
    Ok(Font { face, size })
}

impl Font {
    pub fn with_size(&self, size: u32) -> Font {
        Font { face: self.face, size }
    }

    pub fn metrics(&self) -> ttf::Metrics {
        with(|s| s.fonts[self.face].metrics(self.size as f32))
    }

    /// Size of the rendered text (width, height), like pygame's font.size.
    pub fn size_of(&self, text: &str) -> (i32, i32) {
        with(|s| {
            let f = &s.fonts[self.face];
            let m = f.metrics(self.size as f32);
            let lines: Vec<&str> = text.split('\n').collect();
            let w = lines.iter().map(|l| f.text_width(self.size as f32, l)).max().unwrap_or(0);
            let h = if lines.len() == 1 { m.height } else { m.line_skip * lines.len() as i32 };
            (w, h)
        })
    }

    /// Draw text with its top-left corner at screen pixel (px, py).
    pub fn draw_screen(&self, px: i32, py: i32, text: &str, color: [f32; 4]) {
        if text.is_empty() || color[3] <= 0.0 {
            return;
        }
        with(|s| {
            let m = s.fonts[self.face].metrics(self.size as f32);
            let (r, g, b, a) = (c8(color[0]), c8(color[1]), c8(color[2]), c8(color[3].min(1.0)));
            for (li, line) in text.split('\n').enumerate() {
                let mut pen = 0;
                let base = py + li as i32 * m.line_skip + m.ascent;
                for ch in line.chars() {
                    let key = (self.face, self.size, ch);
                    if !s.glyphs.contains_key(&key) {
                        let gt = make_glyph(s, self.face, self.size, ch);
                        s.glyphs.insert(key, gt);
                    }
                    let gt = &s.glyphs[&key];
                    if !gt.tex.is_null() {
                        let dst = Rect::new(px + pen + gt.left, base + gt.top, gt.w, gt.h);
                        unsafe {
                            let api = sdl::api();
                            (api.SDL_SetTextureColorMod)(gt.tex, r, g, b);
                            (api.SDL_SetTextureAlphaMod)(gt.tex, a);
                            (api.SDL_RenderCopy)(s.renderer, gt.tex, std::ptr::null(), &dst);
                        }
                    }
                    pen += gt.advance;
                }
            }
        })
    }

    /// The old Font.render: in ui mode `y` is the bottom of the text, in
    /// world mode its top.
    pub fn render(&self, x: f64, y: f64, text: &str, color: [f32; 4]) {
        if text.is_empty() || color[3] <= 0.0 {
            return;
        }
        let (_, h) = self.size_of(text);
        let (px, py) = with(|s| {
            let px = (s.tx + s.sx * x + 0.5).floor() as i32;
            let gy = s.ty + s.sy * y;
            let py = if s.yup { (s.h as f64 - gy - h as f64 + 0.5).floor() as i32 } else { (gy + 0.5).floor() as i32 };
            (px, py)
        });
        self.draw_screen(px, py, text, color);
    }

    pub fn render3(&self, x: f64, y: f64, text: &str, c: [f32; 3]) {
        self.render(x, y, text, [c[0], c[1], c[2], 1.0])
    }
}

fn make_glyph(s: &mut State, face: usize, size: u32, ch: char) -> GlyphTex {
    let f = &s.fonts[face];
    let g = f.rasterize(f.glyph_index(ch), f.scale_for(size as f32));
    let mut tex = std::ptr::null_mut();
    if g.w > 0 && g.h > 0 {
        let mut img = Image::new(g.w, g.h);
        for (i, c) in g.coverage.iter().enumerate() {
            img.px[i * 4..i * 4 + 4].copy_from_slice(&[255, 255, 255, *c]);
        }
        unsafe {
            let api = sdl::api();
            tex = (api.SDL_CreateTexture)(s.renderer, sdl::PIXELFORMAT_RGBA32, sdl::TEXTUREACCESS_STATIC, g.w as i32, g.h as i32);
            if !tex.is_null() {
                (api.SDL_UpdateTexture)(tex, std::ptr::null(), img.px.as_ptr() as *const c_void, (g.w * 4) as i32);
                (api.SDL_SetTextureBlendMode)(tex, sdl::BLENDMODE_BLEND);
            }
        }
    }
    GlyphTex { tex, w: g.w as i32, h: g.h as i32, left: g.left, top: g.top, advance: g.advance }
}
