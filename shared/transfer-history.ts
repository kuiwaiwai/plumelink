import { t } from "./i18n";

export type TransferHistoryEntry = string | { kind: "received" | "sent"; name: string } | { kind: "batch"; count: number };

export function readTransferHistory(value: unknown): TransferHistoryEntry[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry): TransferHistoryEntry[] => {
    if (typeof entry === "string") return entry.length <= 512 ? [entry] : [];
    if (!entry || typeof entry !== "object") return [];
    if (entry.kind === "batch" && Number.isSafeInteger(entry.count) && entry.count >= 1 && entry.count <= 16)
      return [{ kind: "batch", count: entry.count }];
    if ((entry.kind === "received" || entry.kind === "sent") && typeof entry.name === "string"
      && entry.name.length > 0 && entry.name.length <= 512 && !/[\u0000-\u001f\u007f]/.test(entry.name))
      return [{ kind: entry.kind, name: entry.name }];
    return [];
  }).slice(0, 10);
}

export function nativeTransferHistory(value: { direction?: unknown; fileName?: unknown; fileCount?: unknown }, fallback: string): TransferHistoryEntry {
  const kind = value.direction === "receive" ? "received" : value.direction === "send" ? "sent" : "";
  const candidate = kind === "sent" && typeof value.fileCount === "number" && value.fileCount > 1
    ? { kind: "batch", count: value.fileCount } : { kind, name: value.fileName };
  return readTransferHistory([candidate])[0] ?? fallback;
}

export function formatTransferHistory(entry: TransferHistoryEntry): string {
  if (typeof entry === "string") return entry;
  if (entry.kind === "batch") return t("已发送 {0} 个文件", entry.count);
  return entry.kind === "received" ? t("已保存 {0}", entry.name) : t("发送：{0}", entry.name);
}
