use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) clip_x: i32,
    pub(crate) clip_y: i32,
    pub(crate) clip_width: i32,
    pub(crate) clip_height: i32,
    pub(crate) corner_radius: i32,
}

impl PlayerBounds {
    pub(crate) fn visible_clip(self) -> Option<(i32, i32, i32, i32)> {
        let left = self.clip_x.clamp(0, self.width.max(0));
        let top = self.clip_y.clamp(0, self.height.max(0));
        let right = self
            .clip_x
            .saturating_add(self.clip_width)
            .clamp(left, self.width.max(0));
        let bottom = self
            .clip_y
            .saturating_add(self.clip_height)
            .clamp(top, self.height.max(0));
        (right > left && bottom > top).then_some((left, top, right, bottom))
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::PlayerBounds;

    #[test]
    fn visible_clip_is_limited_to_surface() {
        let bounds = PlayerBounds {
            x: 10,
            y: -100,
            width: 800,
            height: 450,
            clip_x: -20,
            clip_y: 100,
            clip_width: 900,
            clip_height: 500,
            corner_radius: 18,
        };

        assert_eq!(bounds.visible_clip(), Some((0, 100, 800, 450)));
    }

    #[test]
    fn fully_hidden_surface_has_no_clip() {
        let bounds = PlayerBounds {
            x: 0,
            y: 900,
            width: 800,
            height: 450,
            clip_x: 0,
            clip_y: 0,
            clip_width: 0,
            clip_height: 0,
            corner_radius: 18,
        };

        assert_eq!(bounds.visible_clip(), None);
    }
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
pub async fn player_create_surface(
    app: tauri::AppHandle,
    runtime: tauri::State<'_, crate::mpv_runtime::MpvRuntimeManager>,
    window: tauri::WebviewWindow,
    controller: tauri::State<'_, PlayerController>,
    bounds: PlayerBounds,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let runtime_paths = runtime.ensure(&app).await?;
        let result = controller
            .inner
            .lock()
            .map_err(|_| "Контроллер плеера недоступен".to_owned())?
            .create_surface(&window, bounds, &runtime_paths);
        if let Err(error) = &result {
            eprintln!("Failed to create video surface: {error}");
        }
        result
    }

    #[cfg(not(windows))]
    {
        let _ = (app, runtime, window, controller, bounds);
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
        ptr,
    };

    use libloading::Library;
    use windows::Win32::Foundation::HWND;

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
    type MpvRenderContextCreate = unsafe extern "C" fn(
        *mut *mut c_void,
        *mut c_void,
        *mut crate::player_render::MpvRenderParam,
    ) -> i32;
    type MpvRenderContextSetUpdateCallback =
        unsafe extern "C" fn(*mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void);
    type MpvRenderContextUpdate = unsafe extern "C" fn(*mut c_void) -> u64;
    type MpvRenderContextRender =
        unsafe extern "C" fn(*mut c_void, *mut crate::player_render::MpvRenderParam) -> i32;
    type MpvRenderContextVoid = unsafe extern "C" fn(*mut c_void);

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
        render_context_create: MpvRenderContextCreate,
        render_context_set_update_callback: MpvRenderContextSetUpdateCallback,
        render_context_update: MpvRenderContextUpdate,
        render_context_render: MpvRenderContextRender,
        render_context_report_swap: MpvRenderContextVoid,
        render_context_free: MpvRenderContextVoid,
    }

    impl MpvApi {
        fn load(runtime_path: &std::path::Path) -> Result<Self, String> {
            let library = unsafe { Library::new(runtime_path) }.map_err(|error| {
                format!(
                    "Не удалось открыть подготовленную libmpv ({}): {error}",
                    runtime_path.display()
                )
            })?;

            unsafe {
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
                    render_context_create: *library
                        .get(b"mpv_render_context_create\0")
                        .map_err(symbol_error)?,
                    render_context_set_update_callback: *library
                        .get(b"mpv_render_context_set_update_callback\0")
                        .map_err(symbol_error)?,
                    render_context_update: *library
                        .get(b"mpv_render_context_update\0")
                        .map_err(symbol_error)?,
                    render_context_render: *library
                        .get(b"mpv_render_context_render\0")
                        .map_err(symbol_error)?,
                    render_context_report_swap: *library
                        .get(b"mpv_render_context_report_swap\0")
                        .map_err(symbol_error)?,
                    render_context_free: *library
                        .get(b"mpv_render_context_free\0")
                        .map_err(symbol_error)?,
                    _library: library,
                })
            }
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

