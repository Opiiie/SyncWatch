#![cfg(windows)]

use std::{
    ffi::{c_char, c_void, CStr},
    path::Path,
    ptr,
    sync::mpsc,
    thread,
};

use libloading::Library;
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::{
            Direct3D11::ID3D11Texture2D,
            DirectComposition::{
                DCompositionCreateDevice, IDCompositionDevice, IDCompositionRectangleClip,
                IDCompositionSurface, IDCompositionTarget, IDCompositionVisual,
            },
            Dxgi::{
                Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM},
                IDXGIDevice, IDXGISurface,
            },
        },
        System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
    },
};

use crate::player::PlayerBounds;

const EGL_FALSE: u32 = 0;
const EGL_NONE: i32 = 0x3038;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES3_BIT: i32 = 0x0040;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_PBUFFER_BIT: i32 = 0x0001;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_WIDTH: i32 = 0x3057;
const EGL_PLATFORM_ANGLE_ANGLE: u32 = 0x3202;
const EGL_PLATFORM_ANGLE_TYPE_ANGLE: i32 = 0x3203;
const EGL_PLATFORM_ANGLE_TYPE_D3D11_ANGLE: i32 = 0x3208;
const EGL_DEVICE_EXT: i32 = 0x322C;
const EGL_D3D11_DEVICE_ANGLE: i32 = 0x33A1;
const EGL_D3D_TEXTURE_ANGLE: u32 = 0x33A3;
const EGL_TEXTURE_OFFSET_X_ANGLE: i32 = 0x3490;
const EGL_TEXTURE_OFFSET_Y_ANGLE: i32 = 0x3491;
const MPV_RENDER_PARAM_API_TYPE: i32 = 1;
const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: i32 = 2;
const MPV_RENDER_PARAM_OPENGL_FBO: i32 = 3;
const MPV_RENDER_PARAM_FLIP_Y: i32 = 4;
const MPV_RENDER_UPDATE_FRAME: u64 = 1;

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;
type EglGetPlatformDisplay =
    unsafe extern "system" fn(u32, *mut c_void, *const isize) -> EglDisplay;
type EglInitialize = unsafe extern "system" fn(EglDisplay, *mut i32, *mut i32) -> u32;
type EglBindApi = unsafe extern "system" fn(u32) -> u32;
type EglChooseConfig =
    unsafe extern "system" fn(EglDisplay, *const i32, *mut EglConfig, i32, *mut i32) -> u32;
type EglCreateContext =
    unsafe extern "system" fn(EglDisplay, EglConfig, EglContext, *const i32) -> EglContext;
type EglCreatePbufferSurface =
    unsafe extern "system" fn(EglDisplay, EglConfig, *const i32) -> EglSurface;
type EglDestroyContext = unsafe extern "system" fn(EglDisplay, EglContext) -> u32;
type EglCreatePbufferFromClientBuffer =
    unsafe extern "system" fn(EglDisplay, u32, *mut c_void, EglConfig, *const i32) -> EglSurface;
type EglDestroySurface = unsafe extern "system" fn(EglDisplay, EglSurface) -> u32;
type EglMakeCurrent =
    unsafe extern "system" fn(EglDisplay, EglSurface, EglSurface, EglContext) -> u32;
type EglTerminate = unsafe extern "system" fn(EglDisplay) -> u32;
type EglGetProcAddress = unsafe extern "system" fn(*const c_char) -> *mut c_void;
type EglQueryDisplayAttribExt = unsafe extern "system" fn(EglDisplay, i32, *mut isize) -> u32;
type EglQueryDeviceAttribExt = unsafe extern "system" fn(*mut c_void, i32, *mut isize) -> u32;

#[repr(C)]
pub struct MpvRenderParam {
    kind: i32,
    data: *mut c_void,
}

#[repr(C)]
struct MpvOpenGlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    context: *mut c_void,
}

