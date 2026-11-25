import { shortcuts } from "./shortcuts";
import type { t } from "../../shared/i18n";

export const shortcutIconLabels: Record<string, Parameters<typeof t>[0]> = {
  star: "星标", bolt: "闪电", bell: "铃铛", grid: "方格", monitor: "电脑", pointer: "指针",
  volume: "音量", search: "搜索", switch: "切换", left: "向左箭头", right: "向右箭头",
  layers: "图层", keyboard: "键盘", command: "快捷键", mic: "麦克风图标", gamepad: "游戏手柄",
  settings: "设置图标", activity: "活动", heart: "爱心", idea: "灯泡", play: "播放图标",
  previous: "上一首图标", nexttrack: "下一首图标", stop: "停止图标",
};
export const shortcutIconNames = Object.keys(shortcutIconLabels);
export function defaultShortcutIcon(action: string) {
  if (action.startsWith("key:")) return "keyboard";
  return shortcuts.find(item => item.value === action)?.icon || "star";
}
