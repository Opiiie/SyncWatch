export const PROTOCOL_VERSION = 2 as const;

export interface PlaybackState {
  positionSeconds: number;
  playing: boolean;
  playbackRate: number;
  updatedAtMs: number;
  revision: number;
}

export interface Participant {
  participantId: string;
  displayName: string;
  isHost: boolean;
  pingMs: number | null;
}

export interface ExternalSubtitle {
  id: string;
  name: string;
  language: string | null;
}

export interface PlaylistItem {
  id: string;
  name: string;
  progressSeconds: number;
  durationSeconds: number;
  externalSubtitles: ExternalSubtitle[];
}

export interface RoomSnapshot {
  roomCode: string;
  mediaName: string;
  playlist: PlaylistItem[];
  activePlaylistItemId: string | null;
  participantCount: number;
  participants?: Participant[];
  hostParticipantId: string;
  mediaAccessToken: string;
  playback: PlaybackState;
}

export type PlaybackAction = "play" | "pause" | "seek";

export type ClientMessage =
  | {
      type: "create_room";
      payload: {
        version: typeof PROTOCOL_VERSION;
        roomCode: string;
        mediaName: string;
        playlist: PlaylistItem[];
        activePlaylistItemId: string | null;
        participantId: string;
        displayName: string;
        mediaAccessToken: string;
      };
    }
  | {
      type: "join_room";
      payload: {
        version: typeof PROTOCOL_VERSION;
        roomCode: string;
        participantId: string;
        displayName: string;
      };
    }
  | {
      type: "playback_command";
      payload: {
        roomCode: string;
        action: PlaybackAction;
        positionSeconds: number;
      };
    }
  | {
      type: "playback_rate";
      payload: {
        roomCode: string;
        playbackRate: number;
        positionSeconds: number;
      };
    }
  | {
      type: "playlist_update";
      payload: {
        roomCode: string;
        playlist: PlaylistItem[];
        activePlaylistItemId: string | null;
      };
    }
  | { type: "ping"; payload: { clientTimeMs: number } }
  | { type: "latency_report"; payload: { pingMs: number } };

export type ServerMessage =
  | { type: "room_snapshot"; payload: { room: RoomSnapshot } }
  | {
      type: "participant_count";
      payload: { roomCode: string; participantCount: number };
    }
  | {
      type: "participant_joined";
      payload: { roomCode: string; participantId: string; displayName: string };
    }
  | {
      type: "participant_list";
      payload: { roomCode: string; participants: Participant[] };
    }
  | {
      type: "playback_state";
      payload: { roomCode: string; playback: PlaybackState };
    }
  | {
      type: "playlist_state";
      payload: {
        roomCode: string;
        playlist: PlaylistItem[];
        activePlaylistItemId: string | null;
      };
    }
  | {
      type: "room_closed";
      payload: { roomCode: string; reason: "host_left" | string };
    }
  | {
      type: "pong";
      payload: { clientTimeMs: number; serverTimeMs: number };
    }
  | { type: "error"; payload: { code: string; message: string } };

export function parseServerMessage(value: string): ServerMessage | null {
  try {
    const parsed: unknown = JSON.parse(value);
    if (!parsed || typeof parsed !== "object" || !("type" in parsed) || !("payload" in parsed)) {
      return null;
    }

    const type = (parsed as { type: unknown }).type;
    if (
      type !== "room_snapshot" &&
      type !== "participant_count" &&
      type !== "participant_joined" &&
      type !== "participant_list" &&
      type !== "playback_state" &&
      type !== "playlist_state" &&
      type !== "room_closed" &&
      type !== "pong" &&
      type !== "error"
    ) {
      return null;
    }

    return parsed as ServerMessage;
  } catch {
    return null;
  }
}
