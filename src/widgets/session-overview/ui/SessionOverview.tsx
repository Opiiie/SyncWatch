import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import type { ConnectionState, Session } from "../../../entities/session/model/types";
import type {
  LocalMediaFile,
  PlayerSubtitleSource,
} from "../../../entities/session/model/types";
import { selectPlayerTrack } from "../../../shared/api/player";
import type { PlaybackAction, PlaybackState } from "../../../shared/api/protocol";
import {
  DEFAULT_PLAYBACK_SETTINGS,
  loadPlaybackSettings,
  loadPlayerVolume,
  loadTrackPreferences,
  savePlaybackSettings,
  savePlayerVolume,
  saveTrackPreferences,
  type PlayerHotkeyAction,
} from "../../../shared/lib/preferences";
import { formatDuration } from "../../../shared/lib/room";
import { Button } from "../../../shared/ui/Button/Button";
import { useNativePlayer } from "../model/useNativePlayer";
import "./SessionOverview.css";
import "./SessionOverviewEnhancements.css";

interface SessionOverviewProps {
  session: Session;
  localMediaPath: string | null;
  externalSubtitleSources: PlayerSubtitleSource[];
  playback: PlaybackState;
  connectionState: ConnectionState;
  clockOffsetMs: number;
  clockLatencyMs: number | null;
  clockSyncState: "idle" | "syncing" | "synced";
  notification: { id: number; message: string } | null;
  error: string | null;
  onLeave: (positionSeconds: number, durationSeconds: number) => void;
  onPlaybackCommand: (action: PlaybackAction, positionSeconds: number) => void;
  onPlaybackRate: (playbackRate: number, positionSeconds: number) => void;
  onAddPlaylistFiles: (files: LocalMediaFile[]) => Promise<string[]>;
  onAddExternalSubtitles: (itemId: string, paths: string[]) => Promise<number>;
  onRemovePlaylistItem: (itemId: string) => void;
  onMovePlaylistItem: (itemId: string, direction: -1 | 1) => void;
  onSelectPlaylistItem: (
    itemId: string,
    currentPositionSeconds: number,
    currentDurationSeconds: number,
  ) => void;
  onPlaylistProgress: (itemId: string, positionSeconds: number, durationSeconds: number) => void;
  onSyncClock: () => void;
}

function currentPosition(playback: PlaybackState, clockOffsetMs: number): number {
  return playback.playing
    ? playback.positionSeconds
      + Math.max(0, Date.now() + clockOffsetMs - playback.updatedAtMs)
      / 1000 * playback.playbackRate
    : playback.positionSeconds;
}

function pingClass(pingMs: number | null): string {
  if (pingMs === null) return "participant-ping--unknown";
  if (pingMs <= 80) return "participant-ping--good";
  if (pingMs <= 180) return "participant-ping--medium";
  return "participant-ping--high";
}

const HOTKEY_ACTIONS: { action: PlayerHotkeyAction; label: string }[] = [
  { action: "togglePlayback", label: "Пауза / воспроизведение" },
  { action: "toggleFullscreen", label: "Полноэкранный режим" },
  { action: "syncClock", label: "Синхронизация" },
  { action: "seekForward", label: "Вперёд" },
  { action: "seekBackward", label: "Назад" },
  { action: "previousPlaylistItem", label: "Предыдущее видео" },
  { action: "nextPlaylistItem", label: "Следующее видео" },
  { action: "volumeUp", label: "Звук громче на 2,5%" },
  { action: "volumeDown", label: "Звук тише на 2,5%" },
];

function hotkeyLabel(code: string): string {
  if (!code) return "Не назначено";
  const labels: Record<string, string> = {
    Space: "Пробел",
    ArrowRight: "→",
    ArrowLeft: "←",
    ArrowUp: "↑",
    ArrowDown: "↓",
    PageUp: "Page Up",
    PageDown: "Page Down",
  };
  return labels[code] ?? code.replace(/^Key/, "").replace(/^Digit/, "");
}

