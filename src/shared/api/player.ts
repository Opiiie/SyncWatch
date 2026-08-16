import { invoke } from "@tauri-apps/api/core";

export interface PlayerBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  clipX: number;
  clipY: number;
  clipWidth: number;
  clipHeight: number;
  cornerRadius: number;
}

export interface PlayerTrack {
  id: number;
  label: string;
  title: string | null;
  language: string | null;
  selected: boolean;
}

export interface NativePlayerState {
  positionSeconds: number;
  durationSeconds: number;
  speed: number;
  audioTracks: PlayerTrack[];
  subtitleTracks: PlayerTrack[];
}

export function createPlayerSurface(bounds: PlayerBounds): Promise<void> {
  return invoke("player_create_surface", { bounds });
}

export function setPlayerSurfaceBounds(bounds: PlayerBounds): Promise<void> {
  return invoke("player_set_surface_bounds", { bounds });
}

export function loadPlayerMedia(path: string): Promise<void> {
  return invoke("player_load", { path });
}

export function setPlayerPaused(paused: boolean): Promise<void> {
  return invoke("player_set_paused", { paused });
}

export function setPlayerVolume(volume: number): Promise<void> {
  return invoke("player_set_volume", { volume });
}

export function seekPlayer(positionSeconds: number): Promise<void> {
  return invoke("player_seek", { positionSeconds });
}

export function setPlayerSpeed(speed: number): Promise<void> {
  return invoke("player_set_speed", { speed });
}

export function getPlayerState(): Promise<NativePlayerState> {
  return invoke("player_get_state");
}

export function selectPlayerTrack(kind: "audio" | "subtitle", trackId: number | null): Promise<void> {
  return invoke("player_select_track", { kind, trackId });
}

export function addPlayerSubtitle(
  path: string,
  title: string,
  language: string | null,
): Promise<void> {
  return invoke("player_add_subtitle", { path, title, language });
}

export function destroyPlayer(): Promise<void> {
  return invoke("player_destroy");
}
