import { t } from "../../shared/i18n";
export const MAX_FILE_BYTES = 2 * 1024 ** 3;
export function isSafeFileName(name: unknown): name is string {
  return typeof name === "string" && name.length > 0 && new TextEncoder().encode(name).length <= 240 && name !== "." && name !== ".." &&
    !name.endsWith(".") && !name.endsWith(" ") && !name.startsWith(".plumelink-") &&
    ![...name].some(char => char.charCodeAt(0) < 0x20 || char === "\u007f" || "\\/:<>\"|?*".includes(char)) &&
    !isWindowsDeviceName(name);
}
type Progress = {
  offset: number;
  size: number;
  chunkBytes: number;
  complete: boolean;
  name: string | null;
};
type Options = {
  token: string;
  signal: AbortSignal;
  valid: () => boolean;
  progress: (bytes: number, retrying: boolean) => void;
  realtime?: () => boolean;
  latency?: () => number | null;
};
const delay = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    const abort = () => {
      clearTimeout(timer);
      reject(new Error(t('已取消传输')));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", abort);
      resolve();
    }, ms);
    if (signal.aborted) abort();
    else signal.addEventListener("abort", abort, { once: true });
  });
class PermanentFailure extends Error {}
export async function uploadResumable(
  file: File,
  options: Options,
): Promise<string> {
  if (!Number.isSafeInteger(file.size) || file.size < 0 || file.size > MAX_FILE_BYTES)
    throw new Error(t('{0} 大小无效或超过 2 GB', file.name));
  if (!isSafeFileName(file.name))
    throw new Error(t('文件名无效，请重命名后重试'));
  const id = Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, "0")).join("");
  const path = `/api/uploads/${id}`;
  let completed = false;
  const check = () => {
    if (options.signal.aborted || !options.valid())
      throw new PermanentFailure(t('传输已取消或电脑已变化'));
  };
  const request = async (url: string, init: RequestInit): Promise<Progress> => {
    check();
    const controller = new AbortController();
    const abort = () => controller.abort();
    options.signal.addEventListener("abort", abort, { once: true });
    const timer = setTimeout(abort, 90_000);
    try {
      const response = await fetch(url, {
        ...init,
        signal: controller.signal,
        headers: { Authorization: `Bearer ${options.token}`, ...init.headers },
      });
      const data = await response.json();
      check();
      if (!response.ok) {
        if ([400, 401, 403, 404, 413].includes(response.status))
          throw new PermanentFailure(data.error || t('文件传输被拒绝'));
        throw new Error(data.error || t('网络暂时不可用'));
      }
      if (
        !Number.isSafeInteger(data.offset) ||
        data.offset < 0 ||
        data.offset > file.size ||
        data.size !== file.size ||
        data.chunkBytes !== 4 * 1024 ** 2
      )
        throw new PermanentFailure(t('电脑返回的传输位置无效'));
      return data;
    } finally {
      clearTimeout(timer);
      options.signal.removeEventListener("abort", abort);
    }
  };
  let progress: Progress | undefined;
  try {
    for (let attempt = 0; !progress; attempt++) {
      try {
        progress = await request("/api/uploads", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ id, name: file.name, size: file.size }),
        });
      } catch (error) {
        if (error instanceof PermanentFailure || attempt >= 3) throw error;
        await delay(500 * 2 ** attempt, options.signal);
      }
    }
    let failures = 0;
    while (!progress.complete) {
      check();
      options.progress(progress.offset, false);
      try {
        if (progress.offset === file.size)
          progress = await request(path, { method: "POST" });
        else {
          // Small requests avoid queuing megabytes ahead of live audio on the phone's uplink.
          const realtime = options.realtime?.() === true;
          const began = performance.now();
          const bytes = await file
            .slice(
              progress.offset,
              progress.offset + (realtime ? 64 * 1024 : progress.chunkBytes),
            )
            .arrayBuffer();
          const digest = [
            ...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
          ]
            .map((v) => v.toString(16).padStart(2, "0"))
            .join("");
          progress = await request(path, {
            method: "PUT",
            headers: {
              "Content-Type": "application/octet-stream",
              "X-Upload-Offset": String(progress.offset),
              "X-Chunk-SHA256": digest,
            },
            body: bytes,
          });
          if (realtime) {
            const rate =
              (options.latency?.() ?? 0) > 100 ? 256 * 1024 : 1024 * 1024;
            const remaining =
              (bytes.byteLength / rate) * 1000 - (performance.now() - began);
            if (remaining > 0) await delay(remaining, options.signal);
          }
        }
        failures = 0;
      } catch (error) {
        if (error instanceof PermanentFailure) throw error;
        let recovered = false;
        while (!recovered && failures < 5) {
          options.progress(progress.offset, true);
          await delay(Math.min(4000, 500 * 2 ** failures++), options.signal);
          try {
            progress = await request(path, { method: "GET" });
            recovered = true;
          } catch (failure) {
            if (failure instanceof PermanentFailure) throw failure;
          }
        }
        if (!recovered)
          throw new Error(t('网络持续中断，传输已停止，请恢复网络后重新选择文件'));
      }
    }
    if (!progress.name || progress.offset !== file.size)
      throw new Error(t('文件完成确认无效'));
    completed = true;
    options.progress(file.size, false);
    return progress.name;
  } finally {
    // A cancelled transfer cannot leave a partial file occupying a slot indefinitely.
    // If cleanup cannot reach the server, its bounded retention timer removes it.
    if (!completed) {
      const cleanup = new AbortController(),
        timer = setTimeout(() => cleanup.abort(), 3000);
      void fetch(path, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${options.token}` },
        signal: cleanup.signal,
      })
        .catch(() => {})
        .finally(() => clearTimeout(timer));
    }
  }
}
function isWindowsDeviceName(name: string) {
  const stem = name.split(".", 1)[0].trim().toUpperCase();
  if (["CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$"].includes(stem)) return true;
  return /^(COM|LPT)[1-9]$/.test(stem) || ["COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"].includes(stem);
}
