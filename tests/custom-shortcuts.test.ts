import { it, expect } from "vitest";
import { ref } from "vue";
import { chordCommand, chordLabel, customShortcutValue, keyGroups, parseCustomShortcut } from "../apps/mobile/custom-shortcuts";
import { useCustomShortcut } from "../apps/mobile/composables/useCustomShortcut";
import { commandSchema, keyboardKeys, keyboardModifiers } from "../shared/protocol";
import { isShortcutValue, shortcutLabel, shortcutOptions } from "../apps/mobile/shortcuts";
import { defaultShortcutIcon } from "../apps/mobile/shortcut-icons";

it("roundtrips every supported key and modifier set through the command contract", () => {
  for (const key of keyboardKeys) for (let mask = 0; mask < 16; mask++) {
    const chord = { key, modifiers: keyboardModifiers.filter((_, index) => mask & (1 << index)) };
    expect(parseCustomShortcut(customShortcutValue(chord))).toEqual(chord);
    expect(commandSchema.safeParse(chordCommand(chord)).success).toBe(true);
  }
  expect(keyGroups.flatMap(group => group.keys).sort()).toEqual([...keyboardKeys].sort());
});
it("rejects malformed, duplicate and noncanonical persisted shortcuts", () => {
  for (const value of [null, {}, "key:", "key:win", "key:f99", "key:ctrl+ctrl+s", "key:shift+ctrl+s", "key:Ctrl+s", "key:ctrl+s+", "key:ctrl+shell", "custom-key"])
    expect(parseCustomShortcut(value)).toBeUndefined();
  expect(isShortcutValue("custom-key")).toBe(false);
});
it("keeps custom labels and icons alongside existing catalog choices", () => {
  expect(chordLabel({ modifiers: ["ctrl", "shift"], key: "s" })).toBe("Ctrl + Shift + S");
  expect(shortcutLabel("key:ctrl+s")).toBe("Ctrl + S");
  expect(defaultShortcutIcon("key:ctrl+s")).toBe("keyboard");
  expect(shortcutOptions("key:ctrl+s").filter(item => item.value === "key:ctrl+s")).toHaveLength(1);
  expect(isShortcutValue("copy")).toBe(true);
  expect(isShortcutValue("key:ctrl+s")).toBe(true);
});
it("keeps canceled drafts separate and changes exactly one slot on save", () => {
  const slots = [ref("taskview"), ref("notifications"), ref("nextapp")];
  const editor = useCustomShortcut(slots);
  editor.models[0].value = "custom-key";
  editor.draft.value.key = "p"; editor.toggle("shift"); editor.close();
  expect(slots.map(slot => slot.value)).toEqual(["taskview", "notifications", "nextapp"]);
  editor.models[1].value = "custom-key"; editor.save();
  expect(slots.map(slot => slot.value)).toEqual(["taskview", "key:ctrl+s", "nextapp"]);
  editor.models[1].value = "custom-key";
  expect(editor.draft.value).toEqual({ key: "s", modifiers: ["ctrl"] });
});
it("rejects duplicate slot assignments and the Windows secure-attention sequence", () => {
  const slots = [ref("key:ctrl+s"), ref("notifications"), ref("nextapp")];
  const editor = useCustomShortcut(slots);
  editor.models[1].value = "custom-key"; editor.save();
  expect(editor.editing.value).toBe(1); expect(editor.error.value).toBeTruthy();
  editor.draft.value = { key: "delete", modifiers: ["ctrl", "alt"] }; editor.save();
  expect(slots[1].value).toBe("notifications");
  expect(editor.editing.value).toBe(1); expect(editor.error.value).toBeTruthy();
  editor.draft.value = { key: "e", modifiers: ["win"] }; editor.save();
  expect(slots[1].value).toBe("key:win+e");
});
