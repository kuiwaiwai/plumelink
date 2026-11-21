<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
import { ArrowLeftRight, Volume1, Volume2, VolumeX, ChevronLeft, ChevronRight, Play, CornerDownLeft, Keyboard, Ellipsis, Maximize2, Minimize2 } from "@lucide/vue";
import { displayMessage, languageOptions, languagePreference, locale, t } from "../../shared/i18n";
import { errorText, formatBytes } from "../../shared/format";
import type { Command } from "../../shared/protocol";
import { androidBridge } from "./native/bridge";
import { useConnection, type RemoteWindow } from "./connection";
import { useFileTransfers } from "./composables/useFileTransfers";
import { useTouchpad } from "./composables/useTouchpad";
import { useMobileSettings } from "./composables/useMobileSettings";
import BottomSheet from "./components/BottomSheet.vue";
import AppNavigation, { type Destination } from "./components/AppNavigation.vue";
import MicrophoneControl from "./components/MicrophoneControl.vue";
import TouchSurface from "./components/TouchSurface.vue";
import SettingsScreen from "./components/SettingsScreen.vue";
import FilesScreen from "./components/FilesScreen.vue";
import KeyboardSheet from "./components/sheets/KeyboardSheet.vue";
import MoreActionsSheet from "./components/sheets/MoreActionsSheet.vue";
import WindowSwitchSheet from "./components/sheets/WindowSwitchSheet.vue";
import ShutdownSheet from "./components/sheets/ShutdownSheet.vue";
import PairSheet from "./components/sheets/PairSheet.vue";
import ConnectionSheet from "./components/sheets/ConnectionSheet.vue";
import HelpSheet from "./components/sheets/HelpSheet.vue";
import { asPointerEvent, heading, row, text, type NativeControl, type NativePointer, type NativePointerMoveBatch } from "./native/ui";
import { useMicrophone } from "./composables/useMicrophone";
import { shortcutIconNames } from "./shortcut-icons";
import "./style.css";

const remote = useConnection();
const { status, message, connectionLabel } = remote;
const settingsConnectionLabel = remote.connectionBadgeLabel;
const online = computed(() => status.value === "connected");
const hasControl = computed(() => remote.controlMode.value === "free" || remote.primary.value);
const canInput = computed(() => online.value && remote.permissions.value.input && hasControl.value);
const canText = computed(() => online.value && remote.permissions.value.text && hasControl.value);
const canFiles = computed(() => online.value && remote.permissions.value.files);
const canPower = computed(() => online.value && remote.permissions.value.power && hasControl.value);

type Panel = "keyboard" | "more" | "settings" | "windows" | "pair" | "shutdown" | "files" | "connection" | "help";
const panel = ref<Panel | null>(null);
const destination = ref<Destination>("remote");
const code = ref("");
const input = ref("");
const sending = ref(false);
const feedback = ref("");
const switcher = ref(false);
const switchExpired = ref(false);
const windowList = ref<RemoteWindow[]>([]);
const windowLoading = ref(false);
const windowError = ref("");
const windowNotice = ref("");
const windowSwitching = ref(0);
const openingTaskView = ref(false);
const switchHelp = computed(() => switcher.value ? t("看着电脑屏幕，连续选择后确认。")
  : switchExpired.value ? t("选择已结束，可继续切换窗口") : t("点击上一个或下一个，继续选择窗口。"));
const magnifier = ref(false);
const lensPending = ref(false);
const shutdownPending = ref(false);
const shutdownSent = ref(false);
const controlRequestPending = remote.controlRequestPending;
const microphoneLocked = ref(false);
const microphoneActive = ref(false);
const textFeedback = ref("");
const pairing = ref(false);
const connectionAddress = location.host;
const keyboardSheet = ref<{ resetComposing?: () => void } | null>(null);

const android = androidBridge();
const {
  sensitivity,
  keepAwake,
  keepAlive,
  haptics,
  theme,
  darkAppearance,
  shortcutA,
  shortcutB,
  shortcutC,
  shortcutIconA,
  shortcutIconB,
  shortcutIconC,
  longPressDrag,
  scrollDirection,
  responseMode,
  padOnRight,
  focusMode,
  controlMode,
  dialSize,
  immersive,
  syncWakeLock,
} = useMobileSettings(android, online, destination, mode => remote.setResponseMode(mode));

const { sharedFiles, filesLoading, filesError, transferBusy, transferCancelling, transferProgress, transferMessage, transferHistory, androidFolder, folderReady, loadFiles, uploadFiles, downloadFile, cancelTransfer, clearTransferHistory } =
  useFileTransfers(remote, canFiles, microphoneActive);

let feedbackTimer: ReturnType<typeof setTimeout>;
let switchTimer: ReturnType<typeof setTimeout>;

function notice(text: string, durationMs = 4500) {
  feedback.value = text;
  clearTimeout(feedbackTimer);
  feedbackTimer = setTimeout(() => { feedback.value = ""; }, durationMs);
}

function clearSwitchTimer() {
  switcher.value = false;
  switchExpired.value = false;
  clearTimeout(switchTimer);
}

const touchpad = useTouchpad({
  canInput,
  online,
  panelOpen: computed(() => panel.value !== null),
  sensitivity,
  longPressDrag,
  scrollDirection,
  sendCommand: command => { void action(command); },
  sendMotion: sample => remote.sendMotion(sample),
  clearRemoteMotion: () => remote.clearMotion(),
  onMotionSent: clearSwitchTimer,
});

async function action(command: Command) {
  if (command.type === "release") {
    touchpad.clearMotion();
    remote.clearMotion();
  } else {
    remote.sendMotion(touchpad.drainMotion());
  }
  if (command.type !== "switch") clearSwitchTimer();
  try {
    await remote.send(command);
    if (haptics.value && command.type !== "move" && command.type !== "scroll") {
      if (android?.haptic) android.haptic();
      else navigator.vibrate?.(10);
    }
    return true;
  } catch (e) {
    if (online.value && command.type !== "release") notice(errorText(e, t("操作失败")));
    return false;
  }
}

function resetInteraction() {
  touchpad.reset();
  clearSwitchTimer();
}

function cancelInteraction() {
  resetInteraction();
  if (canInput.value) void action({ type: "release" });
}

function navigate(next: Destination) {
  if (destination.value === next) {
    if (next === "files" && !filesLoading.value) void loadFiles();
    return;
  }
  cancelInteraction();
  feedback.value = "";
  panel.value = null;
  destination.value = next;
}

watch(destination, next => {
  if (next === "files") void loadFiles();

});

function handleBack() {
  if (android?.renderNativeUi && panel.value) { closePanel(); return true; }
  const dialogs = document.querySelectorAll<HTMLDialogElement>("dialog[open]");
  const active = dialogs[dialogs.length - 1];
  if (active) {
    active.dispatchEvent(new Event("cancel", { cancelable: true }));
    return true;
  }
  if (destination.value !== "remote") { navigate("remote"); return true; }
  if (focusMode.value) { focusMode.value = false; return true; }
  return false;
}

