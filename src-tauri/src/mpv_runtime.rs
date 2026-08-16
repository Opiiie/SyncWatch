use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::Manager;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex, RwLock},
};

const RUNTIME_VERSION: &str = "mpv-v1";
const RUNTIME_FILE_NAME: &str = "libmpv-2.dll";
const RUNTIME_URL: &str =
    "https://github.com/Opiiie/SyncWatch/releases/download/runtime-v1/libmpv-2.dll";
const RUNTIME_SHA256: &str = "b7ce1d6145dd86be99b3eb04cd4307d484f22f1b957104c0c437b14999451bd2";
const ANGLE_FILE_NAME: &str = "av_libglesv2.dll";
const ANGLE_URL: &str =
    "https://github.com/Opiiie/SyncWatch/releases/download/runtime-v1/av_libglesv2.dll";
const ANGLE_SHA256: &str = "53191a77fe783cd757ca7767077c2a64a662e7043777a5b4ab74980d4a0b73e3";

#[derive(Clone, Debug)]
pub struct MpvRuntimePaths {
    pub mpv: PathBuf,
    pub angle: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MpvRuntimeStatus {
    stage: RuntimeStage,
    downloaded_bytes: u64,
    total_bytes: u64,
    message: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum RuntimeStage {
    Checking,
    Downloading,
    Ready,
    Error,
}

impl MpvRuntimeStatus {
    fn checking() -> Self {
        Self {
            stage: RuntimeStage::Checking,
            downloaded_bytes: 0,
            total_bytes: 0,
            message: None,
        }
    }

    fn ready(size: u64) -> Self {
        Self {
            stage: RuntimeStage::Ready,
            downloaded_bytes: size,
            total_bytes: size,
            message: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            stage: RuntimeStage::Error,
            downloaded_bytes: 0,
            total_bytes: 0,
            message: Some(message),
        }
    }
}

pub struct MpvRuntimeManager {
    operation: Mutex<()>,
    ready_paths: RwLock<Option<MpvRuntimePaths>>,
    status: RwLock<MpvRuntimeStatus>,
}

impl Default for MpvRuntimeManager {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            ready_paths: RwLock::new(None),
            status: RwLock::new(MpvRuntimeStatus::checking()),
        }
    }
}

impl MpvRuntimeManager {
    pub async fn status(&self) -> MpvRuntimeStatus {
        self.status.read().await.clone()
    }

    pub async fn ensure(&self, app: &tauri::AppHandle) -> Result<MpvRuntimePaths, String> {
        if let Some(paths) = self.cached_ready_paths().await {
            return Ok(paths);
        }

        let _operation = self.operation.lock().await;
        if let Some(paths) = self.cached_ready_paths().await {
            return Ok(paths);
        }

        self.set_status(MpvRuntimeStatus::checking()).await;
        match self.ensure_inner(app).await {
            Ok(paths) => {
                let size = runtime_size(&paths).await;
                *self.ready_paths.write().await = Some(paths.clone());
                self.set_status(MpvRuntimeStatus::ready(size)).await;
                Ok(paths)
            }
            Err(message) => {
                self.set_status(MpvRuntimeStatus::error(message.clone()))
                    .await;
                Err(message)
            }
        }
    }

    pub async fn install_from_file(
        &self,
        app: &tauri::AppHandle,
        source: &Path,
    ) -> Result<PathBuf, String> {
        let _operation = self.operation.lock().await;
        self.set_status(MpvRuntimeStatus::checking()).await;
        *self.ready_paths.write().await = None;

        let result = async {
            if !file_matches_runtime(source).await? {
                return Err("Выбранный файл не соответствует используемой версии libmpv".to_owned());
            }
            let paths = runtime_paths(app)?;
            install_verified_copy(source, &paths.mpv).await?;
            ensure_angle_runtime(app, self, &paths.angle).await?;
            Ok(paths)
        }
        .await;

        match result {
            Ok(paths) => {
                let size = runtime_size(&paths).await;
                *self.ready_paths.write().await = Some(paths.clone());
                self.set_status(MpvRuntimeStatus::ready(size)).await;
                Ok(paths.mpv)
            }
            Err(message) => {
                self.set_status(MpvRuntimeStatus::error(message.clone()))
                    .await;
                Err(message)
            }
        }
    }

    async fn cached_ready_paths(&self) -> Option<MpvRuntimePaths> {
        let paths = self.ready_paths.read().await.clone()?;
        (fs::try_exists(&paths.mpv).await.unwrap_or(false)
            && fs::try_exists(&paths.angle).await.unwrap_or(false))
        .then_some(paths)
    }

