import { invoke } from "@tauri-apps/api/core";

export type MpvRuntimeStage = "checking" | "downloading" | "ready" | "error";

export interface MpvRuntimeStatus {
  stage: MpvRuntimeStage;
  downloadedBytes: number;
  totalBytes: number;
  message: string | null;
}

export function ensureMpvRuntime(): Promise<MpvRuntimeStatus> {
  return invoke("ensure_mpv_runtime");
}

export function getMpvRuntimeStatus(): Promise<MpvRuntimeStatus> {
  return invoke("get_mpv_runtime_status");
}

export function installMpvRuntime(path: string): Promise<MpvRuntimeStatus> {
  return invoke("install_mpv_runtime", { path });
}