const screenTitle = computed(() => ({ remote: t("遥控"), files: t("文件"), settings: t("设置"), keyboard: t("键盘"), windows: t("切换窗口") })[destination.value]);
const titles = computed(() => ({
  keyboard: t("键盘"),
  more: t("更多操作"),
  settings: t("设置"),
  windows: t("切换窗口"),
  pair: t("连接电脑"),
  shutdown: t("关闭电脑"),
  files: t("文件传输"),
  connection: t("当前电脑"),
  help: t("操作指南"),
}));

async function toggleMagnifier() {
  if (!canInput.value || lensPending.value) return;
  lensPending.value = true;
  const enabled = !magnifier.value;
  try {
    await remote.send({ type: "magnifier", enabled });
    magnifier.value = online.value && enabled;
  } catch (e) {
    notice(errorText(e, t("操作失败")));
  } finally {
    lensPending.value = false;
  }
}

async function shutdown() {
  if (!canPower.value || shutdownPending.value || shutdownSent.value) return;
  shutdownPending.value = true;
  shutdownSent.value = true; // Never automatically retry a destructive request after a lost acknowledgement.
  try {
    await remote.send({ type: "shutdown", confirmed: true });
    notice(t("已请求 Windows 关机；未保存的文件可能阻止关机，请查看电脑。"));
  } catch (e) {
    notice(t("{0}。请先查看电脑，勿重复提交。", errorText(e, t("操作失败"))));
  } finally {
    shutdownPending.value = false;
  }
}

function cycle(actionName: "next" | "previous" | "confirm" | "cancel") {
  switchExpired.value = false;
  switcher.value = actionName === "next" || actionName === "previous";
  clearTimeout(switchTimer);
  const sent = action({ type: "switch", action: actionName });
  if (switcher.value) {
    switchTimer = setTimeout(() => {
      void action({ type: "release" });
      switchExpired.value = true;
    }, 3000);
  }
  return sent;
}

async function sendText() {
  keyboardSheet.value?.resetComposing?.();
  if (!canText.value || sending.value || !input.value) return;
  const value = input.value;
  sending.value = true;
  textFeedback.value = "";
  try {
    await remote.send({ type: "text", text: value });
    if (input.value === value) input.value = "";
    textFeedback.value = t("已发送到电脑当前光标位置");
  } catch (e) {
    textFeedback.value = errorText(e, t("文字发送失败"));
  } finally {
    sending.value = false;
  }
}

let windowRequest: AbortController | undefined;
let windowPageGeneration = 0;
watch([destination, canInput], ([page, allowed]) => {
  windowPageGeneration++;
  windowRequest?.abort();
  windowRequest = undefined;
  windowLoading.value = false;
  windowList.value = [];
  windowError.value = "";
  windowNotice.value = "";
  windowSwitching.value = 0;
  openingTaskView.value = false;
  if (page === "windows" && allowed) void loadWindows();
});

async function loadWindows() {
  if (!canInput.value || windowLoading.value) return;
  const request = new AbortController();
  windowRequest = request;
  windowList.value = [];
  windowLoading.value = true;
  windowError.value = "";
  try {
    const windows = await remote.listWindows(request.signal);
    if (request !== windowRequest) return;
    windowList.value = windows;
    windowNotice.value = "";
  } catch (e) {
    if (request === windowRequest) windowError.value = errorText(e, t("无法获取窗口列表"));
  } finally {
    if (request === windowRequest) {
      windowRequest = undefined;
      windowLoading.value = false;
    }
  }
}

async function switchWindow(entry: RemoteWindow) {
  if (!canInput.value || openingTaskView.value || windowSwitching.value) return;
  const generation = windowPageGeneration;
  windowSwitching.value = entry.hwnd;
  windowNotice.value = t("正在切换…");
  windowError.value = "";
  try {
    await remote.switchWindow(entry.hwnd);
    if (generation !== windowPageGeneration) return;
    windowNotice.value = t("已切换到 {0}", entry.title);
  } catch (e) {
    if (generation !== windowPageGeneration) return;
    const message = errorText(e, t("切换窗口失败"));
    windowNotice.value = "";
    windowError.value = message === "系统拒绝切换到该窗口，请先在电脑上激活任意窗口后重试"
      ? "窗口尚未切到前台，请打开任务视图选择窗口" : message;
    if (message.includes("窗口已关闭")) void loadWindows();
  } finally {
    if (generation === windowPageGeneration) windowSwitching.value = 0;
  }
}

async function openWindowTaskView() {
  if (!canInput.value || windowSwitching.value || openingTaskView.value) return;
  const generation = windowPageGeneration;
  openingTaskView.value = true;
  try {
    const opened = await action({ type: "taskview" });
    if (opened && generation === windowPageGeneration) navigate("remote");
  } finally {
    if (generation === windowPageGeneration) openingTaskView.value = false;
  }
}

function closePanel() {
  cancelInteraction();
  feedback.value = "";
  panel.value = null;
  keyboardSheet.value?.resetComposing?.();
}

function openPanel(name: Panel) {
  if (name === "settings" || name === "files") {
    navigate(name);
    return;
  }
  cancelInteraction();
  feedback.value = "";
  panel.value = name;
  if (name === "windows") cycle("next");
}

function runShortcut(value: string) {
  if (value === "mute") void action({ type: "volume", action: "mute" });
  else if (value === "magnifier") void toggleMagnifier();
  else if (value === "taskview") void action({ type: "taskview" });
  else if (value === "notifications") void action({ type: "notifications" });
  else if (value === "previousapp" || value === "nextapp") void action({ type: "appswitch", direction: value === "previousapp" ? "left" : "right" });
  else if (value === "virtualleft" || value === "virtualright") void action({ type: "virtualdesktop", direction: value === "virtualleft" ? "left" : "right" });
}

function requestShutdown() {
  if (android) android.confirmSensitiveAction("shutdown");
  else void shutdown();
}

