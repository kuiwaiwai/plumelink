import { computed, onScopeDispose, ref, watch, type ComputedRef, type Ref } from "vue";
import { displayMessage, locale, t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";
import { thumbnail } from "../../../shared/thumbnail";
import type { Command } from "../../../shared/protocol";

type Client = { api(path: string, init?: RequestInit): Promise<Response>; send(command: Command): Promise<void> };

export type LauncherEntry = { id: string; name: string; kind: "application" | "folder"; available?: boolean; icon?: string };

export function useApplications(remote: Client, panel: Readonly<Ref<string | null>>, canInput: ComputedRef<boolean>) {
  const applications = ref<LauncherEntry[]>([]);
  const applicationQuery = ref("");
  const applicationSearchShown = ref(false);
  const visibleApplications = computed(() => {
    const normalize = (value: string) => value.normalize("NFKC").toLocaleLowerCase(locale.value);
    const words = normalize(applicationQuery.value).trim().split(/\s+/).filter(Boolean);
    return applications.value.filter(entry => words.every(word => normalize(entry.name).includes(word)));
  });
  const applicationsLoading = ref(false);
  const applicationsReady = ref(false);
  const launchJobs = ref(false);
  const applicationPending = ref("");
  const applicationOpening = ref(false);
  const applicationPendingDetail = computed(() => applicationOpening.value ? t("正在打开…") : t("发送中…"));
  const applicationsError = ref("");
  const applicationFailure = ref("");
  const applicationHint = ref<"" | "removed" | LauncherEntry["kind"]>("");
  const requestedApplication = ref<Pick<LauncherEntry, "name" | "kind">>();
  const applicationsNotice = computed(() => requestedApplication.value ? t(requestedApplication.value.kind === "folder" ? "已请求打开 {0}" : "已请求启动 {0}", requestedApplication.value.name) : "");
  let applicationPanelGeneration = 0;
  let applicationResultAbort: AbortController | undefined;
  watch(panel, current => {
    applicationPanelGeneration++;
    applicationResultAbort?.abort();
    applicationFailure.value = "";
    applicationHint.value = "";
    requestedApplication.value = undefined;
    if (current !== "applications") { applicationQuery.value = ""; applicationSearchShown.value = false; }
  });
  let applicationsGeneration = 0;
  let applicationsAbort: AbortController | undefined;
  async function loadApplications() {
    const generation = ++applicationsGeneration;
    applicationsAbort?.abort();
    const controller = new AbortController();
    applicationsAbort = controller;
    applicationsLoading.value = true;
    applicationsReady.value = false;
    applicationsError.value = "";
    try {
      const response = await remote.api("/api/applications", { signal: controller.signal });
      const result = await response.json();
      if (response.status === 404) throw new Error(t("请更新电脑端以使用快捷启动"));
      if (!response.ok) throw new Error(result.error || t("无法读取快捷入口，请重试"));
      if (generation !== applicationsGeneration || !canInput.value) return;
      launchJobs.value = result.launchJobs === true;
      const previousIcons = new Map(applications.value.map(entry => [entry.id, entry.icon]));
      applications.value = Array.isArray(result.applications) ? result.applications.slice(0, 32).flatMap((item: unknown) => {
        if (!item || typeof item !== "object") return [];
        const entry = item as Record<string, unknown>;
        if (typeof entry.id !== "string" || !/^[a-f0-9]{32}$/.test(entry.id) || typeof entry.name !== "string" || !entry.name.length || entry.name.length > 240 || /[\u0000-\u001f\u007f]/.test(entry.name)) return [];
        if (entry.kind !== undefined && entry.kind !== "folder" && entry.kind !== "application") return [];
        const kind: LauncherEntry["kind"] = entry.kind === "folder" ? "folder" : "application";
        return [{ id: entry.id, name: entry.name, kind, available: entry.available !== false, icon: kind === "folder" ? undefined : previousIcons.get(entry.id) }];
      }) : [];
      if (applications.value.length > 5) applicationSearchShown.value = true;
      applicationsReady.value = true;
      void loadApplicationIcons(generation, controller.signal);
    } catch (error) {
      if (generation === applicationsGeneration) applicationsError.value = errorText(error, t("无法读取快捷入口，请重试"));
    } finally {
      if (generation === applicationsGeneration) applicationsLoading.value = false;
    }
  }
  async function loadApplicationIcons(generation: number, signal: AbortSignal) {
    if (signal.aborted || !applications.value.some(entry => entry.kind === "application")) return;
    try {
      const response = await remote.api("/api/applications/icons", { signal });
      if (!response.ok) return;
      const result = await response.json();
      if (signal.aborted || generation !== applicationsGeneration || !canInput.value || !Array.isArray(result.icons)) return;
      const icons = new Map<string, string>();
      for (const item of result.icons.slice(0, 32)) {
        if (!item || typeof item !== "object" || typeof item.id !== "string" || !/^[a-f0-9]{32}$/.test(item.id)) continue;
        const icon = thumbnail(item.icon);
        if (icon) icons.set(item.id, icon);
      }
      applications.value.forEach(entry => { entry.icon = entry.kind === "folder" ? undefined : icons.get(entry.id); });
    } catch {}
  }
  function refreshApplications() {
    if (applicationPending.value) return;
    applicationResultAbort?.abort(); applicationResultAbort = undefined;
    applicationFailure.value = ""; applicationHint.value = ""; requestedApplication.value = undefined;
    return loadApplications();
  }
  const unavailableApplication = (entry: Pick<LauncherEntry, "kind">) => t(entry.kind === "folder" ? "文件夹已移动或删除，请在电脑端重新添加" : "应用文件已移动或删除，请在电脑端重新添加");
  const applicationActionError = computed(() => [...new Set([
    displayMessage(applicationFailure.value),
    applicationHint.value === "removed" ? t("快捷入口已移除，请刷新列表") : applicationHint.value ? unavailableApplication({ kind: applicationHint.value }) : "",
  ])].filter(Boolean).join("\n"));
  async function launchResult(request: string, signal: AbortSignal) {
    const controller = new AbortController();
    const cancel = () => controller.abort();
    signal.addEventListener("abort", cancel, { once: true });
    const timeout = setTimeout(cancel, 6000);
    try {
      if (signal.aborted) controller.abort();
      while (!controller.signal.aborted) {
        const response = await remote.api("/api/applications/launches/" + request, { signal: controller.signal });
        if (!response.ok) throw new Error();
        const result = await response.json();
        if (!["preparing", "starting", "succeeded", "failed", "uncertain"].includes(result.state)
            || typeof result.message !== "string" || result.message.length > 512) throw new Error();
        if (["succeeded", "failed", "uncertain"].includes(result.state)) return result as { state: string; message: string };
        await new Promise<void>((resolve, reject) => {
          if (controller.signal.aborted) { reject(new Error()); return; }
          const stopped = () => { clearTimeout(timer); controller.signal.removeEventListener("abort", stopped); reject(new Error()); };
          const timer = setTimeout(() => { controller.signal.removeEventListener("abort", stopped); resolve(); }, 150);
          controller.signal.addEventListener("abort", stopped, { once: true });
        });
      }
      throw new Error();
    } catch { throw new Error(t("启动结果未确认，请检查电脑；不会自动重发")); }
    finally { clearTimeout(timeout); signal.removeEventListener("abort", cancel); }
  }
  async function launchApplication(entry: LauncherEntry) {
    if (!canInput.value || !applicationsReady.value || applicationPending.value || entry.available === false) return;
    const generation = applicationPanelGeneration;
    const resultController = new AbortController();
    applicationResultAbort?.abort();
    applicationResultAbort = resultController;
    const currentOperation = () => generation === applicationPanelGeneration && applicationResultAbort === resultController && !resultController.signal.aborted;
    applicationPending.value = entry.id;
    applicationOpening.value = false;
    applicationFailure.value = "";
    applicationHint.value = "";
    applicationsError.value = "";
    requestedApplication.value = undefined;
    try {
      const request = launchJobs.value ? Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, "0")).join("") : undefined;
      await remote.send(request ? { type: "launch", id: entry.id, request } : { type: "launch", id: entry.id });
      if (request && currentOperation()) {
        applicationOpening.value = true;
        const result = await launchResult(request, resultController.signal);
        if (result.state !== "succeeded") throw new Error(result.message || t("启动结果未确认，请检查电脑；不会自动重发"));
      }
      if (currentOperation()) requestedApplication.value = { name: entry.name, kind: entry.kind };
    } catch (error) {
      if (!currentOperation()) return;
      applicationPending.value = "";
      applicationFailure.value = errorText(error, t(entry.kind === "folder" ? "无法打开文件夹，请在电脑端检查访问权限" : "应用启动失败"));
      try {
        const response = await remote.api("/api/applications", { signal: resultController.signal });
        const result = await response.json();
        if (currentOperation() && canInput.value && response.ok && Array.isArray(result.applications)) {
          const current = result.applications.find((item: { id?: string }) => item?.id === entry.id);
          if (!current) { applicationHint.value = "removed"; applications.value = applications.value.filter(item => item.id !== entry.id); }
          else if (current.available === false) { applicationHint.value = entry.kind; const local = applications.value.find(item => item.id === entry.id); if (local) local.available = false; }
        }
      } catch {}
    }
    finally { if (applicationResultAbort === resultController) { applicationPending.value = ""; applicationOpening.value = false; applicationResultAbort = undefined; } }
  }
  watch([panel, canInput], ([current, allowed]) => {
    applicationsGeneration++;
    applicationsAbort?.abort();
    applicationsLoading.value = false;
    applicationsReady.value = false;
    if (!allowed) {
      if (applicationOpening.value && applicationPending.value) applicationFailure.value = t("启动结果未确认，请检查电脑；不会自动重发");
      applicationResultAbort?.abort();
      applications.value = [];
    }
    if (current === "applications" && allowed) void loadApplications();
  });
  onScopeDispose(() => { applicationPanelGeneration++; applicationsGeneration++; applicationsAbort?.abort(); applicationResultAbort?.abort(); });

  return { applications, applicationQuery, applicationSearchShown, visibleApplications, applicationsLoading, applicationsReady, applicationPending, applicationPendingDetail, applicationsError, applicationActionError, applicationsNotice, refreshApplications, unavailableApplication, launchApplication };
}
