import { expect, it, vi } from "vitest";
import { ref } from "vue";
import { bindSheetActions, bindScreenActions, dispatchNativeAction as dispatchAction, currentNativeInput, type NativeActions } from "../apps/mobile/native/actions";
import { useCustomShortcut } from "../apps/mobile/composables/useCustomShortcut";
import { recoveryActionId, type NativeControl } from "../apps/mobile/native/ui";

const dispatchNativeAction = (actions: NativeActions, detail: unknown, revision: number, sheetOpen: boolean) =>
  dispatchAction(actions, detail, revision, sheetOpen, 0);

it("rejects queued native effects from an earlier foreground epoch but preserves revision-scoped drafts", () => {
  expect(currentNativeInput({id:"enter:screen:2",inputEpoch:3},true,4)).toBe(false);
  expect(currentNativeInput({target:"pad",phase:"move",inputEpoch:3},true,4)).toBe(false);
  expect(currentNativeInput({id:"send:screen:2",inputEpoch:4},false,4)).toBe(false);
  expect(currentNativeInput({id:"enter:screen:2",inputEpoch:4},true,4)).toBe(true);
  expect(currentNativeInput({id:"enter:screen:2"},true,4)).toBe(false);
  expect(currentNativeInput({id:"draft:screen:2",value:{text:"保留输入",revision:7},inputEpoch:3},false,4)).toBe(true);
  const draft=vi.fn();const actions:NativeActions=new Map([["draft:screen:2",draft]]);
  expect(dispatchAction(actions,{id:"draft:screen:1",value:{text:"旧页面",revision:7}},0,false,2)).toBe(false);expect(draft).not.toHaveBeenCalled();
});

it("a delayed Save cannot commit another shortcut editor, including before its new render", () => {
  const slots = [ref("copy"), ref("paste"), ref("undo")];
  const editor = useCustomShortcut(slots);
  const actions: NativeActions = new Map();
  const bind = (revision: number) => {
    actions.clear(); actions.set("save", editor.save); actions.set("dismiss", editor.close);
    const save: NativeControl = { kind: "button", id: "save" };
    bindSheetActions(actions, [save], revision);
    return save.actionId!;
  };
  editor.models[0].value = "custom-key";
  const previousSave = bind(1);
  editor.close(); editor.models[1].value = "custom-key"; editor.draft.value.key = "p";
  expect(dispatchNativeAction(actions, { id: previousSave }, 3, true)).toBe(false);
  const currentSave = bind(3);
  expect(dispatchNativeAction(actions, { id: previousSave }, 3, true)).toBe(false);
  expect(editor.editing.value).toBe(1);
  expect(slots.map(slot => slot.value)).toEqual(["copy", "paste", "undo"]);
  expect(dispatchNativeAction(actions, { id: currentSave }, 3, true)).toBe(true);
  expect(slots.map(slot => slot.value)).toEqual(["copy", "key:ctrl+p", "undo"]);
});

it("a reopened chooser rejects old selections and dismiss callbacks", () => {
  const select = vi.fn(), dismiss = vi.fn();
  const actions: NativeActions = new Map([["languageen", select], ["dismiss", dismiss]]);
  const item: NativeControl = { kind: "button", id: "languageen" };
  const close = bindSheetActions(actions, [item], 7);
  expect(dispatchNativeAction(actions, { id: item.actionId }, 9, true)).toBe(false);
  expect(dispatchNativeAction(actions, { id: close }, 9, true)).toBe(false);
  expect(dispatchNativeAction(actions, { id: close }, 7, false)).toBe(false);
  expect(select).not.toHaveBeenCalled(); expect(dismiss).not.toHaveBeenCalled();
});

