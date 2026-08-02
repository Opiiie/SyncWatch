import { useEffect, useState } from "react";

import { useSessionController } from "../entities/session/model/useSessionController";
import { HomePage } from "../pages/home/ui/HomePage";
import { SessionOverview } from "../widgets/session-overview/ui/SessionOverview";

interface ContextMenuPosition {
  x: number;
  y: number;
}

const BLOCKED_BROWSER_SHORTCUTS = new Set([
  "KeyF", "KeyG", "KeyH", "KeyJ", "KeyL", "KeyN", "KeyO", "KeyP", "KeyR", "KeyS",
  "KeyT", "KeyU", "KeyW", "Digit0", "Minus", "Equal", "NumpadAdd", "NumpadSubtract",
]);

function blocksBrowserAction(event: KeyboardEvent): boolean {
  if (["F1", "F3", "F5", "F6", "F7", "F11", "F12"].includes(event.code)) return true;
  if (event.altKey && ["ArrowLeft", "ArrowRight", "Home"].includes(event.code)) return true;
  return (event.ctrlKey || event.metaKey) && BLOCKED_BROWSER_SHORTCUTS.has(event.code);
}

export function App() {
  const controller = useSessionController();
  const [contextMenu, setContextMenu] = useState<ContextMenuPosition | null>(null);

  useEffect(() => {
    const handleContextMenu = (event: MouseEvent) => {
      event.preventDefault();
      setContextMenu({
        x: Math.min(event.clientX, Math.max(8, window.innerWidth - 230)),
        y: Math.min(event.clientY, Math.max(8, window.innerHeight - 190)),
      });
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!blocksBrowserAction(event)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
    };
    const closeMenu = () => setContextMenu(null);

    window.addEventListener("contextmenu", handleContextMenu);
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("pointerdown", closeMenu);
    window.addEventListener("blur", closeMenu);
    window.addEventListener("resize", closeMenu);
    return () => {
      window.removeEventListener("contextmenu", handleContextMenu);
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("pointerdown", closeMenu);
      window.removeEventListener("blur", closeMenu);
      window.removeEventListener("resize", closeMenu);
    };
  }, []);

  const page = controller.session
    ? (
      <SessionOverview
        session={controller.session}
        localMediaPath={controller.localMediaPath}
        externalSubtitleSources={controller.externalSubtitleSources}
        playback={controller.playback}
        connectionState={controller.connectionState}
        clockOffsetMs={controller.clockOffsetMs}
        clockLatencyMs={controller.clockLatencyMs}
        clockSyncState={controller.clockSyncState}
        notification={controller.notification}
        error={controller.error}
        onLeave={controller.leaveSession}
        onPlaybackCommand={controller.sendPlaybackCommand}
        onPlaybackRate={controller.sendPlaybackRate}
        onAddPlaylistFiles={controller.addPlaylistFiles}
        onAddExternalSubtitles={controller.addExternalSubtitles}
        onRemovePlaylistItem={controller.removePlaylistItem}
        onMovePlaylistItem={controller.movePlaylistItem}
        onSelectPlaylistItem={controller.selectPlaylistItem}
        onPlaylistProgress={controller.updatePlaylistProgress}
        onSyncClock={controller.syncClock}
      />
    )
    : (
      <HomePage
        error={controller.error}
        connectionState={controller.connectionState}
        onCreateSession={controller.createSession}
        onCreateSavedSession={controller.createSessionFromSavedPlaylist}
        onJoinSession={controller.joinSession}
      />
    );

  return (
    <>
      {page}
      {contextMenu && (
        <div
          className="app-context-menu"
          role="menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onContextMenu={(event) => event.preventDefault()}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <button role="menuitem" disabled>Воспроизведение</button>
          <button role="menuitem" disabled>О видео</button>
          <div className="app-context-menu__separator" />
          <button role="menuitem" disabled>Настройки приложения</button>
        </div>
      )}
    </>
  );
}
