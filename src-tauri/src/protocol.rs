use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub position_seconds: f64,
    pub playing: bool,
    pub playback_rate: f64,
    pub updated_at_ms: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantSnapshot {
    pub participant_id: String,
    pub display_name: String,
    pub is_host: bool,
    pub ping_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSubtitle {
    pub id: String,
    pub name: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub progress_seconds: f64,
    #[serde(default)]
    pub duration_seconds: f64,
    #[serde(default)]
    pub external_subtitles: Vec<ExternalSubtitle>,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            position_seconds: 0.0,
            playing: false,
            playback_rate: 1.0,
            updated_at_ms: 0,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshot {
    pub room_code: String,
    pub media_name: String,
    pub playlist: Vec<PlaylistItem>,
    pub active_playlist_item_id: Option<String>,
    pub participant_count: usize,
    pub participants: Vec<ParticipantSnapshot>,
    pub host_participant_id: String,
    pub media_access_token: String,
    pub playback: PlaybackState,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientMessage {
    CreateRoom(CreateRoomPayload),
    JoinRoom(JoinRoomPayload),
    PlaybackCommand(PlaybackCommandPayload),
    PlaybackRate(PlaybackRatePayload),
    PlaylistUpdate(PlaylistUpdatePayload),
    Ping(PingPayload),
    LatencyReport(LatencyReportPayload),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomPayload {
    pub version: u8,
    pub room_code: String,
    pub media_name: String,
    #[serde(default)]
    pub playlist: Vec<PlaylistItem>,
    #[serde(default)]
    pub active_playlist_item_id: Option<String>,
    pub participant_id: String,
    #[allow(dead_code)]
    pub display_name: String,
    pub media_access_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRoomPayload {
    pub version: u8,
    pub room_code: String,
    pub participant_id: String,
    #[allow(dead_code)]
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackCommandPayload {
    pub room_code: String,
    pub action: PlaybackAction,
    pub position_seconds: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackRatePayload {
    pub room_code: String,
    pub playback_rate: f64,
    pub position_seconds: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistUpdatePayload {
    pub room_code: String,
    pub playlist: Vec<PlaylistItem>,
    pub active_playlist_item_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackAction {
    Play,
    Pause,
    Seek,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingPayload {
    pub client_time_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyReportPayload {
    pub ping_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerMessage {
    RoomSnapshot {
        room: RoomSnapshot,
    },
    ParticipantCount {
        #[serde(rename = "roomCode")]
        room_code: String,
        #[serde(rename = "participantCount")]
        participant_count: usize,
    },
    ParticipantJoined {
        #[serde(rename = "roomCode")]
        room_code: String,
        #[serde(rename = "participantId")]
        participant_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    },
    ParticipantList {
        #[serde(rename = "roomCode")]
        room_code: String,
        participants: Vec<ParticipantSnapshot>,
    },
    PlaybackState {
        #[serde(rename = "roomCode")]
        room_code: String,
        playback: PlaybackState,
    },
    PlaylistState {
        #[serde(rename = "roomCode")]
        room_code: String,
        playlist: Vec<PlaylistItem>,
        #[serde(rename = "activePlaylistItemId")]
        active_playlist_item_id: Option<String>,
    },
    RoomClosed {
        #[serde(rename = "roomCode")]
        room_code: String,
        reason: String,
    },
    Pong {
        #[serde(rename = "clientTimeMs")]
        client_time_ms: u64,
        #[serde(rename = "serverTimeMs")]
        server_time_ms: u64,
    },
    Error {
        code: String,
        message: String,
    },
}

impl ServerMessage {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
}
