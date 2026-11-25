import { computed, ref, type Ref } from "vue";
import { keyboardModifiers } from "../../../shared/keyboard";
import { t } from "../../../shared/i18n";
import { parseCustomShortcut, customShortcutValue, keyGroups, type KeyChord, type ShortcutModifier } from "../custom-shortcuts";

export function useCustomShortcut(slots: Ref<string>[]) {
  const editing = ref<number | null>(null);
  const draft = ref<KeyChord>({ key: "s", modifiers: ["ctrl"] });
  const group = ref<string>("letters");
  const error = ref("");
  function open(index: number) {
    draft.value = parseCustomShortcut(slots[index].value) || { key: "s", modifiers: ["ctrl"] };
    group.value = keyGroups.find(item => item.keys.includes(draft.value.key))?.value || "letters";
    error.value = "";
    editing.value = index;
  }
  const models = slots.map((slot, index) => computed({
    get: () => slot.value,
    set: value => { if (value === "custom-key") open(index); else slot.value = value; },
  }));
  function toggle(modifier: ShortcutModifier) {
    draft.value.modifiers = keyboardModifiers.filter(key => key === modifier ? !draft.value.modifiers.includes(key) : draft.value.modifiers.includes(key));
    error.value = "";
  }
  function save() {
    if (editing.value === null) return;
    if (draft.value.key === "delete" && draft.value.modifiers.includes("ctrl") && draft.value.modifiers.includes("alt")) {
      error.value = t("此组合键由 Windows 保留，请选择其它按键"); return;
    }
    const value = customShortcutValue(draft.value);
    if (slots.some((slot, index) => index !== editing.value && slot.value === value)) {
      error.value = t("该按键已用于其它快捷操作"); return;
    }
    slots[editing.value].value = value;
    editing.value = null;
    error.value = "";
  }
  return { editing, draft, group, error, models, toggle, save, close: () => { editing.value = null; error.value = ""; } };
}
