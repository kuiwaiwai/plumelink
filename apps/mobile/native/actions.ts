import type { NativeControl } from "./ui";

export type NativeActions = Map<string, (value: unknown) => unknown>;
export function currentNativeInput(detail: unknown, foreground: boolean, epoch?: number): boolean {
  if (!detail || typeof detail !== "object") return false;
  const event = detail as { id?: unknown; value?: unknown; inputEpoch?: unknown };
  const logical = typeof event.id === "string" ? event.id.split(/:(?:screen|sheet):/)[0] : "";
  const edit = event.value as { text?: unknown; revision?: unknown } | null;
  if (["draft", "code", "file-filter"].includes(logical) && (typeof event.value === "string" || (typeof edit?.text === "string" && typeof edit.revision === "number"))) return true;
  return foreground && (epoch === undefined || (Number.isSafeInteger(epoch) && epoch > 0 && event.inputEpoch === epoch));
}
const sheetSuffix = ":sheet:";
const screenSuffix = ":screen:";

function bindActions(actions: NativeActions, items: NativeControl[], suffix: string) {
  const callbacks = new Map(actions);
  const scopedId = (id: string) => id + suffix;
  function bind(id: string) {
    const action = callbacks.get(id);
    if (action) { actions.set(scopedId(id), action); actions.delete(id); }
    return scopedId(id);
  }
  function walk(controls: NativeControl[]) {
    for (const control of controls) {
      if (callbacks.has(control.id) || ["button", "action-row", "switch", "slider", "edit", "window", "mic", "pad"].includes(control.kind))
        control.actionId = bind(control.id);
      if (control.kind === "edit" && ["draft", "code"].includes(control.id))
        control.submitAction = scopedId(control.id === "code" ? "pair" : "send");
      if (control.children) walk(control.children);
    }
  }
  walk(items);
  return bind;
}

export function bindSheetActions(actions: NativeActions, items: NativeControl[], revision: number): string {
  const bind = bindActions(actions, items, sheetSuffix + revision);
  return bind("dismiss");
}

export function bindScreenActions(actions: NativeActions, items: NativeControl[], revision: number): Record<string, string> {
  const ids = [...new Set(["back-remote", "focus", "mic-stop", "dismiss-feedback", ...actions.keys()])].filter(id => !id.includes(sheetSuffix));
  const bind = bindActions(actions, items, screenSuffix + revision);
  return Object.fromEntries(ids.map(id => [id, bind(id)]));
}

export function dispatchNativeAction(actions: NativeActions, detail: unknown, revision: number, sheetOpen: boolean, screenRevision: number): boolean {
  if (!detail || typeof detail !== "object") return false;
  const event = detail as { id?: unknown; value?: unknown };
  if (typeof event.id !== "string") return false;
  const screenAction = event.id.endsWith(screenSuffix + screenRevision);
  const logical = screenAction ? event.id.slice(0, -(`${screenSuffix}${screenRevision}`).length) : "";
  const edit = event.value as { text?: unknown; revision?: unknown } | null;
  const draftUpdate = ["draft", "file-filter"].includes(logical)
    && typeof edit?.text === "string" && typeof edit.revision === "number";
  if (sheetOpen ? !event.id.endsWith(sheetSuffix + revision) && !draftUpdate : !screenAction) return false;
  const action = actions.get(event.id);
  if (!action) return false;
  action(event.value);
  return true;
}