#[repr(C)]
struct MpvOpenGlFbo {
    fbo: i32,
    width: i32,
    height: i32,
    internal_format: i32,
}

#[derive(Clone, Copy)]
pub struct MpvRenderApi {
    pub create: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut MpvRenderParam) -> i32,
    pub set_update_callback:
        unsafe extern "C" fn(*mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    pub update: unsafe extern "C" fn(*mut c_void) -> u64,
    pub render: unsafe extern "C" fn(*mut c_void, *mut MpvRenderParam) -> i32,
    pub report_swap: unsafe extern "C" fn(*mut c_void),
    pub free: unsafe extern "C" fn(*mut c_void),
}

enum RenderMessage {
    Frame,
    Bounds(PlayerBounds),
    Shutdown,
}

pub struct CompositionRenderer {
    sender: mpsc::SyncSender<RenderMessage>,
    thread: Option<thread::JoinHandle<()>>,
}

impl CompositionRenderer {
    pub fn start(
        host: HWND,
        bounds: PlayerBounds,
        angle_path: &Path,
        mpv_handle: *mut c_void,
        api: MpvRenderApi,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(8);
        let callback_sender = sender.clone();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let angle_path = angle_path.to_owned();
        let host_value = host.0 as isize;
        let mpv_value = mpv_handle as usize;
        let thread = thread::Builder::new()
            .name("syncwatch-video-render".to_owned())
            .spawn(move || {
                let result = RenderLoop::new(
                    HWND(host_value as *mut c_void),
                    bounds,
                    &angle_path,
                    mpv_value as *mut c_void,
                    api,
                    callback_sender,
                );
                match result {
                    Ok(mut render_loop) => {
                        let _ = ready_sender.send(Ok(()));
                        render_loop.run(receiver);
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                    }
                }
            })
            .map_err(|error| format!("Не удалось запустить поток видео: {error}"))?;
        ready_receiver
            .recv()
            .map_err(|_| "Поток видео завершился во время запуска".to_owned())??;
        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }

    pub fn set_bounds(&self, bounds: PlayerBounds) -> Result<(), String> {
        self.sender
            .send(RenderMessage::Bounds(bounds))
            .map_err(|_| "Поток видео недоступен".to_owned())
    }
}

