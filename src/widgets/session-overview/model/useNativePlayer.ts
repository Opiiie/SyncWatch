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
  const radiusSource = element.parentElement ?? element;
  const cornerRadius = Number.parseFloat(
    window.getComputedStyle(radiusSource).borderTopLeftRadius,
  ) || 0;
  let visibleLeft = Math.max(0, rect.left);
  let visibleTop = Math.max(0, rect.top);
  let visibleRight = Math.min(document.documentElement.clientWidth, rect.right);
  let visibleBottom = Math.min(document.documentElement.clientHeight, rect.bottom);

  for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
    const style = window.getComputedStyle(ancestor);
    const clipsX = /(auto|scroll|hidden|clip)/.test(style.overflowX);
    const clipsY = /(auto|scroll|hidden|clip)/.test(style.overflowY);
    if (!clipsX && !clipsY) continue;
    const ancestorRect = ancestor.getBoundingClientRect();
    const clientLeft = ancestorRect.left + ancestor.clientLeft;
    const clientTop = ancestorRect.top + ancestor.clientTop;
    if (clipsX) {
      visibleLeft = Math.max(visibleLeft, clientLeft);
      visibleRight = Math.min(visibleRight, clientLeft + ancestor.clientWidth);
    }
    if (clipsY) {
      visibleTop = Math.max(visibleTop, clientTop);
      visibleBottom = Math.min(visibleBottom, clientTop + ancestor.clientHeight);
    }
  }

  const left = Math.floor(rect.left * scale);
  const top = Math.floor(rect.top * scale);
  const right = Math.ceil(rect.right * scale);
  const bottom = Math.ceil(rect.bottom * scale);
  const clipLeft = Math.max(left, Math.ceil(visibleLeft * scale));
  const clipTop = Math.max(top, Math.ceil(visibleTop * scale));
  const clipRight = Math.min(right, Math.floor(visibleRight * scale));
  const clipBottom = Math.min(bottom, Math.floor(visibleBottom * scale));
  return {
    x: left,
    y: top,
    width: Math.max(1, right - left),
    height: Math.max(1, bottom - top),
    clipX: Math.max(0, clipLeft - left),
    clipY: Math.max(0, clipTop - top),
    clipWidth: Math.max(0, clipRight - clipLeft),
    clipHeight: Math.max(0, clipBottom - clipTop),
    cornerRadius: Math.max(0, Math.round(cornerRadius * scale)),
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
    let framesRemaining = 0;
    let pendingBounds: PlayerBounds | null = null;
    let sendingBounds = false;

    const flushBounds = async () => {
      if (sendingBounds) return;
      sendingBounds = true;
      try {
        while (active && pendingBounds) {
          const next = pendingBounds;
          pendingBounds = null;
          await setPlayerSurfaceBounds(next);
        }
      } finally {
        sendingBounds = false;
      }
    };

    const captureBounds = () => {
      frame = 0;
      if (!active) return;
      pendingBounds = surfaceBounds(surface);
      void flushBounds().catch((reason: unknown) => {
        if (active) setError(reason instanceof Error ? reason.message : String(reason));
      });
      if (framesRemaining > 0) {
        framesRemaining -= 1;
        frame = window.requestAnimationFrame(captureBounds);
      }
    };

    const updateBounds = () => {
      framesRemaining = 8;
      if (!frame) frame = window.requestAnimationFrame(captureBounds);
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
          window.visualViewport?.addEventListener("resize", updateBounds);
          window.visualViewport?.addEventListener("scroll", updateBounds);
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
      window.visualViewport?.removeEventListener("resize", updateBounds);
      window.visualViewport?.removeEventListener("scroll", updateBounds);
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
