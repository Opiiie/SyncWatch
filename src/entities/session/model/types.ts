import type {
  ExternalSubtitle,
  Participant,
  PlaybackState,
  PlaylistItem,
} from "../../../shared/api/protocol";

export type SessionRole = "host" | "viewer";
export type ConnectionState = "idle" | "connecting" | "connected" | "disconnected";

export interface LocalMediaFile {
  id?: string;
  name: string;
  path: string;
  progressSeconds?: number;
  durationSeconds?: number;
  externalSubtitles?: LocalExternalSubtitle[];
}

export interface LocalExternalSubtitle extends ExternalSubtitle {
  path: string;
}

export interface PlayerSubtitleSource extends ExternalSubtitle {
  source: string;
}

export interface Session {
  roomCode: string;
  role: SessionRole;
  mediaName: string;
  playlist: PlaylistItem[];
  activePlaylistItemId: string | null;
  participantCount: number;
  participants: Participant[];
  serverUrl: string;
  mediaAccessToken: string;
}

export interface SessionControllerState {
  session: Session | null;
  localMediaPath: string | null;
  externalSubtitleSources: PlayerSubtitleSource[];
  playback: PlaybackState;
  connectionState: ConnectionState;
  clockOffsetMs: number;
  clockLatencyMs: number | null;
  clockSyncState: "idle" | "syncing" | "synced";
  notification: { id: number; message: string } | null;
  error: string | null;
}
