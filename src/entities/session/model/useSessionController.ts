import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  PROTOCOL_VERSION,
  type PlaybackAction,
  type PlaybackState,
  type PlaylistItem,
  type ExternalSubtitle,
  type ServerMessage,
} from "../../../shared/api/protocol";
import { normalizeWsUrl, SyncWatchSocket } from "../../../shared/api/syncwatchSocket";
import {
  deleteSavedPlaylist,
  saveSavedPlaylist,
  type SavedPlaylist,
} from "../../../shared/lib/preferences";
import { createRoomCode } from "../../../shared/lib/room";
import { normalizeMediaPath } from "../../../shared/lib/mediaPath";
import type {
  LocalExternalSubtitle,
  LocalMediaFile,
  PlayerSubtitleSource,
  SessionControllerState,
} from "./types";

interface WsServerInfo {
  port: number;
  localAddress: string;
  wsUrl: string;
}

interface RoomMediaSource {
  itemId: string;
  path: string;
  subtitles: { subtitleId: string; path: string }[];
}

interface DetectedSubtitles {
  videoPath: string;
  subtitles: { path: string; name: string; language: string | null }[];
}

const initialPlayback: PlaybackState = {
  positionSeconds: 0,
  playing: false,
  playbackRate: 1,
  updatedAtMs: 0,
  revision: 0,
};

const initialClockState = {
  clockOffsetMs: 0,
  clockLatencyMs: null,
  clockSyncState: "idle" as const,
};

interface ClockSample {
  roundTripMs: number;
  offsetMs: number;
}

function playlistTitle(files: LocalMediaFile[]): string {
  if (files.length === 0) return `Пустой плейлист · ${new Date().toLocaleDateString("ru-RU")}`;
  if (files.length === 1) return files[0].name;
  return `${files[0].name} · ещё ${files.length - 1}`;
}

function withItemProgress(
  playlist: PlaylistItem[],
  itemId: string | null,
  progressSeconds: number,
  durationSeconds: number,
): PlaylistItem[] {
  if (!itemId) return playlist;
  const duration = Number.isFinite(durationSeconds) ? Math.max(0, durationSeconds) : 0;
  const progress = Number.isFinite(progressSeconds)
    ? Math.max(0, duration > 0 ? Math.min(progressSeconds, duration) : progressSeconds)
    : 0;
  return playlist.map((item) => item.id === itemId
    ? { ...item, progressSeconds: progress, durationSeconds: duration }
    : item);
}

function createMediaUrl(
  serverAddress: string,
  roomCode: string,
  itemId: string,
  accessToken: string,
): string | null {
  try {
    const url = new URL(normalizeWsUrl(serverAddress));
    url.protocol = url.protocol === "wss:" ? "https:" : "http:";
    url.pathname = `/media/${encodeURIComponent(roomCode)}/${encodeURIComponent(itemId)}`;
    url.search = "";
    url.searchParams.set("token", accessToken);
    return url.toString();
  } catch {
    return null;
  }
}

function createSubtitleUrl(
  serverAddress: string,
  roomCode: string,
  itemId: string,
  subtitleId: string,
  accessToken: string,
): string | null {
  try {
    const url = new URL(normalizeWsUrl(serverAddress));
    url.protocol = url.protocol === "wss:" ? "https:" : "http:";
    url.pathname = `/subtitles/${encodeURIComponent(roomCode)}/${encodeURIComponent(itemId)}/${encodeURIComponent(subtitleId)}`;
    url.search = "";
    url.searchParams.set("token", accessToken);
    return url.toString();
  } catch {
    return null;
  }
}

async function withDetectedSubtitles(files: LocalMediaFile[]): Promise<LocalMediaFile[]> {
  if (files.length === 0) return files;
  const detected = await invoke<DetectedSubtitles[]>("find_external_subtitles", {
    videoPaths: files.map((file) => file.path),
  });
  const byPath = new Map(detected.map((result) => [normalizeMediaPath(result.videoPath), result]));
  return files.map((file) => {
    const existing = file.externalSubtitles ?? [];
    const knownPaths = new Set(existing.map((subtitle) => normalizeMediaPath(subtitle.path)));
    const automatic = (byPath.get(normalizeMediaPath(file.path))?.subtitles ?? [])
      .filter((subtitle) => !knownPaths.has(normalizeMediaPath(subtitle.path)))
      .map((subtitle): LocalExternalSubtitle => ({
        id: crypto.randomUUID(),
        ...subtitle,
      }));
    return { ...file, externalSubtitles: [...existing, ...automatic] };
  });
}

