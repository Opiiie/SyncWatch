import { useEffect, useState } from "react";
import type { ConnectionState } from "../../../entities/session/model/types";
import { CreateSession } from "../../../features/create-session/ui/CreateSession";
import { JoinSession } from "../../../features/join-session/ui/JoinSession";
import {
  deleteSavedPlaylist,
  loadDisplayName,
  loadSavedPlaylists,
  saveDisplayName,
  type SavedPlaylist,
} from "../../../shared/lib/preferences";
import type { LocalMediaFile } from "../../../entities/session/model/types";

import "./HomePage.css";

interface HomePageProps {
  error: string | null;
  connectionState: ConnectionState;
  onCreateSession: (files: LocalMediaFile[], displayName: string) => Promise<void>;
  onCreateSavedSession: (playlist: SavedPlaylist, displayName: string) => Promise<void>;
  onJoinSession: (address: string, roomCode: string, displayName: string) => Promise<void>;
}

export function HomePage({ error, connectionState, onCreateSession, onCreateSavedSession, onJoinSession }: HomePageProps) {
  const isConnecting = connectionState === "connecting";
  const [displayName, setDisplayName] = useState(loadDisplayName);
  const [savedPlaylists, setSavedPlaylists] = useState(loadSavedPlaylists);
  function deleteSavedSession(id: string) {
    deleteSavedPlaylist(id);
    setSavedPlaylists((current) => current.filter((session) => session.id !== id));
  }
  useEffect(() => saveDisplayName(displayName), [displayName]);
  const hasDisplayName = Boolean(displayName.trim());
  return <main className="home-page"><section className="hero"><div className="home-toolbar"><span className="brand"><i>S</i> SyncWatch</span><label className="profile-name" htmlFor="display-name"><span>Ваше имя</span><input id="display-name" value={displayName} onChange={(event) => setDisplayName(event.target.value.slice(0, 32))} placeholder="Имя, которое увидят друзья" autoComplete="nickname" /></label></div><div className="hero__copy"><h1>Совместный просмотр</h1><p>Создайте свою комнату или подключитесь по приглашению от друга.</p></div></section>{error && <div className="connection-error" role="alert">{error}</div>}{!hasDisplayName && <div className="name-hint" role="status">Сначала укажите имя.</div>}<section className="actions"><CreateSession savedPlaylists={savedPlaylists} onCreate={(files) => onCreateSession(files, displayName)} onCreateSaved={(playlist) => onCreateSavedSession(playlist, displayName)} onDeleteSaved={deleteSavedSession} disabled={isConnecting || !hasDisplayName} /><JoinSession onJoin={(address, roomCode) => onJoinSession(address, roomCode, displayName)} disabled={isConnecting || !hasDisplayName} /></section><p className="footer-note">Для подключения компьютеры должны быть в одной сети. Если вы далеко друг от друга, понадобится программа для общей сети.</p></main>;
}