    pub struct MpvInstance {
        api: MpvApi,
        handle: *mut c_void,
        renderer: Option<crate::player_render::CompositionRenderer>,
    }

    // All access is serialized by PlayerController's mutex.
    unsafe impl Send for MpvInstance {}

    impl MpvInstance {
        fn new(
            host: HWND,
            bounds: PlayerBounds,
            runtime_paths: &crate::mpv_runtime::MpvRuntimePaths,
        ) -> Result<Self, String> {
            let api = MpvApi::load(&runtime_paths.mpv)?;
            let handle = unsafe { (api.create)() };
            if handle.is_null() {
                return Err("libmpv не смогла создать контекст плеера".to_owned());
            }
            let mut instance = Self {
                api,
                handle,
                renderer: None,
            };
            instance.set_option("osc", "no")?;
            instance.set_option("vo", "libmpv")?;
            instance.set_option("input-default-bindings", "no")?;
            instance.set_option("input-vo-keyboard", "no")?;
            instance.set_option("sub-auto", "no")?;
            instance.set_option("keep-open", "yes")?;
            instance.set_option("idle", "yes")?;
            instance.set_option("hwdec", "auto-safe")?;
            instance.set_option("cache", "auto")?;
            instance.set_option("cache-secs", "90")?;
            instance.set_option("demuxer-max-bytes", "512MiB")?;
            instance.set_option("demuxer-max-back-bytes", "64MiB")?;
            instance.set_option("demuxer-hysteresis-secs", "15")?;
            instance.set_option("cache-pause", "yes")?;
            instance.set_option("cache-pause-wait", "5")?;
            instance.api.check(
                unsafe { (instance.api.initialize)(instance.handle) },
                "инициализация",
            )?;
            let render_api = crate::player_render::MpvRenderApi {
                create: instance.api.render_context_create,
                set_update_callback: instance.api.render_context_set_update_callback,
                update: instance.api.render_context_update,
                render: instance.api.render_context_render,
                report_swap: instance.api.render_context_report_swap,
                free: instance.api.render_context_free,
            };
            instance.renderer = Some(crate::player_render::CompositionRenderer::start(
                host,
                bounds,
                &runtime_paths.angle,
                handle,
                render_api,
            )?);
            instance.set_property("pause", "yes")?;
            Ok(instance)
        }

        fn set_bounds(&self, bounds: PlayerBounds) -> Result<(), String> {
            self.renderer
                .as_ref()
                .ok_or_else(|| "Видеослой не инициализирован".to_owned())?
                .set_bounds(bounds)
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
            self.renderer = None;
            unsafe { (self.api.terminate_destroy)(self.handle) };
        }
    }

    #[derive(Default)]
    pub struct PlayerInner {
        pub mpv: Option<MpvInstance>,
    }

    impl PlayerInner {
        pub fn create_surface(
            &mut self,
            window: &tauri::WebviewWindow,
            bounds: PlayerBounds,
            runtime_paths: &crate::mpv_runtime::MpvRuntimePaths,
        ) -> Result<(), String> {
            if let Some(mpv) = &self.mpv {
                return mpv.set_bounds(bounds);
            }
            let host = HWND(window.hwnd().map_err(|error| error.to_string())?.0);
            let mpv = MpvInstance::new(host, bounds, runtime_paths)?;
            self.mpv = Some(mpv);
            Ok(())
        }

        pub fn set_bounds(&self, bounds: PlayerBounds) -> Result<(), String> {
            if let Some(mpv) = &self.mpv {
                mpv.set_bounds(bounds)?;
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
        }
    }
}