it("the modal blocks background actions while current text edits and IME submissions retain their owner", () => {
  const key = vi.fn(), edit = vi.fn(), send = vi.fn();
  const actions: NativeActions = new Map([["enter", key], ["draft", edit], ["send", send]]);
  const field: NativeControl = { kind: "edit", id: "draft" }, submit: NativeControl = { kind: "button", id: "send" };
  bindSheetActions(actions, [{ kind: "row", id: "entry", children: [field, submit] }], 3);
  const screen = bindScreenActions(actions, [], 0);
  expect(dispatchNativeAction(actions, { id: screen.enter }, 3, true)).toBe(false);
  expect(dispatchNativeAction(actions, { id: field.actionId, value: { text: "文字", revision: 1 } }, 3, true)).toBe(true);
  expect(edit).toHaveBeenCalledWith({ text: "文字", revision: 1 });
  expect(field.submitAction).toBe(submit.actionId);
  expect(dispatchNativeAction(actions, { id: field.submitAction }, 3, true)).toBe(true);
  expect(send).toHaveBeenCalledTimes(1);
  expect(dispatchNativeAction(actions, { id: screen.enter }, 3, false)).toBe(true);
});

it("malformed and removed actions are ignored and scoped IDs do not affect the visual identity", () => {
  const callback = vi.fn();
  const actions: NativeActions = new Map([["save", callback]]);
  const first: NativeControl = { kind: "button", id: "save" }, second: NativeControl = { kind: "button", id: "save" };
  bindSheetActions(actions, [first, second], 4);
  expect(first.id).toBe("save"); expect(first.actionId).toBe(second.actionId);
  for (const event of [null, 1, [], {}, { id: 3 }, { id: "removed:sheet:4" }])
    expect(dispatchNativeAction(actions, event, 4, true)).toBe(false);
  expect(callback).not.toHaveBeenCalled();
});

it("normal enablement and meter renders retain the same action identity within a sheet", () => {
  const disabled: NativeControl = { kind: "button", id: "send", enabled: false };
  const actions: NativeActions = new Map();
  bindSheetActions(actions, [disabled], 6);
  expect(dispatchNativeAction(actions, { id: disabled.actionId }, 6, true)).toBe(false);
  const send = vi.fn(); actions.set("send", send);
  const enabled: NativeControl = { kind: "button", id: "send", enabled: true };
  bindSheetActions(actions, [enabled], 6);
  expect(enabled.actionId).toBe(disabled.actionId);
  expect(dispatchNativeAction(actions, { id: enabled.actionId }, 6, true)).toBe(true);
  expect(send).toHaveBeenCalledTimes(1);
});

it("opening a modal can finish a background IME draft without sending its text to the PC", () => {
  const draft = vi.fn(), send = vi.fn();
  const actions: NativeActions = new Map([["draft", draft], ["send", send]]);
  const screen = bindScreenActions(actions, [], 0);
  expect(dispatchNativeAction(actions, { id: screen.draft, value: { text: "已选候选词", revision: 4 } }, 8, true)).toBe(true);
  expect(draft).toHaveBeenCalledWith({ text: "已选候选词", revision: 4 });
  expect(dispatchNativeAction(actions, { id: screen.send }, 8, true)).toBe(false);
  expect(send).not.toHaveBeenCalled();
});

it("a retired keyboard cannot send or replace the new page's draft before or after rendering", () => {
  let draft = "旧草稿";
  const sent: string[] = [];
  const actions: NativeActions = new Map();
  const bind = (revision: number) => {
    actions.clear();
    actions.set("draft", value => { draft = (value as { text: string }).text; });
    actions.set("send", () => { sent.push(draft); draft = ""; });
    const field: NativeControl = { kind: "edit", id: "draft" };
    const screen = bindScreenActions(actions, [field], revision);
    expect(field.submitAction).toBe(screen.send);
    return screen;
  };
  const previous = bind(1);
  draft = "新草稿";
  expect(dispatchAction(actions, { id: previous.send }, 0, false, 3)).toBe(false);
  const current = bind(3);
  expect(dispatchAction(actions, { id: previous.draft, value: { text: "迟到的输入", revision: 100 } }, 0, false, 3)).toBe(false);
  expect(dispatchAction(actions, { id: previous.send }, 0, false, 3)).toBe(false);
  expect(dispatchAction(actions, { id: "send" }, 0, false, 3)).toBe(false);
  expect(draft).toBe("新草稿"); expect(sent).toEqual([]);
  expect(dispatchAction(actions, { id: current.send }, 0, false, 3)).toBe(true);
  expect(sent).toEqual(["新草稿"]);
});

