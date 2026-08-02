import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import type { LocalMediaFile } from "../../../entities/session/model/types";
import type { SavedPlaylist } from "../../../shared/lib/preferences";
import { normalizeMediaPath } from "../../../shared/lib/mediaPath";
import { Button } from "../../../shared/ui/Button/Button";
import "./CreateSession.css";

interface CreateSessionProps {
  disabled?: boolean;
  onCreate: (files: LocalMediaFile[]) => Promise<void>;
  savedPlaylists: SavedPlaylist[];
  onCreateSaved: (playlist: SavedPlaylist) => Promise<void>;
  onDeleteSaved: (id: string) => void;
}

function mediaFile(path: string): LocalMediaFile {
  return { path, name: path.split(/[\\/]/).pop() ?? path };
}

export function CreateSession({
  disabled,
  onCreate,
  savedPlaylists,
  onCreateSaved,
  onDeleteSaved,
}: CreateSessionProps) {
  const [files, setFiles] = useState<LocalMediaFile[]>([]);
  const [selectionError, setSelectionError] = useState("");

  async function selectVideos() {
    try {
      const selected = await open({
        multiple: true,
        directory: false,
        filters: [{ name: "Видео", extensions: ["mkv", "mp4", "webm", "avi", "mov", "m4v"] }],
      });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      const uniquePaths = paths.filter((path, index) => (
        paths.findIndex((candidate) => normalizeMediaPath(candidate) === normalizeMediaPath(path))
          === index
      ));
      setFiles(uniquePaths.map(mediaFile));
      setSelectionError("");
    } catch (error) {
      setSelectionError(error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <section className="action-card" aria-labelledby="host-title">
      <span className="action-card__icon" aria-hidden="true">▣</span>
      <h2 id="host-title">Создать комнату</h2>
      <p>Выберите видео сейчас или добавьте их в плейлист позже.</p>
      <Button variant="secondary" onClick={() => void selectVideos()} disabled={disabled}>
        Выбрать видео
      </Button>
      <div className={`file-name ${files.length ? "file-name--selected" : ""}`}>
        {files.length === 0
          ? "Видео пока не выбрано"
          : files.length === 1
            ? files[0].name
            : `Выбрано видео: ${files.length}`}
      </div>
      {selectionError && <span className="file-selection-error" role="alert">{selectionError}</span>}
      <Button onClick={() => void onCreate(files)} disabled={disabled}>
        {disabled ? "Подключение…" : "Создать комнату"}
      </Button>
      {savedPlaylists.length > 0 && (
        <div className="saved-playlists">
          <span>Предыдущие сессии</span>
          {savedPlaylists.map((playlist) => (
            <div className="saved-session" key={playlist.id}>
              <button
                className="saved-playlist"
                onClick={() => void onCreateSaved(playlist)}
                disabled={disabled}
                title={`Открыть: ${playlist.title}`}
              >
                <strong>{playlist.title}</strong>
                <small>
                  {playlist.items.length} видео · {new Date(playlist.updatedAt).toLocaleDateString("ru-RU")}
                </small>
              </button>
              <button
                className="saved-session__delete"
                onClick={() => onDeleteSaved(playlist.id)}
                disabled={disabled}
                aria-label={`Удалить сессию: ${playlist.title}`}
                title="Удалить из списка"
              >×</button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