export function useSessionController() {
  const [state, setState] = useState<SessionControllerState>({
    session: null,
    localMediaPath: null,
    externalSubtitleSources: [],
    playback: initialPlayback,
    connectionState: "idle",
    ...initialClockState,
    notification: null,
    error: null,
  });
  const socketRef = useRef<SyncWatchSocket | null>(null);
  const participantIdRef = useRef(crypto.randomUUID());
  const notificationIdRef = useRef(0);
  const notificationTimerRef = useRef<number | null>(null);
  const clockSamplesRef = useRef<ClockSample[]>([]);
  const hostMediaPathsRef = useRef(new Map<string, string>());
  const hostSubtitlePathsRef = useRef(new Map<string, Map<string, string>>());
  const savedPlaylistIdRef = useRef<string | null>(null);
  const savedPlaylistTitleRef = useRef("");

  const syncHostMediaSources = useCallback((roomCode: string, accessToken: string) => {
    const sources: RoomMediaSource[] = Array.from(
      hostMediaPathsRef.current,
      ([itemId, path]) => ({
        itemId,
        path,
        subtitles: Array.from(
          hostSubtitlePathsRef.current.get(itemId) ?? [],
          ([subtitleId, subtitlePath]) => ({ subtitleId, path: subtitlePath }),
        ),
      }),
    );
    return invoke<void>("set_room_media_sources", { roomCode, accessToken, sources });
  }, []);

  const subtitleSourcesFor = useCallback((
    role: "host" | "viewer",
    serverUrl: string,
    roomCode: string,
    item: PlaylistItem | undefined,
    accessToken: string,
  ): PlayerSubtitleSource[] => {
    if (!item) return [];
    if (role === "host") {
      const paths = hostSubtitlePathsRef.current.get(item.id);
      return item.externalSubtitles.flatMap((subtitle) => {
        const source = paths?.get(subtitle.id);
        return source ? [{ ...subtitle, source }] : [];
      });
    }
    return item.externalSubtitles.flatMap((subtitle) => {
      const source = createSubtitleUrl(
        serverUrl,
        roomCode,
        item.id,
        subtitle.id,
        accessToken,
      );
      return source ? [{ ...subtitle, source }] : [];
    });
  }, []);

  const persistHostPlaylist = useCallback((
    playlist: PlaylistItem[],
    activePlaylistItemId: string | null,
  ) => {
    const id = savedPlaylistIdRef.current;
    if (!id) return;
    if (playlist.length === 0) {
      deleteSavedPlaylist(id);
      return;
    }
    saveSavedPlaylist({
      id,
      title: savedPlaylistTitleRef.current,
      activePlaylistItemId,
      updatedAt: Date.now(),
      items: playlist.flatMap((item) => {
        const path = hostMediaPathsRef.current.get(item.id);
        const subtitlePaths = hostSubtitlePathsRef.current.get(item.id);
        return path ? [{
          ...item,
          path,
          externalSubtitles: item.externalSubtitles.flatMap((subtitle) => {
            const subtitlePath = subtitlePaths?.get(subtitle.id);
            return subtitlePath ? [{ ...subtitle, path: subtitlePath }] : [];
          }),
        }] : [];
      }),
    });
  }, []);

  const showNotification = useCallback((message: string) => {
    const id = ++notificationIdRef.current;
    if (notificationTimerRef.current !== null) window.clearTimeout(notificationTimerRef.current);
    setState((current) => ({ ...current, notification: { id, message } }));
    notificationTimerRef.current = window.setTimeout(() => {
      setState((current) => current.notification?.id === id ? { ...current, notification: null } : current);
    }, 3200);
  }, []);

  const measureLatency = useCallback((showSyncStatus: boolean) => {
    const clientTimeMs = Date.now();
    try {
      socketRef.current?.send({ type: "ping", payload: { clientTimeMs } });
      if (showSyncStatus) {
        setState((current) => ({ ...current, clockSyncState: "syncing" }));
      }
    } catch (error) {
      setState((current) => ({ ...current, error: error instanceof Error ? error.message : String(error) }));
    }
  }, []);

  const syncClock = useCallback(() => measureLatency(true), [measureLatency]);

  const handleMessage = useCallback((message: ServerMessage) => {
    if (message.type === "error") {
      setState((current) => ({
        ...current,
        session: current.connectionState === "connecting" ? null : current.session,
        error: message.payload.message,
      }));
      return;
    }
    if (message.type === "room_snapshot") {
      const { room } = message.payload;
      setState((current) => {
        const role = room.hostParticipantId === participantIdRef.current ? "host" : "viewer";
        const serverUrl = current.session?.serverUrl ?? "";
        const activeItem = room.playlist?.find((item) => item.id === room.activePlaylistItemId);
        const localMediaPath = room.activePlaylistItemId
          ? role === "host"
            ? hostMediaPathsRef.current.get(room.activePlaylistItemId) ?? null
            : createMediaUrl(
              serverUrl,
              room.roomCode,
              room.activePlaylistItemId,
              room.mediaAccessToken,
            )
          : null;
        return {
          ...current,
          session: {
            roomCode: room.roomCode,
            mediaName: room.mediaName,
            playlist: room.playlist ?? [],
            activePlaylistItemId: room.activePlaylistItemId ?? null,
            participantCount: room.participantCount,
            participants: room.participants ?? [],
            role,
            serverUrl,
            mediaAccessToken: room.mediaAccessToken,
          },
          playback: room.playback,
          localMediaPath,
          externalSubtitleSources: subtitleSourcesFor(
            role,
            serverUrl,
            room.roomCode,
            activeItem,
            room.mediaAccessToken,
          ),
          connectionState: "connected",
          error: null,
        };
      });
      syncClock();
      return;
    }
    if (message.type === "room_closed") {
      socketRef.current?.disconnect();
      clockSamplesRef.current = [];
      setState({
        session: null,
        localMediaPath: null,
        externalSubtitleSources: [],
        playback: initialPlayback,
        connectionState: "disconnected",
        ...initialClockState,
        notification: null,
        error: message.payload.reason === "host_left"
          ? "Хост покинул комнату. Сессия завершена для всех участников."
          : "Сессия была завершена.",
      });
      return;
    }
    if (message.type === "participant_count") {
      setState((current) => current.session ? {
        ...current,
        session: { ...current.session, participantCount: message.payload.participantCount },
      } : current);
      return;
    }
    if (message.type === "participant_joined") {
      if (message.payload.participantId !== participantIdRef.current) {
        showNotification(`${message.payload.displayName} присоединился к комнате`);
      }
      return;
    }
    if (message.type === "participant_list") {
      setState((current) => current.session ? {
        ...current,
        session: {
          ...current.session,
          participantCount: message.payload.participants.length,
          participants: message.payload.participants,
        },
      } : current);
      return;
    }
    if (message.type === "pong") {
      const receivedAtMs = Date.now();
      const roundTripMs = Math.max(0, receivedAtMs - message.payload.clientTimeMs);
      const midpointMs = message.payload.clientTimeMs + roundTripMs / 2;
      clockSamplesRef.current = [
        ...clockSamplesRef.current,
        {
          roundTripMs,
          offsetMs: message.payload.serverTimeMs - midpointMs,
        },
      ].slice(-20);
      const bestSample = clockSamplesRef.current.reduce((best, sample) => (
        sample.roundTripMs < best.roundTripMs ? sample : best
      ));
      const sortedRoundTrips = clockSamplesRef.current
        .map((sample) => sample.roundTripMs)
        .sort((left, right) => left - right);
      const medianRoundTrip = sortedRoundTrips[Math.floor(sortedRoundTrips.length / 2)];
      setState((current) => ({
        ...current,
        clockOffsetMs: bestSample.offsetMs,
        clockLatencyMs: medianRoundTrip / 2,
        clockSyncState: "synced",
        error: null,
      }));
      try {
        socketRef.current?.send({
          type: "latency_report",
          payload: { pingMs: Math.round(roundTripMs) },
        });
      } catch {
        // A closing socket will update the connection state separately.
      }
      return;
    }
    if (message.type === "playback_state") {
      setState((current) => ({ ...current, playback: message.payload.playback }));
      return;
    }
    if (message.type === "playlist_state") {
      const active = message.payload.playlist.find(
        (item) => item.id === message.payload.activePlaylistItemId,
      );
      setState((current) => {
        if (!current.session) return current;
        const activeChanged = current.session.activePlaylistItemId
          !== message.payload.activePlaylistItemId;
        return {
          ...current,
          playback: activeChanged ? {
            ...current.playback,
            positionSeconds: active?.progressSeconds ?? 0,
            playing: false,
            updatedAtMs: Date.now(),
          } : current.playback,
          localMediaPath: message.payload.activePlaylistItemId
            ? current.session.role === "host"
              ? hostMediaPathsRef.current.get(message.payload.activePlaylistItemId) ?? null
              : createMediaUrl(
                current.session.serverUrl,
                current.session.roomCode,
                message.payload.activePlaylistItemId,
                current.session.mediaAccessToken,
              )
            : null,
          externalSubtitleSources: subtitleSourcesFor(
            current.session.role,
            current.session.serverUrl,
            current.session.roomCode,
            active,
            current.session.mediaAccessToken,
          ),
          session: {
            ...current.session,
            playlist: message.payload.playlist,
            activePlaylistItemId: message.payload.activePlaylistItemId,
            mediaName: active?.name ?? "Плейлист пуст",
          },
        };
      });
      persistHostPlaylist(message.payload.playlist, message.payload.activePlaylistItemId);
    }
  }, [persistHostPlaylist, showNotification, subtitleSourcesFor, syncClock]);

  const prepareSocket = useCallback(async (url: string) => {
    socketRef.current?.disconnect();
    setState((current) => ({ ...current, connectionState: "connecting", error: null }));
    const socket = new SyncWatchSocket(handleMessage, (socketStatus) => {
      if (socketStatus === "disconnected") {
        setState((current) => ({ ...current, connectionState: "disconnected" }));
      }
    });
    socketRef.current = socket;
    await socket.connect(url);
    return socket;
  }, [handleMessage]);

  const startHostSession = useCallback(async (
    files: LocalMediaFile[],
    displayName: string,
    savedPlaylist?: SavedPlaylist,
  ) => {
    try {
      clockSamplesRef.current = [];
      const preparedFiles = await withDetectedSubtitles(files);
      const server = await invoke<WsServerInfo>("get_ws_server_info");
      const roomCode = createRoomCode();
      const mediaAccessToken = crypto.randomUUID();
      const playlist = preparedFiles.map((file) => ({
        id: file.id ?? crypto.randomUUID(),
        name: file.name,
        progressSeconds: file.progressSeconds ?? 0,
        durationSeconds: file.durationSeconds ?? 0,
        externalSubtitles: (file.externalSubtitles ?? []).map((subtitle) => ({
          id: subtitle.id,
          name: subtitle.name,
          language: subtitle.language,
        })),
      }));
      hostMediaPathsRef.current = new Map(
        playlist.map((item, index) => [item.id, preparedFiles[index].path]),
      );
      hostSubtitlePathsRef.current = new Map(
        playlist.map((item, index) => [
          item.id,
          new Map((preparedFiles[index].externalSubtitles ?? []).map((subtitle) => [
            subtitle.id,
            subtitle.path,
          ])),
        ]),
      );
      savedPlaylistIdRef.current = savedPlaylist?.id ?? crypto.randomUUID();
      savedPlaylistTitleRef.current = savedPlaylist?.title ?? playlistTitle(preparedFiles);
      const activePlaylistItemId = savedPlaylist?.activePlaylistItemId
        && playlist.some((item) => item.id === savedPlaylist.activePlaylistItemId)
        ? savedPlaylist.activePlaylistItemId
        : playlist[0]?.id ?? null;
      await syncHostMediaSources(roomCode, mediaAccessToken);
      await invoke<void>("start_room_discovery");
      persistHostPlaylist(playlist, activePlaylistItemId);
      setState((current) => ({
        ...current,
        localMediaPath: activePlaylistItemId
          ? hostMediaPathsRef.current.get(activePlaylistItemId) ?? null
          : null,
        externalSubtitleSources: subtitleSourcesFor(
          "host",
          server.localAddress,
          roomCode,
          playlist.find((item) => item.id === activePlaylistItemId),
          mediaAccessToken,
        ),
        session: {
          roomCode,
          role: "host",
          mediaName: playlist[0]?.name ?? "Плейлист пуст",
          playlist,
          activePlaylistItemId,
          participantCount: 1,
          participants: [{
            participantId: participantIdRef.current,
            displayName: displayName.trim(),
            isHost: true,
            pingMs: null,
          }],
          serverUrl: server.localAddress,
          mediaAccessToken,
        },
      }));
      const socket = await prepareSocket(server.wsUrl);
      socket.send({
        type: "create_room",
        payload: {
          version: PROTOCOL_VERSION,
          roomCode,
          mediaName: playlist[0]?.name ?? "Плейлист пуст",
          playlist,
          activePlaylistItemId,
          participantId: participantIdRef.current,
          displayName: displayName.trim(),
          mediaAccessToken,
        },
      });
    } catch (error) {
      void invoke("stop_room_discovery");
      savedPlaylistIdRef.current = null;
      setState((current) => ({ ...current, session: null, localMediaPath: null, externalSubtitleSources: [], connectionState: "disconnected", error: error instanceof Error ? error.message : String(error) }));
    }
  }, [persistHostPlaylist, prepareSocket, subtitleSourcesFor, syncHostMediaSources]);

  const createSession = useCallback(
    (files: LocalMediaFile[], displayName: string) => startHostSession(files, displayName),
    [startHostSession],
  );

  const createSessionFromSavedPlaylist = useCallback(
    (playlist: SavedPlaylist, displayName: string) => startHostSession(
      playlist.items.map((item) => ({ ...item })),
      displayName,
      playlist,
    ),
    [startHostSession],
  );

  const joinSession = useCallback(async (address: string, roomCode: string, displayName: string) => {
    try {
      savedPlaylistIdRef.current = null;
      hostMediaPathsRef.current.clear();
      hostSubtitlePathsRef.current.clear();
      clockSamplesRef.current = [];
      const wsUrl = normalizeWsUrl(address);
      setState((current) => ({
        ...current,
        localMediaPath: null,
        externalSubtitleSources: [],
        session: {
          roomCode,
          role: "viewer",
          mediaName: "Подключение…",
          playlist: [],
          activePlaylistItemId: null,
          participantCount: 1,
          participants: [],
          serverUrl: address.trim(),
          mediaAccessToken: "",
        },
      }));
      const socket = await prepareSocket(wsUrl);
      socket.send({
        type: "join_room",
        payload: {
          version: PROTOCOL_VERSION,
          roomCode,
          participantId: participantIdRef.current,
          displayName: displayName.trim(),
        },
      });
    } catch (error) {
      setState((current) => ({ ...current, session: null, localMediaPath: null, externalSubtitleSources: [], connectionState: "disconnected", error: error instanceof Error ? error.message : String(error) }));
    }
  }, [prepareSocket]);

  const sendPlaybackCommand = useCallback((action: PlaybackAction, positionSeconds: number) => {
    if (!state.session) return;
    socketRef.current?.send({
      type: "playback_command",
      payload: { roomCode: state.session.roomCode, action, positionSeconds },
    });
  }, [state.session]);

  const sendPlaybackRate = useCallback((playbackRate: number, positionSeconds: number) => {
    if (!state.session) return;
    socketRef.current?.send({
      type: "playback_rate",
      payload: {
        roomCode: state.session.roomCode,
        playbackRate,
        positionSeconds,
      },
    });
  }, [state.session]);

  const sendPlaylist = useCallback((playlist: PlaylistItem[], activePlaylistItemId: string | null) => {
    if (!state.session || state.session.role !== "host") return;
    socketRef.current?.send({
      type: "playlist_update",
      payload: { roomCode: state.session.roomCode, playlist, activePlaylistItemId },
    });
  }, [state.session]);

  const addPlaylistFiles = useCallback(async (files: LocalMediaFile[]) => {
    if (!state.session || state.session.role !== "host" || files.length === 0) return [];
    const preparedFiles = await withDetectedSubtitles(files);
    if (state.session.playlist.length === 0) savedPlaylistTitleRef.current = playlistTitle(preparedFiles);
    const knownPaths = new Map(
      Array.from(hostMediaPathsRef.current, ([itemId, path]) => [normalizeMediaPath(path), itemId]),
    );
    const duplicateItemIds = new Set<string>();
    const added: PlaylistItem[] = [];
    for (const file of preparedFiles) {
      const pathKey = normalizeMediaPath(file.path);
      const duplicateItemId = knownPaths.get(pathKey);
      if (duplicateItemId) {
        duplicateItemIds.add(duplicateItemId);
        continue;
      }
      const item = {
        id: crypto.randomUUID(),
        name: file.name,
        progressSeconds: 0,
        durationSeconds: 0,
        externalSubtitles: (file.externalSubtitles ?? []).map((subtitle) => ({
          id: subtitle.id,
          name: subtitle.name,
          language: subtitle.language,
        })),
      };
      hostMediaPathsRef.current.set(item.id, file.path);
      hostSubtitlePathsRef.current.set(
        item.id,
        new Map((file.externalSubtitles ?? []).map((subtitle) => [subtitle.id, subtitle.path])),
      );
      knownPaths.set(pathKey, item.id);
      added.push(item);
    }
    if (added.length === 0) return Array.from(duplicateItemIds);
    const playlist = [...state.session.playlist, ...added];
    try {
      await syncHostMediaSources(state.session.roomCode, state.session.mediaAccessToken);
      sendPlaylist(playlist, state.session.activePlaylistItemId ?? added[0].id);
    } catch (error) {
      setState((current) => ({
        ...current,
        error: error instanceof Error ? error.message : String(error),
      }));
    }
    return Array.from(duplicateItemIds);
  }, [sendPlaylist, state.session, syncHostMediaSources]);

  const removePlaylistItem = useCallback((itemId: string) => {
    if (!state.session || state.session.role !== "host") return;
    const index = state.session.playlist.findIndex((item) => item.id === itemId);
    if (index < 0) return;
    const playlist = state.session.playlist.filter((item) => item.id !== itemId);
    const activePlaylistItemId = state.session.activePlaylistItemId === itemId
      ? playlist[Math.min(index, playlist.length - 1)]?.id ?? null
      : state.session.activePlaylistItemId;
    hostMediaPathsRef.current.delete(itemId);
    hostSubtitlePathsRef.current.delete(itemId);
    void syncHostMediaSources(state.session.roomCode, state.session.mediaAccessToken)
      .then(() => sendPlaylist(playlist, activePlaylistItemId))
      .catch((error: unknown) => setState((current) => ({
        ...current,
        error: error instanceof Error ? error.message : String(error),
      })));
  }, [sendPlaylist, state.session, syncHostMediaSources]);

  const addExternalSubtitles = useCallback(async (itemId: string, paths: string[]) => {
    if (!state.session || state.session.role !== "host" || paths.length === 0) return 0;
    const item = state.session.playlist.find((candidate) => candidate.id === itemId);
    if (!item) return 0;
    const previousPaths = new Map(hostSubtitlePathsRef.current.get(itemId) ?? []);
    const nextPaths = new Map(previousPaths);
    const knownPaths = new Set(Array.from(previousPaths.values(), normalizeMediaPath));
    const added: ExternalSubtitle[] = [];
    for (const path of paths) {
      const normalizedPath = normalizeMediaPath(path);
      if (knownPaths.has(normalizedPath) || added.length + item.externalSubtitles.length >= 64) {
        continue;
      }
      const name = path.split(/[\\/]/).pop() ?? "Субтитры";
      const stem = name.replace(/\.[^.]+$/, "");
      const language = stem
        .split(/[. _-]/)
        .reverse()
        .find((part) => /^[a-z]{2,3}$/i.test(part))
        ?.toLowerCase() ?? null;
      const subtitle = { id: crypto.randomUUID(), name, language };
      added.push(subtitle);
      nextPaths.set(subtitle.id, path);
      knownPaths.add(normalizedPath);
    }
    if (added.length === 0) return 0;
    hostSubtitlePathsRef.current.set(itemId, nextPaths);
    const playlist = state.session.playlist.map((candidate) => candidate.id === itemId
      ? { ...candidate, externalSubtitles: [...candidate.externalSubtitles, ...added] }
      : candidate);
    try {
      await syncHostMediaSources(state.session.roomCode, state.session.mediaAccessToken);
      sendPlaylist(playlist, state.session.activePlaylistItemId);
      return added.length;
    } catch (error) {
      hostSubtitlePathsRef.current.set(itemId, previousPaths);
      setState((current) => ({
        ...current,
        error: error instanceof Error ? error.message : String(error),
      }));
      return 0;
    }
  }, [sendPlaylist, state.session, syncHostMediaSources]);

  const movePlaylistItem = useCallback((itemId: string, direction: -1 | 1) => {
    if (!state.session || state.session.role !== "host") return;
    const from = state.session.playlist.findIndex((item) => item.id === itemId);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= state.session.playlist.length) return;
    const playlist = [...state.session.playlist];
    [playlist[from], playlist[to]] = [playlist[to], playlist[from]];
    sendPlaylist(playlist, state.session.activePlaylistItemId);
  }, [sendPlaylist, state.session]);

  const selectPlaylistItem = useCallback((
    itemId: string,
    currentPositionSeconds: number,
    currentDurationSeconds: number,
  ) => {
    if (!state.session || state.session.role !== "host") return;
    if (!state.session.playlist.some((item) => item.id === itemId)) return;
    sendPlaylist(withItemProgress(
      state.session.playlist,
      state.session.activePlaylistItemId,
      currentPositionSeconds,
      currentDurationSeconds,
    ), itemId);
  }, [sendPlaylist, state.session]);

  const updatePlaylistProgress = useCallback((
    itemId: string,
    progressSeconds: number,
    durationSeconds: number,
  ) => {
    if (!state.session || state.session.role !== "host") return;
    sendPlaylist(withItemProgress(
      state.session.playlist,
      itemId,
      progressSeconds,
      durationSeconds,
    ), state.session.activePlaylistItemId);
  }, [sendPlaylist, state.session]);

  const leaveSession = useCallback((positionSeconds = 0, durationSeconds = 0) => {
    if (state.session?.role === "host") {
      const playlist = withItemProgress(
        state.session.playlist,
        state.session.activePlaylistItemId,
        positionSeconds,
        durationSeconds,
      );
      persistHostPlaylist(playlist, state.session.activePlaylistItemId);
      void invoke("stop_room_discovery");
    }
    socketRef.current?.disconnect();
    hostMediaPathsRef.current.clear();
    hostSubtitlePathsRef.current.clear();
    clockSamplesRef.current = [];
    savedPlaylistIdRef.current = null;
    setState({ session: null, localMediaPath: null, externalSubtitleSources: [], playback: initialPlayback, connectionState: "idle", ...initialClockState, notification: null, error: null });
  }, [persistHostPlaylist, state.session]);

  useEffect(() => () => {
    socketRef.current?.disconnect();
    if (notificationTimerRef.current !== null) window.clearTimeout(notificationTimerRef.current);
  }, []);

  useEffect(() => {
    if (state.connectionState !== "connected" || !state.session) return;
    const timer = window.setInterval(() => measureLatency(false), 2000);
    return () => window.clearInterval(timer);
  }, [measureLatency, state.connectionState, state.session?.roomCode]);

  return {
    ...state,
    createSession,
    createSessionFromSavedPlaylist,
    joinSession,
    sendPlaybackCommand,
    sendPlaybackRate,
    addPlaylistFiles,
    addExternalSubtitles,
    removePlaylistItem,
    movePlaylistItem,
    selectPlaylistItem,
    updatePlaylistProgress,
    syncClock,
    leaveSession,
  };
}
