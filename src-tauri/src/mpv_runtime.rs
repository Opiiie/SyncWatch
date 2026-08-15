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
    ready_path: RwLock<Option<PathBuf>>,
    status: RwLock<MpvRuntimeStatus>,
}

impl Default for MpvRuntimeManager {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            ready_path: RwLock::new(None),
            status: RwLock::new(MpvRuntimeStatus::checking()),
        }
    }
}

impl MpvRuntimeManager {
    pub async fn status(&self) -> MpvRuntimeStatus {
        self.status.read().await.clone()
    }

    pub async fn ensure(&self, app: &tauri::AppHandle) -> Result<PathBuf, String> {
        if let Some(path) = self.cached_ready_path().await {
            return Ok(path);
        }

        let _operation = self.operation.lock().await;
        if let Some(path) = self.cached_ready_path().await {
            return Ok(path);
        }

        self.set_status(MpvRuntimeStatus::checking()).await;
        match self.ensure_inner(app).await {
            Ok(path) => {
                let size = fs::metadata(&path)
                    .await
                    .map(|value| value.len())
                    .unwrap_or(0);
                *self.ready_path.write().await = Some(path.clone());
                self.set_status(MpvRuntimeStatus::ready(size)).await;
                Ok(path)
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
        *self.ready_path.write().await = None;

        let result = async {
            if !file_matches_runtime(source).await? {
                return Err("Выбранный файл не соответствует используемой версии libmpv".to_owned());
            }
            let target = runtime_path(app)?;
            install_verified_copy(source, &target).await?;
            Ok(target)
        }
        .await;

        match result {
            Ok(path) => {
                let size = fs::metadata(&path)
                    .await
                    .map(|value| value.len())
                    .unwrap_or(0);
                *self.ready_path.write().await = Some(path.clone());
                self.set_status(MpvRuntimeStatus::ready(size)).await;
                Ok(path)
            }
            Err(message) => {
                self.set_status(MpvRuntimeStatus::error(message.clone()))
                    .await;
                Err(message)
            }
        }
    }

    async fn cached_ready_path(&self) -> Option<PathBuf> {
        let path = self.ready_path.read().await.clone()?;
        fs::try_exists(&path).await.unwrap_or(false).then_some(path)
    }

    async fn ensure_inner(&self, app: &tauri::AppHandle) -> Result<PathBuf, String> {
        let target = runtime_path(app)?;
        if file_matches_runtime(&target).await? {
            return Ok(target);
        }

        for candidate in legacy_candidates(app) {
            if candidate == target || !file_matches_runtime(&candidate).await? {
                continue;
            }
            install_verified_copy(&candidate, &target).await?;
            return Ok(target);
        }

        self.download_runtime(&target).await?;
        Ok(target)
    }

    async fn download_runtime(&self, target: &Path) -> Result<(), String> {
        let directory = target
            .parent()
            .ok_or_else(|| "Не удалось определить папку libmpv".to_owned())?;
        fs::create_dir_all(directory)
            .await
            .map_err(|error| format!("Не удалось создать папку libmpv: {error}"))?;
        let temporary = directory.join(format!("{RUNTIME_FILE_NAME}.download"));
        let _ = fs::remove_file(&temporary).await;

        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15 * 60))
            .user_agent("SyncWatch runtime manager")
            .build()
            .map_err(|error| format!("Не удалось подготовить загрузку libmpv: {error}"))?;
        let response = client
            .get(RUNTIME_URL)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| format!("Не удалось скачать libmpv: {error}"))?;
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
                .map_err(|error| format!("Не удалось сохранить libmpv: {error}"))?;
            let mut stream = response.bytes_stream();
            let mut digest = Sha256::new();
            let mut downloaded = 0_u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| format!("Загрузка libmpv прервана: {error}"))?;
                file.write_all(&chunk)
                    .await
                    .map_err(|error| format!("Не удалось сохранить libmpv: {error}"))?;
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
                .map_err(|error| format!("Не удалось завершить сохранение libmpv: {error}"))?;
            drop(file);
            if digest_hex(digest.finalize().as_slice()) != RUNTIME_SHA256 {
                return Err("Проверка загруженной libmpv не пройдена".to_owned());
            }
            activate_temporary_file(&temporary, target).await
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

fn runtime_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|directory| {
            directory
                .join("runtime")
                .join(RUNTIME_VERSION)
                .join(RUNTIME_FILE_NAME)
        })
        .map_err(|error| format!("Не удалось определить папку данных приложения: {error}"))
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
    let directory = target
        .parent()
        .ok_or_else(|| "Не удалось определить папку libmpv".to_owned())?;
    fs::create_dir_all(directory)
        .await
        .map_err(|error| format!("Не удалось создать папку libmpv: {error}"))?;
    let temporary = directory.join(format!("{RUNTIME_FILE_NAME}.copy"));
    let _ = fs::remove_file(&temporary).await;
    fs::copy(source, &temporary)
        .await
        .map_err(|error| format!("Не удалось скопировать libmpv: {error}"))?;
    if !file_matches_runtime(&temporary).await? {
        let _ = fs::remove_file(&temporary).await;
        return Err("Проверка скопированной libmpv не пройдена".to_owned());
    }
    activate_temporary_file(&temporary, target).await
}

async fn activate_temporary_file(temporary: &Path, target: &Path) -> Result<(), String> {
    if fs::try_exists(target).await.unwrap_or(false) {
        fs::remove_file(target)
            .await
            .map_err(|error| format!("Не удалось заменить libmpv: {error}"))?;
    }
    fs::rename(temporary, target)
        .await
        .map_err(|error| format!("Не удалось активировать libmpv: {error}"))
}

async fn file_matches_runtime(path: &Path) -> Result<bool, String> {
    let mut file = match fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Не удалось проверить libmpv: {error}")),
    };
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("Не удалось проверить libmpv: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest_hex(digest.finalize().as_slice()) == RUNTIME_SHA256)
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
