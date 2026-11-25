import { expect, it } from "vitest";
import { browseChoices } from "../shared/choice-list";

const options = [
  { value: "copy", label: "Copy", description: "Ctrl + C", group: "Editing" },
  { value: "paste", label: "Paste", description: "Ctrl + V", group: "Editing" },
  { value: "next", label: "Next track", group: "Media" },
];
it("keeps option identity, category order and the input array unchanged", () => {
  const snapshot = structuredClone(options);
  const groups = browseChoices(options, "");
  expect(groups.map(group => group.label)).toEqual(["Editing", "Media"]);
  expect(groups[0].options[0]).toBe(options[0]);
  expect(options).toEqual(snapshot);
});
it("finds an action by its shortcut with fullwidth input, whitespace and either plus notation", () => {
  for (const query of ["ctrl v", "Ｃｔｒｌ＋Ｖ", "  V  ctrl  "])
    expect(browseChoices(options, query).flatMap(group => group.options).map(item => item.value)).toEqual(["paste"]);
  expect(browseChoices(options, "ctrl c").flatMap(group => group.options).map(item => item.value)).toEqual(["copy"]);
});
it("finds a whole category and removes groups with no matching options", () => {
  expect(browseChoices(options, "editing")).toEqual([{ label: "Editing", options: options.slice(0, 2) }]);
  expect(browseChoices(options, "missing")).toEqual([]);
  expect(browseChoices([], "")).toEqual([]);
});
it("matches composed accents and localized labels", () => {
  expect(browseChoices([{ value: "cafe", label: "Café" }], "CAFE\u0301")[0].options[0].value).toBe("cafe");
  expect(browseChoices([{ value: "copy", label: "複製", group: "文字編輯" }], "複製", "zh-TW")[0].label).toBe("文字編輯");
});