impl Drop for CompositionRenderer {
    fn drop(&mut self) {
        let _ = self.sender.send(RenderMessage::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct RenderLoop {
    api: MpvRenderApi,
    context: *mut c_void,
    callback_sender: Box<mpsc::SyncSender<RenderMessage>>,
    angle: Box<AngleContext>,
    layer: CompositionLayer,
}

impl RenderLoop {
    fn new(
        host: HWND,
        bounds: PlayerBounds,
        angle_path: &Path,
        mpv_handle: *mut c_void,
        api: MpvRenderApi,
        callback_sender: mpsc::SyncSender<RenderMessage>,
    ) -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.ok();
        let angle = Box::new(AngleContext::new(angle_path)?);
        let layer = CompositionLayer::new(host, bounds, angle.d3d_device()?)?;
        let mut init = MpvOpenGlInitParams {
            get_proc_address: resolve_gl,
            context: (&*angle as *const AngleContext).cast_mut().cast(),
        };
        let api_name = b"opengl\0";
        let mut params = [
            MpvRenderParam {
                kind: MPV_RENDER_PARAM_API_TYPE,
                data: api_name.as_ptr().cast_mut().cast(),
            },
            MpvRenderParam {
                kind: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&mut init as *mut MpvOpenGlInitParams).cast(),
            },
            MpvRenderParam {
                kind: 0,
                data: ptr::null_mut(),
            },
        ];
        let mut context = ptr::null_mut();
        let code = unsafe { (api.create)(&mut context, mpv_handle, params.as_mut_ptr()) };
        if code < 0 || context.is_null() {
            return Err(format!("libmpv не создала OpenGL Render API (код {code})"));
        }
        let callback_sender = Box::new(callback_sender);
        unsafe {
            (api.set_update_callback)(
                context,
                Some(render_update),
                (&*callback_sender as *const mpsc::SyncSender<RenderMessage>)
                    .cast_mut()
                    .cast(),
            )
        };
        Ok(Self {
            api,
            context,
            callback_sender,
            angle,
            layer,
        })
    }

    fn run(&mut self, receiver: mpsc::Receiver<RenderMessage>) {
        while let Ok(message) = receiver.recv() {
            match message {
                RenderMessage::Frame => {
                    let flags = unsafe { (self.api.update)(self.context) };
                    if flags & MPV_RENDER_UPDATE_FRAME != 0 {
                        if let Err(error) = self.render_frame() {
                            eprintln!("Failed to render video frame: {error}");
                        }
                    }
                }
                RenderMessage::Bounds(bounds) => {
                    if let Err(error) = self.layer.set_bounds(bounds) {
                        eprintln!("Failed to update video geometry: {error}");
                    } else if let Err(error) = self.render_frame() {
                        eprintln!("Failed to redraw video frame: {error}");
                    }
                }
                RenderMessage::Shutdown => break,
            }
        }
    }

    fn render_frame(&mut self) -> Result<(), String> {
        let (texture, offset) = self.layer.begin_draw()?;
        let surface = match self.angle.surface_for_texture(&texture, offset) {
            Ok(surface) => surface,
            Err(error) => {
                let _ = self.layer.end_draw();
                return Err(error);
            }
        };
        let mut fbo = MpvOpenGlFbo {
            fbo: 0,
            width: self.layer.width(),
            height: self.layer.height(),
            internal_format: 0,
        };
        let mut flip = 0_i32;
        let mut params = [
            MpvRenderParam {
                kind: MPV_RENDER_PARAM_OPENGL_FBO,
                data: (&mut fbo as *mut MpvOpenGlFbo).cast(),
            },
            MpvRenderParam {
                kind: MPV_RENDER_PARAM_FLIP_Y,
                data: (&mut flip as *mut i32).cast(),
            },
            MpvRenderParam {
                kind: 0,
                data: ptr::null_mut(),
            },
        ];
        let code = unsafe { (self.api.render)(self.context, params.as_mut_ptr()) };
        self.angle.release_surface(surface);
        self.layer.end_draw()?;
        if code < 0 {
            return Err(format!("libmpv не отрисовала кадр (код {code})"));
        }
        unsafe { (self.api.report_swap)(self.context) };
        Ok(())
    }
}

impl Drop for RenderLoop {
    fn drop(&mut self) {
        unsafe {
            (self.api.set_update_callback)(self.context, None, ptr::null_mut());
            (self.api.free)(self.context)
        };
        let _ = &self.callback_sender;
    }
}

unsafe extern "C" fn render_update(context: *mut c_void) {
    if !context.is_null() {
        let sender = unsafe { &*(context as *const mpsc::SyncSender<RenderMessage>) };
        let _ = sender.try_send(RenderMessage::Frame);
    }
}

unsafe extern "C" fn resolve_gl(context: *mut c_void, name: *const c_char) -> *mut c_void {
    if context.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    let angle = unsafe { &*(context as *const AngleContext) };
    let resolved = unsafe { (angle.get_proc_address)(name) };
    if !resolved.is_null() {
        return resolved;
    }
    unsafe {
        angle
            .library
            .get::<*mut c_void>(CStr::from_ptr(name).to_bytes_with_nul())
            .map(|value| *value)
            .unwrap_or(ptr::null_mut())
    }
}

struct AngleContext {
    library: Library,
    display: EglDisplay,
    config: EglConfig,
    context: EglContext,
    bootstrap_surface: EglSurface,
    get_proc_address: EglGetProcAddress,
    create_pbuffer: EglCreatePbufferFromClientBuffer,
    destroy_surface: EglDestroySurface,
    make_current: EglMakeCurrent,
    terminate: EglTerminate,
    destroy_context: EglDestroyContext,
    query_display: EglQueryDisplayAttribExt,
    query_device: EglQueryDeviceAttribExt,
}

impl AngleContext {
    fn new(path: &Path) -> Result<Self, String> {
        let library = unsafe { Library::new(path) }
            .map_err(|error| format!("Не удалось открыть ANGLE: {error}"))?;
        unsafe {
            let get_platform: EglGetPlatformDisplay = *library
                .get(b"EGL_GetPlatformDisplay\0")
                .map_err(angle_symbol)?;
            let initialize: EglInitialize =
                *library.get(b"EGL_Initialize\0").map_err(angle_symbol)?;
            let bind_api: EglBindApi = *library.get(b"EGL_BindAPI\0").map_err(angle_symbol)?;
            let choose_config: EglChooseConfig =
                *library.get(b"EGL_ChooseConfig\0").map_err(angle_symbol)?;
            let create_context: EglCreateContext =
                *library.get(b"EGL_CreateContext\0").map_err(angle_symbol)?;
            let destroy_context = *library.get(b"EGL_DestroyContext\0").map_err(angle_symbol)?;
            let create_pbuffer = *library
                .get(b"EGL_CreatePbufferFromClientBuffer\0")
                .map_err(angle_symbol)?;
            let create_pbuffer_surface: EglCreatePbufferSurface = *library
                .get(b"EGL_CreatePbufferSurface\0")
                .map_err(angle_symbol)?;
            let destroy_surface = *library.get(b"EGL_DestroySurface\0").map_err(angle_symbol)?;
            let make_current: EglMakeCurrent =
                *library.get(b"EGL_MakeCurrent\0").map_err(angle_symbol)?;
            let terminate = *library.get(b"EGL_Terminate\0").map_err(angle_symbol)?;
            let get_proc_address: EglGetProcAddress =
                *library.get(b"EGL_GetProcAddress\0").map_err(angle_symbol)?;
            let query_display: EglQueryDisplayAttribExt =
                proc(&get_proc_address, b"eglQueryDisplayAttribEXT\0")?;
            let query_device: EglQueryDeviceAttribExt =
                proc(&get_proc_address, b"eglQueryDeviceAttribEXT\0")?;
            let display_attrs = [
                EGL_PLATFORM_ANGLE_TYPE_ANGLE as isize,
                EGL_PLATFORM_ANGLE_TYPE_D3D11_ANGLE as isize,
                EGL_NONE as isize,
            ];
            let display = get_platform(
                EGL_PLATFORM_ANGLE_ANGLE,
                ptr::null_mut(),
                display_attrs.as_ptr(),
            );
            if display.is_null()
                || initialize(display, ptr::null_mut(), ptr::null_mut()) == EGL_FALSE
                || bind_api(EGL_OPENGL_ES_API) == EGL_FALSE
            {
                return Err("ANGLE не создала D3D11-контекст".to_owned());
            }
            let config_attrs = [
                EGL_SURFACE_TYPE,
                EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_ES3_BIT,
                EGL_RED_SIZE,
                8,
                EGL_GREEN_SIZE,
                8,
                EGL_BLUE_SIZE,
                8,
                EGL_ALPHA_SIZE,
                8,
                EGL_NONE,
            ];
            let mut config = ptr::null_mut();
            let mut count = 0;
            if choose_config(display, config_attrs.as_ptr(), &mut config, 1, &mut count)
                == EGL_FALSE
                || count == 0
            {
                return Err("ANGLE не нашла подходящую конфигурацию".to_owned());
            }
            let context_attrs = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
            let context = create_context(display, config, ptr::null_mut(), context_attrs.as_ptr());
            if context.is_null() {
                return Err("ANGLE не создала OpenGL ES-контекст".to_owned());
            }
            let bootstrap_attrs = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
            let bootstrap_surface =
                create_pbuffer_surface(display, config, bootstrap_attrs.as_ptr());
            if bootstrap_surface.is_null()
                || make_current(display, bootstrap_surface, bootstrap_surface, context) == EGL_FALSE
            {
                return Err("ANGLE не активировала OpenGL ES-контекст".to_owned());
            }
            Ok(Self {
                library,
                display,
                config,
                context,
                bootstrap_surface,
                get_proc_address,
                create_pbuffer,
                destroy_surface,
                make_current,
                terminate,
                destroy_context,
                query_display,
                query_device,
            })
        }
    }

