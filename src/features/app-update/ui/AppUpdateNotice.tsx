import { useEffect, useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";

import "./AppUpdateNotice.css";

interface AppUpdateNoticeProps {
  sessionActive: boolean;
}

type UpdateStage = "available" | "downloading" | "restarting" | "error";

let automaticCheck: Promise<Update | null> | null = null;

function checkAfterStartup(): Promise<Update | null> {
  if (!automaticCheck) {
    automaticCheck = new Promise((resolve) => window.setTimeout(resolve, 2500)).then(() => check());
  }
  return automaticCheck;
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const megabytes = bytes / (1024 * 1024);
  return `${megabytes >= 10 ? megabytes.toFixed(0) : megabytes.toFixed(1)} МБ`;
}

export function AppUpdateNotice({ sessionActive }: AppUpdateNoticeProps) {
  const [update, setUpdate] = useState<Update | null>(null);
  const [stage, setStage] = useState<UpdateStage>("available");
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(0);

  useEffect(() => {
    let active = true;
    void checkAfterStartup()
      .then((availableUpdate) => {
        if (active && availableUpdate) setUpdate(availableUpdate);
      })
      .catch(() => {
        // A missing release or a temporary network problem should not disturb startup.
      });
    return () => { active = false; };
  }, []);

  if (!update) return null;

  async function installUpdate() {
    if (sessionActive || stage === "downloading" || stage === "restarting") return;
    setStage("downloading");
    setDownloaded(0);
    setTotal(0);
    try {
      await update!.downloadAndInstall((event) => {
        if (event.event === "Started") {
          setTotal(event.data.contentLength ?? 0);
        } else if (event.event === "Progress") {
          setDownloaded((current) => current + event.data.chunkLength);
        }
      });
      setStage("restarting");
      await relaunch();
    } catch {
      setStage("error");
    }
  }

  const progress = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0;
  const subtitle = sessionActive
    ? "Установить обновление можно после выхода из комнаты."
    : stage === "downloading"
      ? `Загрузка${total > 0 ? ` · ${progress}% из ${formatBytes(total)}` : "…"}`
      : stage === "restarting"
        ? "Обновление готово. Перезапускаем приложение…"
        : stage === "error"
          ? "Не удалось скачать обновление. Можно попробовать ещё раз."
          : "Можно установить сейчас — приложение перезапустится.";

  return (
    <aside className="app-update" role="status" aria-live="polite">
      <div className="app-update__mark" aria-hidden="true">↑</div>
      <div className="app-update__content">
        <strong>Доступна версия {update.version}</strong>
        <span>{subtitle}</span>
        {stage === "downloading" && (
          <div className="app-update__progress" aria-label={`Загружено ${progress}%`}>
            <i style={{ width: `${progress}%` }} />
          </div>
        )}
      </div>
      <button
        type="button"
        onClick={() => void installUpdate()}
        disabled={sessionActive || stage === "downloading" || stage === "restarting"}
      >
        {stage === "error" ? "Повторить" : "Обновить"}
      </button>
      {stage !== "downloading" && stage !== "restarting" && (
        <button className="app-update__dismiss" type="button" aria-label="Скрыть" onClick={() => setUpdate(null)}>×</button>
      )}
    </aside>
  );
}
