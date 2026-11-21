import { Activity, ArrowLeft, ArrowLeftRight, ArrowRight, Bell, Command, Gamepad2, Heart, Keyboard, Layers, LayoutGrid, Lightbulb, Mic, Monitor, MousePointer2, Search, Settings2, Star, Volume2, Zap } from "@lucide/vue";
import type { Component } from "vue";

export const shortcutIcons: Record<string, Component> = {
  star: Star,
  bolt: Zap,
  bell: Bell,
  grid: LayoutGrid,
  monitor: Monitor,
  pointer: MousePointer2,
  volume: Volume2,
  search: Search,
  switch: ArrowLeftRight,
  left: ArrowLeft,
  right: ArrowRight,
  layers: Layers,
  keyboard: Keyboard,
  command: Command,
  mic: Mic,
  gamepad: Gamepad2,
  settings: Settings2,
  activity: Activity,
  heart: Heart,
  idea: Lightbulb,
};
export const shortcutIconNames = Object.keys(shortcutIcons);
export function defaultShortcutIcon(action: string) {
  const defaults: Record<string, string> = {
    mute: "volume", magnifier: "search", taskview: "grid", notifications: "bell",
    previousapp: "left", nextapp: "right", virtualleft: "layers", virtualright: "layers",
  };
  return defaults[action] || "star";
}