    fn d3d_device(&self) -> Result<IDXGIDevice, String> {
        let mut device = 0_isize;
        let mut d3d = 0_isize;
        unsafe {
            if (self.query_display)(self.display, EGL_DEVICE_EXT, &mut device) == EGL_FALSE
                || (self.query_device)(device as *mut c_void, EGL_D3D11_DEVICE_ANGLE, &mut d3d)
                    == EGL_FALSE
                || d3d == 0
            {
                return Err("ANGLE не предоставила D3D11-устройство".to_owned());
            }
            let borrowed = std::mem::ManuallyDrop::new(
                windows::Win32::Graphics::Direct3D11::ID3D11Device::from_raw(d3d as *mut c_void),
            );
            borrowed.cast::<IDXGIDevice>().map_err(win_error)
        }
    }

    fn surface_for_texture(
        &self,
        texture: &ID3D11Texture2D,
        offset: POINT,
    ) -> Result<EglSurface, String> {
        let attrs = [
            EGL_TEXTURE_OFFSET_X_ANGLE,
            offset.x,
            EGL_TEXTURE_OFFSET_Y_ANGLE,
            offset.y,
            EGL_NONE,
        ];
        let surface = unsafe {
            (self.create_pbuffer)(
                self.display,
                EGL_D3D_TEXTURE_ANGLE,
                texture.as_raw(),
                self.config,
                attrs.as_ptr(),
            )
        };
        if surface.is_null() {
            return Err("ANGLE не привязала поверхность кадра".to_owned());
        }
        if unsafe { (self.make_current)(self.display, surface, surface, self.context) } == EGL_FALSE
        {
            unsafe { (self.destroy_surface)(self.display, surface) };
            return Err("ANGLE не активировала поверхность кадра".to_owned());
        }
        Ok(surface)
    }

