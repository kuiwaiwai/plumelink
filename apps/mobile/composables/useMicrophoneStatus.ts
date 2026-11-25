import { computed, onScopeDispose, ref, watch, type Ref } from "vue";
import { t } from "../../../shared/i18n";
import { errorText } from "../../../shared/format";

type Signal = {
  active: boolean; error: string;
  receivedFrames: number; receivedSignalFrames: number; renderedSignalSamples: number;
  queuedMs: number; targetMs: number; jitterMs: number; underruns: number; droppedFrames: number;
};
export type MicrophoneStatus = {
  configured: boolean; allowed: boolean; output: string; gainDb?: number; owned?: boolean; busy: boolean; status: Signal;
};
function emptySignal(): Signal {
  return { active: false, error: "", receivedFrames: 0, receivedSignalFrames: 0, renderedSignalSamples: 0,
    queuedMs: 0, targetMs: 0, jitterMs: 0, underruns: 0, droppedFrames: 0 };
}
export function parseMicrophoneStatus(value: unknown): MicrophoneStatus | undefined {
  if (!value || typeof value !== "object") return;
  const item = value as Record<string, unknown>;
  if (item.allowed === false) return { configured: false, allowed: false, output: "", owned: false, busy: false, status: emptySignal() };
  if (typeof item.configured !== "boolean" || typeof item.allowed !== "boolean" || typeof item.output !== "string"
    || item.output.length > 1024 || (item.owned !== undefined && typeof item.owned !== "boolean")
    || (item.busy !== undefined && typeof item.busy !== "boolean")
    || (item.gainDb !== undefined && (typeof item.gainDb !== "number" || !Number.isInteger(item.gainDb) || item.gainDb < 0 || item.gainDb > 36))) return;
  if (!item.status || typeof item.status !== "object") return;
  const signal = item.status as Record<string, unknown>;
  if (typeof signal.active !== "boolean" || typeof signal.error !== "string" || signal.error.length > 1024) return;
  const read = (key: string, max = Number.MAX_SAFE_INTEGER) => {
    const value = signal[key] ?? 0;
    return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= max ? value : undefined;
  };
  const counters = ["receivedFrames", "receivedSignalFrames", "renderedSignalSamples", "underruns", "droppedFrames"] as const;
  for (const key of counters) if (!Number.isSafeInteger(read(key))) return;
  for (const key of ["queuedMs", "targetMs", "jitterMs"]) if (read(key, 60000) === undefined) return;
  let status: Signal = {
    active: signal.active, error: signal.error,
    receivedFrames: read("receivedFrames")!, receivedSignalFrames: read("receivedSignalFrames")!, renderedSignalSamples: read("renderedSignalSamples")!,
    queuedMs: read("queuedMs")!, targetMs: read("targetMs")!, jitterMs: read("jitterMs")!, underruns: read("underruns")!, droppedFrames: read("droppedFrames")!,
  };
  // Older desktops return a global stream, whose ownership cannot be established.
  if (item.owned !== true) status = emptySignal();
  return { configured: item.configured, allowed: item.allowed, output: item.output, gainDb: item.gainDb as number | undefined,
    owned: item.owned as boolean | undefined, busy: item.busy === true, status };
}

export function useMicrophoneStatus(
  remote: { api(path: string, init?: RequestInit): Promise<Response> },
  open: Readonly<Ref<boolean>>, online: Readonly<Ref<boolean>>, allowed: Readonly<Ref<boolean>>, foreground: Readonly<Ref<boolean>>,
) {
  const data = ref<MicrophoneStatus>(), loading = ref(false), error = ref("");
  const manualRefreshing = ref(false);
  const unsupported = ref(false);
  const denied = ref(false);
  const active = computed(() => open.value && online.value && allowed.value && foreground.value);
  let request: AbortController | undefined, pendingRead: Promise<void> | undefined, disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  function stop() { clearTimeout(timer); request?.abort(); request = undefined; pendingRead = undefined; loading.value = false; manualRefreshing.value = false; }
  function refresh(manual = true): Promise<void> {
    if (!active.value || disposed) return Promise.resolve();
    if (manual) manualRefreshing.value = true;
    if (request) return pendingRead || Promise.resolve();
    clearTimeout(timer); unsupported.value = false;
    const owner = new AbortController(); request = owner;
    loading.value = !data.value;
    const deadline = setTimeout(() => owner.abort(), 4000);
    pendingRead = (async () => {
      try {
        const response = await remote.api("/api/microphone/status", { signal: owner.signal });
        if (request !== owner || !active.value) return;
        if (owner.signal.aborted) throw new Error(t("读取麦克风状态超时，请刷新重试"));
        if (response.status === 403) { denied.value = true; data.value = undefined; error.value = ""; return; }
        if (response.status === 404) { unsupported.value = true; throw new Error(t("当前电脑版本不支持查看麦克风状态，请在电脑端检查。")); }
        const value: unknown = await response.json();
        if (request !== owner || !active.value) return;
        if (owner.signal.aborted) throw new Error(t("读取麦克风状态超时，请刷新重试"));
        if (!response.ok) {
          const reason = value && typeof value === "object" ? (value as { error?: unknown }).error : undefined;
          throw new Error(typeof reason === "string" && reason.length <= 1024 ? reason : t("无法读取麦克风状态"));
        }
        const status = parseMicrophoneStatus(value);
        if (!status) throw new Error(t("电脑返回的麦克风状态无效"));
        if (!status.allowed) { denied.value = true; data.value = undefined; error.value = ""; return; }
        data.value = status;
        denied.value = false;
        error.value = "";
      } catch (failure) {
        if (request !== owner) return;
        error.value = owner.signal.aborted ? t("读取麦克风状态超时，请刷新重试") : errorText(failure, t("无法读取麦克风状态"));
      } finally {
        clearTimeout(deadline);
        if (request === owner) {
          request = undefined; pendingRead = undefined; loading.value = false; manualRefreshing.value = false;
          if (active.value && !disposed && !unsupported.value && !denied.value) timer = setTimeout(() => { void refresh(false); }, error.value ? 5000 : 2000);
        }
      }
    })();
    return pendingRead;
  }
  watch(active, enabled => {
    stop(); data.value = undefined; error.value = ""; unsupported.value = false; denied.value = false;
    if (enabled) void refresh(false);
  }, { immediate: true });
  onScopeDispose(() => { disposed = true; stop(); });
  return { data, loading, manualRefreshing, error, unsupported, denied, refresh };
}

