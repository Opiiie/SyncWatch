import { invoke } from "@tauri-apps/api/core";

export interface DiscoveredRoom {
  roomCode: string;
  hostDisplayName: string;
  participantCount: number;
  hasVideo: boolean;
  address: string;
}

export function discoverLocalRooms(roomCode?: string): Promise<DiscoveredRoom[]> {
  return invoke("discover_local_rooms", {
    roomCode: roomCode?.trim().toUpperCase() || null,
  });
}
