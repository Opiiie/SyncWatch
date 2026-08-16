use std::{
    collections::{HashMap, HashSet},
    io::SeekFrom,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    path::{Path as FilePath, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::{broadcast, Mutex, RwLock},
    time::{sleep_until, Instant},
};
use tokio_util::io::ReaderStream;

use crate::protocol::{
    ClientMessage, ParticipantSnapshot, PlaybackAction, PlaybackState, PlaylistItem, RoomSnapshot,
    ServerMessage, PROTOCOL_VERSION,
};

#[derive(Clone)]
pub struct ServerState {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    media_sources: Arc<RwLock<HashMap<String, RoomMediaSources>>>,
}

struct Room {
    media_name: String,
    playlist: Vec<PlaylistItem>,
    active_playlist_item_id: Option<String>,
    host_participant_id: String,
    participants: HashMap<String, Participant>,
    playback: PlaybackState,
    media_access_token: String,
    bandwidth: Arc<BandwidthGovernor>,
    events: broadcast::Sender<ServerMessage>,
}

struct RoomMediaSources {
    access_token: String,
    items: HashMap<String, MediaFileSource>,
    subtitles: HashMap<String, HashMap<String, PathBuf>>,
}

#[derive(Clone)]
struct MediaFileSource {
    path: PathBuf,
    size: u64,
    required_rate_per_viewer: Arc<AtomicU64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSourceInput {
    item_id: String,
    path: String,
    #[serde(default)]
    subtitles: Vec<SubtitleSourceInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSourceInput {
    subtitle_id: String,
    path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSubtitleFile {
    path: String,
    name: String,
    language: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSubtitles {
    video_path: String,
    subtitles: Vec<DetectedSubtitleFile>,
}

#[derive(Deserialize)]
struct MediaQuery {
    token: String,
}

struct Participant {
    display_name: String,
    ping_ms: Option<u64>,
    minimum_ping_ms: Option<u64>,
}

const MIN_MEDIA_RATE: u64 = 2_097_152;
const INITIAL_MEDIA_RATE: u64 = 8_388_608;
const MAX_MEDIA_RATE: u64 = 67_108_864;
const UNKNOWN_MEDIA_RATE_PER_VIEWER: u64 = 12_582_912;
const MEDIA_RATE_SAFETY_NUMERATOR: u64 = 7;
const MEDIA_RATE_SAFETY_DENOMINATOR: u64 = 4;
const MEDIA_CHUNK_SIZE: usize = 256 * 1024;

struct MediaTransferProfile {
    bandwidth: Arc<BandwidthGovernor>,
    required_rate_per_viewer: Arc<AtomicU64>,
}

struct BandwidthGovernor {
    bytes_per_second: AtomicU64,
    viewer_count: AtomicU64,
    next_send: Mutex<Instant>,
}

impl BandwidthGovernor {
    fn new() -> Self {
        Self {
            bytes_per_second: AtomicU64::new(INITIAL_MEDIA_RATE),
            viewer_count: AtomicU64::new(0),
            next_send: Mutex::new(Instant::now()),
        }
    }

    async fn pace(&self, bytes: usize, required_rate_per_viewer: &AtomicU64) {
        let viewers = self.viewer_count.load(Ordering::Relaxed).max(1);
        let required_rate = required_rate_per_viewer
            .load(Ordering::Relaxed)
            .saturating_mul(viewers)
            .clamp(MIN_MEDIA_RATE, MAX_MEDIA_RATE);
        let rate = self
            .bytes_per_second
            .load(Ordering::Relaxed)
            .max(required_rate)
            .clamp(MIN_MEDIA_RATE, MAX_MEDIA_RATE);
        let spacing = Duration::from_secs_f64(bytes as f64 / rate as f64);
        let scheduled = {
            let mut next = self.next_send.lock().await;
            let now = Instant::now();
            if *next < now {
                *next = now;
            }
            let scheduled = *next;
            *next += spacing;
            scheduled
        };
        sleep_until(scheduled).await;
    }

    fn set_viewer_count(&self, viewer_count: usize) {
        self.viewer_count
            .store(viewer_count as u64, Ordering::Relaxed);
    }

    fn report_ping(&self, ping_ms: u64, minimum_ping_ms: u64) {
        let current = self.bytes_per_second.load(Ordering::Relaxed);
        self.bytes_per_second.store(
            adjusted_media_rate(current, ping_ms, minimum_ping_ms),
            Ordering::Relaxed,
        );
    }
}

fn adjusted_media_rate(current: u64, ping_ms: u64, minimum_ping_ms: u64) -> u64 {
    let queue_delay = ping_ms.saturating_sub(minimum_ping_ms);
    let adjusted = if ping_ms >= 240 || queue_delay >= 120 {
        current.saturating_mul(3) / 4
    } else if ping_ms >= 150 || queue_delay >= 70 {
        current.saturating_mul(7) / 8
    } else if ping_ms <= 90 && queue_delay <= 30 {
        current.saturating_add(384 * 1024)
    } else {
        current
    };
    adjusted.clamp(MIN_MEDIA_RATE, MAX_MEDIA_RATE)
}

fn required_media_rate_per_viewer(file_size: u64, duration_seconds: f64) -> u64 {
    let per_viewer = if duration_seconds.is_finite() && duration_seconds > 0.0 {
        let average_rate = (file_size as f64 / duration_seconds).ceil() as u64;
        average_rate
            .saturating_mul(MEDIA_RATE_SAFETY_NUMERATOR)
            .div_ceil(MEDIA_RATE_SAFETY_DENOMINATOR)
    } else {
        UNKNOWN_MEDIA_RATE_PER_VIEWER
    };
    per_viewer.clamp(MIN_MEDIA_RATE, MAX_MEDIA_RATE)
}

impl Room {
    fn participant_snapshots(&self) -> Vec<ParticipantSnapshot> {
        let mut participants = self
            .participants
            .iter()
            .map(|(participant_id, participant)| ParticipantSnapshot {
                participant_id: participant_id.clone(),
                display_name: participant.display_name.clone(),
                is_host: participant_id == &self.host_participant_id,
                ping_ms: participant.ping_ms,
            })
            .collect::<Vec<_>>();
        participants.sort_by(|left, right| {
            right.is_host.cmp(&left.is_host).then_with(|| {
                left.display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase())
            })
        });
        participants
    }

    fn snapshot(&self, room_code: &str) -> RoomSnapshot {
        RoomSnapshot {
            room_code: room_code.to_owned(),
            media_name: self.media_name.clone(),
            playlist: self.playlist.clone(),
            active_playlist_item_id: self.active_playlist_item_id.clone(),
            participant_count: self.participants.len(),
            participants: self.participant_snapshots(),
            host_participant_id: self.host_participant_id.clone(),
            media_access_token: self.media_access_token.clone(),
            playback: self.playback.clone(),
        }
    }
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            media_sources: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn set_media_sources(
        &self,
        room_code: &str,
        access_token: &str,
        sources: Vec<MediaSourceInput>,
    ) -> Result<(), String> {
        let room_code = normalize_room_code(room_code).map_err(server_error_message)?;
        validate_media_token(access_token).map_err(server_error_message)?;
        if sources.len() > 500 {
            return Err("В сессии может быть не более 500 видео".to_owned());
        }
        let mut items = HashMap::with_capacity(sources.len());
        let mut subtitles = HashMap::with_capacity(sources.len());
        for source in sources {
            let item_id = source.item_id.trim();
            if item_id.is_empty() || item_id.len() > 128 || items.contains_key(item_id) {
                return Err("Не удалось подготовить плейлист для передачи".to_owned());
            }
            let path = tokio::fs::canonicalize(&source.path)
                .await
                .map_err(|_| format!("Файл не найден: {}", source.path))?;
            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|_| format!("Не удалось открыть файл: {}", source.path))?;
            if !metadata.is_file() {
                return Err(format!("Это не видеофайл: {}", source.path));
            }
            items.insert(
                item_id.to_owned(),
                MediaFileSource {
                    path,
                    size: metadata.len(),
                    required_rate_per_viewer: Arc::new(AtomicU64::new(
                        UNKNOWN_MEDIA_RATE_PER_VIEWER,
                    )),
                },
            );
            if source.subtitles.len() > 64 {
                return Err("К одному видео можно добавить не более 64 файлов субтитров".to_owned());
            }
            let mut subtitle_items = HashMap::with_capacity(source.subtitles.len());
            for subtitle in source.subtitles {
                let subtitle_id = subtitle.subtitle_id.trim();
                if subtitle_id.is_empty()
                    || subtitle_id.len() > 128
                    || subtitle_items.contains_key(subtitle_id)
                {
                    return Err("Не удалось подготовить внешние субтитры".to_owned());
                }
                let subtitle_path = tokio::fs::canonicalize(&subtitle.path)
                    .await
                    .map_err(|_| format!("Файл субтитров не найден: {}", subtitle.path))?;
                let metadata = tokio::fs::metadata(&subtitle_path)
                    .await
                    .map_err(|_| format!("Не удалось открыть субтитры: {}", subtitle.path))?;
                if !metadata.is_file() || !is_subtitle_path(&subtitle_path) {
                    return Err(format!(
                        "Неподдерживаемый файл субтитров: {}",
                        subtitle.path
                    ));
                }
                subtitle_items.insert(subtitle_id.to_owned(), subtitle_path);
            }
            subtitles.insert(item_id.to_owned(), subtitle_items);
        }
        self.media_sources.write().await.insert(
            room_code,
            RoomMediaSources {
                access_token: access_token.to_owned(),
                items,
                subtitles,
            },
        );
        Ok(())
    }

    async fn update_media_rate_hints(&self, room_code: &str, playlist: &[PlaylistItem]) {
        let sources = self.media_sources.read().await;
        let Some(room_sources) = sources.get(room_code) else {
            return;
        };
        for item in playlist {
            let Some(source) = room_sources.items.get(&item.id) else {
                continue;
            };
            source.required_rate_per_viewer.store(
                required_media_rate_per_viewer(source.size, item.duration_seconds),
                Ordering::Relaxed,
            );
        }
    }

    pub async fn discoverable_rooms(&self, room_code: Option<&str>) -> Vec<DiscoverableRoom> {
        let requested_code = room_code.map(|code| code.trim().to_uppercase());
        let rooms = self.rooms.read().await;
        rooms
            .iter()
            .filter(|(code, _)| {
                requested_code
                    .as_ref()
                    .is_none_or(|requested| requested == *code)
            })
            .map(|(code, room)| DiscoverableRoom {
                room_code: code.clone(),
                host_display_name: room
                    .participants
                    .get(&room.host_participant_id)
                    .map(|participant| participant.display_name.clone())
                    .unwrap_or_else(|| "Хост".to_owned()),
                participant_count: room.participants.len(),
                has_video: !room.playlist.is_empty(),
            })
            .collect()
    }
}

pub async fn find_external_subtitles(video_paths: Vec<String>) -> Vec<DetectedSubtitles> {
    let mut results = Vec::with_capacity(video_paths.len());
    for video_path in video_paths {
        let path = PathBuf::from(&video_path);
        let Some(directory) = path.parent() else {
            results.push(DetectedSubtitles {
                video_path,
                subtitles: Vec::new(),
            });
            continue;
        };
        let video_stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned();
        let video_stem_lower = video_stem.to_lowercase();
        let mut found = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(directory).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let subtitle_path = entry.path();
                if !is_subtitle_path(&subtitle_path) {
                    continue;
                }
                let subtitle_stem = subtitle_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                let subtitle_stem_lower = subtitle_stem.to_lowercase();
                if !subtitle_matches_video(&video_stem_lower, &subtitle_stem_lower) {
                    continue;
                }
                let name = subtitle_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Субтитры")
                    .to_owned();
                found.push(DetectedSubtitleFile {
                    path: subtitle_path.to_string_lossy().into_owned(),
                    language: infer_subtitle_language(&video_stem, subtitle_stem),
                    name,
                });
                if found.len() == 64 {
                    break;
                }
            }
        }
        found.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        results.push(DetectedSubtitles {
            video_path,
            subtitles: found,
        });
    }
    results
}

pub struct DiscoverableRoom {
    pub room_code: String,
    pub host_display_name: String,
    pub participant_count: usize,
    pub has_video: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsServerInfo {
    port: u16,
    local_address: String,
    ws_url: String,
}

impl WsServerInfo {
    fn local(port: u16) -> Self {
        let ip = discover_local_ip();
        Self {
            port,
            local_address: format!("{ip}:{port}"),
            ws_url: format!("ws://127.0.0.1:{port}/ws"),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

pub fn bind_available_port() -> Result<(std::net::TcpListener, WsServerInfo), std::io::Error> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    listener.set_nonblocking(true)?;
    let info = WsServerInfo::local(listener.local_addr()?.port());
    Ok((listener, info))
}

pub async fn run(
    state: ServerState,
    listener: std::net::TcpListener,
) -> Result<(), std::io::Error> {
    let app = server_router(state);
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, app).await
}

fn server_router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/media/{room_code}/{item_id}", get(stream_media))
        .route(
            "/subtitles/{room_code}/{item_id}/{subtitle_id}",
            get(stream_subtitle),
        )
        .route("/ws", get(upgrade_websocket))
        .with_state(state)
}

async fn stream_media(
    Path((room_code, item_id)): Path<(String, String)>,
    Query(query): Query<MediaQuery>,
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    let room_code = room_code.trim().to_uppercase();
    let bandwidth = {
        let rooms = state.rooms.read().await;
        rooms.get(&room_code).and_then(|room| {
            (room.media_access_token == query.token
                && room.playlist.iter().any(|item| item.id == item_id))
            .then(|| room.bandwidth.clone())
        })
    };
    let Some(bandwidth) = bandwidth else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let source = {
        let sources = state.media_sources.read().await;
        sources.get(&room_code).and_then(|room| {
            (room.access_token == query.token)
                .then(|| room.items.get(&item_id).cloned())
                .flatten()
        })
    };
    let Some(source) = source else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let transfer = MediaTransferProfile {
        bandwidth,
        required_rate_per_viewer: source.required_rate_per_viewer,
    };
    stream_file(source.path, headers, Some(transfer)).await
}

async fn stream_subtitle(
    Path((room_code, item_id, subtitle_id)): Path<(String, String, String)>,
    Query(query): Query<MediaQuery>,
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Response {
    let room_code = room_code.trim().to_uppercase();
    let authorized = {
        let rooms = state.rooms.read().await;
        rooms.get(&room_code).is_some_and(|room| {
            room.media_access_token == query.token
                && room.playlist.iter().any(|item| {
                    item.id == item_id
                        && item
                            .external_subtitles
                            .iter()
                            .any(|subtitle| subtitle.id == subtitle_id)
                })
        })
    };
    if !authorized {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = {
        let sources = state.media_sources.read().await;
        sources.get(&room_code).and_then(|room| {
            (room.access_token == query.token)
                .then(|| {
                    room.subtitles
                        .get(&item_id)
                        .and_then(|items| items.get(&subtitle_id))
                        .cloned()
                })
                .flatten()
        })
    };
    let Some(path) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    stream_file(path, headers, None).await
}

async fn stream_file(
    path: PathBuf,
    headers: HeaderMap,
    transfer: Option<MediaTransferProfile>,
) -> Response {
    let Ok(mut file) = tokio::fs::File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(metadata) = file.metadata().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let size = metadata.len();
    let transfer = transfer.map(|profile| (profile.bandwidth, profile.required_rate_per_viewer));
    let requested_range = match headers.get(header::RANGE) {
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|value| parse_byte_range(value, size))
        {
            Some(range) => Some(range),
            None => return range_not_satisfiable(size),
        },
        None => None,
    };
    let (status, start, end) = requested_range
        .map(|(start, end)| (StatusCode::PARTIAL_CONTENT, start, end))
        .unwrap_or((StatusCode::OK, 0, size.saturating_sub(1)));
    let length = if size == 0 { 0 } else { end - start + 1 };
    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let stream =
        ReaderStream::with_capacity(file.take(length), MEDIA_CHUNK_SIZE).then(move |chunk| {
            let transfer = transfer.clone();
            async move {
                if let (Ok(bytes), Some((governor, required_rate))) = (&chunk, transfer) {
                    governor.pace(bytes.len(), &required_rate).await;
                }
                chunk
            }
        });
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    let response_headers = response.headers_mut();
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(media_content_type(&path)),
    );
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        response_headers.insert(header::CONTENT_LENGTH, value);
    }
    if status == StatusCode::PARTIAL_CONTENT {
        if let Ok(value) = HeaderValue::from_str(&format!("bytes {start}-{end}/{size}")) {
            response_headers.insert(header::CONTENT_RANGE, value);
        }
    }
    response
}

fn is_subtitle_path(path: &FilePath) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("srt") | Some("ass") | Some("ssa") | Some("vtt") | Some("sub")
    )
}

