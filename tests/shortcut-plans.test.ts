import { computed, effectScope, nextTick, ref } from "vue";
import { describe, expect, it } from "vitest";
import { useShortcutPlans } from "../apps/mobile/composables/useShortcutPlans";
import { builtinPlan, cloneSlots, matchingSlots, parseShortcutPlans, validPlanName } from "../apps/mobile/shortcut-plans";

const slots = () => [{ action: "copy", icon: "heart" }, { action: "key:ctrl+shift+f5", icon: "bell" }, { action: "undo", icon: "left" }];
const saved = (index = 0) => ({ id: "p" + index.toString(16).padStart(32, "0"), name: "Plan " + index, shortcuts: slots() });
function setup() {
  const plans = ref([saved()]), current = ref(slots());
  const scope = effectScope();
  const model = scope.run(() => useShortcutPlans(plans, () => current.value, values => { current.value = values; }))!;
  return { model, plans, current, scope };
}
describe("saved shortcut plans", () => {
  it("bounds names by Unicode characters, rejects layout controls, and parses independent snapshots", () => {
    for (const name of ["", " x", "x\n", "bad\u202ename", "x".repeat(33), "e\u0301"]) expect(validPlanName(name), name).toBe(false);
    expect(validPlanName("😀".repeat(32))).toBe(true);
    const input = [saved()]; const parsed = parseShortcutPlans(input)!;
    input[0].shortcuts[0].icon = "star"; expect(parsed[0].shortcuts[0].icon).toBe("heart");
  });
  it("rejects duplicate IDs/names/actions, unsupported actions/icons, extra fields and excess entries", () => {
    expect(parseShortcutPlans([saved(), saved()])).toBeUndefined();
    expect(parseShortcutPlans([saved(), { ...saved(1), name: "PLAN 0" }])).toBeUndefined();
    expect(parseShortcutPlans(Array.from({ length: 9 }, (_, i) => saved(i)))).toBeUndefined();
    for (const shortcuts of [[...slots().slice(0, 2)], [slots()[0], slots()[0], slots()[2]],
      [{ action: "desktop", icon: "star" }, ...slots().slice(1)], [{ action: "copy", icon: "bad" }, ...slots().slice(1)]])
      expect(parseShortcutPlans([{ ...saved(), shortcuts }])).toBeUndefined();
    expect(parseShortcutPlans([{ ...saved(), token: "private" }])).toBeUndefined();
    expect(parseShortcutPlans([])).toEqual([]);
  });
  it("does not apply on selection, supports builtin previews, and preserves explicitly chosen icons", () => {
    const { model, current, scope } = setup();
    model.select("builtin:media"); expect(current.value).toEqual(slots());
    expect(model.plan.value?.builtin).toBe(true); model.requestDelete(); expect(model.stage.value).toBe("preview");
    expect(model.apply()).toBe(true); expect(current.value).toEqual(builtinPlan("builtin:media")!.shortcuts);
    model.select(saved().id); model.apply(); expect(current.value).toEqual(slots()); scope.stop();
  });
  it("freezes the save snapshot, normalizes names and preserves the current combination", () => {
    const { model, current, plans, scope } = setup();
    model.saveCurrent(); current.value = builtinPlan("builtin:browser")!.shortcuts;
    model.name.value = "  Cafe\u0301  "; model.commit();
    expect(plans.value[1].name).toBe("Café"); expect(plans.value[1].shortcuts).toEqual(slots());
    expect(current.value).toEqual(builtinPlan("builtin:browser")!.shortcuts); scope.stop();
  });
  it("rejects invalid and duplicate names, a full collection, and invalid current slots without partial changes", () => {
    const { model, plans, current, scope } = setup();
    model.saveCurrent(); model.commit(); expect(model.error.value).toBeTruthy(); expect(plans.value).toHaveLength(1);
    model.name.value = "plan 0"; model.commit(); expect(model.stage.value).toBe("save");
    plans.value = Array.from({ length: 8 }, (_, i) => saved(i)); model.name.value = "New"; model.commit(); expect(plans.value).toHaveLength(8);
    plans.value = []; current.value = [slots()[0], slots()[0], slots()[2]]; model.saveCurrent(); model.name.value = "New"; model.commit(); expect(plans.value).toHaveLength(0); scope.stop();
  });
  it("renames without replacing keys, requires explicit delete confirmation, and never resets active slots", () => {
    const { model, plans, current, scope } = setup(); model.select(saved().id);
    model.remove(); expect(plans.value).toHaveLength(1);
    model.rename(); model.name.value = "Work"; model.commit(); expect(plans.value[0]).toEqual({ ...saved(), name: "Work" });
    model.requestDelete(); model.back(); expect(plans.value).toHaveLength(1);
    model.requestDelete(); model.remove(); expect(plans.value).toHaveLength(0); expect(current.value).toEqual(slots()); scope.stop();
  });
  it("discards cancelled drafts and safely rejects a selection removed before apply", () => {
    const { model, plans, current, scope } = setup(); model.saveCurrent(); model.name.value = "Draft"; model.back();
    expect(model.name.value).toBe(""); expect(plans.value).toHaveLength(1);
    model.select(saved().id); plans.value = []; expect(model.apply()).toBe(false); expect(current.value).toEqual(slots()); scope.stop();
  });
  it("tracks the active plan by the three actions and their icons", async () => {
    const { model, plans, current, scope } = setup(); const name = computed(() => model.currentName.value);
    expect(name.value).toBe("Plan 0"); current.value[0].icon = "star"; await nextTick(); expect(name.value).toBeUndefined();
    expect(matchingSlots(cloneSlots(plans.value[0].shortcuts), current.value)).toBe(false); scope.stop();
  });
});
