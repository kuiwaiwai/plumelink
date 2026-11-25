import { afterEach, expect, it } from "vitest";
import { languagePreference } from "../shared/i18n";
import { formatTransferHistory, nativeTransferHistory, readTransferHistory } from "../shared/transfer-history";

afterEach(() => { languagePreference.value = "system"; });

it("bounds saved history and rejects invalid record fields without mutating its input", () => {
  const value = [{ kind: "received", name: "report.bin", path: "private detail" }, { kind: "batch", count: 2 },
    { kind: "batch", count: 17 }, { kind: "sent", name: "bad\nname" }, null, {},
    ...Array.from({ length: 20 }, (_, i) => "old record " + i)];
  const records = readTransferHistory(value);
  expect(records).toHaveLength(10);
  expect(records.slice(0, 2)).toEqual([{ kind: "received", name: "report.bin" }, { kind: "batch", count: 2 }]);
  expect(value[0]).toHaveProperty("path");
  expect(readTransferHistory({})).toEqual([]);
});

it("keeps ambiguous filenames intact while translating structured results", () => {
  for (const language of ["en", "zh-TW", "zh-CN"]) {
    languagePreference.value = language;
    for (const name of ["Report to PC", "2 files", "报告 Saved notes {0}.bin"]) {
      expect(formatTransferHistory({ kind: "received", name })).toBe((language === "en" ? "Saved " : "已保存 ") + name);
      expect(formatTransferHistory({ kind: "sent", name })).toBe((language === "en" ? "Sent: " : language === "zh-TW" ? "發送：" : "发送：") + name);
    }
    expect(formatTransferHistory({ kind: "batch", count: 2 })).toBe(language === "en" ? "Sent 2 files" : language === "zh-TW" ? "已發送 2 個檔案" : "已发送 2 个文件");
    expect(formatTransferHistory("Saved Report to PC")).toBe("Saved Report to PC");
  }
});

it("uses native result metadata and preserves unknown historical messages", () => {
  expect(nativeTransferHistory({ direction: "receive", fileName: "2 files", fileCount: 1 }, "Saved 2 files"))
    .toEqual({ kind: "received", name: "2 files" });
  expect(nativeTransferHistory({ direction: "send", fileName: "last.bin", fileCount: 2 }, "Sent 2 files"))
    .toEqual({ kind: "batch", count: 2 });
  expect(nativeTransferHistory({ direction: "send", fileCount: 100 }, "unknown result")).toBe("unknown result");
  expect(nativeTransferHistory({}, "Saved old.bin")).toBe("Saved old.bin");
});