fn subtitle_matches_video(video_stem: &str, subtitle_stem: &str) -> bool {
    subtitle_stem
        .strip_prefix(video_stem)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
}

fn infer_subtitle_language(video_stem: &str, subtitle_stem: &str) -> Option<String> {
    let suffix = subtitle_stem
        .get(video_stem.len()..)?
        .trim_start_matches(['.', ' ', '_', '-']);
    suffix
        .split(['.', ' ', '_', '-'])
        .find(|part| {
            (2..=3).contains(&part.len())
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
        })
        .map(str::to_ascii_lowercase)
}

fn parse_byte_range(value: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix.min(size)), size - 1));
    }
    let start = start.parse::<u64>().ok()?;
    if start >= size {
        return None;
    }
    let end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then_some((start, end))
}

fn range_not_satisfiable(size: u64) -> Response {
    let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response
}

fn media_content_type(path: &FilePath) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mkv") => "video/x-matroska",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("mov") => "video/quicktime",
        Some("mpeg") | Some("mpg") => "video/mpeg",
        Some("ts") | Some("m2ts") => "video/mp2t",
        Some("srt") | Some("sub") => "application/x-subrip; charset=utf-8",
        Some("ass") | Some("ssa") => "text/x-ssa; charset=utf-8",
        Some("vtt") => "text/vtt; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn upgrade_websocket(ws: WebSocketUpgrade, State(state): State<ServerState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: ServerState) {
    let Some(Ok(Message::Text(first_message))) = socket.recv().await else {
        return;
    };

    let parsed = match serde_json::from_str::<ClientMessage>(&first_message) {
        Ok(message) => message,
        Err(_) => {
            send_direct(
                &mut socket,
                ServerMessage::error("invalid_message", "Некорректное сообщение протокола"),
            )
            .await;
            return;
        }
    };

    let handshake = match perform_handshake(parsed, &state).await {
        Ok(handshake) => handshake,
        Err(error) => {
            send_direct(&mut socket, error).await;
            return;
        }
    };

    send_direct(
        &mut socket,
        ServerMessage::RoomSnapshot {
            room: handshake.snapshot,
        },
    )
    .await;

    let (mut sender, mut receiver) = socket.split();
    let mut room_events = handshake.events.subscribe();
    let room_code = handshake.room_code;
    let participant_id = handshake.participant_id;

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(message) = serde_json::from_str::<ClientMessage>(&text) {
                            if let Some(response) = handle_client_message(message, &room_code, &participant_id, &state).await {
                                if send_json(&mut sender, &response).await.is_err() { break; }
                            }
                        } else {
                            let error = ServerMessage::error("invalid_message", "Некорректное сообщение протокола");
                            if send_json(&mut sender, &error).await.is_err() { break; }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            event = room_events.recv() => {
                match event {
                    Ok(message) => if send_json(&mut sender, &message).await.is_err() { break; },
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    remove_participant(&state, &room_code, &participant_id).await;
}

struct Handshake {
    room_code: String,
    participant_id: String,
    snapshot: RoomSnapshot,
    events: broadcast::Sender<ServerMessage>,
}

async fn perform_handshake(
    message: ClientMessage,
    state: &ServerState,
) -> Result<Handshake, ServerMessage> {
    match message {
        ClientMessage::CreateRoom(payload) => {
            validate_version(payload.version)?;
            let room_code = normalize_room_code(&payload.room_code)?;
            let media_access_token = validate_media_token(&payload.media_access_token)?;
            let (events, _) = broadcast::channel(64);
            let display_name = normalize_display_name(&payload.display_name)?;
            let mut participants = HashMap::new();
            participants.insert(
                payload.participant_id.clone(),
                Participant {
                    display_name,
                    ping_ms: None,
                    minimum_ping_ms: None,
                },
            );
            let playlist = normalize_playlist(payload.playlist)?;
            let active_playlist_item_id =
                normalize_active_item(&playlist, payload.active_playlist_item_id);
            let media_name = active_playlist_item_id
                .as_deref()
                .and_then(|id| playlist.iter().find(|item| item.id == id))
                .map(|item| item.name.clone())
                .unwrap_or_else(|| payload.media_name.trim().to_owned());
            let initial_position = active_playlist_item_id
                .as_deref()
                .and_then(|id| playlist.iter().find(|item| item.id == id))
                .map(|item| item.progress_seconds)
                .unwrap_or(0.0);
            let room = Room {
                media_name,
                playlist,
                active_playlist_item_id,
                host_participant_id: payload.participant_id.clone(),
                participants,
                playback: PlaybackState {
                    position_seconds: initial_position,
                    ..PlaybackState::default()
                },
                media_access_token,
                bandwidth: Arc::new(BandwidthGovernor::new()),
                events: events.clone(),
            };
            let snapshot = room.snapshot(&room_code);
            let rate_playlist = room.playlist.clone();
            let mut rooms = state.rooms.write().await;
            if rooms.contains_key(&room_code) {
                return Err(ServerMessage::error(
                    "room_exists",
                    "Комната с таким кодом уже существует",
                ));
            }
            rooms.insert(room_code.clone(), room);
            drop(rooms);
            state
                .update_media_rate_hints(&room_code, &rate_playlist)
                .await;
            Ok(Handshake {
                room_code,
                participant_id: payload.participant_id,
                snapshot,
                events,
            })
        }
        ClientMessage::JoinRoom(payload) => {
            validate_version(payload.version)?;
            let room_code = normalize_room_code(&payload.room_code)?;
            let display_name = normalize_display_name(&payload.display_name)?;
            let mut rooms = state.rooms.write().await;
            let room = rooms
                .get_mut(&room_code)
                .ok_or_else(|| ServerMessage::error("room_not_found", "Комната не найдена"))?;
            room.participants.insert(
                payload.participant_id.clone(),
                Participant {
                    display_name: display_name.clone(),
                    ping_ms: None,
                    minimum_ping_ms: None,
                },
            );
            room.bandwidth
                .set_viewer_count(room.participants.len().saturating_sub(1));
            let snapshot = room.snapshot(&room_code);
            let events = room.events.clone();
            let _ = events.send(ServerMessage::ParticipantJoined {
                room_code: room_code.clone(),
                participant_id: payload.participant_id.clone(),
                display_name,
            });
            let _ = events.send(ServerMessage::ParticipantCount {
                room_code: room_code.clone(),
                participant_count: room.participants.len(),
            });
            let _ = events.send(ServerMessage::ParticipantList {
                room_code: room_code.clone(),
                participants: room.participant_snapshots(),
            });
            Ok(Handshake {
                room_code,
                participant_id: payload.participant_id,
                snapshot,
                events,
            })
        }
        _ => Err(ServerMessage::error(
            "handshake_required",
            "Первым сообщением должно быть создание комнаты или подключение",
        )),
    }
}

async fn handle_client_message(
    message: ClientMessage,
    connected_room_code: &str,
    connected_participant_id: &str,
    state: &ServerState,
) -> Option<ServerMessage> {
    match message {
        ClientMessage::PlaybackCommand(payload) => {
            let requested_room_code = payload.room_code.trim().to_uppercase();
            if requested_room_code != connected_room_code {
                return Some(ServerMessage::error(
                    "room_mismatch",
                    "Команда относится к другой комнате",
                ));
            }
            let mut rooms = state.rooms.write().await;
            let room = rooms.get_mut(connected_room_code)?;
            if room.active_playlist_item_id.is_none() || room.playlist.is_empty() {
                return None;
            }
            room.playback.position_seconds = payload.position_seconds.max(0.0);
            room.playback.updated_at_ms = now_ms();
            room.playback.revision += 1;
            match payload.action {
                PlaybackAction::Play => room.playback.playing = true,
                PlaybackAction::Pause => room.playback.playing = false,
                PlaybackAction::Seek => {}
            }
            let message = ServerMessage::PlaybackState {
                room_code: connected_room_code.to_owned(),
                playback: room.playback.clone(),
            };
            let _ = room.events.send(message);
            None
        }
        ClientMessage::PlaybackRate(payload) => {
            let requested_room_code = payload.room_code.trim().to_uppercase();
            if requested_room_code != connected_room_code {
                return Some(ServerMessage::error(
                    "room_mismatch",
                    "Команда относится к другой комнате",
                ));
            }
            let mut rooms = state.rooms.write().await;
            let room = rooms.get_mut(connected_room_code)?;
            if room.active_playlist_item_id.is_none() || room.playlist.is_empty() {
                return None;
            }
            room.playback.position_seconds = payload.position_seconds.max(0.0);
            room.playback.playback_rate = payload.playback_rate.clamp(0.25, 3.0);
            room.playback.updated_at_ms = now_ms();
            room.playback.revision += 1;
            let message = ServerMessage::PlaybackState {
                room_code: connected_room_code.to_owned(),
                playback: room.playback.clone(),
            };
            let _ = room.events.send(message);
            None
        }
        ClientMessage::PlaylistUpdate(payload) => {
            let requested_room_code = payload.room_code.trim().to_uppercase();
            if requested_room_code != connected_room_code {
                return Some(ServerMessage::error(
                    "room_mismatch",
                    "Плейлист относится к другой комнате",
                ));
            }
            let playlist = match normalize_playlist(payload.playlist) {
                Ok(playlist) => playlist,
                Err(error) => return Some(error),
            };
            let mut rooms = state.rooms.write().await;
            let room = rooms.get_mut(connected_room_code)?;
            if room.host_participant_id != connected_participant_id {
                return Some(ServerMessage::error(
                    "host_required",
                    "Изменять плейлист может только хост",
                ));
            }
            let next_active = normalize_active_item(&playlist, payload.active_playlist_item_id);
            let active_changed = room.active_playlist_item_id != next_active;
            room.playlist = playlist;
            room.active_playlist_item_id = next_active;
            room.media_name = room
                .active_playlist_item_id
                .as_deref()
                .and_then(|id| room.playlist.iter().find(|item| item.id == id))
                .map(|item| item.name.clone())
                .unwrap_or_else(|| "Плейлист пуст".to_owned());

            let playlist_message = ServerMessage::PlaylistState {
                room_code: connected_room_code.to_owned(),
                playlist: room.playlist.clone(),
                active_playlist_item_id: room.active_playlist_item_id.clone(),
            };
            let _ = room.events.send(playlist_message);
            if active_changed {
                room.playback.position_seconds = room
                    .active_playlist_item_id
                    .as_deref()
                    .and_then(|id| room.playlist.iter().find(|item| item.id == id))
                    .map(|item| item.progress_seconds)
                    .unwrap_or(0.0);
                room.playback.playing = false;
                room.playback.updated_at_ms = now_ms();
                room.playback.revision += 1;
                let _ = room.events.send(ServerMessage::PlaybackState {
                    room_code: connected_room_code.to_owned(),
                    playback: room.playback.clone(),
                });
            }
            let rate_playlist = room.playlist.clone();
            drop(rooms);
            state
                .update_media_rate_hints(connected_room_code, &rate_playlist)
                .await;
            None
        }
        ClientMessage::Ping(payload) => Some(ServerMessage::Pong {
            client_time_ms: payload.client_time_ms,
            server_time_ms: now_ms(),
        }),
        ClientMessage::LatencyReport(payload) => {
            let mut rooms = state.rooms.write().await;
            let room = rooms.get_mut(connected_room_code)?;
            let is_viewer = room.host_participant_id != connected_participant_id;
            let ping_ms = payload.ping_ms.min(60_000);
            {
                let participant = room.participants.get_mut(connected_participant_id)?;
                participant.ping_ms = Some(ping_ms);
                participant.minimum_ping_ms = Some(
                    participant
                        .minimum_ping_ms
                        .map_or(ping_ms, |minimum| minimum.min(ping_ms)),
                );
            }
            if is_viewer {
                let (worst_ping, worst_queue_delay) = room
                    .participants
                    .iter()
                    .filter(|(participant_id, _)| *participant_id != &room.host_participant_id)
                    .filter_map(|(_, participant)| {
                        let ping = participant.ping_ms?;
                        let minimum = participant.minimum_ping_ms.unwrap_or(ping);
                        Some((ping, ping.saturating_sub(minimum)))
                    })
                    .fold((0, 0), |(worst_ping, worst_queue), (ping, queue)| {
                        (worst_ping.max(ping), worst_queue.max(queue))
                    });
                room.bandwidth
                    .report_ping(worst_ping, worst_ping.saturating_sub(worst_queue_delay));
            }
            let message = ServerMessage::ParticipantList {
                room_code: connected_room_code.to_owned(),
                participants: room.participant_snapshots(),
            };
            let _ = room.events.send(message);
            None
        }
        ClientMessage::CreateRoom(_) | ClientMessage::JoinRoom(_) => Some(ServerMessage::error(
            "already_joined",
            "Клиент уже подключён к комнате",
        )),
    }
}

fn normalize_playlist(items: Vec<PlaylistItem>) -> Result<Vec<PlaylistItem>, ServerMessage> {
    if items.len() > 500 {
        return Err(ServerMessage::error(
            "playlist_too_large",
            "В плейлисте может быть не более 500 видео",
        ));
    }
    let mut ids = HashSet::new();
    let mut normalized = Vec::with_capacity(items.len());
    for item in items {
        let id = item.id.trim();
        let name = item.name.trim();
        if id.is_empty() || id.len() > 128 || !ids.insert(id.to_owned()) {
            return Err(ServerMessage::error(
                "invalid_playlist_item",
                "Некорректный идентификатор видео в плейлисте",
            ));
        }
        if name.is_empty() || name.chars().count() > 255 {
            return Err(ServerMessage::error(
                "invalid_playlist_item",
                "Название видео должно содержать от 1 до 255 символов",
            ));
        }
        if item.external_subtitles.len() > 64 {
            return Err(ServerMessage::error(
                "too_many_subtitles",
                "К одному видео можно добавить не более 64 файлов субтитров",
            ));
        }
        let mut subtitle_ids = HashSet::new();
        let mut external_subtitles = Vec::with_capacity(item.external_subtitles.len());
        for subtitle in item.external_subtitles {
            let subtitle_id = subtitle.id.trim();
            let subtitle_name = subtitle.name.trim();
            if subtitle_id.is_empty()
                || subtitle_id.len() > 128
                || !subtitle_ids.insert(subtitle_id.to_owned())
                || subtitle_name.is_empty()
                || subtitle_name.chars().count() > 255
            {
                return Err(ServerMessage::error(
                    "invalid_subtitle",
                    "Не удалось добавить внешние субтитры",
                ));
            }
            external_subtitles.push(crate::protocol::ExternalSubtitle {
                id: subtitle_id.to_owned(),
                name: subtitle_name.to_owned(),
                language: subtitle
                    .language
                    .map(|language| language.trim().chars().take(16).collect())
                    .filter(|language: &String| !language.is_empty()),
            });
        }
        normalized.push(PlaylistItem {
            id: id.to_owned(),
            name: name.to_owned(),
            duration_seconds: if item.duration_seconds.is_finite() {
                item.duration_seconds.clamp(0.0, 604_800.0)
            } else {
                0.0
            },
            progress_seconds: if item.progress_seconds.is_finite() {
                item.progress_seconds.max(0.0).min(
                    if item.duration_seconds.is_finite() && item.duration_seconds > 0.0 {
                        item.duration_seconds.min(604_800.0)
                    } else {
                        604_800.0
                    },
                )
            } else {
                0.0
            },
            external_subtitles,
        });
    }
    Ok(normalized)
}

fn normalize_active_item(playlist: &[PlaylistItem], requested: Option<String>) -> Option<String> {
    requested
        .filter(|id| playlist.iter().any(|item| &item.id == id))
        .or_else(|| playlist.first().map(|item| item.id.clone()))
}

async fn remove_participant(state: &ServerState, room_code: &str, participant_id: &str) {
    let mut rooms = state.rooms.write().await;
    let Some(room) = rooms.get_mut(room_code) else {
        return;
    };

    if room.host_participant_id == participant_id {
        let events = room.events.clone();
        rooms.remove(room_code);
        drop(rooms);
        state.media_sources.write().await.remove(room_code);
        let _ = events.send(ServerMessage::RoomClosed {
            room_code: room_code.to_owned(),
            reason: "host_left".to_owned(),
        });
        return;
    }

    room.participants.remove(participant_id);
    let participant_count = room.participants.len();
    room.bandwidth
        .set_viewer_count(participant_count.saturating_sub(1));
    let events = room.events.clone();
    if participant_count == 0 {
        rooms.remove(room_code);
    } else {
        let _ = events.send(ServerMessage::ParticipantCount {
            room_code: room_code.to_owned(),
            participant_count,
        });
        let _ = events.send(ServerMessage::ParticipantList {
            room_code: room_code.to_owned(),
            participants: room.participant_snapshots(),
        });
    }
}

fn validate_version(version: u8) -> Result<(), ServerMessage> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ServerMessage::error(
            "unsupported_version",
            "Версия протокола не поддерживается",
        ))
    }
}

