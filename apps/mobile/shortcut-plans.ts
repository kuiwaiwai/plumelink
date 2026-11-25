import { isShortcutValue, shortcutPresets } from "./shortcuts";
import { defaultShortcutIcon, shortcutIconNames } from "./shortcut-icons";

export type ShortcutSlot = { action: string; icon: string };
export type ShortcutPlan = { id: string; name: string; shortcuts: ShortcutSlot[] };
export const maxShortcutPlans = 8;
export const maxPlanName = 32;
export function planName(value: string) { return value.normalize("NFC").trim(); }
export function validPlanName(value: unknown): value is string {
  return typeof value === "string" && value === planName(value) && [...value].length > 0 && [...value].length <= maxPlanName
    && !/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/.test(value);
}
export function validShortcutSlots(value: unknown): value is ShortcutSlot[] {
  return Array.isArray(value) && value.length === 3 && value.every(item => item && typeof item === "object"
    && Object.keys(item).length === 2 && isShortcutValue(item.action) && shortcutIconNames.includes(item.icon))
    && new Set(value.map(item => item.action)).size === 3;
}
export function parseShortcutPlans(value: unknown): ShortcutPlan[] | undefined {
  if (!Array.isArray(value) || value.length > maxShortcutPlans) return;
  if (!value.every(item => item && typeof item === "object" && Object.keys(item).length === 3
    && /^p[a-f0-9]{32}$/.test(item.id) && validPlanName(item.name) && validShortcutSlots(item.shortcuts))) return;
  if (new Set(value.map(item => item.id)).size !== value.length || new Set(value.map(item => item.name.toLowerCase())).size !== value.length) return;
  return value.map(item => ({ id: item.id, name: item.name, shortcuts: cloneSlots(item.shortcuts) }));
}
export function cloneSlots(slots: readonly ShortcutSlot[]): ShortcutSlot[] { return slots.map(slot => ({ ...slot })); }
export function matchingSlots(left: readonly ShortcutSlot[], right: readonly ShortcutSlot[]) {
  return left.length === right.length && left.every((slot, index) => slot.action === right[index].action && slot.icon === right[index].icon);
}
export function builtinPlan(value: string) {
  const preset = shortcutPresets.find(item => "builtin:" + item.value === value);
  return preset && { id: value, label: preset.label, shortcuts: preset.actions.map(action => ({ action, icon: defaultShortcutIcon(action) })) };
}
