import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import {
  ensureMpvRuntime,
  getMpvRuntimeStatus,
  installMpvRuntime,
  type MpvRuntimeStatus,
} from "../../../shared/api/mpvRuntime";
import "./MpvRuntimeNotice.css";

const INITIAL_STATUS: MpvRuntimeStatus = {
  stage: "checking",
  downloadedBytes: 0,
  totalBytes: 0,
  message: null,
};

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const megabytes = bytes / (1024 * 1024);
  return `${megabytes >= 10 ? megabytes.toFixed(0) : megabytes.toFixed(1)} МБ`;
}

export function MpvRuntimeNotice() {
  const [status, setStatus] = useState<MpvRuntimeStatus>(INITIAL_STATUS);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    let timer = 0;
    const refresh = async () => {
      try {
        const next = await getMpvRuntimeStatus();
        if (!active) return;
        setStatus(next);
        if (next.stage === "ready") window.clearInterval(timer);
      } catch {
        // The ensure command below reports a useful error if preparation fails.
      }
    };

    timer = window.setInterval(() => void refresh(), 300);
    void ensureMpvRuntime()
      .then((next) => {
        if (active) setStatus(next);
      })
      .catch(() => void refresh());
    void refresh();

    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, []);

  if (status.stage === "ready") return null;

  async function retry() {
    if (busy) return;
    setBusy(true);
    setStatus(INITIAL_STATUS);
    try {
      setStatus(await ensureMpvRuntime());
    } catch {
      setStatus(await getMpvRuntimeStatus());
    } finally {
      setBusy(false);
    }
  }

  async function chooseLibrary() {
    if (busy) return;
    const selected = await open({
      multiple: false,
      title: "Выберите libmpv-2.dll",
      filters: [{ name: "libmpv", extensions: ["dll"] }],
    });
    if (typeof selected !== "string") return;
    setBusy(true);
    setStatus(INITIAL_STATUS);
    try {
      setStatus(await installMpvRuntime(selected));
    } catch {
      setStatus(await getMpvRuntimeStatus());
    } finally {
      setBusy(false);
    }
  }

  const progress = status.totalBytes > 0
    ? Math.min(100, Math.round(status.downloadedBytes / status.totalBytes * 100))
    : 0;
  const subtitle = status.stage === "downloading"
    ? `Загрузка${status.totalBytes > 0 ? ` · ${progress}% из ${formatBytes(status.totalBytes)}` : "…"}`
    : status.stage === "error"
      ? status.message ?? "Не удалось подготовить компоненты плеера."
      : "Проверяем компоненты плеера…";

  return (
    <aside className="app-update mpv-runtime" role="status" aria-live="polite">
      <div className="app-update__mark" aria-hidden="true">▶</div>
      <div className="app-update__content">
        <strong>Подготовка плеера</strong>
        <span>{subtitle}</span>
        {status.stage === "downloading" && (
          <div className="app-update__progress" aria-label={`Загружено ${progress}%`}>
            <i style={{ width: `${progress}%` }} />
          </div>
        )}
      </div>
      {status.stage === "error" && (
        <>
          <button type="button" onClick={() => void retry()} disabled={busy}>Повторить</button>
          <button
            className="mpv-runtime__secondary"
            type="button"
            onClick={() => void chooseLibrary()}
            disabled={busy}
          >
            Выбрать DLL
          </button>
        </>
      )}
    </aside>
  );
}