it("an old background edit cannot replace the draft underneath a new modal", () => {
  const edit = vi.fn();
  const actions: NativeActions = new Map([["draft", edit]]);
  const screen = bindScreenActions(actions, [], 4);
  const value = { text: "候选词", revision: 4 };
  expect(dispatchAction(actions, { id: "draft:screen:2", value }, 8, true, 4)).toBe(false);
  expect(dispatchAction(actions, { id: screen.draft, value }, 8, true, 4)).toBe(true);
  expect(edit).toHaveBeenCalledOnce();
});

it("recording and connection updates retain button identities while actions become available", () => {
  const actions: NativeActions = new Map();
  const stop = vi.fn(), focus = vi.fn();
  const before = bindScreenActions(actions, [], 2);
  actions.clear(); actions.set("mic-stop", stop); actions.set("focus", focus);
  const after = bindScreenActions(actions, [], 2);
  expect(after["mic-stop"]).toBe(before["mic-stop"]);
  expect(after.focus).toBe(before.focus);
  expect(dispatchAction(actions, { id: before["mic-stop"] }, 0, false, 2)).toBe(true);
  expect(stop).toHaveBeenCalledOnce();
});

it("sheet footer actions share modal ownership and disabled refresh states retain their visual identity", () => {
  const actions: NativeActions = new Map();
  const restart = vi.fn();
  actions.set("microphone-restart", restart);
  const footer: NativeControl[] = [{ kind: "button", id: "microphone-restart", enabled: true }];
  bindSheetActions(actions, footer, 9);
  const original = footer[0].actionId;
  expect(dispatchAction(actions, { id: original }, 10, true, 0)).toBe(false);
  expect(dispatchAction(actions, { id: original }, 9, false, 0)).toBe(false);
  actions.clear();
  const disabled: NativeControl = { kind: "button", id: "microphone-restart", enabled: false };
  bindSheetActions(actions, [disabled], 9);
  expect(disabled.actionId).toBe(original);
  expect(dispatchAction(actions, { id: original }, 9, true, 0)).toBe(false);
  expect(restart).not.toHaveBeenCalled();
  actions.clear(); actions.set("microphone-restart", restart);
  bindSheetActions(actions, footer, 9);
  expect(dispatchAction(actions, { id: original }, 9, true, 0)).toBe(true);
  expect(restart).toHaveBeenCalledOnce();
});

it("a queued recovery confirmation cannot become Keep after the target version changes", () => {
  const actions: NativeActions = new Map();
  const resolve = vi.fn();
  const oldFile = { id: "a".repeat(32), version: "b".repeat(64) };
  const newFile = { ...oldFile, version: "c".repeat(64) };
  const bind = (operation: "keep" | "confirm-delete", file: typeof oldFile) => {
    actions.clear();
    const id = recoveryActionId(operation, file);
    actions.set(id, () => resolve(file.id, file.version, operation === "keep"));
    const control: NativeControl = { kind: "button", id };
    bindSheetActions(actions, [control], 6);
    return control.actionId;
  };
  const oldKeep = bind("keep", oldFile);
  const oldConfirm = bind("confirm-delete", oldFile);
  expect(dispatchAction(actions, { id: oldKeep }, 6, true, 0)).toBe(false);
  const currentKeep = bind("keep", newFile);
  expect(dispatchAction(actions, { id: oldConfirm }, 6, true, 0)).toBe(false);
  expect(resolve).not.toHaveBeenCalled();
  expect(dispatchAction(actions, { id: currentKeep }, 6, true, 0)).toBe(true);
  expect(resolve).toHaveBeenCalledExactlyOnceWith(newFile.id, newFile.version, true);
  resolve.mockClear();
  const currentConfirm = bind("confirm-delete", newFile);
  expect(dispatchAction(actions, { id: currentKeep }, 6, true, 0)).toBe(false);
  expect(dispatchAction(actions, { id: currentConfirm }, 7, true, 0)).toBe(false);
  expect(dispatchAction(actions, { id: currentConfirm }, 6, true, 0)).toBe(true);
  expect(resolve).toHaveBeenCalledExactlyOnceWith(newFile.id, newFile.version, false);
});
