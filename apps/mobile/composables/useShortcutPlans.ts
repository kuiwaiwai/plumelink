import { computed, ref, type Ref } from "vue";
import { t } from "../../../shared/i18n";
import { builtinPlan, cloneSlots, matchingSlots, maxShortcutPlans, planName, validPlanName, validShortcutSlots, type ShortcutPlan, type ShortcutSlot } from "../shortcut-plans";

export function useShortcutPlans(plans: Ref<ShortcutPlan[]>, current: () => ShortcutSlot[], applySlots: (slots: ShortcutSlot[]) => void) {
  const stage = ref<"list" | "preview" | "save" | "rename" | "delete">("list");
  const selected = ref("");
  const name = ref("");
  const error = ref("");
  const captured = ref<ShortcutSlot[]>([]);
  const plan = computed(() => {
    const builtin = builtinPlan(selected.value);
    if (builtin) return { id: builtin.id, name: t(builtin.label), shortcuts: builtin.shortcuts, builtin: true };
    const saved = plans.value.find(item => item.id === selected.value);
    return saved && { ...saved, builtin: false };
  });
  const currentName = computed(() => plans.value.find(item => matchingSlots(item.shortcuts, current()))?.name);
  function reset() { stage.value = "list"; selected.value = ""; name.value = ""; error.value = ""; captured.value = []; }
  function select(id: string) {
    if (!builtinPlan(id) && !plans.value.some(item => item.id === id)) return;
    selected.value = id; stage.value = "preview"; error.value = "";
  }
  function saveCurrent() { captured.value = cloneSlots(current()); name.value = ""; error.value = ""; stage.value = "save"; }
  function rename() { if (!plan.value || plan.value.builtin) return; name.value = plan.value.name; error.value = ""; stage.value = "rename"; }
  function commit() {
    if (stage.value !== "save" && stage.value !== "rename") return;
    const clean = planName(name.value);
    if (!validPlanName(clean)) { error.value = t("名称需为 1–32 个字符，不能包含换行或控制字符"); return; }
    const existing = stage.value === "rename" ? plans.value.find(item => item.id === selected.value) : undefined;
    if (stage.value === "rename" && !existing) { reset(); return; }
    if (plans.value.some(item => item.id !== existing?.id && item.name.toLowerCase() === clean.toLowerCase())) { error.value = t("已有同名方案，请换一个名称"); return; }
    if (existing) {
      plans.value = plans.value.map(item => item.id === existing.id ? { ...item, name: clean } : item);
    } else {
      if (plans.value.length >= maxShortcutPlans) { error.value = t("最多保存 8 个方案，请先删除不需要的方案"); return; }
      if (!validShortcutSlots(captured.value)) { error.value = t("请先选择三个不同的快捷操作"); return; }
      const id = "p" + Array.from(crypto.getRandomValues(new Uint8Array(16)), n => n.toString(16).padStart(2, "0")).join("");
      plans.value = [...plans.value, { id, name: clean, shortcuts: cloneSlots(captured.value) }]; selected.value = id;
    }
    name.value = ""; error.value = ""; stage.value = "preview";
  }
  function apply() {
    if (stage.value !== "preview" || !plan.value) return false;
    applySlots(cloneSlots(plan.value.shortcuts)); return true;
  }
  function requestDelete() { if (plan.value && !plan.value.builtin) { stage.value = "delete"; error.value = ""; } }
  function remove() {
    if (stage.value !== "delete" || !plan.value || plan.value.builtin) return;
    plans.value = plans.value.filter(item => item.id !== selected.value); reset();
  }
  function back() {
    if (stage.value === "list") return false;
    if (stage.value === "rename" || stage.value === "delete") stage.value = plan.value ? "preview" : "list";
    else { stage.value = "list"; selected.value = ""; }
    name.value = ""; error.value = ""; captured.value = []; return true;
  }
  return { stage, selected, name, error, captured, plan, currentName, reset, select, saveCurrent, rename, commit, apply, requestDelete, remove, back };
}