fn validate_media_token(value: &str) -> Result<String, ServerMessage> {
    let token = value.trim();
    if token.len() < 16 || token.len() > 128 || !token.is_ascii() {
        Err(ServerMessage::error(
            "invalid_media_token",
            "Не удалось подготовить передачу видео",
        ))
    } else {
        Ok(token.to_owned())
    }
}

fn server_error_message(error: ServerMessage) -> String {
    match error {
        ServerMessage::Error { message, .. } => message,
        _ => "Не удалось подготовить передачу видео".to_owned(),
    }
}

fn normalize_room_code(value: &str) -> Result<String, ServerMessage> {
    let room_code = value.trim().to_uppercase();
    if room_code.len() < 4
        || room_code.len() > 12
        || !room_code
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        Err(ServerMessage::error(
            "invalid_room_code",
            "Код комнаты должен содержать 4–12 латинских букв или цифр",
        ))
    } else {
        Ok(room_code)
    }
}

fn normalize_display_name(value: &str) -> Result<String, ServerMessage> {
    let display_name = value.trim();
    if display_name.is_empty() || display_name.chars().count() > 32 {
        Err(ServerMessage::error(
            "invalid_display_name",
            "Имя должно содержать от 1 до 32 символов",
        ))
    } else {
        Ok(display_name.to_owned())
    }
}