    fn release_surface(&self, surface: EglSurface) {
        unsafe {
            (self.make_current)(
                self.display,
                self.bootstrap_surface,
                self.bootstrap_surface,
                self.context,
            );
            (self.destroy_surface)(self.display, surface);
        }
    }
}

impl Drop for AngleContext {
    fn drop(&mut self) {
        unsafe {
            (self.make_current)(self.display, ptr::null_mut(), ptr::null_mut(), self.context);
            (self.destroy_surface)(self.display, self.bootstrap_surface);
            (self.destroy_context)(self.display, self.context);
            (self.terminate)(self.display);
        }
    }
}

struct CompositionLayer {
    device: IDCompositionDevice,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    clip: IDCompositionRectangleClip,
    surface: IDCompositionSurface,
    bounds: PlayerBounds,
}

impl CompositionLayer {
    fn new(host: HWND, bounds: PlayerBounds, dxgi: IDXGIDevice) -> Result<Self, String> {
        unsafe {
            let device: IDCompositionDevice = DCompositionCreateDevice(&dxgi).map_err(win_error)?;
            let target = device.CreateTargetForHwnd(host, false).map_err(win_error)?;
            let visual = device.CreateVisual().map_err(win_error)?;
            let clip = device.CreateRectangleClip().map_err(win_error)?;
            let surface = device
                .CreateSurface(
                    bounds.width.max(1) as u32,
                    bounds.height.max(1) as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_IGNORE,
                )
                .map_err(win_error)?;
            visual.SetContent(&surface).map_err(win_error)?;
            visual.SetClip(&clip).map_err(win_error)?;
            target.SetRoot(&visual).map_err(win_error)?;
            let mut layer = Self {
                device,
                _target: target,
                visual,
                clip,
                surface,
                bounds,
            };
            layer.apply_geometry(bounds)?;
            Ok(layer)
        }
    }

