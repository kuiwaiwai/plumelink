import type { Command } from "../../shared/protocol";
import { t } from "../../shared/i18n";
import { parseCustomShortcut, chordLabel } from "./custom-shortcuts";

type Category = "daily" | "editing" | "browser" | "media" | "slides";
type Shortcut = { category: Category; value: string; label: Parameters<typeof t>[0]; icon: string; command?: Command };

export const shortcuts: readonly Shortcut[] = [
  { category: "media", value: "mediaplay", label: "播放/暂停", icon: "play", command: { type: "media", action: "playpause" } },
  { category: "media", value: "mediaprevious", label: "上一首", icon: "previous", command: { type: "media", action: "previous" } },
  { category: "media", value: "medianext", label: "下一首", icon: "nexttrack", command: { type: "media", action: "next" } },
  { category: "media", value: "mediastop", label: "停止播放", icon: "stop", command: { type: "media", action: "stop" } },
  { category: "daily", value: "applications", label: "快捷启动", icon: "monitor" },
  { category: "media", value: "mute", label: "静音", icon: "volume", command: { type: "volume", action: "mute" } },
  { category: "daily", value: "magnifier", label: "鼠标放大镜", icon: "search" },
  { category: "daily", value: "taskview", label: "任务视图", icon: "grid", command: { type: "taskview" } },
  { category: "daily", value: "notifications", label: "通知中心", icon: "bell", command: { type: "notifications" } },
  { category: "daily", value: "previousapp", label: "上一个应用", icon: "left", command: { type: "appswitch", direction: "left" } },
  { category: "daily", value: "nextapp", label: "下一个应用", icon: "right", command: { type: "appswitch", direction: "right" } },
  { category: "daily", value: "virtualleft", label: "上一个桌面", icon: "layers", command: { type: "virtualdesktop", direction: "left" } },
  { category: "daily", value: "virtualright", label: "下一个桌面", icon: "layers", command: { type: "virtualdesktop", direction: "right" } },
  { category: "editing", value: "copy", label: "复制", icon: "layers", command: { type: "combo", modifiers: ["ctrl"], key: "c" } },
  { category: "editing", value: "paste", label: "粘贴", icon: "keyboard", command: { type: "combo", modifiers: ["ctrl"], key: "v" } },
  { category: "editing", value: "cut", label: "剪切", icon: "command", command: { type: "combo", modifiers: ["ctrl"], key: "x" } },
  { category: "editing", value: "undo", label: "撤销", icon: "left", command: { type: "combo", modifiers: ["ctrl"], key: "z" } },
  { category: "editing", value: "redo", label: "重做", icon: "right", command: { type: "combo", modifiers: ["ctrl"], key: "y" } },
  { category: "editing", value: "selectall", label: "全选", icon: "grid", command: { type: "combo", modifiers: ["ctrl"], key: "a" } },
  { category: "editing", value: "find", label: "查找", icon: "search", command: { type: "combo", modifiers: ["ctrl"], key: "f" } },
  { category: "browser", value: "previoustab", label: "上个标签页", icon: "left", command: { type: "combo", modifiers: ["ctrl", "shift"], key: "tab" } },
  { category: "browser", value: "nexttab", label: "下个标签页", icon: "right", command: { type: "combo", modifiers: ["ctrl"], key: "tab" } },
  { category: "browser", value: "refresh", label: "刷新页面", icon: "switch", command: { type: "key", key: "f5" } },
  { category: "browser", value: "fullscreen", label: "全屏切换", icon: "monitor", command: { type: "key", key: "f11" } },
  { category: "slides", value: "presentation", label: "开始放映", icon: "monitor", command: { type: "key", key: "f5" } },
  { category: "slides", value: "slidecurrent", label: "当前页放映", icon: "play", command: { type: "combo", modifiers: ["shift"], key: "f5" } },
  { category: "slides", value: "slideprevious", label: "上一页", icon: "left", command: { type: "key", key: "pageup" } },
  { category: "slides", value: "slidenext", label: "下一页", icon: "right", command: { type: "key", key: "pagedown" } },
  { category: "slides", value: "slideblack", label: "黑屏切换", icon: "monitor", command: { type: "key", key: "b" } },
];

export const shortcutPresets = [
  { value: "media", label: "媒体控制", actions: ["mediaprevious", "mediaplay", "medianext"] },
  { value: "daily", label: "日常操控", actions: ["taskview", "notifications", "nextapp"] },
  { value: "editing", label: "文字编辑", actions: ["copy", "paste", "undo"] },
  { value: "browser", label: "网页浏览", actions: ["previoustab", "nexttab", "refresh"] },
  { value: "slides", label: "演示放映", actions: ["slideprevious", "slideblack", "slidenext"] },
] as const;

export const shortcutValues = shortcuts.map(item => item.value);
export const shortcutGroups = [
  { value: "daily", label: "日常操控" }, { value: "editing", label: "文字编辑" },
  { value: "browser", label: "网页浏览" }, { value: "media", label: "媒体控制" },
  { value: "slides", label: "演示放映" },
] as const;

export function shortcutOptions(current?: string) {
  const options = shortcutGroups.flatMap(group => shortcuts.filter(item => item.category === group.value).map(item => ({
    value: item.value, label: t(item.label), group: t(group.label), icon: item.icon,
    description: item.command?.type === "combo"
      ? [...item.command.modifiers.map(key => key[0].toUpperCase() + key.slice(1)), item.command.key.toUpperCase()].join(" + ")
      : item.command?.type === "key" ? item.command.key.toUpperCase() : undefined,
  })));
  if (current !== undefined) {
    const chord = parseCustomShortcut(current);
    if (chord) options.push({ value: current, label: chordLabel(chord), group: t("自定义"), icon: "keyboard", description: undefined });
    options.push({ value: "custom-key", label: t("自定义按键"), group: t("自定义"), icon: "keyboard", description: undefined });
  }
  return options;
}
export const isShortcutValue = (value: unknown): value is string => typeof value === "string" && (shortcutValues.includes(value) || !!parseCustomShortcut(value));
export function shortcutLabel(value: string) {
  const chord = parseCustomShortcut(value);
  return chord ? chordLabel(chord) : shortcutOptions().find(item => item.value === value)?.label || "";
}
export function presetOptions() {
  return [{ value: "custom", label: t("自定义") }, ...shortcutPresets.map(item => ({ value: item.value, label: t(item.label) }))];
}
