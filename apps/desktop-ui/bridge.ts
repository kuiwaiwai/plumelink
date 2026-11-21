import { t } from "../../shared/i18n";
import { invoke } from "@tauri-apps/api/core";
import type { DesktopState } from "../../shared/protocol";
export const DEFAULT_SERVICE_PORT = 17840;
let revision = 0;
let stateRequest = 0;
const INVOKE_TIMEOUT_MS = 15_000;
class StaleStateError extends Error { stale = true; }
async function invokeWithTimeout<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      invoke<T>(command, args),
      new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error(t('电脑服务响应超时，请稍后重试'))), INVOKE_TIMEOUT_MS);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}
async function mutate(command: string, args: Record<string, unknown>, waitForApproval = false) {
  revision++;
  stateRequest++;
  try {
    // Windows owns the UAC prompt. A frontend timeout cannot cancel that request.
    return waitForApproval
      ? await invoke<DesktopState>(command, args)
      : await invokeWithTimeout<DesktopState>(command, args);
  }
  finally { revision++; }
}
async function readState() {
  const request = ++stateRequest;
  const value = await invokeWithTimeout<DesktopState>("desktop_state");
  if (request !== stateRequest) throw new StaleStateError(t('状态请求已过期'));
  return value;
}
export const desktop = {
  state: readState,
  setPort: (port: number, address: string) => mutate("set_service_port", { port, address }),
  action: (action: string, address?: string) =>
    mutate("desktop_action", { action, address }),
  transfer: (action: string, id?: number, ids?: number[]) =>
    mutate("transfer_action", { action, id, ids }),
  sharePaths: (paths: string[]) =>
    mutate("transfer_action", { action: "addpaths", paths }),
  device: (action: string, id: string, name?: string, permissions?: DesktopState["devices"][number]["permissions"]) =>
    mutate("device_action", { action, id, name, permissions }),
  admin: (action: "start" | "stop") =>
    mutate("admin_action", { action }, action === "start"),
  diagnostic: () => invokeWithTimeout<{ pointer: { x: number; y: number }; volume: number; muted: boolean; altHeld: boolean; magnifier: boolean }>("diagnostic_action"),
  openLogs: () => invokeWithTimeout<string>("open_logs_dir"),
  subscribe: (callback: (state: DesktopState) => void, onError: (error: unknown) => void = () => {}) => {
    let closed = false,
      busy = false,
      pending = false;
    const poll = async () => {
      if (closed) return;
      if (busy) { pending = true; return; }
      if (typeof document !== "undefined" && document.hidden) return;
      busy = true;
      pending = false;
      const requestedAt = revision;
      try {
        const state = await desktop.state();
        if (!closed && requestedAt === revision) callback(state);
      } catch (error) {
        if (!closed && requestedAt === revision && !(error as { stale?: boolean })?.stale) onError(error);
      } finally {
        busy = false;
        if (pending && !closed) queueMicrotask(() => { void poll(); });
      }
    };
    const timer = setInterval(() => {
      void poll();
    }, 1000);
    const onVisible = () => {
      if (typeof document !== "undefined" && !document.hidden) {
        void poll();
      }
    };
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVisible);
    }
    return () => {
      closed = true;
      clearInterval(timer);
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", onVisible);
      }
    };
  },
};