export function SessionOverview({
  session,
  localMediaPath,
  externalSubtitleSources,
  playback,
  connectionState,
  clockOffsetMs,
  clockLatencyMs,
  clockSyncState,
  notification,
  error,
  onLeave,
  onPlaybackCommand,
  onPlaybackRate,
  onAddPlaylistFiles,
  onAddExternalSubtitles,
  onRemovePlaylistItem,
  onMovePlaylistItem,
  onSelectPlaylistItem,
  onPlaylistProgress,
  onSyncClock,
}: SessionOverviewProps) {
  const playerRef = useRef<HTMLElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const controlsTimerRef = useRef<number | null>(null);
  const duplicateHighlightTimerRef = useRef<number | null>(null);
  const seekCommitTimerRef = useRef<number | null>(null);
  const seekCommittedRef = useRef(false);
  const autoAdvancedItemRef = useRef<string | null>(null);
  const progressSnapshotRef = useRef({ positionSeconds: 0, durationSeconds: 0 });
  const lastProgressSentRef = useRef({ itemId: "", positionSeconds: -1, durationSeconds: -1 });
  const trackPreferencesRef = useRef(loadTrackPreferences());
  const appliedTrackPreferencesRef = useRef(new Set<string>());
  const [roomPosition, setRoomPosition] = useState(() => currentPosition(playback, clockOffsetMs));
  const [seekPosition, setSeekPosition] = useState<number | null>(null);
  const [volume, setVolume] = useState(loadPlayerVolume);
  const [fullscreen, setFullscreen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [controlsVisible, setControlsVisible] = useState(true);
  const [playbackSettings, setPlaybackSettings] = useState(loadPlaybackSettings);
  const [capturingHotkey, setCapturingHotkey] = useState<PlayerHotkeyAction | null>(null);
  const [controlError, setControlError] = useState<string | null>(null);
  const [duplicateItemIds, setDuplicateItemIds] = useState<Set<string>>(new Set());
  const player = useNativePlayer(
    surfaceRef,
    localMediaPath,
    playback,
    clockOffsetMs,
    volume,
    externalSubtitleSources,
  );

  useEffect(() => {
    appliedTrackPreferencesRef.current.clear();
  }, [localMediaPath]);

  useEffect(() => {
    if (!player.ready || !localMediaPath || player.loadedMediaPath !== localMediaPath) return;
    const apply = async (kind: "audio" | "subtitle") => {
      const key = `${localMediaPath}:${kind}`;
      if (appliedTrackPreferencesRef.current.has(key)) return;
      const preference = trackPreferencesRef.current[kind];
      if (!preference) return;
      const tracks = kind === "audio" ? player.state.audioTracks : player.state.subtitleTracks;
      if (preference.disabled) {
        appliedTrackPreferencesRef.current.add(key);
        await selectPlayerTrack(kind, null);
        return;
      }
      if (tracks.length === 0) return;
      const match = tracks
        .map((track) => ({
          track,
          score: (preference.language && track.language === preference.language ? 4 : 0)
            + (preference.title && track.title === preference.title ? 2 : 0),
        }))
        .sort((left, right) => right.score - left.score)[0];
      if (match?.score > 0) {
        appliedTrackPreferencesRef.current.add(key);
        await selectPlayerTrack(kind, match.track.id);
      }
    };
    void apply("audio").catch(() => undefined);
    void apply("subtitle").catch(() => undefined);
  }, [
    localMediaPath,
    player.loadedMediaPath,
    player.ready,
    player.state.audioTracks,
    player.state.subtitleTracks,
  ]);

  useEffect(() => {
    autoAdvancedItemRef.current = null;
    lastProgressSentRef.current = { itemId: "", positionSeconds: -1, durationSeconds: -1 };
  }, [session.activePlaylistItemId]);

  useEffect(() => {
    if (
      session.role !== "host"
      || player.loadedMediaPath !== localMediaPath
      || !session.activePlaylistItemId
      || player.state.durationSeconds <= 0
      || player.state.positionSeconds < player.state.durationSeconds - 0.5
      || autoAdvancedItemRef.current === session.activePlaylistItemId
    ) return;
    const currentIndex = session.playlist.findIndex(
      (item) => item.id === session.activePlaylistItemId,
    );
    const next = session.playlist[currentIndex + 1];
    if (next) {
      autoAdvancedItemRef.current = session.activePlaylistItemId;
      onSelectPlaylistItem(next.id, player.state.positionSeconds, player.state.durationSeconds);
    }
  }, [
    onSelectPlaylistItem,
    localMediaPath,
    player.loadedMediaPath,
    player.state.durationSeconds,
    player.state.positionSeconds,
    session.activePlaylistItemId,
    session.playlist,
    session.role,
  ]);

  const scheduleControlsHide = useCallback(() => {
    if (controlsTimerRef.current !== null) window.clearTimeout(controlsTimerRef.current);
    if (settingsOpen) return;
    controlsTimerRef.current = window.setTimeout(() => setControlsVisible(false), 2_000);
  }, [settingsOpen]);

  const showControls = useCallback(() => {
    setControlsVisible(true);
    scheduleControlsHide();
  }, [scheduleControlsHide]);

  useEffect(() => {
    if (settingsOpen) {
      if (controlsTimerRef.current !== null) window.clearTimeout(controlsTimerRef.current);
      setControlsVisible(true);
    } else {
      scheduleControlsHide();
    }
    return () => {
      if (controlsTimerRef.current !== null) window.clearTimeout(controlsTimerRef.current);
    };
  }, [scheduleControlsHide, settingsOpen]);

  useEffect(() => () => {
    if (duplicateHighlightTimerRef.current !== null) {
      window.clearTimeout(duplicateHighlightTimerRef.current);
    }
    if (seekCommitTimerRef.current !== null) window.clearTimeout(seekCommitTimerRef.current);
  }, []);

  useEffect(() => {
    setRoomPosition(currentPosition(playback, clockOffsetMs));
    if (!playback.playing) return;
    const timer = window.setInterval(
      () => setRoomPosition(currentPosition(playback, clockOffsetMs)),
      250,
    );
    return () => window.clearInterval(timer);
  }, [playback, clockOffsetMs]);

  useEffect(() => {
    if (
      !seekCommittedRef.current
      || seekPosition === null
      || Math.abs(player.state.positionSeconds - seekPosition) > 0.8
    ) return;
    seekCommittedRef.current = false;
    setSeekPosition(null);
    if (seekCommitTimerRef.current !== null) window.clearTimeout(seekCommitTimerRef.current);
  }, [player.state.positionSeconds, seekPosition]);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    void appWindow.isFullscreen().then(setFullscreen);
    let unlisten: (() => void) | undefined;
    void appWindow.onResized(() => {
      void appWindow.isFullscreen().then(setFullscreen);
    }).then((callback) => { unlisten = callback; });
    return () => unlisten?.();
  }, []);

  const nativePosition = player.ready ? player.state.positionSeconds : roomPosition;
  const shownPosition = seekPosition ?? nativePosition;
  const duration = Math.max(player.state.durationSeconds, shownPosition, 0);
  progressSnapshotRef.current = {
    positionSeconds: shownPosition,
    durationSeconds: player.state.durationSeconds,
  };

  useEffect(() => {
    if (
      session.role !== "host"
      || !session.activePlaylistItemId
      || !localMediaPath
      || player.loadedMediaPath !== localMediaPath
    ) return;
    const itemId = session.activePlaylistItemId;
    const timer = window.setInterval(() => {
      const snapshot = progressSnapshotRef.current;
      if (snapshot.durationSeconds <= 0) return;
      const previous = lastProgressSentRef.current;
      if (
        previous.itemId === itemId
        && Math.abs(previous.positionSeconds - snapshot.positionSeconds) < 0.5
        && Math.abs(previous.durationSeconds - snapshot.durationSeconds) < 0.5
      ) return;
      lastProgressSentRef.current = { itemId, ...snapshot };
      onPlaylistProgress(itemId, snapshot.positionSeconds, snapshot.durationSeconds);
    }, 2_000);
    return () => window.clearInterval(timer);
  }, [
    localMediaPath,
    onPlaylistProgress,
    player.loadedMediaPath,
    session.activePlaylistItemId,
    session.role,
  ]);
  const syncLabel = clockSyncState === "syncing"
    ? "Синхронизация…"
    : clockLatencyMs === null
      ? "Синхронизировать"
      : `Повторить · ${Math.round(clockLatencyMs)} мс`;

  function togglePlayback() {
    onPlaybackCommand(playback.playing ? "pause" : "play", shownPosition);
  }

  function changeVolume(value: number) {
    setVolume(value);
    savePlayerVolume(value);
  }

  function commitSeek(position = seekPosition) {
    if (position === null) return;
    seekCommittedRef.current = true;
    setSeekPosition(position);
    onPlaybackCommand("seek", position);
    if (seekCommitTimerRef.current !== null) window.clearTimeout(seekCommitTimerRef.current);
    seekCommitTimerRef.current = window.setTimeout(() => {
      seekCommittedRef.current = false;
      setSeekPosition(null);
    }, 2_000);
  }

  async function toggleFullscreen() {
    try {
      const appWindow = getCurrentWindow();
      await appWindow.setFullscreen(!fullscreen);
      setFullscreen(await appWindow.isFullscreen());
      setControlError(null);
    } catch (reason) {
      setControlError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  async function changeTrack(kind: "audio" | "subtitle", value: string) {
    try {
      const trackId = value === "off" ? null : Number(value);
      await selectPlayerTrack(kind, trackId);
      const tracks = kind === "audio" ? player.state.audioTracks : player.state.subtitleTracks;
      const track = trackId === null ? null : tracks.find((candidate) => candidate.id === trackId);
      trackPreferencesRef.current = {
        ...trackPreferencesRef.current,
        [kind]: track
          ? { disabled: false, language: track.language, title: track.title }
          : { disabled: true, language: null, title: null },
      };
      saveTrackPreferences(trackPreferencesRef.current);
      if (localMediaPath) {
        appliedTrackPreferencesRef.current.add(`${localMediaPath}:${kind}`);
      }
      setControlError(null);
    } catch (reason) {
      setControlError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  function changeSeekSeconds(value: number) {
    updatePlaybackSettings({
      ...playbackSettings,
      seekSeconds: Math.min(10, Math.max(1, value)),
    });
  }

  function updatePlaybackSettings(settings: typeof playbackSettings) {
    setPlaybackSettings(settings);
    savePlaybackSettings(settings);
  }

  function clearHotkey(action: PlayerHotkeyAction) {
    updatePlaybackSettings({
      ...playbackSettings,
      hotkeys: { ...playbackSettings.hotkeys, [action]: "" },
    });
    setCapturingHotkey(null);
  }

  function captureHotkey(event: ReactKeyboardEvent<HTMLButtonElement>, action: PlayerHotkeyAction) {
    if (capturingHotkey !== action) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.code === "Escape") {
      setCapturingHotkey(null);
      if (fullscreen) void toggleFullscreen();
      return;
    }
    const nextSettings = (() => {
      const hotkeys = { ...playbackSettings.hotkeys };
      const duplicate = HOTKEY_ACTIONS.find(({ action: candidate }) => (
        candidate !== action && hotkeys[candidate] === event.code
      ));
      if (duplicate) hotkeys[duplicate.action] = hotkeys[action];
      hotkeys[action] = event.code;
      return { ...playbackSettings, hotkeys };
    })();
    updatePlaybackSettings(nextSettings);
    setCapturingHotkey(null);
  }

  function handlePlayerKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    showControls();
    if (event.code === "Tab") {
      event.preventDefault();
      return;
    }
    if (event.code === "Escape") {
      event.preventDefault();
      if (settingsOpen) setSettingsOpen(false);
      if (fullscreen) void toggleFullscreen();
      return;
    }
    if (settingsOpen) return;

    const action = HOTKEY_ACTIONS.find(
      ({ action: candidate }) => playbackSettings.hotkeys[candidate] === event.code,
    )?.action;
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();

    if (event.repeat && [
      "togglePlayback",
      "toggleFullscreen",
      "syncClock",
      "previousPlaylistItem",
      "nextPlaylistItem",
    ].includes(action)) return;
    switch (action) {
      case "togglePlayback":
        togglePlayback();
        break;
      case "toggleFullscreen":
        void toggleFullscreen();
        break;
      case "syncClock":
        onSyncClock();
        break;
      case "seekForward":
        onPlaybackCommand("seek", Math.min(duration || Infinity, shownPosition + playbackSettings.seekSeconds));
        break;
      case "seekBackward":
        onPlaybackCommand("seek", Math.max(0, shownPosition - playbackSettings.seekSeconds));
        break;
      case "previousPlaylistItem":
        switchPlaylist(-1);
        break;
      case "nextPlaylistItem":
        switchPlaylist(1);
        break;
      case "volumeUp":
        changeVolume(Math.min(100, volume + 2.5));
        break;
      case "volumeDown":
        changeVolume(Math.max(0, volume - 2.5));
        break;
    }
  }

  function switchPlaylist(direction: -1 | 1) {
    if (session.role !== "host") return;
    const index = session.playlist.findIndex((item) => item.id === session.activePlaylistItemId);
    const target = session.playlist[index + direction];
    if (target) onSelectPlaylistItem(target.id, shownPosition, player.state.durationSeconds);
  }

  function handlePlayerPointerDown(event: ReactPointerEvent<HTMLElement>) {
    showControls();
    const target = event.target as HTMLElement;
    if (!target.closest("button, input, select")) playerRef.current?.focus({ preventScroll: true });
  }

  async function leaveSession() {
    if (fullscreen) await getCurrentWindow().setFullscreen(false);
    onLeave(shownPosition, player.state.durationSeconds);
  }

  async function addVideos() {
    try {
      const selected = await open({
        multiple: true,
        directory: false,
        filters: [{ name: "Видео", extensions: ["mkv", "mp4", "webm", "avi", "mov", "m4v"] }],
      });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      const duplicates = await onAddPlaylistFiles(paths.map((path) => ({
        path,
        name: path.split(/[\\/]/).pop() ?? path,
      })));
      if (duplicateHighlightTimerRef.current !== null) {
        window.clearTimeout(duplicateHighlightTimerRef.current);
      }
      setDuplicateItemIds(new Set(duplicates));
      duplicateHighlightTimerRef.current = window.setTimeout(
        () => setDuplicateItemIds(new Set()),
        3_200,
      );
    } catch (reason) {
      setControlError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  async function addSubtitles() {
    if (session.role !== "host" || !session.activePlaylistItemId) return;
    try {
      const selected = await open({
        multiple: true,
        directory: false,
        filters: [{ name: "Субтитры", extensions: ["srt", "ass", "ssa", "vtt", "sub"] }],
      });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      const added = await onAddExternalSubtitles(session.activePlaylistItemId, paths);
      if (added > 0) setControlError(null);
    } catch (reason) {
      setControlError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  const playerMessage = !localMediaPath
    ? session.role === "host"
      ? "Добавьте видео в плейлист"
      : "Ожидаем видео от хоста"
    : player.error
      ? "Не удалось открыть видео. Попробуйте перезапустить приложение."
      : player.ready
        ? ""
        : "Открываем видео…";

  const selectedAudio = player.state.audioTracks.find((track) => track.selected)?.id;
  const selectedSubtitle = player.state.subtitleTracks.find((track) => track.selected)?.id;
  const selectedAudioLabel = player.state.audioTracks.find((track) => track.selected)?.label
    ?? "Без звука";
  const selectedSubtitleLabel = player.state.subtitleTracks.find((track) => track.selected)?.label
    ?? "Выключены";

  return (
    <main className={`session-view ${fullscreen ? "session-view--fullscreen" : ""}`}>
      <header className="session-header">
        <div>
          <span className="eyebrow">КОМНАТА {session.roomCode}</span>
          <h1>{session.mediaName}</h1>
          <button
            className="server-address"
            onClick={() => void navigator.clipboard.writeText(session.roomCode)}
            title="Скопировать код комнаты"
          >
            Скопировать код комнаты
          </button>
        </div>
        <div className="session-header__actions">
          <div className="participants-menu">
            <button
              className={`status-dot participants-trigger status-dot--${connectionState}`}
              aria-haspopup="true"
            >
              {connectionState === "connected"
                ? `Подключено: ${session.participantCount}`
                : "Соединение потеряно"}
            </button>
            <div className="participants-popover">
              <div className="participants-popover__header">
                <strong>Участники</strong><span>{session.participantCount}</span>
              </div>
              <ul>
                {session.participants.map((participant) => (
                  <li key={participant.participantId}>
                    <span className="participant-avatar" aria-hidden="true">
                      {Array.from(participant.displayName)[0]?.toUpperCase()}
                    </span>
                    <span className="participant-name">
                      {participant.displayName}{participant.isHost && <small>Хост</small>}
                    </span>
                    <span className={`participant-ping ${pingClass(participant.pingMs)}`}>
                      {participant.pingMs === null ? "—" : `${participant.pingMs} мс`}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          </div>
          <Button variant="ghost" onClick={() => void leaveSession()}>Покинуть</Button>
        </div>
      </header>

      {error && <div className="connection-error" role="alert">{error}</div>}
      {notification && (
        <div key={notification.id} className="room-toast" role="status" aria-live="polite">
          {notification.message}
        </div>
      )}

      <section
        ref={playerRef}
        className={`player ${controlsVisible ? "player--controls-visible" : ""} ${
          !localMediaPath ? "player--without-video" : ""
        }`}
        aria-label="Видеоплеер"
        tabIndex={-1}
        onKeyDown={handlePlayerKeyDown}
        onPointerMove={showControls}
        onPointerDown={handlePlayerPointerDown}
        onPointerLeave={() => {
          if (!settingsOpen) setControlsVisible(false);
        }}
        onWheel={showControls}
      >
        <div ref={surfaceRef} className="native-video-surface">
          {playerMessage && (
            <div className={`player__center ${player.error ? "player__center--error" : ""}`}>
              <span className="player__mark">S</span>
              <p>{playerMessage}</p>
            </div>
          )}
        </div>

        {(controlError || (!player.error && error)) && (
          <div className="player-control-error" role="alert">{controlError ?? error}</div>
        )}

        {fullscreen && session.playlist.length > 0 && (
          <aside className="fullscreen-playlist player-transient-ui" aria-label="Плейлист">
            <strong>Плейлист</strong>
            <ol>
              {session.playlist.map((item, index) => {
                const active = item.id === session.activePlaylistItemId;
                return (
                  <li key={item.id} className={active ? "is-active" : ""}>
                    <button
                      type="button"
                      disabled={session.role !== "host" || active}
                      onClick={() => onSelectPlaylistItem(
                        item.id,
                        shownPosition,
                        player.state.durationSeconds,
                      )}
                      title={item.name}
                    >
                      <span aria-hidden="true">{active ? "▶" : index + 1}</span>
                      <b>{item.name}</b>
                    </button>
                  </li>
                );
              })}
            </ol>
          </aside>
        )}

        <div className="player-settings-wrap player-transient-ui">
          <button
            className="player-icon-button"
            onClick={() => setSettingsOpen(true)}
            aria-label="Настройки плеера"
            aria-expanded={settingsOpen}
            title="Настройки"
          >
            ⚙
          </button>
        </div>

        {settingsOpen && (
          <div className="playback-settings-backdrop" onPointerDown={() => setSettingsOpen(false)}>
            <div
              className="playback-settings-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="playback-settings-title"
              onPointerDown={(event) => event.stopPropagation()}
            >
              <div className="playback-settings-dialog__header">
                <div>
                  <span>ВОСПРОИЗВЕДЕНИЕ</span>
                  <h2 id="playback-settings-title">Настройки плеера</h2>
                </div>
                <button
                  className="player-icon-button"
                  onClick={() => setSettingsOpen(false)}
                  aria-label="Закрыть настройки"
                  title="Закрыть"
                >
                  ×
                </button>
              </div>

              <div className="playback-settings-grid">
                <section>
                  <h3>Скорость и дорожки</h3>
                  <div className="player-settings">
              <label>
                <span>Скорость (для всех)</span>
                <select
                  value={playback.playbackRate}
                  onChange={(event) => onPlaybackRate(Number(event.target.value), shownPosition)}
                >
                  {[0.5, 0.75, 1, 1.25, 1.5, 2].map((speed) => (
                    <option key={speed} value={speed}>{speed}×</option>
                  ))}
                </select>
              </label>
              <label>
                <span>Аудиодорожка</span>
                <div className={`track-select ${selectedAudioLabel.length > 28 ? "track-select--long" : ""}`}>
                  <select
                    value={selectedAudio ?? "off"}
                    disabled={!player.ready || player.state.audioTracks.length === 0}
                    onChange={(event) => void changeTrack("audio", event.target.value)}
                    aria-label="Аудиодорожка"
                  >
                    <option value="off">Без звука</option>
                    {player.state.audioTracks.map((track) => (
                      <option key={track.id} value={track.id}>{track.label}</option>
                    ))}
                  </select>
                  <span className="track-select__value" aria-hidden="true">
                    <i>{selectedAudioLabel}</i>
                    {selectedAudioLabel.length > 28 && <i>{selectedAudioLabel}</i>}
                  </span>
                </div>
              </label>
              <label>
                <span>Субтитры</span>
                <div className={`track-select ${selectedSubtitleLabel.length > 28 ? "track-select--long" : ""}`}>
                  <select
                    value={selectedSubtitle ?? "off"}
                    disabled={!player.ready || player.state.subtitleTracks.length === 0}
                    onChange={(event) => void changeTrack("subtitle", event.target.value)}
                    aria-label="Субтитры"
                  >
                    <option value="off">Выключены</option>
                    {player.state.subtitleTracks.map((track) => (
                      <option key={track.id} value={track.id}>{track.label}</option>
                    ))}
                  </select>
                  <span className="track-select__value" aria-hidden="true">
                    <i>{selectedSubtitleLabel}</i>
                    {selectedSubtitleLabel.length > 28 && <i>{selectedSubtitleLabel}</i>}
                  </span>
                </div>
                {session.role === "host" && session.activePlaylistItemId && (
                  <button
                    type="button"
                    className="add-subtitles-button"
                    onClick={() => void addSubtitles()}
                  >
                    Добавить субтитры
                  </button>
                )}
              </label>
                  </div>
                </section>

                <section>
                  <h3>Управление</h3>
                  <label className="seek-step-setting">
                    <span>Шаг перемотки</span>
                    <output>{playbackSettings.seekSeconds} с</output>
                    <input
                      type="range"
                      min="1"
                      max="10"
                      step="1"
                      value={playbackSettings.seekSeconds}
                      onChange={(event) => changeSeekSeconds(Number(event.target.value))}
                    />
                  </label>
                  <div className="hotkey-settings">
                    {HOTKEY_ACTIONS.map(({ action, label }) => (
                      <div className="hotkey-setting" key={action}>
                        <span>{label}</span>
                        <div className="hotkey-binding-controls">
                          <button
                            className={capturingHotkey === action ? "is-capturing" : ""}
                            onClick={() => setCapturingHotkey(action)}
                            onKeyDown={(event) => captureHotkey(event, action)}
                          >
                            {capturingHotkey === action
                              ? "Нажмите клавишу…"
                              : hotkeyLabel(playbackSettings.hotkeys[action])}
                          </button>
                          <button
                            className="clear-hotkey"
                            onClick={() => clearHotkey(action)}
                            disabled={!playbackSettings.hotkeys[action]}
                            aria-label={`Убрать назначение: ${label}`}
                            title="Убрать назначение"
                          >
                            ×
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                  <button
                    className="reset-hotkeys"
                    onClick={() => updatePlaybackSettings({
                      ...DEFAULT_PLAYBACK_SETTINGS,
                      hotkeys: { ...DEFAULT_PLAYBACK_SETTINGS.hotkeys },
                    })}
                  >
                    Вернуть значения по умолчанию
                  </button>
                  <p className="hotkey-note">
                    Клавиши работают, когда выбран плеер, и не зависят от языка раскладки.
                    Esc всегда выходит из полноэкранного режима.
                  </p>
                </section>
              </div>
            </div>
          </div>
        )}

        <div className="player__controls player-transient-ui">
          <div className="player-progress">
            <input
              aria-label="Позиция воспроизведения"
              type="range"
              min="0"
              max={Math.max(duration, 1)}
              step="0.1"
              value={Math.min(shownPosition, Math.max(duration, 1))}
              disabled={duration <= 0}
              onChange={(event) => {
                seekCommittedRef.current = false;
                setSeekPosition(Number(event.target.value));
              }}
              onPointerUp={(event) => commitSeek(Number(event.currentTarget.value))}
              onKeyUp={(event) => {
                if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
                  commitSeek(Number(event.currentTarget.value));
                }
              }}
            />
            <div className="player-progress__time">
              <span>{formatDuration(shownPosition)}</span>
              <span>{formatDuration(duration)}</span>
            </div>
          </div>
          <div className="player__footer">
            <div className="playlist-navigation">
              <button
                className="player-icon-button"
                onClick={() => switchPlaylist(-1)}
                disabled={session.role !== "host" || session.playlist.findIndex((item) => item.id === session.activePlaylistItemId) <= 0}
                aria-label="Предыдущее видео"
                title="Предыдущее видео"
              >◀│</button>
              <button
                className="player-icon-button player-play-button"
                onClick={togglePlayback}
                aria-label={playback.playing ? "Пауза" : "Воспроизвести"}
                title={playback.playing ? "Пауза" : "Воспроизвести"}
              >
                {playback.playing ? "Ⅱ" : "▶"}
              </button>
              <button
                className="player-icon-button"
                onClick={() => switchPlaylist(1)}
                disabled={session.role !== "host" || session.playlist.findIndex((item) => item.id === session.activePlaylistItemId) >= session.playlist.length - 1}
                aria-label="Следующее видео"
                title="Следующее видео"
              >│▶</button>
            </div>
            <label className="volume-control">
              <span className="volume-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24" focusable="false">
                  <path d="M4 9v6h4l5 4V5L8 9H4Z" />
                  {volume > 0 && <path d="M16 9.2a4 4 0 0 1 0 5.6M18.8 6.5a7.7 7.7 0 0 1 0 11" />}
                  {volume === 0 && <path d="m16 9 5 6m0-6-5 6" />}
                </svg>
              </span>
              <input
                aria-label={`Громкость: ${Math.round(volume)}%`}
                type="range"
                min="0"
                max="100"
                step="0.5"
                value={volume}
                onChange={(event) => changeVolume(Number(event.target.value))}
              />
            </label>
            <button
              className="player-icon-button"
              onClick={() => void toggleFullscreen()}
              title={fullscreen ? "Выйти из полноэкранного режима" : "На весь экран"}
              aria-label={fullscreen ? "Выйти из полноэкранного режима" : "На весь экран"}
            >
              {fullscreen ? "↙" : "⛶"}
            </button>
          </div>
        </div>
      </section>

      <section className="playlist-panel" aria-labelledby="playlist-title">
        <div className="playlist-panel__header">
          <div>
            <span className="eyebrow">ВИДЕО В КОМНАТЕ</span>
            <h2 id="playlist-title">Плейлист</h2>
          </div>
          {session.role === "host" && (
            <Button variant="secondary" onClick={() => void addVideos()}>Добавить видео</Button>
          )}
        </div>
        {session.playlist.length === 0 ? (
          <p className="playlist-empty">
            {session.role === "host"
              ? "Здесь пока нет видео. Добавьте первое."
              : "В плейлисте пока ничего нет."}
          </p>
        ) : (
          <ol className="playlist-items">
            {session.playlist.map((item, index) => {
              const active = item.id === session.activePlaylistItemId;
              const itemDuration = active && session.role === "host"
                ? player.state.durationSeconds
                : item.durationSeconds;
              const itemProgress = active && session.role === "host"
                ? shownPosition
                : item.progressSeconds;
              const progressPercent = itemDuration > 0
                ? Math.min(100, Math.max(0, itemProgress / itemDuration * 100))
                : 0;
              return (
                <li
                  key={item.id}
                  className={`${active ? "playlist-item--active" : ""} ${
                    duplicateItemIds.has(item.id) ? "playlist-item--duplicate" : ""
                  }`}
                >
                  <button
                    className="playlist-item__select"
                    onClick={() => session.role === "host" && onSelectPlaylistItem(
                      item.id,
                      shownPosition,
                      player.state.durationSeconds,
                    )}
                    disabled={session.role !== "host" || active}
                    aria-label={`${active ? "Сейчас воспроизводится" : "Воспроизвести"}: ${item.name}`}
                  >
                    <span aria-hidden="true">{active ? "▶" : index + 1}</span>
                    <span className="playlist-item__content">
                      <strong>{item.name}</strong>
                      <span
                        className="playlist-item__progress"
                        title={`${formatDuration(itemProgress)} / ${formatDuration(itemDuration)}`}
                      ><i style={{ width: `${progressPercent}%` }} /></span>
                    </span>
                    {active && <small>Сейчас воспроизводится</small>}
                  </button>
                  {session.role === "host" && (
                    <div className="playlist-item__actions">
                      <button
                        onClick={() => onMovePlaylistItem(item.id, -1)}
                        disabled={index === 0}
                        aria-label={`Переместить выше: ${item.name}`}
                        title="Переместить выше"
                      >↑</button>
                      <button
                        onClick={() => onMovePlaylistItem(item.id, 1)}
                        disabled={index === session.playlist.length - 1}
                        aria-label={`Переместить ниже: ${item.name}`}
                        title="Переместить ниже"
                      >↓</button>
                      <button
                        className="playlist-item__remove"
                        onClick={() => onRemovePlaylistItem(item.id)}
                        aria-label={`Удалить из плейлиста: ${item.name}`}
                        title="Удалить"
                      >×</button>
                    </div>
                  )}
                </li>
              );
            })}
          </ol>
        )}
      </section>

      <section className="sync-note">
        <span>◉</span>
        <div>
          <strong>
            {clockSyncState === "synced"
              ? "Время синхронизировано"
              : "Синхронизация времени"}
          </strong>
          <p>Приложение сверяет время между компьютерами, чтобы видео шло одновременно. При необходимости синхронизацию можно повторить.</p>
        </div>
        <Button
          className="sync-button"
          variant="secondary"
          onClick={onSyncClock}
          disabled={clockSyncState === "syncing" || connectionState !== "connected"}
        >
          {syncLabel}
        </Button>
      </section>
    </main>
  );
}