    async fn ensure_inner(&self, app: &tauri::AppHandle) -> Result<MpvRuntimePaths, String> {
        let paths = runtime_paths(app)?;
        if !file_matches_runtime(&paths.mpv).await? {
            let mut installed = false;

            for candidate in legacy_candidates(app) {
                if candidate == paths.mpv || !file_matches_runtime(&candidate).await? {
                    continue;
                }
                install_verified_copy(&candidate, &paths.mpv).await?;
                installed = true;
                break;
            }

            if !installed {
                self.download_runtime(&paths.mpv).await?;
            }
        }

        ensure_angle_runtime(app, self, &paths.angle).await?;
        Ok(paths)
    }

    async fn download_runtime(&self, target: &Path) -> Result<(), String> {
        self.download_component(
            target,
            RUNTIME_FILE_NAME,
            RUNTIME_URL,
            RUNTIME_SHA256,
            "libmpv",
        )
        .await
    }

    async fn download_component(
        &self,
        target: &Path,
        file_name: &str,
        url: &str,
        expected_sha256: &str,
        label: &str,
    ) -> Result<(), String> {
        let directory = target
            .parent()
            .ok_or_else(|| format!("Не удалось определить папку {label}"))?;
        fs::create_dir_all(directory)
            .await
            .map_err(|error| format!("Не удалось создать папку {label}: {error}"))?;
        let temporary = directory.join(format!("{file_name}.download"));
        let _ = fs::remove_file(&temporary).await;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15 * 60))
            .user_agent("SyncWatch runtime manager")
            .build()
            .map_err(|error| format!("Не удалось подготовить загрузку libmpv: {error}"))?;
        let response = client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| format!("Не удалось скачать {label}: {error}"))?;
        let total = response.content_length().unwrap_or(0);
        self.set_status(MpvRuntimeStatus {
            stage: RuntimeStage::Downloading,
            downloaded_bytes: 0,
            total_bytes: total,
            message: None,
        })
        .await;

        let download_result = async {
            let mut file = fs::File::create(&temporary)
                .await
                .map_err(|error| format!("Не удалось сохранить {label}: {error}"))?;
            let mut stream = response.bytes_stream();
            let mut digest = Sha256::new();
            let mut downloaded = 0_u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| format!("Загрузка {label} прервана: {error}"))?;
                file.write_all(&chunk)
                    .await
                    .map_err(|error| format!("Не удалось сохранить {label}: {error}"))?;
                digest.update(&chunk);
                downloaded = downloaded.saturating_add(chunk.len() as u64);
                self.set_status(MpvRuntimeStatus {
                    stage: RuntimeStage::Downloading,
                    downloaded_bytes: downloaded,
                    total_bytes: total,
                    message: None,
                })
                .await;
            }
            file.flush()
                .await
                .map_err(|error| format!("Не удалось завершить сохранение {label}: {error}"))?;
            drop(file);
            if digest_hex(digest.finalize().as_slice()) != expected_sha256 {
                return Err(format!("Проверка загруженной {label} не пройдена"));
            }
            activate_temporary_file(&temporary, target, label).await
        }
        .await;

        if download_result.is_err() {
            let _ = fs::remove_file(&temporary).await;
        }
        download_result
    }

    async fn set_status(&self, status: MpvRuntimeStatus) {
        *self.status.write().await = status;
    }
}

fn runtime_paths(app: &tauri::AppHandle) -> Result<MpvRuntimePaths, String> {
    app.path()
        .app_local_data_dir()
        .map(|directory| {
            let directory = directory.join("runtime").join(RUNTIME_VERSION);
            MpvRuntimePaths {
                mpv: directory.join(RUNTIME_FILE_NAME),
                angle: directory.join(ANGLE_FILE_NAME),
            }
        })
        .map_err(|error| format!("Не удалось определить папку данных приложения: {error}"))
}

async fn runtime_size(paths: &MpvRuntimePaths) -> u64 {
    let mpv = fs::metadata(&paths.mpv)
        .await
        .map(|value| value.len())
        .unwrap_or(0);
    let angle = fs::metadata(&paths.angle)
        .await
        .map(|value| value.len())
        .unwrap_or(0);
    mpv.saturating_add(angle)
}

async fn ensure_angle_runtime(
    app: &tauri::AppHandle,
    manager: &MpvRuntimeManager,
    target: &Path,
) -> Result<(), String> {
    if file_matches(target, ANGLE_SHA256, "ANGLE").await? {
        return Ok(());
    }
    if let Some(source) = angle_candidates(app)
        .into_iter()
        .find(|path| std::fs::exists(path).unwrap_or(false))
    {
        if file_matches(&source, ANGLE_SHA256, "ANGLE").await? {
            install_verified_component(&source, target, ANGLE_SHA256, ANGLE_FILE_NAME, "ANGLE")
                .await?;
            return Ok(());
        }
    }
    manager
        .download_component(target, ANGLE_FILE_NAME, ANGLE_URL, ANGLE_SHA256, "ANGLE")
        .await
}

