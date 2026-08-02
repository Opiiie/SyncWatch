use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTrack {
    id: i64,
    label: String,
    title: Option<String>,
    language: Option<String>,
    selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    position_seconds: f64,
    duration_seconds: f64,
    speed: f64,
    audio_tracks: Vec<PlayerTrack>,
    subtitle_tracks: Vec<PlayerTrack>,
}

#[derive(Default)]
pub struct PlayerController {
    #[cfg(windows)]
    inner: std::sync::Mutex<windows_player::PlayerInner>,
}

#[tauri::command]
pub fn player_create_surface(
    window: tauri::WebviewWindow,
    controller: tauri::State<'_, PlayerController>,
    bounds: PlayerBounds,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .create_surface(&window, bounds);
    }

    #[cfg(not(windows))]
    {
        let _ = (window, controller, bounds);
        Err("Встраивание libmpv пока реализовано только для Windows".to_owned())
    }
}

#[tauri::command]
pub fn player_set_surface_bounds(
    controller: tauri::State<'_, PlayerController>,
    bounds: PlayerBounds,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .set_bounds(bounds);
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, bounds);
        Ok(())
    }
}

#[tauri::command]
pub fn player_load(
    controller: tauri::State<'_, PlayerController>,
    path: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .load(&path);
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, path);
        Err("Встраивание libmpv пока реализовано только для Windows".to_owned())
    }
}

#[tauri::command]
pub fn player_set_paused(
    controller: tauri::State<'_, PlayerController>,
    paused: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .set_property("pause", if paused { "yes" } else { "no" });
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, paused);
        Ok(())
    }
}

#[tauri::command]
pub fn player_set_volume(
    controller: tauri::State<'_, PlayerController>,
    volume: f64,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .set_property("volume", &volume.clamp(0.0, 100.0).to_string());
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, volume);
        Ok(())
    }
}

#[tauri::command]
pub fn player_seek(
    controller: tauri::State<'_, PlayerController>,
    position_seconds: f64,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .command(&[
                "seek",
                &position_seconds.max(0.0).to_string(),
                "absolute+exact",
            ]);
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, position_seconds);
        Ok(())
    }
}

#[tauri::command]
pub fn player_set_speed(
    controller: tauri::State<'_, PlayerController>,
    speed: f64,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .set_property("speed", &speed.clamp(0.25, 3.0).to_string());
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, speed);
        Ok(())
    }
}

#[tauri::command]
pub fn player_get_state(
    controller: tauri::State<'_, PlayerController>,
) -> Result<PlayerState, String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .state();
    }

    #[cfg(not(windows))]
    {
        let _ = controller;
        Err("Встраивание libmpv пока реализовано только для Windows".to_owned())
    }
}

#[tauri::command]
pub fn player_select_track(
    controller: tauri::State<'_, PlayerController>,
    kind: String,
    track_id: Option<i64>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let property = match kind.as_str() {
            "audio" => "aid",
            "subtitle" => "sid",
            _ => return Err("Неизвестный тип дорожки".to_owned()),
        };
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .set_property(
                property,
                &track_id.map_or_else(|| "no".to_owned(), |id| id.to_string()),
            );
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, kind, track_id);
        Ok(())
    }
}

#[tauri::command]
pub fn player_add_subtitle(
    controller: tauri::State<'_, PlayerController>,
    path: String,
    title: String,
    language: Option<String>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        return controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .mpv_mut()?
            .add_subtitle(&path, &title, language.as_deref());
    }

    #[cfg(not(windows))]
    {
        let _ = (controller, path, title, language);
        Ok(())
    }
}

#[tauri::command]
pub fn player_destroy(controller: tauri::State<'_, PlayerController>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut inner = controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?;
        inner.destroy();
    }
    Ok(())
}

#[cfg(windows)]
mod windows_player {
    use std::{
        ffi::{c_char, c_void, CStr, CString},
        path::PathBuf,
        ptr,
    };

