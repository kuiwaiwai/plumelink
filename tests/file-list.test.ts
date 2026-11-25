import { expect, it } from "vitest";
import { browseFiles } from "../shared/file-list";

it("matches filenames across case, whitespace and Unicode normalization", () => {
  const files = [{ name: "Café-2.txt", size: 2 }, { name: "其他.txt", size: 1 }];
  expect(browseFiles(files, " CAFE\u0301 ", "default", "en")).toEqual([files[0]]);
  expect(browseFiles(files, "不存在", "default", "zh-CN")).toEqual([]);
});
it("sorts naturally and by size without mutating the original order", () => {
  const files = [{ name: "file10.txt", size: 10 }, { name: "file2.txt", size: 2 }, { name: "file1.txt", size: 10 }];
  expect(browseFiles(files, "", "name", "en").map(f => f.name)).toEqual(["file1.txt", "file2.txt", "file10.txt"]);
  expect(browseFiles(files, "", "largest", "en").map(f => f.name)).toEqual(["file1.txt", "file10.txt", "file2.txt"]);
  expect(browseFiles(files, "", "default", "en")).toEqual(files);
  expect(files[0].name).toBe("file10.txt");
});