async function nativeConfirm(event: Event) {
  const detail = (event as CustomEvent<{ action: string; approved: boolean }>).detail;
  if (detail.action !== "shutdown") return;
  if (!detail.approved) { notice(t("已取消身份验证，未发送关机请求")); return; }
  const approvedToken = remote.token();
  for (let attempt = 0; attempt < 40 && !online.value && approvedToken === remote.token() && panel.value === "shutdown"; attempt++) {
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (approvedToken !== remote.token() || panel.value !== "shutdown") return;
  if (!canPower.value) { notice(t("连接未恢复或权限已变化，未发送关机请求")); return; }
  await shutdown();
}

async function finishSwitch(name: "confirm" | "cancel") {
  await cycle(name);
  if (panel.value === "windows") closePanel();
}

async function pair() {
  if (pairing.value || !/^[0-9]{6}$/.test(code.value) || status.value === "connecting") return;
  pairing.value = true;
  try { await remote.pair(code.value, android?.getDeviceName() || t("Android 设备")); }
  finally { pairing.value = false; }
}

function connectionClick() {
  if (status.value === "unpaired") openPanel("pair");
  else if (!online.value) remote.resume();
  else openPanel("connection");
}

async function requestControl() {
  if (controlRequestPending.value || !online.value || hasControl.value) return;
  try { await remote.requestControl(); }
  catch (error) { notice(errorText(error, t("请求失败"))); }
}

function visibility() {
  resetInteraction();
  if (document.hidden && !(android && keepAlive.value)) remote.suspend();
  else if (panel.value !== "pair") remote.resume();
  void syncWakeLock();
}

function pagehide() {
  resetInteraction();
  if (!(android && keepAlive.value)) remote.suspend();
}

function pageshow() {
  if (document.hidden || pairing.value || panel.value === "pair") return;
  remote.resume();
  if (destination.value === "files" && canFiles.value && !filesLoading.value) void loadFiles();
  if (keepAwake.value && remote.status.value === "connected") android?.setKeepAwake?.(true);
}

watch([online, () => remote.primary.value], ([connected]) => {
  if (!connected) {
    feedback.value = "";
    resetInteraction();
    magnifier.value = false;
    shutdownSent.value = false;
  } else if (panel.value === "pair") {
    closePanel();
  }
  void syncWakeLock();
});

watch(status, value => {
  if (value === "unpaired") openPanel("pair");
});
watch([keepAlive, status], ([enabled]) => {
  remote.setBackgroundAllowed(!!android && enabled);
  android?.setConnectionKeepAlive?.(enabled && !!remote.token());
}, { immediate: true });

const nativeUi = android?.renderNativeUi;
const nativeMic = nativeUi ? useMicrophone({
  get enabled() { return (online.value && remote.permissions.value.microphone) || microphoneLocked.value; },
  get allowed() { return (status.value !== "unpaired" && remote.permissions.value.microphone) || microphoneLocked.value; },
  get activePage() { return destination.value === "remote" && !panel.value; },
}, notice) : undefined;
if (nativeMic) watch(nativeMic.mode, mode => {
  microphoneLocked.value = mode === "locked" || nativeMic.lockedSession.value;
  microphoneActive.value = mode !== "idle";
});

const nativeActions = new Map<string, (value: unknown) => unknown>();
function nativeEvent(event: Event) {
  const detail = (event as CustomEvent<{ id: string; value: unknown }>).detail;
  nativeActions.get(detail.id)?.(detail.value);
}
function nativePointer(event: Event) {
  const point = (event as CustomEvent<NativePointer | NativePointerMoveBatch>).detail;
  if (point.phase === "move-batch") {
    touchpad.moveTogether(point.contacts);
    return;
  }
  const pointer = asPointerEvent(point);
  if (point.phase === "lost") { touchpad.captureLost(pointer); return; }
  if (point.target === "mic" && nativeMic) {
    if (point.phase === "cancel") nativeMic.stop();
    else if (point.phase === "up") nativeMic.up(pointer, true);
    else nativeMic[point.phase](pointer);
  } else if (point.phase === "cancel") touchpad.cancel();
  else touchpad[point.phase](pointer);
}

const nativeChoices = ref("");
const nativeHistoryOpen = ref(false);
const nativeDiagnosticsOpen = ref(false);
watch(panel, value => { if (value !== "connection") nativeDiagnosticsOpen.value = false; });
const editRevisions = { code: ref(0), draft: ref(0) };
function nativeEdit(id: keyof typeof editRevisions, model: { value: string }, label: string): NativeControl {
  const revision = editRevisions[id];
  nativeActions.set(id, value => {
    const update = value as { text?: unknown; revision?: unknown } | null;
    if (typeof update?.text !== "string" || typeof update.revision !== "number" || update.revision <= revision.value) return;
    revision.value = update.revision;
    model.value = update.text;
  });
  return { kind: "edit", id, label, value: model.value, revision: revision.value };
}
if (nativeUi) watchEffect(() => {
  nativeActions.clear();
  nativeActions.set("back-remote", () => navigate("remote"));
  const button = (id: string, label: string, callback: () => unknown, enabled = true, detail?: string): NativeControl => {
    if (enabled) nativeActions.set(id, callback);
    return { kind: "button", id, label, enabled, detail };
  };
  const key = (id: string, label: string, key: Extract<Command, { type: "key" }>["key"]) =>
    button(id, label, () => action({ type: "key", key }), canInput.value);
  const command = (id: string, label: string, cmd: Command) => button(id, label, () => action(cmd), canInput.value);
  const toggle = (id: string, label: string, model: { value: boolean }, detail: string): NativeControl => {
    nativeActions.set(id, value => { model.value = value === true; });
    return { kind: "switch", id, label, value: model.value, detail };
  };
  const controls: NativeControl[] = [];
  const sheet: NativeControl[] = [];
  let sheetTitle = panel.value ? titles.value[panel.value] : "";
  let sheetIcon = "";
  const choice = (id: string, label: string, model: { value: string }, options: { value: string; label: string; icon?: string; description?: string }[]) => {
    const selected = options.find(item => item.value === model.value);
    controls.push({ ...button(id, label, () => { nativeChoices.value = id; }, true, selected?.label), icon: selected?.icon });
    if (nativeChoices.value !== id) return;
    sheetTitle = label;
    if (id.startsWith("icon-")) {
      const slot = slots[Number(id.slice(5))];
      const actionName = shortcuts.find(([value]) => value === slot?.value)?.[1];
      if (actionName && selected) {
        sheetTitle = actionName + " · " + selected.label;
        sheetIcon = selected.icon || "";
      }
    }
    options.forEach(option => sheet.push({ ...button(id + option.value, option.label, () => {
      model.value = option.value;
      nativeChoices.value = "";
    }, true, option.description), icon: option.icon, selected: option.value === model.value }));
  };

  const shortcuts = [
    ["mute", t("静音")], ["magnifier", t("鼠标放大镜")], ["taskview", t("任务视图")],
    ["notifications", t("通知中心")], ["previousapp", t("上一个应用")], ["nextapp", t("下一个应用")],
    ["virtualleft", t("上一个桌面")], ["virtualright", t("下一个桌面")],
  ];
  const slots = [shortcutA, shortcutB, shortcutC];
  const icons = [shortcutIconA, shortcutIconB, shortcutIconC];
  const iconNames = [t("星标"), t("闪电"), t("铃铛"), t("方格"), t("电脑"), t("指针"), t("音量"), t("搜索"), t("切换"), t("向左箭头"), t("向右箭头"), t("图层"), t("键盘"), t("快捷键"), t("麦克风图标"), t("游戏手柄"), t("设置图标"), t("活动"), t("爱心"), t("灯泡")];
  const micLabel = microphoneLocked.value ? t("持续收音") : t("麦克风");
  const english = locale.value === "en";
  const mic = { kind: "mic", id: "mic", label: english ? (microphoneLocked.value ? "Mic on" : "Mic") : micLabel,
    hint: microphoneLocked.value ? t("立即停止麦克风") : t("按住说话，上滑锁定"),
    enabled: (online.value && remote.permissions.value.microphone) || microphoneLocked.value } satisfies NativeControl;
  const microphone = nativeMic && (microphoneLocked.value || destination.value === "remote" && !panel.value) ? {
    expanded: destination.value === "remote" && !panel.value && nativeMic.expanded.value,
    recording: nativeMic.recording.value,
    locked: microphoneLocked.value,
    intent: nativeMic.intent.value,
    progress: Math.round(nativeMic.progress.value * 100),
    level: Math.round(nativeMic.level.value * 100),
    label: nativeMic.label.value,
    detail: displayMessage(nativeMic.warning.value) || (nativeMic.recording.value
      ? t("{0} · 正在收音", nativeMic.duration.value) : t("连接成功后开始收音")),
    compactLabel: nativeMic.phase.value === "reconnecting" ? t("正在重连…") : nativeMic.phase.value === "connecting"
      ? t("连接中") : nativeMic.warning.value ? t("收音受限") : t("收音中"),
    stopLabel: t("立即停止麦克风"),
    meterLabel: t("实时麦克风音量"),
    duration: nativeMic.duration.value,
    warning: !!nativeMic.warning.value,
  } : null;
  nativeActions.set("mic", value => {
    if (typeof value === "boolean") nativeMic?.keyboard({ key: " ", preventDefault() {} } as KeyboardEvent, value);
    else if (microphoneLocked.value) nativeMic?.stop();
  });
  if (microphoneLocked.value) nativeActions.set("mic-stop", () => nativeMic?.stop());

  if (destination.value === "remote") {
    if (online.value && !hasControl.value) controls.push(button("request", controlRequestPending.value ? t("等待电脑确认") : t("请求接管"), requestControl, !controlRequestPending.value && remote.permissions.value.input));
    const changeComputer = () => android?.changeComputer();
    if (!online.value && focusMode.value) controls.push(button("offline-change", t("管理与切换电脑"), changeComputer));
    const sessionAction = online.value || focusMode.value
      ? { ...button("focus", focusMode.value ? t("显示导航") : t("专注遥控"), () => { focusMode.value = !focusMode.value; }),
          icon: focusMode.value ? "focus-in" : "focus" }
      : button("offline-change", t("管理与切换电脑"), changeComputer);
    const directional = controlMode.value === "dpad";
    const swapLabel = directional
      ? (padOnRight.value ? t("方向盘移到左侧") : t("方向盘移到右侧"))
      : (padOnRight.value ? t("触控板移到左侧") : t("触控板移到右侧"));
    controls.push(row("session", sessionAction,
      { ...button("pad-side", swapLabel, () => { padOnRight.value = !padOnRight.value; }), icon: "side", iconOnly: directional }));
    if (directional) {
      controls.push({ kind: "dpad", id: "dpad", value: dialSize.value, enabled: canInput.value, children: [
        key("dpad-up", t("上"), "up"), key("dpad-right", t("右"), "right"),
        key("dpad-down", t("下"), "down"), key("dpad-left", t("左"), "left"),
        key("dpad-center", t("确认"), "enter"),
      ] });
    } else {
      // The native surface exposes a click action to accessibility services.
      if (canInput.value) nativeActions.set("pad", () => action({ type: "click", button: "left", count: 1 }));
      controls.push({ kind: "pad", id: "pad", label: t("触控板"),
        detail: touchpad.dragLocked.value ? t("松手结束拖拽") : t("滑动，移动指针"), enabled: canInput.value,
        status: touchpad.dragLocked.value ? t("拖拽中") : canInput.value ? t("可操作") : !online.value ? t("等待连接") : !remote.permissions.value.input ? t("未获输入权限") : t("当前无主控权"),
        selected: touchpad.touching.value,
        hint: touchpad.dragLocked.value ? "" : [t("轻点单击"), t("双指滚动"), t("长按拖拽")].join(" · ") });
      controls.push(row("clicks", ...(["left", "middle", "right"] as const).map((name, i) =>
        command(name, [t("左键"), t("中键"), t("右键")][i], { type: "click", button: name, count: 1 }))));
    }
    controls.push(row("volume", command("quieter", t("音量减"), { type: "volume", action: "down" }),
      command("mute", t("静音"), { type: "volume", action: "mute" }), command("louder", t("音量加"), { type: "volume", action: "up" })));
    controls.push(row("playback", key("prev", t("后退"), "left"), key("play", t("播放/暂停"), "space"), key("next", t("前进"), "right")));
    controls.push(row(directional ? "handset-tools" : "tools",
      directional ? { ...key("dpad-esc", t("返回"), "escape"), icon: "back", iconOnly: true } : key("enter", t("回车"), "enter"),
      { ...button("keyboard", english ? "Keys" : t("键盘"), () => navigate("keyboard"), canText.value), status: t("键盘"), iconOnly: directional },
      { ...mic, iconOnly: directional },
      { ...button("more", t("更多"), () => openPanel("more"), canInput.value), iconOnly: directional }));
  } else if (destination.value === "files") {

    controls.push(heading("receive", t("Android 接收目录")), text("folder", androidFolder.value),
      button("pick-folder", t("选择目录"), () => android?.chooseReceiveFolder(), !transferBusy.value));
    if (transferBusy.value) {
      const progress = Math.round(transferProgress.value);
      const stage = transferCancelling.value ? t("正在取消传输") : progress === 0 ? t("正在准备传输") : progress === 100 ? t("正在确认完成") : t("正在传输文件");
      controls.push({ kind: "column", id: "transfer-card", children: [
        heading("transfer-stage", stage + " · " + progress + "%"),
        { kind: "progress", id: "progress", value: progress, label: t("文件传输进度") },
        text("transfer-detail", displayMessage(transferMessage.value) || t("正在与电脑建立传输连接…")),
        button("cancel-transfer", transferCancelling.value ? t("取消中…") : t("取消传输"), cancelTransfer, !transferCancelling.value),
      ] });
    }
    controls.push(button("upload", t("发送文件到 Windows"), () => android?.chooseUploadFiles?.(),
      canFiles.value && !transferBusy.value, t("单文件最大 2 GB")));
    if (transferMessage.value && !transferBusy.value) controls.push(text("transfer", displayMessage(transferMessage.value)));
    if (transferHistory.value.length) {
      controls.push({ ...button("history-toggle", t("最近传输"), () => { nativeHistoryOpen.value = !nativeHistoryOpen.value; }),
        icon: nativeHistoryOpen.value ? "up" : "down" });
      if (nativeHistoryOpen.value) controls.push(...transferHistory.value.map((entry, i) => text("history-" + i, entry)),
        button("clear-history", t("清空记录"), clearTransferHistory));
    }
    if (!canFiles.value) controls.push(text("files-status", t("连接电脑并获得文件权限后，即可开始传输。")));
    else controls.push(heading("shared", t("Windows 共享的文件")),
      button("reload", t("刷新"), loadFiles, canFiles.value && !transferBusy.value && !filesLoading.value));
    if (canFiles.value && filesError.value) controls.push(text("files-status", displayMessage(filesError.value)),
      button("files-retry", filesLoading.value ? t("读取中…") : t("重试"), loadFiles, !transferBusy.value && !filesLoading.value));
    if (canFiles.value && !sharedFiles.value.length && (!filesError.value || filesLoading.value)) {
      controls.push({ kind: "text", id: "files-empty", icon: "folder", label: filesLoading.value ? t("正在读取共享文件…") : t("电脑文件会显示在这里"),
        detail: filesLoading.value ? t("请稍候。") : t("在电脑端选择要共享的文件，即可保存到手机。") });
    }
    if (canFiles.value) sharedFiles.value.forEach(file => controls.push(row("file-" + file.id,
      { kind: "text", id: "file-name-" + file.id, label: file.name, detail: formatBytes(file.size) },
      { ...button("save-" + file.id, t("保存"), () => downloadFile(file), folderReady.value && !transferBusy.value),
        status: t("保存 {0}", file.name) })));
  } else if (destination.value === "keyboard") {
    nativeActions.set("send", () => void sendText());
    const compose: NativeControl[] = [{ kind: "column", id: "keyboard-entry", children: [
      nativeEdit("draft", input, t("输入文字，发送到电脑…")),
      row("draft-meta", text("clipboard", t("不读取或修改剪贴板")), text("character-count", input.value.length + " / 1000"),
        { ...button("send", sending.value ? t("发送中…") : t("发送文字"), sendText, canText.value && !sending.value && !!input.value), icon: "send", iconOnly: true }),
    ] }];
    if (!online.value) compose.push(text("draft-offline", connectionLabel.value + t("，草稿已保留。")));
    if (textFeedback.value) compose.push(text("text-feedback", displayMessage(textFeedback.value)));
    controls.push({ kind: "column", id: "keyboard-compose", children: compose });
    const shortcuts: NativeControl[] = [heading("text-heading", t("快捷键"))];
    const keys = [["escape", "Esc"], ["tab", "Tab"], ["backspace", t("退格")], ["delete", "Del"], ["enter", t("回车")], ["home", "Home"], ["end", "End"], ["pageup", "PgUp"], ["pagedown", "PgDn"], ["f5", "F5"], ["f11", "F11"]] as const;
    shortcuts.push({ kind: "strip", id: "keys", children: keys.map(([value, label]) => key("key-" + value, label, value)) });
    const combos = [["a", t("全选")], ["c", t("复制")], ["v", t("粘贴")], ["x", t("剪切")], ["z", t("撤销")], ["f", t("查找")], ["w", t("关闭页")]] as const;
    shortcuts.push({ kind: "strip", id: "combos", children: combos.map(([value, label]) => ({
      ...command("combo-" + value, "Ctrl+" + value.toUpperCase(), { type: "combo", modifiers: ["ctrl"], key: value }), status: label })) });
    controls.push({ kind: "column", id: "keyboard-shortcuts", children: shortcuts });
  } else if (destination.value === "windows") {
    if (!windowError.value && canInput.value) controls.push(row("window-actions", text("window-actions-spacer", ""),
      { ...button("window-refresh", windowLoading.value ? t("读取中…") : t("刷新"), () => void loadWindows(), canInput.value && !windowLoading.value && !windowSwitching.value), icon: "refresh", iconOnly: true },
      { ...button("taskview", t("任务视图"), () => void openWindowTaskView(), !openingTaskView.value && !windowSwitching.value), icon: "taskview", iconOnly: true },
    ));
    if (!canInput.value) {
      controls.push(text("windows-offline", t("连接电脑并获得遥控权限后，可以切换窗口。")));
    } else {
      if (windowError.value) {
        const unsupported = windowError.value.includes("窗口枚举不可用");
        controls.push(text("windows-error", unsupported ? t("这台电脑暂时无法提供窗口列表") : displayMessage(windowError.value)),
          row("window-recovery", ...(windowList.value.length ? [] : [button("window-retry", t("重试"), () => void loadWindows(), !windowLoading.value)]),
            button("open-taskview", t("打开任务视图"), () => void openWindowTaskView(), !openingTaskView.value && !windowSwitching.value)));
      }
      else if (windowNotice.value) controls.push(text("windows-notice", windowNotice.value));
      if (windowLoading.value && !windowList.value.length) controls.push(text("windows-loading", t("正在读取窗口列表…")));
      if (!windowLoading.value && !windowList.value.length && !windowError.value) controls.push(text("windows-empty", t("电脑上没有可切换的窗口")));
      windowList.value.forEach(entry => {
        const id = "window-" + entry.hwnd;
        nativeActions.set(id, () => void switchWindow(entry));
        controls.push({
          kind: "window", id, label: entry.title,
          detail: entry.app,
          icon: entry.icon ?? "monitor",
          status: windowSwitching.value === entry.hwnd ? t("正在切换…") : entry.minimized ? t("已最小化") : "",
          enabled: canInput.value && !windowSwitching.value && !openingTaskView.value,
        });
      });
    }
  } else {
    controls.push(heading("touch-heading", t("触控与反馈")));
    choice("control-mode", t("操控形式"), controlMode, [
      { value: "trackpad", label: t("触控板"), description: t("滑动移动指针，轻点单击") },
      { value: "dpad", label: t("方向遥控"), description: t("圆形方向盘，适合菜单导航") },
    ]);
    if (controlMode.value === "dpad") choice("dial-size", t("方向盘大小"), dialSize, [
      { value: "small", label: t("小号") }, { value: "standard", label: t("标准") }, { value: "large", label: t("大号") },
    ]);
    controls.push({ kind: "slider", id: "sensitivity", label: t("指针灵敏度"), value: sensitivity.value, hint: t("精准"), detail: t("快速") });
    nativeActions.set("sensitivity", value => { if (typeof value === "number") sensitivity.value = value; });
    choice("response", t("触控响应"), responseMode, [
      { value: "balanced", label: t("均衡"), description: t("兼顾响应与耗电，适合日常使用") },
      { value: "responsive", label: t("低延迟"), description: t("提高采样频率，耗电略有增加") },
    ]);
    controls.push(toggle("drag", t("长按拖拽"), longPressDrag, t("按住片刻后移动，松手结束")));
    choice("scroll-direction", t("滚动逻辑"), scrollDirection, [
      { value: "follow", label: t("与手指方向一致") },
      { value: "opposite", label: t("与手指方向相反") },
    ]);
    controls.push(toggle("haptics", t("触觉反馈"), haptics, t("操作成功后轻触反馈")), heading("display", t("显示")));
    choice("language", t("语言"), languagePreference, languageOptions.value);
    choice("theme", t("外观"), theme, [{ value: "system", label: t("跟随系统") }, { value: "light", label: t("浅色") }, { value: "dark", label: t("深色") }]);
    controls.push(toggle("awake", t("保持屏幕常亮"), keepAwake, t("仅在前台连接电脑时生效")),
      toggle("alive", t("应用保活"), keepAlive, t("连接电脑时在后台保持运行，并显示常驻通知")),
      button("magnify", t("鼠标放大镜"), toggleMagnifier, canInput.value && !lensPending.value, magnifier.value ? t("已开启") : t("已关闭")),
      heading("shortcuts", t("快捷操作")));
    slots.forEach((slot, i) => {
      choice("slot-" + i, [t("快捷操作一"), t("快捷操作二"), t("快捷操作三")][i], slot,
        shortcuts.filter(([value]) => value === slot.value || !slots.some(other => other.value === value)).map(([value, label]) => ({ value, label })));
      choice("icon-" + i, [t("图标一"), t("图标二"), t("图标三")][i], icons[i],
        shortcutIconNames.map((value, index) => ({ value, label: iconNames[index], icon: value })));
    });
    controls.push(text("shortcut-tip", t("在遥控页的“更多”中使用。")), heading("computer-help", t("电脑与帮助")), button("connection", t("当前电脑"), () => openPanel("connection"), true, settingsConnectionLabel.value),
      button("help", t("手势与操作指南"), () => openPanel("help")), button("privacy", t("隐私政策"), () => android?.showPrivacyPolicy?.()), button("shutdown", t("关闭电脑"), () => openPanel("shutdown"), canPower.value));
    controls.push({ ...text("settings-footnote", t("羽连 PlumeLink")), detail: t("让控制触手可及") });
  }

  if (panel.value === "pair") {
    sheet.push(heading("pair-name", android?.getComputerName?.() || t("Windows 电脑")), text("address", connectionAddress),
      button("scan", t("扫描电脑二维码"), () => android?.scanPairingCode?.(), !pairing.value),
      text("pair-instructions", t("输入电脑上显示的 6 位配对码")), nativeEdit("code", code, t("6 位配对码")),
      text("pair-help", displayMessage(message.value)),
      button("pair", pairing.value ? t("正在配对…") : t("配对连接"), pair, !pairing.value && status.value !== "connecting" && /^[0-9]{6}$/.test(code.value)),
      button("pair-change", t("管理与切换电脑"), () => android?.changeComputer()));
  } else if (panel.value === "keyboard") {
    const keys = [["escape", "Esc"], ["tab", "Tab"], ["backspace", t("退格")], ["delete", "Del"], ["enter", t("回车")], ["home", "Home"], ["end", "End"], ["pageup", "PgUp"], ["pagedown", "PgDn"], ["f5", "F5"], ["f11", "F11"]] as const;
    sheet.push({ kind: "strip", id: "keys", children: keys.map(([value, label]) => key("key-" + value, label, value)) });
    const combos = [["a", t("全选")], ["c", t("复制")], ["v", t("粘贴")], ["x", t("剪切")], ["z", t("撤销")], ["f", t("查找")], ["w", t("关闭页")]] as const;
    sheet.push({ kind: "strip", id: "combos", children: combos.map(([value, label]) => ({
      ...command("combo-" + value, "Ctrl+" + value.toUpperCase(), { type: "combo", modifiers: ["ctrl"], key: value }), status: label })) });
    sheet.push(text("draft-label", t("输入到电脑当前光标位置")), nativeEdit("draft", input, t("输入文字，发送到电脑…")),
      row("draft-meta", text("clipboard", t("不读取或修改剪贴板")), text("character-count", input.value.length + " / 1000")));
    if (!online.value) sheet.push(text("draft-offline", connectionLabel.value + t("，草稿已保留。")));
    if (textFeedback.value) sheet.push(text("text-feedback", displayMessage(textFeedback.value)));
    sheet.push(button("send", sending.value ? t("发送中…") : t("发送文字"), sendText, canText.value && !sending.value && !!input.value));
  } else if (panel.value === "more") {
    sheet.push(row("more-keys", key("esc", "Esc", "escape"), command("desktop", t("显示桌面"), { type: "desktop" }), key("backspace", t("删除"), "backspace")),
      row("more-actions", command("scroll-up", t("向上滚动"), { type: "scroll", dy: -120 }), command("scroll-down", t("向下滚动"), { type: "scroll", dy: 120 }), button("windows", t("切换窗口"), () => navigate("windows"), canInput.value)),
      row("more-slots", ...slots.map((slot, i) => ({ ...button("run-" + i, shortcuts.find(([value]) => slot.value === value)?.[1] || "", () => runShortcut(slot.value), canInput.value), icon: icons[i].value }))));
  } else if (panel.value === "windows") {
    sheet.push(text("switch-help", switchHelp.value),
      row("window-cycle", button("window-prev", t("上一个"), () => cycle("previous"), canInput.value), button("window-next", t("下一个"), () => cycle("next"), canInput.value)),
      row("window-finish", button("window-cancel", t("取消选择"), () => finishSwitch("cancel"), canInput.value),
        button("window-confirm", t("确认窗口"), () => finishSwitch("confirm"), canInput.value && switcher.value)),
      text("switch-timeout", t("选择时保持 Alt；停止操作 3 秒自动释放。")));
  } else if (panel.value === "shutdown") {
    sheet.push(text("power-help", t("让 Windows 正常关闭应用（包括羽连）并关机。未保存的文件可能阻止关机，请先保存重要内容。")),
      text("power-detail", t("不会强制结束应用。如果取消了 Windows 关机，羽连仍可继续使用。")),
      button("power", shutdownSent.value ? t("已提交，请查看电脑") : t("验证身份并关机"), requestShutdown, canPower.value && !shutdownPending.value && !shutdownSent.value),
      button("power-back", t("返回遥控"), closePanel));
  } else if (panel.value === "connection") {
    sheet.push({ kind: "column", id: "connection-hero", children: [
      { kind: "heading", id: "pc-name", label: android?.getComputerName?.() || t("Windows 电脑"), icon: "monitor" },
      text("pc-status", displayMessage(message.value)), text("address", connectionAddress),
    ] }, { kind: "column", id: "connection-metrics", children: [
      row("quality-row", { kind: "text", id: "quality", label: t("网络质量"), detail: remote.qualityLabel.value },
        text("connection-latency", remote.latency.value === null ? "—" : remote.latency.value + " ms")),
      { kind: "text", id: "permission", label: t("控制权限"),
        detail: !online.value ? t("等待连接") : (hasControl.value ? t("可以控制") : t("当前由{0}主控", remote.primaryName.value || t("另一台设备"))) + (remote.adminGranted.value ? t(" · 管理员控制已开启") : "") },
    ] },
      ...(!online.value ? [button("reconnect", status.value === "unpaired" ? t("输入配对码") : t("重新连接"), () => {
        if (status.value === "unpaired") openPanel("pair");
        else remote.resume();
      })] : []),
      button("change", t("管理与切换电脑"), () => android?.changeComputer()),
      { ...button("diagnostics", t("连接详情"), () => { nativeDiagnosticsOpen.value = !nativeDiagnosticsOpen.value; }),
        icon: nativeDiagnosticsOpen.value ? "down" : "next" });
    if (nativeDiagnosticsOpen.value) {
      const permission = (allowed: boolean) => allowed ? t("允许") : t("已关闭");
      sheet.push(text("transport", t("QFastTransmission · 发送间隔") + " " + remote.transportStats.value.intervalMs + " " + t("ms · 电脑排队") + " " + remote.transportStats.value.queueMs + " ms"),
        text("permissions", [t("权限：遥控"), permission(remote.permissions.value.input), t("· 文字"), permission(remote.permissions.value.text), t("· 文件"), permission(remote.permissions.value.files)].join(" ")));
    }
  } else if (panel.value === "help") {
    sheet.push(heading("one", t("一根手指")), text("one-detail", t("滑动移动指针，轻点单击，连续轻点双击。按住片刻后移动可拖拽，松手结束。")),
      heading("two", t("两根手指")), text("two-detail", t("上下或左右滑动滚动页面，轻点右键，捏合或张开缩放。")),
      heading("shortcuts-help", t("更多快捷操作")), text("shortcuts-detail", t("在设置中选择三个快捷操作，再到遥控页的“更多”中使用。")),
      heading("input-help", t("输入与播放")), text("input-detail", t("先在电脑上选中输入位置，再发送文字。播放按钮发送空格键，前后按钮发送方向键，效果取决于当前应用。")));
  }
  if (sheetTitle && feedback.value) sheet.push(text("sheet-feedback", displayMessage(feedback.value)));
  nativeActions.set("dismiss", () => { if (nativeChoices.value) nativeChoices.value = ""; else closePanel(); });
  const keepDialPosition = immersive.value && destination.value === "remote" && controlMode.value === "dpad";
  const nav = ((immersive.value && !keepDialPosition) || destination.value === "keyboard" || destination.value === "windows") ? [] : ([
    ["remote", t("遥控")], ["files", t("文件")], ["settings", t("设置")],
  ] as const).map(([id, label]) => button("nav-" + id, label, () => navigate(id), destination.value !== id));
  const connection = { ...button("connection-chip", connectionLabel.value, () => openPanel(status.value === "unpaired" ? "pair" : "connection")),
    status: online.value ? remote.quality.value : status.value,
    hint: remote.connectionBadgeLabel.value,
    detail: online.value && remote.latency.value !== null ? remote.latency.value + " ms" : "" };
  android?.renderNativeUi?.(JSON.stringify({
    destination: destination.value, side: padOnRight.value, title: screenTitle.value, sessionStatus: status.value,
    subtitle: android?.getComputerName?.(), connection, controls, nav, keepDialPosition, feedback: displayMessage(feedback.value),
    dark: darkAppearance.value, microphone,
    sheetKey: nativeChoices.value || panel.value, sheetTitle, sheetIcon, sheet,
  }));
});

onMounted(() => {
  window.addEventListener("plumelink-native-ui", nativeEvent);
  window.addEventListener("plumelink-native-pointer", nativePointer);
  (window as Window & { plumeLinkBack?: () => boolean }).plumeLinkBack = handleBack;
  const scannedCode = android?.takePairingCode?.() || "";
  if (/^[0-9]{6}$/.test(scannedCode)) {
    code.value = scannedCode;
    openPanel("pair");
    void pair();
  } else {
    remote.resume();
  }
  if (status.value === "unpaired") openPanel("pair");
  document.addEventListener("visibilitychange", visibility);
  window.addEventListener("pagehide", pagehide);
  window.addEventListener("pageshow", pageshow);
  window.addEventListener("plumelink-native-confirm", nativeConfirm);
});

onUnmounted(() => {
  windowPageGeneration++;
  windowRequest?.abort();
  windowRequest = undefined;
  window.removeEventListener("plumelink-native-ui", nativeEvent);
  window.removeEventListener("plumelink-native-pointer", nativePointer);
  delete (window as Window & { plumeLinkBack?: () => boolean }).plumeLinkBack;
  resetInteraction();
  remote.suspend();
  clearTimeout(feedbackTimer);
  document.removeEventListener("visibilitychange", visibility);
  window.removeEventListener("pagehide", pagehide);
  window.removeEventListener("pageshow", pageshow);
  window.removeEventListener("plumelink-native-confirm", nativeConfirm);
});
</script>

<template>
  <main v-if="!nativeUi" class="app-shell" :class="{ immersive }" @contextmenu.prevent>
    <header class="app-bar">
      <div><h1>{{ screenTitle }}</h1><span class="app-subtitle">{{ android?.getComputerName?.() || t('羽连') }}</span></div>
      <button class="connection" :class="{ connected: online, [`quality-${remote.quality.value}`]: online }" :aria-label="connectionLabel" @click="connectionClick">
        <span class="dot" :class="{ live: online }" aria-hidden="true"></span>
        <span>{{ remote.connectionBadgeLabel.value }}</span>
        <small v-if="online && remote.latency.value !== null">{{ remote.latency.value }} ms</small>
        <ChevronRight aria-hidden="true" />
      </button>
    </header>
    <div id="microphone-status" class="microphone-status-host"></div>
    <div v-show="destination === 'remote' || microphoneLocked" class="session-tools">
      <button v-if="destination === 'remote'" class="pad-side-toggle" @click="padOnRight = !padOnRight">
        <ArrowLeftRight aria-hidden="true" /><span>{{ padOnRight ? t('触控板移到左侧') : t('触控板移到右侧') }}</span>
      </button>
      <button v-if="destination === 'remote'" class="focus-toggle" :aria-label="focusMode ? t('显示导航') : t('隐藏导航')" :aria-pressed="focusMode" @click="focusMode = !focusMode">
        <Minimize2 v-if="focusMode" aria-hidden="true" /><Maximize2 v-else aria-hidden="true" /><span>{{ focusMode ? t('显示导航') : t('专注遥控') }}</span>
      </button>
    </div>
    <div class="app-content">
      <section v-show="destination === 'remote'" class="remote-view" :class="{ 'pad-on-right': padOnRight }" :aria-label="t('遥控')">
        <div v-if="!online || !hasControl" class="inline-notice">
          <span>{{ online ? t('当前由{0}主控', remote.primaryName.value || t('另一台设备')) : displayMessage(message) }}</span>
          <button v-if="online" :disabled="controlRequestPending || !remote.permissions.value.input" @click="requestControl">{{ controlRequestPending ? t('等待电脑确认') : t('请求接管') }}</button>
          <button v-else @click="connectionClick">{{ status === 'unpaired' ? t('配对') : t('重试') }}</button>
        </div>
        <TouchSurface
          :enabled="canInput"
          :unavailable-reason="!online ? t('等待连接') : !remote.permissions.value.input ? t('未获输入权限') : t('当前无主控权')"
          :touching="touchpad.touching.value"
          :dragging="touchpad.dragLocked.value"
          @down="touchpad.down"
          @move="touchpad.move"
          @up="touchpad.up"
          @cancel="cancelInteraction"
          @lost="touchpad.captureLost"
          @click="button => action({ type: 'click', button, count: 1 })"
        />
        <div class="media-controls">
          <section class="volume-row" :aria-label="t('音量操作')">
            <button :disabled="!canInput" :aria-label="t('音量减')" @click="action({ type: 'volume', action: 'down' })"><Volume1 aria-hidden="true" /></button>
            <button :disabled="!canInput" :aria-label="t('静音')" @click="action({ type: 'volume', action: 'mute' })"><VolumeX aria-hidden="true" /></button>
            <button :disabled="!canInput" :aria-label="t('音量加')" @click="action({ type: 'volume', action: 'up' })"><Volume2 aria-hidden="true" /></button>
          </section>
          <section class="play-row" :aria-label="t('播放快捷键')">
            <button :disabled="!canInput" :aria-label="t('后退')" @click="action({ type: 'key', key: 'left' })"><ChevronLeft aria-hidden="true" /></button>
            <button class="play-button accent" :disabled="!canInput" :aria-label="t('播放/暂停')" @click="action({ type: 'key', key: 'space' })"><Play aria-hidden="true" /></button>
            <button :disabled="!canInput" :aria-label="t('前进')" @click="action({ type: 'key', key: 'right' })"><ChevronRight aria-hidden="true" /></button>
          </section>
        </div>
        <div class="remote-tools" role="group" :aria-label="t('遥控工具')">
          <button :disabled="!canInput" @click="action({ type: 'key', key: 'enter' })"><CornerDownLeft aria-hidden="true" /><span>{{ t('回车') }}</span></button>
          <button :disabled="!canText" @click="openPanel('keyboard')"><Keyboard aria-hidden="true" /><span>{{ t('键盘') }}</span><i v-if="input" class="draft-dot" :aria-label="t('有未发送草稿')"></i></button>
          <MicrophoneControl :enabled="(online && remote.permissions.value.microphone) || microphoneLocked" :allowed="(status !== 'unpaired' && remote.permissions.value.microphone) || microphoneLocked" :active-page="destination === 'remote' && !panel" @locked="microphoneLocked = $event" @active="microphoneActive = $event" @notice="notice" />
          <button :disabled="!canInput" @click="openPanel('more')"><Ellipsis aria-hidden="true" /><span>{{ t('更多') }}</span></button>
        </div>
      </section>
      <FilesScreen v-show="destination === 'files'" :native="!!android" :can-files="canFiles" :android-folder="androidFolder" :folder-ready="folderReady" :shared-files="sharedFiles" :files-loading="filesLoading" :files-error="filesError" :transfer-busy="transferBusy" :transfer-cancelling="transferCancelling" :transfer-progress="transferProgress" :transfer-message="transferMessage" :transfer-history="transferHistory" @folder="android?.chooseReceiveFolder()" @upload="uploadFiles" @refresh="loadFiles" @download="downloadFile" @cancel="cancelTransfer" @clear-history="clearTransferHistory" />
      <SettingsScreen v-show="destination === 'settings'" :native="!!android" v-model:sensitivity="sensitivity" v-model:keep-awake="keepAwake" v-model:keep-alive="keepAlive" v-model:haptics="haptics" v-model:long-press-drag="longPressDrag" v-model:scroll-direction="scrollDirection" v-model:theme="theme" v-model:response-mode="responseMode" v-model:shortcut-a="shortcutA" v-model:shortcut-b="shortcutB" v-model:shortcut-c="shortcutC" v-model:shortcut-icon-a="shortcutIconA" v-model:shortcut-icon-b="shortcutIconB" v-model:shortcut-icon-c="shortcutIconC" :can-input="canInput" :can-power="canPower" :magnifier="magnifier" :lens-pending="lensPending" :connection-label="settingsConnectionLabel" @connection="openPanel('connection')" @magnify="toggleMagnifier" @help="openPanel('help')" @shutdown="openPanel('shutdown')" />
    </div>
    <AppNavigation v-show="!immersive" :active="destination" :transferring="transferBusy" @navigate="navigate" />
    <div v-if="feedback && !panel" class="snackbar" role="status">{{ displayMessage(feedback) }}</div>
  </main>
  <BottomSheet v-if="!nativeUi" :open="panel !== null" :title="panel ? titles[panel] : ''" @close="closePanel">
    <KeyboardSheet v-if="panel === 'keyboard'" ref="keyboardSheet" v-model:input="input" :can-input="canInput" :can-text="canText" :online="online" :sending="sending" :connection-label="connectionLabel" :text-feedback="textFeedback" @command="action" @submit="sendText" />
    <MoreActionsSheet v-else-if="panel === 'more'" :can-input="canInput" :shortcut-a="shortcutA" :shortcut-b="shortcutB" :shortcut-c="shortcutC" :shortcut-icon-a="shortcutIconA" :shortcut-icon-b="shortcutIconB" :shortcut-icon-c="shortcutIconC" @command="action" @shortcut="runShortcut" @windows="panel = 'windows'" />
    <WindowSwitchSheet v-else-if="panel === 'windows'" :can-input="canInput" :hint="switchHelp" @cycle="cycle" @finish="finishSwitch" />
    <ShutdownSheet v-else-if="panel === 'shutdown'" :can-power="canPower" :shutdown-pending="shutdownPending" :shutdown-sent="shutdownSent" @shutdown="requestShutdown" @close="closePanel" />
    <PairSheet v-else-if="panel === 'pair'" v-model:code="code" :computer-name="android?.getComputerName?.() || t('Windows 电脑')" :connection-address="connectionAddress" :has-scanner="!!android?.scanPairingCode" :pairing="pairing" :status="status" :message="message" @scan="android?.scanPairingCode?.()" @submit="pair" />
    <ConnectionSheet v-else-if="panel === 'connection'" :computer-name="android?.getComputerName?.() || t('Windows 电脑')" :connection-address="connectionAddress" :message="message" :quality-label="remote.qualityLabel.value" :latency="remote.latency.value" :has-control="hasControl" :primary-name="remote.primaryName.value" :admin-granted="remote.adminGranted.value" :online="online" :status="status" :has-native="!!android" :transport-stats="remote.transportStats.value" :permissions="remote.permissions.value" @open-pair="panel = 'pair'" @resume="remote.resume()" @change-computer="android?.changeComputer()" @forget="remote.forget(); panel = 'pair'" />
    <HelpSheet v-else-if="panel === 'help'" />
    <p v-if="feedback" class="panel-feedback" role="status">{{ displayMessage(feedback) }}</p>
  </BottomSheet>
</template>
