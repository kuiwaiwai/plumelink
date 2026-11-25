export type ControlPreferences = {
  sensitivity: number;
  keepAwake: boolean;
  haptics: boolean;
  hardwareVolume: boolean;
  longPressDrag: boolean;
  scrollDirection: "follow" | "opposite";
  responseMode: "balanced" | "responsive";
  padOnRight: boolean;
  focusMode: boolean;
  controlMode: "trackpad" | "dpad";
  dialSize: "small" | "standard" | "large";
  microphoneMode: "standard" | "voice";
};

export const controlPreferenceStorage: Record<keyof ControlPreferences, string> = {
  sensitivity: "sensitivity", keepAwake: "keep-awake", haptics: "haptics", hardwareVolume: "hardware-volume",
  longPressDrag: "long-press-drag", scrollDirection: "scroll-direction", responseMode: "response-mode",
  padOnRight: "pad-side", focusMode: "focus-mode", controlMode: "control-mode", dialSize: "dial-size",
  microphoneMode: "microphone-mode",
};
export const controlPreferenceKeys = Object.keys(controlPreferenceStorage) as (keyof ControlPreferences)[];
const rules: Record<keyof ControlPreferences, (value: unknown) => boolean> = {
  sensitivity: value => typeof value === "number" && Number.isFinite(value) && value >= 0.5 && value <= 3,
  keepAwake: value => typeof value === "boolean",
  haptics: value => typeof value === "boolean",
  hardwareVolume: value => typeof value === "boolean",
  longPressDrag: value => typeof value === "boolean",
  scrollDirection: value => value === "follow" || value === "opposite",
  responseMode: value => value === "balanced" || value === "responsive",
  padOnRight: value => typeof value === "boolean",
  focusMode: value => typeof value === "boolean",
  controlMode: value => value === "trackpad" || value === "dpad",
  dialSize: value => value === "small" || value === "standard" || value === "large",
  microphoneMode: value => value === "standard" || value === "voice",
};
export function validControlPreference<Key extends keyof ControlPreferences>(key: Key, value: unknown): value is ControlPreferences[Key] {
  return rules[key](value);
}
export function controlStorageValue(key: keyof ControlPreferences, value: ControlPreferences[keyof ControlPreferences]) {
  return key === "padOnRight" ? value ? "right" : "left" : String(value);
}