    fn width(&self) -> i32 {
        self.bounds.width.max(1)
    }
    fn height(&self) -> i32 {
        self.bounds.height.max(1)
    }

    fn set_bounds(&mut self, bounds: PlayerBounds) -> Result<(), String> {
        if bounds.width.max(1) != self.width() || bounds.height.max(1) != self.height() {
            self.surface = unsafe {
                self.device.CreateSurface(
                    bounds.width.max(1) as u32,
                    bounds.height.max(1) as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_IGNORE,
                )
            }
            .map_err(win_error)?;
            unsafe { self.visual.SetContent(&self.surface) }.map_err(win_error)?;
        }
        self.bounds = bounds;
        self.apply_geometry(bounds)
    }

    fn apply_geometry(&mut self, bounds: PlayerBounds) -> Result<(), String> {
        let visible = bounds.visible_clip();
        let (left, top, right, bottom) = visible.unwrap_or((0, 0, 0, 0));
        unsafe {
            self.visual
                .SetOffsetX2(bounds.x as f32)
                .map_err(win_error)?;
            self.visual
                .SetOffsetY2(bounds.y as f32)
                .map_err(win_error)?;
            self.clip.SetLeft2(left as f32).map_err(win_error)?;
            self.clip.SetTop2(top as f32).map_err(win_error)?;
            self.clip.SetRight2(right as f32).map_err(win_error)?;
            self.clip.SetBottom2(bottom as f32).map_err(win_error)?;
            let radius = bounds.corner_radius.max(0) as f32;
            self.clip.SetTopLeftRadiusX2(radius).map_err(win_error)?;
            self.clip.SetTopLeftRadiusY2(radius).map_err(win_error)?;
            self.clip.SetTopRightRadiusX2(radius).map_err(win_error)?;
            self.clip.SetTopRightRadiusY2(radius).map_err(win_error)?;
            self.clip.SetBottomLeftRadiusX2(radius).map_err(win_error)?;
            self.clip.SetBottomLeftRadiusY2(radius).map_err(win_error)?;
            self.clip
                .SetBottomRightRadiusX2(radius)
                .map_err(win_error)?;
            self.clip
                .SetBottomRightRadiusY2(radius)
                .map_err(win_error)?;
            self.device.Commit().map_err(win_error)
        }
    }

    fn begin_draw(&self) -> Result<(ID3D11Texture2D, POINT), String> {
        let rect = RECT {
            left: 0,
            top: 0,
            right: self.width(),
            bottom: self.height(),
        };
        let mut offset = POINT::default();
        let surface: IDXGISurface =
            unsafe { self.surface.BeginDraw(Some(&rect), &mut offset) }.map_err(win_error)?;
        let texture = surface.cast::<ID3D11Texture2D>().map_err(win_error)?;
        Ok((texture, offset))
    }

    fn end_draw(&self) -> Result<(), String> {
        unsafe {
            self.surface.EndDraw().map_err(win_error)?;
            self.device.Commit().map_err(win_error)
        }
    }
}

unsafe fn proc<T: Copy>(get_proc: &EglGetProcAddress, name: &[u8]) -> Result<T, String> {
    let pointer = unsafe { get_proc(name.as_ptr().cast()) };
    if pointer.is_null() {
        return Err(format!(
            "ANGLE не содержит {}",
            String::from_utf8_lossy(&name[..name.len() - 1])
        ));
    }
    Ok(unsafe { std::mem::transmute_copy(&pointer) })
}

fn angle_symbol(error: libloading::Error) -> String {
    format!("Несовместимая библиотека ANGLE: {error}")
}
fn win_error(error: windows::core::Error) -> String {
    format!("DirectComposition: {error}")
}
