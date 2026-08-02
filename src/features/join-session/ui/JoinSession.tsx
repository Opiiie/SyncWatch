import { useState } from "react";

import {
  discoverLocalRooms,
  type DiscoveredRoom,
} from "../../../shared/api/discovery";
import { Button } from "../../../shared/ui/Button/Button";
import "./JoinSession.css";

interface JoinSessionProps {
  disabled?: boolean;
  onJoin: (address: string, roomCode: string) => Promise<void>;
}

export function JoinSession({ disabled, onJoin }: JoinSessionProps) {
  const [roomCode, setRoomCode] = useState("");
  const [searchingByCode, setSearchingByCode] = useState(false);
  const [lookupError, setLookupError] = useState("");
  const [browserOpen, setBrowserOpen] = useState(false);
  const [browserLoading, setBrowserLoading] = useState(false);
  const [browserError, setBrowserError] = useState("");
  const [rooms, setRooms] = useState<DiscoveredRoom[]>([]);
  const normalizedCode = roomCode.trim().toUpperCase();
  const canJoinByCode = normalizedCode.length >= 4 && !disabled && !searchingByCode;

  async function joinByCode() {
    if (!canJoinByCode) return;
    setSearchingByCode(true);
    setLookupError("");
    try {
      const found = await discoverLocalRooms(normalizedCode);
      const room = found.find((candidate) => candidate.roomCode === normalizedCode);
      if (!room) {
        setLookupError("Комната с таким кодом не найдена в вашей сети.");
        return;
      }
      await onJoin(room.address, room.roomCode);
    } catch {
      setLookupError("Не удалось выполнить поиск. Проверьте подключение к общей сети.");
    } finally {
      setSearchingByCode(false);
    }
  }

  async function refreshBrowser() {
    setBrowserLoading(true);
    setBrowserError("");
    try {
      setRooms(await discoverLocalRooms());
    } catch {
      setRooms([]);
      setBrowserError("Не удалось обновить список комнат.");
    } finally {
      setBrowserLoading(false);
    }
  }

  function openBrowser() {
    setBrowserOpen(true);
    void refreshBrowser();
  }

  async function joinDiscoveredRoom(room: DiscoveredRoom) {
    setBrowserOpen(false);
    await onJoin(room.address, room.roomCode);
  }

  return (
    <section className="action-card" aria-labelledby="join-title">
      <span className="action-card__icon action-card__icon--pink" aria-hidden="true">↗</span>
      <h2 id="join-title">Присоединиться</h2>
      <p>Введите код от друга или найдите открытую комнату в общей сети.</p>

      <label className="room-input-label" htmlFor="room-code">Код комнаты</label>
      <input
        id="room-code"
        className="room-input"
        value={roomCode}
        onChange={(event) => {
          setRoomCode(event.target.value.slice(0, 12));
          setLookupError("");
        }}
        onKeyDown={(event) => event.key === "Enter" && void joinByCode()}
        placeholder="Например, 4K8P2W"
        maxLength={12}
      />
      {lookupError && <span className="paste-error" role="alert">{lookupError}</span>}
      <Button onClick={() => void joinByCode()} disabled={!canJoinByCode}>
        {searchingByCode ? "Ищем комнату…" : disabled ? "Подключение…" : "Подключиться по коду"}
      </Button>
      <Button variant="secondary" onClick={openBrowser} disabled={disabled || searchingByCode}>
        Найти в локальной сети
      </Button>

      {browserOpen && (
        <div className="room-browser-backdrop" onPointerDown={() => setBrowserOpen(false)}>
          <div
            className="room-browser"
            role="dialog"
            aria-modal="true"
            aria-labelledby="room-browser-title"
            onPointerDown={(event) => event.stopPropagation()}
          >
            <div className="room-browser__header">
              <div>
                <span>ОБЩАЯ СЕТЬ</span>
                <h2 id="room-browser-title">Доступные комнаты</h2>
              </div>
              <button
                className="room-browser__close"
                onClick={() => setBrowserOpen(false)}
                aria-label="Закрыть список комнат"
              >×</button>
            </div>

            <div className="room-browser__toolbar">
              <p>Здесь показаны комнаты в вашей локальной сети или сети Radmin VPN.</p>
              <Button
                variant="secondary"
                onClick={() => void refreshBrowser()}
                disabled={browserLoading}
              >
                {browserLoading ? "Обновляем…" : "Обновить"}
              </Button>
            </div>

            {browserError ? (
              <div className="room-browser__empty" role="alert">{browserError}</div>
            ) : rooms.length === 0 ? (
              <div className="room-browser__empty">
                {browserLoading ? "Ищем комнаты…" : "В этой сети пока не найдено комнат."}
              </div>
            ) : (
              <ul className="room-browser__list">
                {rooms.map((room) => (
                  <li key={`${room.roomCode}:${room.address}`}>
                    <div className="room-browser__room">
                      <strong>Комната {room.hostDisplayName}</strong>
                      <span>Код {room.roomCode} · участников: {room.participantCount}</span>
                      <small>{room.hasVideo ? "Видео уже добавлено" : "Плейлист пока пуст"}</small>
                    </div>
                    <Button onClick={() => void joinDiscoveredRoom(room)} disabled={disabled}>
                      Подключиться
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
