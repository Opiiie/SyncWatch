import { useEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { PlaybackState } from "../../../shared/api/protocol";
import {
  addPlayerSubtitle,
  createPlayerSurface,
  destroyPlayer,
  getPlayerState,
  loadPlayerMedia,
  seekPlayer,
  setPlayerPaused,
  setPlayerSpeed,
  setPlayerSurfaceBounds,
  setPlayerVolume,
  type PlayerBounds,
  type NativePlayerState,
} from "../../../shared/api/player";
import type { PlayerSubtitleSource } from "../../../entities/session/model/types";

function surfaceBounds(element: HTMLDivElement): PlayerBounds {
  const rect = element.getBoundingClientRect();
  const scale = window.devicePixelRatio || 1;
  return {
    x: Math.round(rect.left * scale),
    y: Math.round(rect.top * scale),
    width: Math.max(1, Math.round(rect.width * scale)),
    height: Math.max(1, Math.round(rect.height * scale)),
  };
}

function playbackPosition(playback: PlaybackState, clockOffsetMs: number): number {
  if (!playback.playing) return playback.positionSeconds;
  return playback.positionSeconds
    + Math.max(0, Date.now() + clockOffsetMs - playback.updatedAtMs) / 1000
    * playback.playbackRate;
}

export function useNativePlayer(
  surfaceRef: RefObject<HTMLDivElement | null>,
  mediaPath: string | null,
  playback: PlaybackState,
  clockOffsetMs: number,
  volume: number,
  externalSubtitles: PlayerSubtitleSource[],
) {
  const [ready, setReady] = useState(false);
  const [loadedMediaPath, setLoadedMediaPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [state, setState] = useState<NativePlayerState>({
    positionSeconds: 0,
    durationSeconds: 0,
    speed: 1,
    audioTracks: [],
    subtitleTracks: [],
  });
  const loadedSubtitleIds = useRef(new Set<string>());
  const lastDriftCorrectionRef = useRef(0);

  useEffect(() => {
    const surface = surfaceRef.current;
    if (!surface || !mediaPath) return;

    setReady(false);
    setLoadedMediaPath(null);
    setError(null);
    loadedSubtitleIds.current.clear();
    setState({
      positionSeconds: 0,
      durationSeconds: 0,
      speed: 1,
      audioTracks: [],
      subtitleTracks: [],
    });

    let active = true;
    let resizeObserver: ResizeObserver | null = null;
    const windowUnlisteners: (() => void)[] = [];
    let frame = 0;

    const updateBounds = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        if (active) void setPlayerSurfaceBounds(surfaceBounds(surface));
      });
    };

    const startTimer = window.setTimeout(() => {
      void (async () => {
        try {
          await createPlayerSurface(surfaceBounds(surface));
          if (!active) {
            await destroyPlayer();
            return;
          }
          await loadPlayerMedia(mediaPath);
          await setPlayerVolume(volume);
          resizeObserver = new ResizeObserver(updateBounds);
          resizeObserver.observe(surface);
          window.addEventListener("resize", updateBounds);
          window.addEventListener("scroll", updateBounds, true);
          const appWindow = getCurrentWindow();
          const listeners = await Promise.all([
            appWindow.onMoved(updateBounds),
            appWindow.onResized(updateBounds),
            appWindow.onFocusChanged(updateBounds),
          ]);
          if (active) windowUnlisteners.push(...listeners);
          else listeners.forEach((unlisten) => unlisten());
          setReady(true);
          setLoadedMediaPath(mediaPath);
          setError(null);
        } catch (reason) {
          if (active) setError(reason instanceof Error ? reason.message : String(reason));
        }
      })();
    }, 0);

    return () => {
      active = false;
      window.clearTimeout(startTimer);
      window.cancelAnimationFrame(frame);
      resizeObserver?.disconnect();
      window.removeEventListener("resize", updateBounds);
      window.removeEventListener("scroll", updateBounds, true);
      windowUnlisteners.forEach((unlisten) => unlisten());
      setReady(false);
      setLoadedMediaPath(null);
      void destroyPlayer();
    };
  }, [mediaPath, surfaceRef]);

  useEffect(() => {
    if (!ready || loadedMediaPath !== mediaPath) return;
    let active = true;
    void (async () => {
      for (const subtitle of externalSubtitles) {
        if (!active || loadedSubtitleIds.current.has(subtitle.id)) continue;
        await addPlayerSubtitle(subtitle.source, subtitle.name, subtitle.language);
        loadedSubtitleIds.current.add(subtitle.id);
      }
    })().catch((reason: unknown) => {
      if (active) setError(reason instanceof Error ? reason.message : String(reason));
    });
    return () => { active = false; };
  }, [externalSubtitles, loadedMediaPath, mediaPath, ready]);

  useEffect(() => {
    if (!ready) return;
    void (async () => {
      try {
        await seekPlayer(playbackPosition(playback, clockOffsetMs));
        await setPlayerPaused(!playback.playing);
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason));
      }
    })();
  // A new clock sample alone must not cause an exact seek. Drift is corrected below
  // only when the actual player position has moved far enough from the room timeline.
  }, [playback.playing, playback.positionSeconds, playback.revision, ready]);

  useEffect(() => {
    if (!ready) return;
    void setPlayerVolume(volume).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [ready, volume]);

  useEffect(() => {
    if (!ready) return;
    void setPlayerSpeed(playback.playbackRate).catch((reason: unknown) => {
      setError(reason instanceof Error ? reason.message : String(reason));
    });
  }, [playback.playbackRate, playback.playing, ready]);

  useEffect(() => {
    if (!ready) return;
    let active = true;
    const update = async () => {
      try {
        const next = await getPlayerState();
        if (active) {
          setState(next);
          setError(null);
        }
        const now = performance.now();
        if (active && now - lastDriftCorrectionRef.current >= 2_000) {
          lastDriftCorrectionRef.current = now;
          const target = playbackPosition(playback, clockOffsetMs);
          const driftSeconds = target - next.positionSeconds;
          if (playback.playing && Math.abs(driftSeconds) >= 0.75) {
            await seekPlayer(target);
            await setPlayerSpeed(playback.playbackRate);
          } else if (playback.playing && Math.abs(driftSeconds) >= 0.12) {
            const correction = Math.max(-0.03, Math.min(0.03, driftSeconds * 0.04));
            const correctedSpeed = playback.playbackRate * (1 + correction);
            if (Math.abs(next.speed - correctedSpeed) >= 0.005) {
              await setPlayerSpeed(correctedSpeed);
            }
          } else if (Math.abs(next.speed - playback.playbackRate) >= 0.005) {
            await setPlayerSpeed(playback.playbackRate);
          }
        }
      } catch (reason) {
        if (active) setError(reason instanceof Error ? reason.message : String(reason));
      }
    };
    void update();
    const timer = window.setInterval(() => void update(), 250);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [clockOffsetMs, playback, ready]);

  return { ready, loadedMediaPath, error, state };
}