fn angle_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("SYNCWATCH_ANGLE_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(directory) = app.path().resource_dir() {
        candidates.push(directory.join(ANGLE_FILE_NAME));
    }
    candidates
}

fn legacy_candidates(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("SYNCWATCH_LIBMPV_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(directory) = app.path().resource_dir() {
        candidates.push(directory.join(RUNTIME_FILE_NAME));
        candidates.push(directory.join("resources").join(RUNTIME_FILE_NAME));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join(RUNTIME_FILE_NAME));
            candidates.push(directory.join("resources").join(RUNTIME_FILE_NAME));
        }
    }
    if let Ok(directory) = std::env::current_dir() {
        candidates.push(
            directory
                .join("src-tauri/resources")
                .join(RUNTIME_FILE_NAME),
        );
        candidates.push(directory.join("resources").join(RUNTIME_FILE_NAME));
    }
    candidates
}

async fn install_verified_copy(source: &Path, target: &Path) -> Result<(), String> {
    install_verified_component(source, target, RUNTIME_SHA256, RUNTIME_FILE_NAME, "libmpv").await
}

async fn install_verified_component(
    source: &Path,
    target: &Path,
    expected_sha256: &str,
    file_name: &str,
    label: &str,
) -> Result<(), String> {
    let directory = target
        .parent()
        .ok_or_else(|| format!("Не удалось определить папку {label}"))?;
    fs::create_dir_all(directory)
        .await
        .map_err(|error| format!("Не удалось создать папку {label}: {error}"))?;
    let temporary = directory.join(format!("{file_name}.copy"));
    let _ = fs::remove_file(&temporary).await;
    fs::copy(source, &temporary)
        .await
        .map_err(|error| format!("Не удалось скопировать {label}: {error}"))?;
    if !file_matches(&temporary, expected_sha256, label).await? {
        let _ = fs::remove_file(&temporary).await;
        return Err(format!("Проверка скопированной {label} не пройдена"));
    }
    activate_temporary_file(&temporary, target, label).await
}

async fn activate_temporary_file(
    temporary: &Path,
    target: &Path,
    label: &str,
) -> Result<(), String> {
    if fs::try_exists(target).await.unwrap_or(false) {
        fs::remove_file(target)
            .await
            .map_err(|error| format!("Не удалось заменить {label}: {error}"))?;
    }
    fs::rename(temporary, target)
        .await
        .map_err(|error| format!("Не удалось активировать {label}: {error}"))
}

async fn file_matches_runtime(path: &Path) -> Result<bool, String> {
    file_matches(path, RUNTIME_SHA256, "libmpv").await
}

async fn file_matches(path: &Path, expected_sha256: &str, label: &str) -> Result<bool, String> {
    let mut file = match fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Не удалось проверить {label}: {error}")),
    };
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("Не удалось проверить {label}: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest_hex(digest.finalize().as_slice()) == expected_sha256)
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[tauri::command]
pub async fn ensure_mpv_runtime(
    app: tauri::AppHandle,
    manager: tauri::State<'_, MpvRuntimeManager>,
) -> Result<MpvRuntimeStatus, String> {
    manager.ensure(&app).await?;
    Ok(manager.status().await)
}

#[tauri::command]
pub async fn get_mpv_runtime_status(
    manager: tauri::State<'_, MpvRuntimeManager>,
) -> Result<MpvRuntimeStatus, String> {
    Ok(manager.status().await)
}

#[tauri::command]
pub async fn install_mpv_runtime(
    app: tauri::AppHandle,
    manager: tauri::State<'_, MpvRuntimeManager>,
    path: String,
) -> Result<MpvRuntimeStatus, String> {
    manager.install_from_file(&app, Path::new(&path)).await?;
    Ok(manager.status().await)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::{digest_hex, MpvRuntimeStatus};

    #[test]
    fn formats_sha256_in_lowercase() {
        let digest = Sha256::digest(b"abc");
        assert_eq!(
            digest_hex(digest.as_slice()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn runtime_status_uses_frontend_stage_names() {
        let value = serde_json::to_value(MpvRuntimeStatus::ready(123)).unwrap();
        assert_eq!(value["stage"], "ready");
        assert_eq!(value["downloadedBytes"], 123);
        assert_eq!(value["totalBytes"], 123);
    }
}