export function microphoneGuidance(
  online: boolean, allowed: boolean, value: MicrophoneStatus | undefined, phase: string, issue: string, warning: string, phoneAllowed = true,
) {
  if (!online) {
    if (phase === "live") return { title: t("遥控连接已断开，麦克风仍在收音"), help: t("可以继续收音或立即停止；声音去向暂时无法查询。") };
    if (phase === "connecting" || phase === "reconnecting") return { title: t("正在连接音频通道"), help: t("可以等待音频连接恢复，或立即停止此次收音。") };
    return { title: t("尚未连接电脑"), help: t("连接电脑后，可以查看声音通路和音频设备配置。") };
  }
  if (!allowed || value?.allowed === false) return { title: t("尚未获得麦克风权限"), help: t("请在电脑端的设备权限中开启「手机麦克风」。") };
  if (value && !value.configured) return { title: t("电脑尚未配置音频设备"), help: t("请在电脑端打开「偏好设置 → 手机麦克风」，在「音频送往」中选择虚拟音频设备，再点击「保存音频设备」。") };
  if (!phoneAllowed) return { title: t("手机尚未允许录音"), help: t("点击「开启持续收音」并允许录音；也可在手机系统的应用权限中允许麦克风。") };
  if (value?.busy) return { title: t("另一台手机正在使用麦克风通道"), help: t("请先停止另一台手机的收音，再返回遥控页按住麦克风。") };
  if (issue || value?.status.error) return { title: t("收音遇到问题"), help: t("检查下方提示。电脑音频问题请在「偏好设置 → 手机麦克风」处理；手机权限或占用问题请在手机系统设置中处理。") };
  if (warning) return { title: t("收音受限"), help: t("检查手机麦克风隐私开关，以及通话或录音应用是否占用麦克风。") };
  if (["connecting", "reconnecting"].includes(phase)) return { title: t("正在连接音频通道"), help: t("连接恢复后会更新状态，点击停止可立即结束此次收音。") };
  if (!value) return { title: t("正在读取麦克风状态…"), help: "" };
  if (value.owned === undefined) return { title: t("电脑已配置音频设备"), help: t("当前电脑版本不提供本机音频详情，请在电脑端查看收音状态。") };
  if (value.owned && value.status.active) {
    const signal = value.status;
    if (!signal.receivedFrames) return { title: t("电脑等待手机音频"), help: t("请对着手机说话，观察状态是否更新。") };
    if (!signal.receivedSignalFrames) return { title: t("已收到音频，暂未检测到声音"), help: t("请对着手机说话；环境安静时这是正常状态。") };
    if (!signal.renderedSignalSamples) return { title: t("电脑已收到声音，等待输出"), help: t("若一直没有输出，请在电脑端重新选择可用的音频设备。") };
    return { title: t("声音已送往电脑音频设备"), help: t("在收音软件中选择对应的录音端；这不代表软件已开始录音。") };
  }
  if (phase === "live") return { title: t("正在确认电脑收音状态"), help: t("手机正在收音，请刷新状态核对电脑是否收到声音。") };
  return { title: t("可以开始收音"), help: t("可开启持续收音，或返回遥控页按住麦克风说话；上滑停留一秒可锁定收音。") };
}

type MicrophoneStartState = {
  online: boolean; allowed: boolean; foreground: boolean; usbExpired: boolean;
  active: boolean; value?: MicrophoneStatus; readError: string; reading: boolean;
};
export function canStartMicrophone(state: MicrophoneStartState) {
  return state.online && state.allowed && state.foreground && !state.usbExpired
    && !state.active && !state.readError && !state.reading
    && state.value?.allowed === true && state.value.configured && !state.value.busy && !state.value.status.active;
}
export function canRestartMicrophone(state: MicrophoneStartState & { phoneAllowed: boolean; issue: string }) {
  return canStartMicrophone(state) && state.phoneAllowed && !!state.issue;
}
