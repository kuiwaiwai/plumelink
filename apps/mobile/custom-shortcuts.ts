import { keyboardKeys, keyboardModifiers } from "../../shared/keyboard";
import type { Command } from "../../shared/protocol";

export type ShortcutKey = typeof keyboardKeys[number];
export type ShortcutModifier = typeof keyboardModifiers[number];
export type KeyChord = { key: ShortcutKey; modifiers: ShortcutModifier[] };
export const keyGroups = [
  { value: "letters", label: "字母", keys: keyboardKeys.filter(key => /^[a-z]$/.test(key)) },
  { value: "numbers", label: "数字", keys: keyboardKeys.filter(key => /^\d$/.test(key)) },
  { value: "function", label: "功能键", keys: keyboardKeys.filter(key => /^f\d+$/.test(key)) },
  { value: "common", label: "常用按键", keys: keyboardKeys.filter(key => key.length > 1 && !/^f\d+$/.test(key)) },
] as const;
export function keyLabel(key: string) {
  return ({ space: "Space", left: "←", right: "→", up: "↑", down: "↓", enter: "Enter", escape: "Esc", backspace: "Backspace", tab: "Tab", delete: "Del", home: "Home", end: "End", pageup: "PgUp", pagedown: "PgDn" } as Record<string, string>)[key] || key.toUpperCase();
}
export function chordLabel(chord: KeyChord) {
  return [...keyboardModifiers.filter(modifier => chord.modifiers.includes(modifier)).map(modifier => modifier === "win" ? "Win" : modifier[0].toUpperCase() + modifier.slice(1)), keyLabel(chord.key)].join(" + ");
}
export function customShortcutValue(chord: KeyChord) {
  return "key:" + [...keyboardModifiers.filter(modifier => chord.modifiers.includes(modifier)), chord.key].join("+");
}
export function parseCustomShortcut(value: unknown): KeyChord | undefined {
  if (typeof value !== "string" || !value.startsWith("key:") || value.length > 48) return;
  const parts = value.slice(4).split("+");
  const key = parts.pop();
  if (!keyboardKeys.includes(key as ShortcutKey) || parts.length > 4 || new Set(parts).size !== parts.length || parts.some(part => !keyboardModifiers.includes(part as ShortcutModifier))) return;
  const result = { key: key as ShortcutKey, modifiers: parts as ShortcutModifier[] };
  return customShortcutValue(result) === value ? result : undefined;
}
export function chordCommand(chord: KeyChord): Command {
  return chord.modifiers.length ? { type: "combo", key: chord.key, modifiers: [...chord.modifiers] } : { type: "key", key: chord.key };
}
