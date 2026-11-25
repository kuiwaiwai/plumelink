import { computed, onScopeDispose, ref, watch, type Ref } from "vue";
import { errorText } from "../../../shared/format";
import { t } from "../../../shared/i18n";
import type { AndroidBridge } from "../native/bridge";

const validDraft = (value: unknown): value is string => typeof value === "string" && value.length <= 1000
  && !/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(value);

export function useConnectionRecovery(
  native: AndroidBridge | undefined,
  remote: { status: Readonly<Ref<string>>; token(): string },
  draft: Ref<string>, open: Readonly<Ref<boolean>>, foreground: Readonly<Ref<boolean>>,
) {
  const available = computed(() => !!native?.recoverConnection && location.hostname !== "127.0.0.1"
    && remote.status.value !== "connected" && remote.status.value !== "unpaired" && /^[a-f0-9]{64}$/i.test(remote.token()));
  const busy = ref(false), error = ref("");
  let request = "", disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const recovered = native?.takeRecoveredDraft?.();
    if (!draft.value && validDraft(recovered)) draft.value = recovered;
  } catch { /* A draft is optional. */ }

  function finish() { clearTimeout(timer); timer = undefined; request = ""; busy.value = false; }
  function cancel() {
    const previous = request;
    finish();
    if (previous) try { native?.cancelConnectionRecovery?.(previous); } catch { /* The native deadline also retires the request. */ }
  }
  function recover() {
    if (disposed || busy.value || !available.value || !open.value || !foreground.value || !validDraft(draft.value)) return;
    request = Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, "0")).join("");
    busy.value = true; error.value = "";
    timer = setTimeout(() => {
      cancel(); error.value = t("未找到可恢复的电脑连接，请确认电脑服务已开启并处于同一网络。");
    }, 15_000);
    try { native!.recoverConnection!(draft.value, request); }
    catch (cause) { cancel(); error.value = errorText(cause, t("查找电脑失败，请重试")); }
  }
  function receive(event: Event) {
    const value = (event as CustomEvent<unknown>).detail;
    if (!request || !value || typeof value !== "object") return;
    const item = value as { request?: unknown; phase?: unknown; message?: unknown };
    if (item.request !== request) return;
    if (item.phase === "searching") return;
    if (item.phase !== "error" && item.phase !== "cancelled") return;
    const failure = item.phase === "error" ? typeof item.message === "string" && item.message.length <= 512
      ? item.message : t("查找电脑失败，请重试") : "";
    finish(); error.value = failure;
  }
  window.addEventListener("plumelink-connection-recovery", receive);
  watch([available, open, foreground], values => {
    if (values.some(value => !value)) { cancel(); error.value = ""; }
  }, { flush: "sync" });
  onScopeDispose(() => { disposed = true; cancel(); window.removeEventListener("plumelink-connection-recovery", receive); });
  return { available, busy, error, recover };
}
