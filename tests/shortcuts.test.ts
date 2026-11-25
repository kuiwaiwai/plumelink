import { describe, expect, it } from "vitest";
import { shortcuts, shortcutPresets, shortcutOptions, shortcutGroups } from "../apps/mobile/shortcuts";
import { shortcutIconNames, shortcutIconLabels } from "../apps/mobile/shortcut-icons";
import { commandSchema } from "../shared/protocol";
import english from "../shared/locales/en.json";
import traditional from "../shared/locales/zh-Hant.json";

describe("shortcut catalog", () => {
  it("sends only supported commands and leaves built-in more actions out of configurable slots", () => {
    expect(new Set(shortcuts.map(item => item.value)).size).toBe(shortcuts.length);
    for (const shortcut of shortcuts) {
      if (shortcut.command) expect(commandSchema.safeParse(shortcut.command).success).toBe(true);
      else expect(["magnifier", "applications"]).toContain(shortcut.value);
      expect(["desktop", "escape", "backspace", "windows", "scrollup", "scrolldown"]).not.toContain(shortcut.value);
      expect(english).toHaveProperty(shortcut.label);
      expect(traditional).toHaveProperty(shortcut.label);
    }
  });
  it("keeps every preset at three distinct, available actions", () => {
    for (const preset of shortcutPresets) {
      expect(preset.actions).toHaveLength(3);
      expect(new Set(preset.actions).size).toBe(3);
      for (const action of preset.actions) expect(shortcuts.some(item => item.value === action)).toBe(true);
    }
  });
  it("uses previous-tab and redo combinations without reversing their direction", () => {
    expect(shortcuts.find(item => item.value === "previoustab")?.command).toEqual({ type: "combo", modifiers: ["ctrl", "shift"], key: "tab" });
    expect(shortcuts.find(item => item.value === "redo")?.command).toEqual({ type: "combo", modifiers: ["ctrl"], key: "y" });
  });
  it("uses presentation navigation and blackout without restarting the slideshow", () => {
    const command = (value: string) => shortcuts.find(item => item.value === value)?.command;
    expect(shortcutPresets.find(item => item.value === "slides")?.actions).toEqual(["slideprevious", "slideblack", "slidenext"]);
    expect(command("slideprevious")).toEqual({ type: "key", key: "pageup" });
    expect(command("slidenext")).toEqual({ type: "key", key: "pagedown" });
    expect(command("slideblack")).toEqual({ type: "key", key: "b" });
    expect(command("slidecurrent")).toEqual({ type: "combo", modifiers: ["shift"], key: "f5" });
    expect(command("presentation")).toEqual({ type: "key", key: "f5" });
    expect(shortcuts.find(item => item.value === "fullscreen")?.category).toBe("browser");
  });
});

it("provides an icon and translated accessible name for every configurable shortcut", () => {
  for (const shortcut of shortcuts) expect(shortcutIconNames).toContain(shortcut.icon);
  for (const id of shortcutIconNames) {
    expect(shortcutIconLabels[id]).toBeTruthy();
    expect(english).toHaveProperty(shortcutIconLabels[id]);
    expect(traditional).toHaveProperty(shortcutIconLabels[id]);
  }
});

it("exposes every shortcut once in a translated category with its icon", () => {
  const options = shortcutOptions();
  expect(options).toHaveLength(shortcuts.length);
  expect(new Set(options.map(item => item.value)).size).toBe(shortcuts.length);
  for (const option of options) {
    expect(option.group).toBeTruthy();
    expect(shortcutIconNames).toContain(option.icon);
  }
  for (const category of shortcutGroups) {
    expect(english).toHaveProperty(category.label);
    expect(traditional).toHaveProperty(category.label);
  }
});