    use libloading::Library;
    use windows_sys::Win32::{
        Foundation::{HWND, POINT},
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, IsIconic, SetWindowPos, ShowWindow, SWP_NOACTIVATE,
            SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
        },
    };

    use super::PlayerBounds;

    type MpvCreate = unsafe extern "C" fn() -> *mut c_void;
    type MpvSetOptionString =
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32;
    type MpvInitialize = unsafe extern "C" fn(*mut c_void) -> i32;
    type MpvCommand = unsafe extern "C" fn(*mut c_void, *const *const c_char) -> i32;
    type MpvSetPropertyString =
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32;
    type MpvGetPropertyString = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char;
    type MpvFree = unsafe extern "C" fn(*mut c_void);
    type MpvTerminateDestroy = unsafe extern "C" fn(*mut c_void);
    type MpvErrorString = unsafe extern "C" fn(i32) -> *const c_char;

    struct MpvApi {
        _library: Library,
        create: MpvCreate,
        set_option_string: MpvSetOptionString,
        initialize: MpvInitialize,
        command: MpvCommand,
        set_property_string: MpvSetPropertyString,
        get_property_string: MpvGetPropertyString,
        free: MpvFree,
        terminate_destroy: MpvTerminateDestroy,
        error_string: MpvErrorString,
    }

    impl MpvApi {
        fn load() -> Result<Self, String> {
            let candidates = library_candidates();
            let mut failures = Vec::new();
            for candidate in candidates {
                let library = match unsafe { Library::new(&candidate) } {
                    Ok(library) => library,
                    Err(error) => {
                        failures.push(format!("{}: {error}", candidate.display()));
                        continue;
                    }
                };

                return unsafe {
                    Ok(Self {
                        create: *library.get(b"mpv_create\0").map_err(symbol_error)?,
                        set_option_string: *library
                            .get(b"mpv_set_option_string\0")
                            .map_err(symbol_error)?,
                        initialize: *library.get(b"mpv_initialize\0").map_err(symbol_error)?,
                        command: *library.get(b"mpv_command\0").map_err(symbol_error)?,
                        set_property_string: *library
                            .get(b"mpv_set_property_string\0")
                            .map_err(symbol_error)?,
                        get_property_string: *library
                            .get(b"mpv_get_property_string\0")
                            .map_err(symbol_error)?,
                        free: *library.get(b"mpv_free\0").map_err(symbol_error)?,
                        terminate_destroy: *library
                            .get(b"mpv_terminate_destroy\0")
                            .map_err(symbol_error)?,
                        error_string: *library.get(b"mpv_error_string\0").map_err(symbol_error)?,
                        _library: library,
                    })
                };
            }

            Err(format!(
                "libmpv-2.dll не найдена. Поместите 64-битную DLL рядом с syncwatch.exe или задайте SYNCWATCH_LIBMPV_PATH. Проверено: {}",
                failures.join("; ")
            ))
        }

        fn check(&self, code: i32, operation: &str) -> Result<(), String> {
            if code >= 0 {
                return Ok(());
            }
            let message = unsafe {
                let value = (self.error_string)(code);
                if value.is_null() {
                    format!("код {code}")
                } else {
                    CStr::from_ptr(value).to_string_lossy().into_owned()
                }
            };
            Err(format!("libmpv: {operation}: {message}"))
        }
    }

    fn symbol_error(error: libloading::Error) -> String {
        format!("Несовместимая библиотека libmpv: {error}")
    }

    fn library_candidates() -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("SYNCWATCH_LIBMPV_PATH") {
            candidates.push(PathBuf::from(path));
        }
        if let Ok(executable) = std::env::current_exe() {
            if let Some(directory) = executable.parent() {
                candidates.push(directory.join("libmpv-2.dll"));
                candidates.push(directory.join("resources/libmpv-2.dll"));
            }
        }
        if let Ok(directory) = std::env::current_dir() {
            candidates.push(directory.join("src-tauri/resources/libmpv-2.dll"));
            candidates.push(directory.join("resources/libmpv-2.dll"));
        }
        candidates.push(PathBuf::from("libmpv-2.dll"));
        candidates
    }

    pub struct MpvInstance {
        api: MpvApi,
        handle: *mut c_void,
    }

    // All access is serialized by PlayerController's mutex.
    unsafe impl Send for MpvInstance {}

    impl MpvInstance {
        fn new(window: HWND) -> Result<Self, String> {
            let api = MpvApi::load()?;
            let handle = unsafe { (api.create)() };
            if handle.is_null() {
                return Err("libmpv не смогла создать контекст плеера".to_owned());
            }
            let instance = Self { api, handle };
            let window_id = (window as usize).to_string();
            instance.set_option("wid", &window_id)?;
            instance.set_option("osc", "no")?;
            instance.set_option("input-default-bindings", "no")?;
            instance.set_option("input-vo-keyboard", "no")?;
            instance.set_option("sub-auto", "no")?;
            instance.set_option("keep-open", "yes")?;
            instance.set_option("idle", "yes")?;
            instance.set_option("hwdec", "auto-safe")?;
            instance.set_option("cache", "auto")?;
            instance.set_option("cache-secs", "20")?;
            instance.set_option("demuxer-max-bytes", "64MiB")?;
            instance.set_option("demuxer-max-back-bytes", "12MiB")?;
            instance.set_option("demuxer-hysteresis-secs", "5")?;
            instance.set_option("cache-pause", "yes")?;
            instance.set_option("cache-pause-wait", "2")?;
            instance.api.check(
                unsafe { (instance.api.initialize)(instance.handle) },
                "инициализация",
            )?;
            instance.set_property("pause", "yes")?;
            Ok(instance)
        }

        fn set_option(&self, name: &str, value: &str) -> Result<(), String> {
            let name = CString::new(name).map_err(|_| "Некорректное имя опции".to_owned())?;
            let value =
                CString::new(value).map_err(|_| "Некорректное значение опции".to_owned())?;
            self.api.check(
                unsafe { (self.api.set_option_string)(self.handle, name.as_ptr(), value.as_ptr()) },
                "настройка плеера",
            )
        }

        pub fn set_property(&self, name: &str, value: &str) -> Result<(), String> {
            let name = CString::new(name).map_err(|_| "Некорректное имя свойства".to_owned())?;
            let value =
                CString::new(value).map_err(|_| "Некорректное значение свойства".to_owned())?;
            self.api.check(
                unsafe {
                    (self.api.set_property_string)(self.handle, name.as_ptr(), value.as_ptr())
                },
                "изменение свойства",
            )
        }

        pub fn command(&self, values: &[&str]) -> Result<(), String> {
            let values = values
                .iter()
                .map(|value| {
                    CString::new(*value).map_err(|_| "Некорректный аргумент команды".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut pointers = values
                .iter()
                .map(|value| value.as_ptr())
                .collect::<Vec<_>>();
            pointers.push(ptr::null());
            self.api.check(
                unsafe { (self.api.command)(self.handle, pointers.as_ptr()) },
                "команда плеера",
            )
        }

        pub fn load(&self, path: &str) -> Result<(), String> {
            self.command(&["loadfile", path, "replace"])
        }

        pub fn add_subtitle(
            &self,
            path: &str,
            title: &str,
            language: Option<&str>,
        ) -> Result<(), String> {
            self.command(&["sub-add", path, "auto", title, language.unwrap_or("")])
        }

        fn get_property(&self, name: &str) -> Option<String> {
            let name = CString::new(name).ok()?;
            let value = unsafe { (self.api.get_property_string)(self.handle, name.as_ptr()) };
            if value.is_null() {
                return None;
            }
            let result = unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() };
            unsafe { (self.api.free)(value.cast()) };
            Some(result)
        }

        fn tracks(&self, expected_type: &str) -> Vec<super::PlayerTrack> {
            let count = self
                .get_property("track-list/count")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            (0..count)
                .filter(|index| {
                    self.get_property(&format!("track-list/{index}/type"))
                        .as_deref()
                        == Some(expected_type)
                })
                .filter_map(|index| {
                    let id = self
                        .get_property(&format!("track-list/{index}/id"))?
                        .parse::<i64>()
                        .ok()?;
                    let title = self.get_property(&format!("track-list/{index}/title"));
                    let language = self.get_property(&format!("track-list/{index}/lang"));
                    let label = match (&title, &language) {
                        (Some(title), Some(language)) => format!("{title} · {language}"),
                        (Some(title), None) => title.clone(),
                        (None, Some(language)) => language.clone(),
                        (None, None) => format!("Дорожка {id}"),
                    };
                    let selected = self
                        .get_property(&format!("track-list/{index}/selected"))
                        .as_deref()
                        == Some("yes");
                    Some(super::PlayerTrack {
                        id,
                        label,
                        title,
                        language,
                        selected,
                    })
                })
                .collect()
        }

        pub fn state(&self) -> Result<super::PlayerState, String> {
            Ok(super::PlayerState {
                position_seconds: self
                    .get_property("time-pos")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.0),
                duration_seconds: self
                    .get_property("duration")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.0),
                speed: self
                    .get_property("speed")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(1.0),
                audio_tracks: self.tracks("audio"),
                subtitle_tracks: self.tracks("sub"),
            })
        }
    }

    impl Drop for MpvInstance {
        fn drop(&mut self) {
            unsafe { (self.api.terminate_destroy)(self.handle) };
        }
    }

    struct NativeSurface {
        window: HWND,
        host: HWND,
    }

    unsafe impl Send for NativeSurface {}

    impl NativeSurface {
        const EDGE_OVERDRAW: i32 = 8;

        fn create(host: HWND, bounds: PlayerBounds) -> Result<Self, String> {
            let class_name = "STATIC\0".encode_utf16().collect::<Vec<_>>();
            let title = "SyncWatch video\0".encode_utf16().collect::<Vec<_>>();
            let window = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | 4,
                    0,
                    0,
                    bounds.width.max(1),
                    bounds.height.max(1),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            if window.is_null() {
                return Err(format!(
                    "Не удалось создать область видео: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let surface = Self { window, host };
            surface.set_bounds(bounds)?;
            Ok(surface)
        }

        fn set_bounds(&self, bounds: PlayerBounds) -> Result<(), String> {
            if unsafe { IsIconic(self.host) } != 0 {
                unsafe { ShowWindow(self.window, SW_HIDE) };
                return Ok(());
            }

            let mut origin = POINT {
                x: bounds.x,
                y: bounds.y,
            };
            if unsafe { ClientToScreen(self.host, &mut origin) } == 0 {
                return Err(format!(
                    "Не удалось определить положение области видео: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let result = unsafe {
                SetWindowPos(
                    self.window,
                    self.host,
                    origin.x - Self::EDGE_OVERDRAW,
                    origin.y - Self::EDGE_OVERDRAW,
                    bounds.width.max(1) + Self::EDGE_OVERDRAW * 2,
                    bounds.height.max(1) + Self::EDGE_OVERDRAW * 2,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
            if result == 0 {
                Err(format!(
                    "Не удалось изменить размер области видео: {}",
                    std::io::Error::last_os_error()
                ))
            } else {
                unsafe { ShowWindow(self.window, SW_SHOWNOACTIVATE) };
                Ok(())
            }
        }
    }

    impl Drop for NativeSurface {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.window) };
        }
    }

    #[derive(Default)]
    pub struct PlayerInner {
        pub mpv: Option<MpvInstance>,
        surface: Option<NativeSurface>,
    }

    impl PlayerInner {
        pub fn create_surface(
            &mut self,
            window: &tauri::WebviewWindow,
            bounds: PlayerBounds,
        ) -> Result<(), String> {
            if let Some(surface) = &self.surface {
                return surface.set_bounds(bounds);
            }
            let host = window.hwnd().map_err(|error| error.to_string())?.0 as HWND;
            let surface = NativeSurface::create(host, bounds)?;
            let mpv = match MpvInstance::new(surface.window) {
                Ok(mpv) => mpv,
                Err(error) => return Err(error),
            };
            self.surface = Some(surface);
            self.mpv = Some(mpv);
            Ok(())
        }

        pub fn set_bounds(&self, bounds: PlayerBounds) -> Result<(), String> {
            if let Some(surface) = &self.surface {
                surface.set_bounds(bounds)?;
            }
            Ok(())
        }

        pub fn mpv_mut(&mut self) -> Result<&mut MpvInstance, String> {
            self.mpv
                .as_mut()
                .ok_or_else(|| "Плеер ещё не инициализирован".to_owned())
        }

        pub fn destroy(&mut self) {
            self.mpv = None;
            self.surface = None;
        }
    }
}
