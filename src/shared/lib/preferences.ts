const DISPLAY_NAME_KEY = "syncwatch.displayName";
const PLAYER_VOLUME_KEY = "syncwatch.player.volume";
const PLAYBACK_SETTINGS_KEY = "syncwatch.player.playbackSettings";
const TRACK_PREFERENCES_KEY = "syncwatch.player.trackPreferences";
const SAVED_PLAYLISTS_KEY = "syncwatch.savedPlaylists";

export interface TrackPreference {
  disabled: boolean;
  language: string | null;
  title: string | null;
}

export interface TrackPreferences {
  audio: TrackPreference | null;
  subtitle: TrackPreference | null;
}

export type PlayerHotkeyAction =
  | "togglePlayback"
  | "toggleFullscreen"
  | "syncClock"
  | "seekForward"
  | "seekBackward"
  | "previousPlaylistItem"
  | "nextPlaylistItem"
  | "volumeUp"
  | "volumeDown";

export interface PlaybackSettings {
  seekSeconds: number;
  hotkeys: Record<PlayerHotkeyAction, string>;
}

export const DEFAULT_PLAYBACK_SETTINGS: PlaybackSettings = {
  seekSeconds: 5,
  hotkeys: {
    togglePlayback: "Space",
    toggleFullscreen: "KeyF",
    syncClock: "KeyS",
    seekForward: "ArrowRight",
    seekBackward: "ArrowLeft",
    previousPlaylistItem: "PageUp",
    nextPlaylistItem: "PageDown",
    volumeUp: "ArrowUp",
    volumeDown: "ArrowDown",
  },
};

export interface SavedPlaylistItem {
  id: string;
  name: string;
  path: string;
  progressSeconds: number;
  durationSeconds: number;
  externalSubtitles: {
    id: string;
    name: string;
    language: string | null;
    path: string;
  }[];
}

export interface SavedPlaylist {
  id: string;
  title: string;
  items: SavedPlaylistItem[];
  activePlaylistItemId: string | null;
  updatedAt: number;
}

function validSavedPlaylist(value: unknown): value is SavedPlaylist {
  if (!value || typeof value !== "object") return false;
  const playlist = value as Partial<SavedPlaylist>;
  return typeof playlist.id === "string"
    && typeof playlist.title === "string"
    && typeof playlist.updatedAt === "number"
    && (playlist.activePlaylistItemId === null || typeof playlist.activePlaylistItemId === "string")
    && Array.isArray(playlist.items)
    && playlist.items.every((item) => (
      item
      && typeof item.id === "string"
      && typeof item.name === "string"
      && typeof item.path === "string"
      && typeof item.progressSeconds === "number"
      && typeof item.durationSeconds === "number"
      && (item.externalSubtitles === undefined || (
        Array.isArray(item.externalSubtitles)
        && item.externalSubtitles.every((subtitle) => (
          subtitle
          && typeof subtitle.id === "string"
          && typeof subtitle.name === "string"
          && (subtitle.language === null || typeof subtitle.language === "string")
          && typeof subtitle.path === "string"
        ))
      ))
    ));
}

export function loadSavedPlaylists(): SavedPlaylist[] {
  try {
    const stored = JSON.parse(localStorage.getItem(SAVED_PLAYLISTS_KEY) ?? "[]") as unknown;
    if (!Array.isArray(stored)) return [];
    const playlists = stored
      .filter((item): item is SavedPlaylist => validSavedPlaylist(item) && item.items.length > 0)
      .map((playlist) => ({
        ...playlist,
        items: playlist.items.map((item) => ({
          ...item,
          externalSubtitles: item.externalSubtitles ?? [],
        })),
      }))
      .sort((left, right) => right.updatedAt - left.updatedAt);
    if (playlists.length !== stored.length) {
      localStorage.setItem(SAVED_PLAYLISTS_KEY, JSON.stringify(playlists));
    }
    return playlists;
  } catch {
    return [];
  }
}

export function saveSavedPlaylist(playlist: SavedPlaylist): void {
  if (playlist.items.length === 0) {
    deleteSavedPlaylist(playlist.id);
    return;
  }
  try {
    const playlists = loadSavedPlaylists().filter((item) => item.id !== playlist.id);
    localStorage.setItem(SAVED_PLAYLISTS_KEY, JSON.stringify([playlist, ...playlists].slice(0, 30)));
  } catch {
    // The room remains usable when persistent browser storage is unavailable.
  }
}

export function deleteSavedPlaylist(id: string): void {
  try {
    const playlists = loadSavedPlaylists().filter((item) => item.id !== id);
    localStorage.setItem(SAVED_PLAYLISTS_KEY, JSON.stringify(playlists));
  } catch {
    // A storage failure leaves the saved session unchanged.
  }
}

export function loadDisplayName(): string {
  try {
    return localStorage.getItem(DISPLAY_NAME_KEY)?.trim() ?? "";
  } catch {
    return "";
  }
}

export function saveDisplayName(value: string): void {
  try {
    localStorage.setItem(DISPLAY_NAME_KEY, value.trim());
  } catch {
    // The app can still work when persistent browser storage is unavailable.
  }
}

export function loadPlayerVolume(): number {
  try {
    const stored = Number(localStorage.getItem(PLAYER_VOLUME_KEY));
    return Number.isFinite(stored) ? Math.min(100, Math.max(0, stored)) : 80;
  } catch {
    return 80;
  }
}

export function savePlayerVolume(value: number): void {
  try {
    localStorage.setItem(PLAYER_VOLUME_KEY, String(Math.min(100, Math.max(0, value))));
  } catch {
    // The current volume still works until the application is restarted.
  }
}

export function loadPlaybackSettings(): PlaybackSettings {
  try {
    const stored = JSON.parse(localStorage.getItem(PLAYBACK_SETTINGS_KEY) ?? "null") as
      | Partial<PlaybackSettings>
      | null;
    const hotkeys = { ...DEFAULT_PLAYBACK_SETTINGS.hotkeys };
    for (const action of Object.keys(hotkeys) as PlayerHotkeyAction[]) {
      const value = stored?.hotkeys?.[action];
      if (typeof value === "string") hotkeys[action] = value;
    }
    return {
      seekSeconds: Math.min(10, Math.max(1, Number(stored?.seekSeconds) || 5)),
      hotkeys,
    };
  } catch {
    return {
      ...DEFAULT_PLAYBACK_SETTINGS,
      hotkeys: { ...DEFAULT_PLAYBACK_SETTINGS.hotkeys },
    };
  }
}

export function savePlaybackSettings(settings: PlaybackSettings): void {
  try {
    localStorage.setItem(PLAYBACK_SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // The current shortcuts still work until the application is restarted.
  }
}

export function loadTrackPreferences(): TrackPreferences {
  try {
    const stored = JSON.parse(localStorage.getItem(TRACK_PREFERENCES_KEY) ?? "null") as
      | Partial<TrackPreferences>
      | null;
    return {
      audio: stored?.audio ?? null,
      subtitle: stored?.subtitle ?? null,
    };
  } catch {
    return { audio: null, subtitle: null };
  }
}

export function saveTrackPreferences(preferences: TrackPreferences): void {
  try {
    localStorage.setItem(TRACK_PREFERENCES_KEY, JSON.stringify(preferences));
  } catch {
    // Track selection still works for the current media file.
  }
}
