export function createRoomCode(): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  return Array.from(
    { length: 6 },
    () => alphabet[Math.floor(Math.random() * alphabet.length)],
  ).join("");
}

export interface RoomInvitation {
  address: string;
  roomCode: string;
}

export function parseRoomInvitation(value: string): RoomInvitation | null {
  const match = value.trim().match(/^(.+?)\s+\/\s+([a-z\d]{4,12})$/i);
  if (!match) return null;

  return { address: match[1].trim(), roomCode: match[2].toUpperCase() };
}

export function formatDuration(seconds: number): string {
  const safeSeconds = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const hours = Math.floor(safeSeconds / 3600);
  const minutes = Math.floor((safeSeconds % 3600) / 60);
  const remainingSeconds = Math.floor(safeSeconds % 60);
  const minuteText = hours > 0 ? minutes.toString().padStart(2, "0") : String(minutes);
  return `${hours > 0 ? `${hours}:` : ""}${minuteText}:${remainingSeconds.toString().padStart(2, "0")}`;
}