async fn send_direct(socket: &mut WebSocket, message: ServerMessage) {
    if let Ok(json) = serde_json::to_string(&message) {
        let _ = socket.send(Message::Text(json.into())).await;
    }
}

async fn send_json<S>(sender: &mut S, message: &ServerMessage) -> Result<(), axum::Error>
where
    S: futures_util::Sink<Message, Error = axum::Error> + Unpin,
{
    let json = serde_json::to_string(message).expect("server messages must serialize");
    sender.send(Message::Text(json.into())).await
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn discover_local_ip() -> IpAddr {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            socket.connect("8.8.8.8:80")?;
            socket.local_addr()
        })
        .map(|address| address.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CreateRoomPayload, ExternalSubtitle, JoinRoomPayload, LatencyReportPayload,
        PlaybackRatePayload, PlaylistUpdatePayload,
    };
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    #[test]
    fn parses_http_byte_ranges() {
        assert_eq!(parse_byte_range("bytes=0-99", 1_000), Some((0, 99)));
        assert_eq!(parse_byte_range("bytes=900-", 1_000), Some((900, 999)));
        assert_eq!(parse_byte_range("bytes=-100", 1_000), Some((900, 999)));
        assert_eq!(parse_byte_range("bytes=0-5000", 1_000), Some((0, 999)));
    }

    #[test]
    fn rejects_invalid_http_byte_ranges() {
        assert_eq!(parse_byte_range("bytes=1000-", 1_000), None);
        assert_eq!(parse_byte_range("bytes=20-10", 1_000), None);
        assert_eq!(parse_byte_range("bytes=0-1,4-5", 1_000), None);
        assert_eq!(parse_byte_range("items=0-1", 1_000), None);
        assert_eq!(parse_byte_range("bytes=-0", 1_000), None);
        assert_eq!(parse_byte_range("bytes=0-0", 0), None);
    }

    #[test]
    fn automatic_subtitle_matching_is_conservative() {
        assert!(subtitle_matches_video("episode 01", "episode 01"));
        assert!(subtitle_matches_video("episode 01", "episode 01.ru"));
        assert!(subtitle_matches_video("episode 01", "episode 01.en.forced"));
        assert!(!subtitle_matches_video(
            "episode 01",
            "episode 01 commentary"
        ));
        assert!(!subtitle_matches_video("episode 01", "episode 010"));
        assert!(!subtitle_matches_video("episode 01", "episode 02"));
    }

    #[test]
    fn media_rate_reacts_to_queue_delay_without_leaving_safe_bounds() {
        let stable = adjusted_media_rate(INITIAL_MEDIA_RATE, 35, 25);
        assert!(stable > INITIAL_MEDIA_RATE);

        let congested = adjusted_media_rate(INITIAL_MEDIA_RATE, 280, 25);
        assert!(congested < INITIAL_MEDIA_RATE);

        assert_eq!(adjusted_media_rate(MIN_MEDIA_RATE, 500, 20), MIN_MEDIA_RATE);
        assert_eq!(adjusted_media_rate(MAX_MEDIA_RATE, 20, 20), MAX_MEDIA_RATE);
    }

    #[test]
    fn media_rate_hint_covers_average_bitrate_per_viewer() {
        let ten_megabytes_per_second = 10 * 1024 * 1024;
        let file_size = ten_megabytes_per_second * 7_200;

        assert_eq!(
            required_media_rate_per_viewer(file_size, 7_200.0),
            35 * 1024 * 1024 / 2
        );
        assert_eq!(
            required_media_rate_per_viewer(file_size, 0.0),
            UNKNOWN_MEDIA_RATE_PER_VIEWER
        );
    }

    #[tokio::test]
    async fn playlist_duration_updates_an_existing_transfer_hint() {
        let state = ServerState::new();
        let token = "test-media-token-123456";
        let path = std::env::temp_dir().join(format!(
            "syncwatch-rate-test-{}-{}.bin",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(&path, vec![0_u8; 1024]).unwrap();
        state
            .set_media_sources(
                "ABC123",
                token,
                vec![MediaSourceInput {
                    item_id: "movie-1".to_owned(),
                    path: path.to_string_lossy().into_owned(),
                    subtitles: Vec::new(),
                }],
            )
            .await
            .unwrap();
        let hint = state.media_sources.read().await["ABC123"].items["movie-1"]
            .required_rate_per_viewer
            .clone();
        assert_eq!(hint.load(Ordering::Relaxed), UNKNOWN_MEDIA_RATE_PER_VIEWER);

        state
            .update_media_rate_hints(
                "ABC123",
                &[PlaylistItem {
                    id: "movie-1".to_owned(),
                    name: "movie.mkv".to_owned(),
                    progress_seconds: 0.0,
                    duration_seconds: 120.0,
                    external_subtitles: Vec::new(),
                }],
            )
            .await;

        assert_eq!(hint.load(Ordering::Relaxed), MIN_MEDIA_RATE);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn media_endpoint_streams_requested_range_for_room_member() {
        let state = ServerState::new();
        let token = "test-media-token-123456";
        let path = std::env::temp_dir().join(format!(
            "syncwatch-range-test-{}-{}.bin",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(&path, b"0123456789").unwrap();
        state
            .set_media_sources(
                "ABC123",
                token,
                vec![MediaSourceInput {
                    item_id: "movie-1".to_owned(),
                    path: path.to_string_lossy().into_owned(),
                    subtitles: Vec::new(),
                }],
            )
            .await
            .unwrap();
        perform_handshake(create_message("host"), &state)
            .await
            .unwrap();

        let response = server_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(format!("/media/ABC123/movie-1?token={token}"))
                    .header(header::RANGE, "bytes=3-6")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 3-6/10");
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "4");
        let body = to_bytes(response.into_body(), 16).await.unwrap();
        assert_eq!(&body[..], b"3456");

        let unauthorized = server_router(state)
            .oneshot(
                Request::builder()
                    .uri("/media/ABC123/movie-1?token=wrong-token-value")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::NOT_FOUND);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn subtitle_endpoint_serves_only_registered_room_subtitles() {
        let state = ServerState::new();
        let token = "test-media-token-123456";
        let video_path = std::env::temp_dir().join(format!(
            "syncwatch-subtitle-video-{}-{}.mkv",
            std::process::id(),
            now_ms()
        ));
        let subtitle_path = video_path.with_extension("ru.srt");
        std::fs::write(&video_path, b"video").unwrap();
        std::fs::write(&subtitle_path, b"1\n00:00:00,000 --> 00:00:01,000\nTest\n").unwrap();
        state
            .set_media_sources(
                "ABC123",
                token,
                vec![MediaSourceInput {
                    item_id: "movie-1".to_owned(),
                    path: video_path.to_string_lossy().into_owned(),
                    subtitles: vec![SubtitleSourceInput {
                        subtitle_id: "subtitle-1".to_owned(),
                        path: subtitle_path.to_string_lossy().into_owned(),
                    }],
                }],
            )
            .await
            .unwrap();
        let mut message = create_message("host");
        if let ClientMessage::CreateRoom(payload) = &mut message {
            payload.playlist[0].external_subtitles = vec![ExternalSubtitle {
                id: "subtitle-1".to_owned(),
                name: "movie.ru.srt".to_owned(),
                language: Some("ru".to_owned()),
            }];
        }
        perform_handshake(message, &state).await.unwrap();

        let response = server_router(state)
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/subtitles/ABC123/movie-1/subtitle-1?token={token}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/x-subrip; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), 128).await.unwrap();
        assert!(body.ends_with(b"Test\n"));
        std::fs::remove_file(video_path).unwrap();
        std::fs::remove_file(subtitle_path).unwrap();
    }

    fn create_message(participant_id: &str) -> ClientMessage {
        ClientMessage::CreateRoom(CreateRoomPayload {
            version: PROTOCOL_VERSION,
            room_code: "ABC123".to_owned(),
            media_name: "movie.mkv".to_owned(),
            playlist: vec![PlaylistItem {
                id: "movie-1".to_owned(),
                name: "movie.mkv".to_owned(),
                progress_seconds: 0.0,
                duration_seconds: 0.0,
                external_subtitles: Vec::new(),
            }],
            active_playlist_item_id: Some("movie-1".to_owned()),
            participant_id: participant_id.to_owned(),
            display_name: "Host".to_owned(),
            media_access_token: "test-media-token-123456".to_owned(),
        })
    }

    fn join_message(participant_id: &str) -> ClientMessage {
        ClientMessage::JoinRoom(JoinRoomPayload {
            version: PROTOCOL_VERSION,
            room_code: "ABC123".to_owned(),
            participant_id: participant_id.to_owned(),
            display_name: "Viewer".to_owned(),
        })
    }

    #[tokio::test]
    async fn host_disconnect_closes_room_and_notifies_viewers() {
        let state = ServerState::new();
        let host = perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let viewer = perform_handshake(join_message("viewer"), &state)
            .await
            .unwrap();
        let mut events = viewer.events.subscribe();

        remove_participant(&state, &host.room_code, &host.participant_id).await;

        assert!(!state.rooms.read().await.contains_key("ABC123"));
        let message = events.recv().await.unwrap();
        assert!(
            matches!(message, ServerMessage::RoomClosed { reason, .. } if reason == "host_left")
        );
    }

    #[tokio::test]
    async fn viewer_disconnect_keeps_room_alive() {
        let state = ServerState::new();
        perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let viewer = perform_handshake(join_message("viewer"), &state)
            .await
            .unwrap();

        remove_participant(&state, &viewer.room_code, &viewer.participant_id).await;

        let rooms = state.rooms.read().await;
        let room = rooms.get("ABC123").unwrap();
        assert_eq!(room.participants.len(), 1);
        assert!(room.participants.contains_key("host"));
    }

    #[tokio::test]
    async fn joining_viewer_broadcasts_their_display_name() {
        let state = ServerState::new();
        let host = perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let mut events = host.events.subscribe();

        perform_handshake(join_message("viewer"), &state)
            .await
            .unwrap();

        let message = events.recv().await.unwrap();
        assert!(matches!(
            message,
            ServerMessage::ParticipantJoined { participant_id, display_name, .. }
                if participant_id == "viewer" && display_name == "Viewer"
        ));
    }

    #[tokio::test]
    async fn latency_report_is_broadcast_in_participant_list() {
        let state = ServerState::new();
        let host = perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let mut events = host.events.subscribe();

        handle_client_message(
            ClientMessage::LatencyReport(LatencyReportPayload { ping_ms: 42 }),
            &host.room_code,
            &host.participant_id,
            &state,
        )
        .await;

        let message = events.recv().await.unwrap();
        assert!(matches!(
            message,
            ServerMessage::ParticipantList { participants, .. }
                if participants.len() == 1
                    && participants[0].is_host
                    && participants[0].ping_ms == Some(42)
        ));
    }

    #[tokio::test]
    async fn playback_rate_is_clamped_and_broadcast() {
        let state = ServerState::new();
        let host = perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let mut events = host.events.subscribe();

        handle_client_message(
            ClientMessage::PlaybackRate(PlaybackRatePayload {
                room_code: host.room_code.clone(),
                playback_rate: 5.0,
                position_seconds: 12.5,
            }),
            &host.room_code,
            &host.participant_id,
            &state,
        )
        .await;

        let message = events.recv().await.unwrap();
        assert!(matches!(
            message,
            ServerMessage::PlaybackState { playback, .. }
                if playback.playback_rate == 3.0
                    && playback.position_seconds == 12.5
                    && playback.revision == 1
        ));
    }

    #[tokio::test]
    async fn host_can_change_active_playlist_item() {
        let state = ServerState::new();
        let host = perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let mut events = host.events.subscribe();

        handle_client_message(
            ClientMessage::PlaylistUpdate(PlaylistUpdatePayload {
                room_code: host.room_code.clone(),
                playlist: vec![
                    PlaylistItem {
                        id: "movie-1".to_owned(),
                        name: "movie.mkv".to_owned(),
                        progress_seconds: 14.0,
                        duration_seconds: 100.0,
                        external_subtitles: Vec::new(),
                    },
                    PlaylistItem {
                        id: "movie-2".to_owned(),
                        name: "episode-2.mkv".to_owned(),
                        progress_seconds: 27.5,
                        duration_seconds: 120.0,
                        external_subtitles: Vec::new(),
                    },
                ],
                active_playlist_item_id: Some("movie-2".to_owned()),
            }),
            &host.room_code,
            &host.participant_id,
            &state,
        )
        .await;

        let playlist_message = events.recv().await.unwrap();
        assert!(matches!(
            playlist_message,
            ServerMessage::PlaylistState { active_playlist_item_id, playlist, .. }
                if active_playlist_item_id.as_deref() == Some("movie-2") && playlist.len() == 2
        ));
        let playback_message = events.recv().await.unwrap();
        assert!(matches!(
            playback_message,
            ServerMessage::PlaybackState { playback, .. }
                if !playback.playing && playback.position_seconds == 27.5 && playback.revision == 1
        ));
    }

    #[tokio::test]
    async fn switching_playlist_items_restores_each_saved_progress() {
        let state = ServerState::new();
        let host = perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let mut events = host.events.subscribe();
        let playlist = vec![
            PlaylistItem {
                id: "movie-1".to_owned(),
                name: "movie.mkv".to_owned(),
                progress_seconds: 41.5,
                duration_seconds: 100.0,
                external_subtitles: Vec::new(),
            },
            PlaylistItem {
                id: "movie-2".to_owned(),
                name: "episode-2.mkv".to_owned(),
                progress_seconds: 72.25,
                duration_seconds: 120.0,
                external_subtitles: Vec::new(),
            },
        ];

        for (active, expected_position) in [("movie-2", 72.25), ("movie-1", 41.5)] {
            handle_client_message(
                ClientMessage::PlaylistUpdate(PlaylistUpdatePayload {
                    room_code: host.room_code.clone(),
                    playlist: playlist.clone(),
                    active_playlist_item_id: Some(active.to_owned()),
                }),
                &host.room_code,
                &host.participant_id,
                &state,
            )
            .await;

            assert!(matches!(
                events.recv().await.unwrap(),
                ServerMessage::PlaylistState { active_playlist_item_id, .. }
                    if active_playlist_item_id.as_deref() == Some(active)
            ));
            assert!(matches!(
                events.recv().await.unwrap(),
                ServerMessage::PlaybackState { playback, .. }
                    if !playback.playing && playback.position_seconds == expected_position
            ));
        }
    }

    #[tokio::test]
    async fn empty_room_ignores_playback_controls() {
        let state = ServerState::new();
        let mut message = create_message("host");
        let ClientMessage::CreateRoom(payload) = &mut message else {
            unreachable!()
        };
        payload.playlist.clear();
        payload.active_playlist_item_id = None;
        payload.media_name = "Плейлист пуст".to_owned();
        let host = perform_handshake(message, &state).await.unwrap();
        let mut events = host.events.subscribe();

        handle_client_message(
            ClientMessage::PlaybackCommand(crate::protocol::PlaybackCommandPayload {
                room_code: host.room_code.clone(),
                action: PlaybackAction::Play,
                position_seconds: 15.0,
            }),
            &host.room_code,
            &host.participant_id,
            &state,
        )
        .await;

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), events.recv())
                .await
                .is_err()
        );
        let rooms = state.rooms.read().await;
        let playback = &rooms[&host.room_code].playback;
        assert!(!playback.playing);
        assert_eq!(playback.position_seconds, 0.0);
        assert_eq!(playback.revision, 0);
    }

    #[tokio::test]
    async fn viewer_cannot_change_playlist() {
        let state = ServerState::new();
        perform_handshake(create_message("host"), &state)
            .await
            .unwrap();
        let viewer = perform_handshake(join_message("viewer"), &state)
            .await
            .unwrap();

        let response = handle_client_message(
            ClientMessage::PlaylistUpdate(PlaylistUpdatePayload {
                room_code: viewer.room_code.clone(),
                playlist: Vec::new(),
                active_playlist_item_id: None,
            }),
            &viewer.room_code,
            &viewer.participant_id,
            &state,
        )
        .await;

        assert!(matches!(
            response,
            Some(ServerMessage::Error { code, .. }) if code == "host_required"
        ));
        let rooms = state.rooms.read().await;
        assert_eq!(rooms["ABC123"].playlist.len(), 1);
    }

    #[test]
    fn server_reserves_an_operating_system_selected_port() {
        let (listener, info) = bind_available_port().unwrap();

        assert_ne!(info.port, 0);
        assert_eq!(listener.local_addr().unwrap().port(), info.port);
    }
}
