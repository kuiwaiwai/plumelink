<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
import { displayMessage, languageOptions, languagePreference, locale, t } from "../../shared/i18n";
import { errorText, formatBytes } from "../../shared/format";
import type { Destination } from "./native/navigation";
import type { Command } from "../../shared/protocol";
import { androidBridge } from "./native/bridge";
import { useConnection } from "./connection";
import { useWindowPicker } from "./composables/useWindowPicker";
import { useConnectionRecovery } from "./composables/useConnectionRecovery";
import { downloadRecoveryLabel, downloadRecoveryStatus, useFileTransfers } from "./composables/useFileTransfers";
import { useSharedFiles, type SharedSelectionFile } from "./composables/useSharedFiles";
import { useTouchpad } from "./composables/useTouchpad";
import { DirectionalRepeat, type Direction } from "./directional-repeat";
import { useMobileSettings } from "./composables/useMobileSettings";
import { useCustomShortcut } from "./composables/useCustomShortcut";
import { useShortcutPlans } from "./composables/useShortcutPlans";
import { builtinPlan, matchingSlots, maxShortcutPlans } from "./shortcut-plans";
import { keyboardModifiers } from "../../shared/keyboard";
import { parseCustomShortcut, chordCommand, chordLabel, keyGroups, keyLabel } from "./custom-shortcuts";
import { asPointerEvent, heading, recoveryActionId, row, text, type NativeControl, type NativePointer, type NativePointerMoveBatch } from "./native/ui";
import { bindSheetActions, bindScreenActions, dispatchNativeAction, currentNativeInput } from "./native/actions";
import { useMicrophone } from "./composables/useMicrophone";
import { canStartMicrophone, canRestartMicrophone, microphoneGuidance, useMicrophoneStatus } from "./composables/useMicrophoneStatus";
import { shortcuts as shortcutCatalog, shortcutOptions, shortcutPresets, presetOptions, shortcutLabel } from "./shortcuts";
import { browseFiles, type FileOrder } from "../../shared/file-list";
import { browseChoices } from "../../shared/choice-list";
import { thumbnail } from "../../shared/thumbnail";
import { useApplications } from "./composables/useApplications";
import { shortcutIconNames, shortcutIconLabels } from "./shortcut-icons";

const remote = useConnection();
const { status, message, connectionLabel } = remote;
const settingsConnectionLabel = remote.connectionBadgeLabel;
const online = computed(() => status.value === "connected");
const hasControl = computed(() => remote.controlMode.value === "free" || remote.primary.value);
const canInput = computed(() => online.value && remote.permissions.value.input && hasControl.value);
const canText = computed(() => online.value && remote.permissions.value.text && hasControl.value);
const canFiles = computed(() => online.value && remote.permissions.value.files);
const canPower = computed(() => online.value && remote.permissions.value.power && hasControl.value);

type Panel = "shortcut-plans" | "applications" | "unfinished" | "shared-file" | "microphone" | "keyboard" | "more" | "settings" | "windows" | "pair" | "shutdown" | "files" | "folder" | "connection" | "help";
const panel = ref<Panel | null>(null);
const destination = ref<Destination>("remote");
const code = ref("");
const input = ref("");
const sending = ref(false);
const feedback = ref("");
const switcher = ref(false);
const switchExpired = ref(false);
const pageForeground = ref(!document.hidden);
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
const connectionAddress = computed(() => location.hostname === "127.0.0.1" ? t("USB 有线连接") : location.host);

const android = androidBridge();
const connectionRecovery = useConnectionRecovery(android, remote, input,
  computed(() => panel.value === "connection"), pageForeground);
const {
  sensitivity,
  keepAwake,
  keepAlive,
  haptics,
  hardwareVolume,
  theme,
  darkAppearance,
  shortcutA,
  shortcutB,
  shortcutC,
  shortcutIconA,
  shortcutIconB,
  shortcutIconC,
  shortcutPlans,
  applyShortcuts,
  longPressDrag,
  scrollDirection,
  responseMode,
  padOnRight,
  focusMode,
  controlMode,
  dialSize,
  microphoneMode,
  immersive,
  syncWakeLock,
} = useMobileSettings(android, online, destination, mode => remote.setResponseMode(mode));
const { editing: customEditing, draft: customDraft, group: customGroup, error: customError, models: shortcutModels,
  toggle: toggleCustomModifier, save: saveCustomShortcut, close: closeCustomShortcut } = useCustomShortcut([shortcutA, shortcutB, shortcutC]);
const currentShortcuts = () => [shortcutA, shortcutB, shortcutC].map((slot, index) => ({
  action: slot.value, icon: [shortcutIconA, shortcutIconB, shortcutIconC][index].value,
}));
const plans = useShortcutPlans(shortcutPlans, currentShortcuts, applyShortcuts);
const planOrigin = ref<"more" | "settings">("settings");
watch(panel, value => { if (value !== "shortcut-plans") plans.reset(); });
function openShortcutPlans() {
  planOrigin.value = panel.value === "more" ? "more" : "settings";
  plans.reset(); openPanel("shortcut-plans");
}
function leaveShortcutPlans() { if (planOrigin.value === "more") openPanel("more"); else closePanel(); }

const { sharedFiles, filesLoading, filesError, transferBusy, canStartTransfer, transferCancelling, transferProgress, transferMessage, transferHistory, androidFolder, folderReady, loadFiles, chooseUploadFiles, downloadFile, cancelTransfer, clearTransferHistory, unfinishedDownloads, recoveryBusy, recoveryMessage, resolveUnfinishedDownload } =
  useFileTransfers(remote, canFiles, microphoneActive);
const sharedSelection = useSharedFiles(canStartTransfer);
const sharedFileDetail = ref<SharedSelectionFile>();
watch(sharedSelection.openRequest, () => navigate("files"));
watch(sharedSelection.selection, value => {
  if (panel.value !== "shared-file" || !sharedFileDetail.value) return;
  sharedFileDetail.value = value.files.find(file => file.id === sharedFileDetail.value?.id);
  if (!sharedFileDetail.value) closePanel();
});
const confirmUnfinished = ref<{ id: string; version: string }>();
const expandedUnfinished = ref<{ id: string; version: string }>();
watch(panel, () => { confirmUnfinished.value = undefined; expandedUnfinished.value = undefined; });

let feedbackTimer: ReturnType<typeof setTimeout>;
let switchTimer: ReturnType<typeof setTimeout>;

function notice(text: string, durationMs = 4500) {
  feedback.value = text;
  clearTimeout(feedbackTimer);
  feedbackTimer = setTimeout(() => { feedback.value = ""; }, durationMs);
}
function dismissFeedback() { feedback.value = ""; clearTimeout(feedbackTimer); }

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

async function action(command: Command, haptic = true) {
  if (command.type === "release") {
    touchpad.clearMotion();
    remote.clearMotion();
  } else {
    remote.sendMotion(touchpad.drainMotion());
  }
  if (command.type !== "switch") clearSwitchTimer();
  try {
    await remote.send(command);
    if (haptic && haptics.value && command.type !== "move" && command.type !== "scroll") {
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
  directionalRepeat.stop();
  volumeRepeat.stop();
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
  closeCustomShortcut();
  nativeChoices.value = "";
  feedback.value = "";
  panel.value = null;
  destination.value = next;
}

watch(destination, next => {
  if (next === "files") void loadFiles();

});

function handleBack() {
  if (customEditing.value !== null) { closeCustomShortcut(); return true; }
  if (nativeChoices.value) { nativeChoices.value = ""; return true; }
  if (panel.value === "shortcut-plans") { if (!plans.back()) leaveShortcutPlans(); return true; }
  if (android?.renderNativeUi && panel.value) { closePanel(); return true; }
  if (destination.value === "windows" && windowSearchShown.value) { toggleWindowSearch(); return true; }
  if (destination.value !== "remote") { navigate("remote"); return true; }
  if (focusMode.value) { focusMode.value = false; return true; }
  return false;
}

const screenTitle = computed(() => ({ remote: t("遥控"), files: t("文件"), settings: t("设置"), keyboard: t("键盘"), windows: t("切换窗口") })[destination.value]);
const directionalRepeat = new DirectionalRepeat({
  enabled: () => canInput.value && destination.value === "remote" && controlMode.value === "dpad" && !panel.value && customEditing.value === null,
  send: (key, repeating) => action({ type: "key", key }, !repeating),
});
watch([canInput, destination, controlMode, panel, customEditing], () => directionalRepeat.stop(), { flush: "sync" });
const hardwareVolumeActive = computed(() => hardwareVolume.value && canInput.value && destination.value === "remote" && !panel.value && customEditing.value === null);
const volumeRepeat = new DirectionalRepeat<"up" | "down">({
  enabled: () => hardwareVolumeActive.value,
  send: direction => action({ type: "volume", action: direction }, false),
});
watch(hardwareVolumeActive, () => volumeRepeat.stop(), { flush: "sync" });
const titles = computed(() => ({
  "shortcut-plans": t("快捷方案"),
  microphone: t("麦克风状态与帮助"),
  "shared-file": t("文件"),
  unfinished: t("未完成的接收"),
  applications: t("快捷启动"),
  keyboard: t("键盘"),
  more: t("更多操作"),
  settings: t("设置"),
  windows: t("切换窗口"),
  pair: t("连接电脑"),
  shutdown: t("关闭电脑"),
  files: t("文件传输"),
  folder: t("Android 接收目录"),
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
  if (!canText.value || sending.value || !input.value) return;
  const value = input.value;
  const revision = editRevisions.draft.value;
  sending.value = true;
  textFeedback.value = "";
  try {
    await remote.send({ type: "text", text: value });
    if (input.value === value && editRevisions.draft.value === revision) input.value = "";
    textFeedback.value = t("已发送到电脑当前光标位置");
  } catch (e) {
    textFeedback.value = errorText(e, t("文字发送失败"));
  } finally {
    sending.value = false;
  }
}

const { windowList, visibleWindows, windowQuery, windowSearchShown, toggleWindowSearch,
  windowLoading, windowListReady, windowError, windowSwitching, openingTaskView,
  loadWindows, switchWindow, openWindowTaskView, setWindowInteracting } = useWindowPicker(
  remote, destination, canInput, computed(() => pageForeground.value && !panel.value), notice, dismissFeedback,
  () => action({ type: "taskview" }), () => navigate("remote"),
);

function closePanel() {
  cancelInteraction();
  feedback.value = "";
  panel.value = null;
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

watch(folderReady, ready => { if (!ready && panel.value === "folder") closePanel(); });

function runShortcut(value: string) {
  const chord = parseCustomShortcut(value);
  if (chord) {
    if (!remote.features.value.includes("custom-keys")) { notice(t("请更新电脑端以使用自定义按键")); return; }
    void action(chordCommand(chord)); return;
  }
  if (value === "applications") { openPanel("applications"); return; }
  if (value === "magnifier") { void toggleMagnifier(); return; }
  const command = shortcutCatalog.find(item => item.value === value)?.command;
  if (command) void action(command);
}

const { applications, applicationQuery, applicationSearchShown, visibleApplications, applicationsLoading, applicationsReady,
  applicationPending, applicationPendingDetail, applicationsError, applicationActionError, applicationsNotice, refreshApplications, unavailableApplication, launchApplication } = useApplications(remote, panel, canInput);

const shortcutPreset = computed({
  get: () => shortcutPresets.find(preset => matchingSlots(builtinPlan("builtin:" + preset.value)!.shortcuts, currentShortcuts()))?.value || "custom",
  set: (value: string) => {
    const preset = shortcutPresets.find(item => item.value === value);
    if (!preset) return;
    applyShortcuts(builtinPlan("builtin:" + preset.value)!.shortcuts);
  },
});

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
  finally {
    pairing.value = false;
    if (pageForeground.value && remote.token()) remote.resume();
  }
}

async function requestControl() {
  if (controlRequestPending.value || !online.value || hasControl.value) return;
  try { await remote.requestControl(); }
  catch (error) { notice(errorText(error, t("请求失败"))); }
}

function visibility() {
  pageForeground.value = !document.hidden;
  resetInteraction();
  if (document.hidden) remote.suspend();
  else pageshow();
  void syncWakeLock();
}

function pagehide() {
  pageForeground.value = false;
  resetInteraction();
  remote.suspend();
}

function pageshow() {
  pageForeground.value = !document.hidden;
  if (!pageForeground.value || pairing.value || (panel.value === "pair" && !remote.token())) return;
  refreshKeepAliveState();
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
  if (value === "unpaired" && !remote.usbExpired.value) openPanel("pair");
});
watch(remote.usbExpired, expired => {
  if (expired && panel.value === "pair") openPanel("connection");
});
const keepAliveFailed = ref(false);
const backgroundBatteryRestricted = ref(false);
const backgroundProtocolUnsupported = ref(false);
function refreshKeepAliveState() {
  try {
    const state = JSON.parse(android?.getConnectionKeepAliveState?.() || "{}");
    keepAliveFailed.value = state.failed === true;
    backgroundBatteryRestricted.value = state.batteryRestricted === true;
    backgroundProtocolUnsupported.value = state.terminalCode === 4002;
  } catch {}
}
watch([keepAlive, status], ([enabled]) => {
  remote.setBackgroundAllowed(false);
  android?.setConnectionKeepAlive?.(enabled && !!remote.token());
}, { immediate: true });

const nativeUi = android?.renderNativeUi;
const nativeMic = nativeUi ? useMicrophone({
  get enabled() { return (online.value && remote.permissions.value.microphone) || microphoneLocked.value; },
  get allowed() { return (status.value !== "unpaired" && remote.permissions.value.microphone) || microphoneLocked.value; },
  get activePage() { return destination.value === "remote" && !panel.value; },
}, notice) : undefined;
const microphoneDetails = ref(false);
const microphoneStatus = useMicrophoneStatus(remote, computed(() => panel.value === "microphone"), online,
  computed(() => remote.permissions.value.microphone), pageForeground);
const microphoneStartState = computed(() => ({
  online: online.value, allowed: remote.permissions.value.microphone && !microphoneStatus.denied.value,
  foreground: pageForeground.value,
  usbExpired: remote.usbExpired.value, active: microphoneActive.value,
  value: online.value ? microphoneStatus.data.value : undefined, readError: microphoneStatus.error.value, reading: false,
}));
const microphoneStartAllowed = computed(() => !!nativeMic && !nativeMic.lastIssue.value && canStartMicrophone(microphoneStartState.value));
const microphoneRecoveryAllowed = computed(() => !!nativeMic && canRestartMicrophone({
  ...microphoneStartState.value, phoneAllowed: nativeMic.permissionGranted.value === true, issue: nativeMic.lastIssue.value,
}));
function startMicrophone() {
  if (microphoneStartAllowed.value && !microphoneStatus.loading.value && !microphoneStatus.manualRefreshing.value)
    nativeMic?.startLocked();
}
function restartMicrophone() {
  if (microphoneRecoveryAllowed.value && !microphoneStatus.loading.value && !microphoneStatus.manualRefreshing.value)
    nativeMic?.restartLocked();
}
watch(panel, next => { if (next !== "microphone") microphoneDetails.value = false; });
if (nativeMic) watch(nativeMic.mode, mode => {
  microphoneLocked.value = mode === "locked" || nativeMic.lockedSession.value;
  microphoneActive.value = mode !== "idle";
});

const nativeActions = new Map<string, (value: unknown) => unknown>();
function nativeEvent(event: Event) {
  if (!currentNativeInput((event as CustomEvent<unknown>).detail,pageForeground.value,android?.getInputEpoch?.())) return;
  dispatchNativeAction(nativeActions, (event as CustomEvent<unknown>).detail, nativeSheetRevision.value, nativeSheetOpen.value, nativeScreenRevision.value);
}
function nativePointer(event: Event) {
  const point = (event as CustomEvent<NativePointer | NativePointerMoveBatch>).detail;
  if (!currentNativeInput(point,pageForeground.value,android?.getInputEpoch?.())) return;
  if (point.phase === "move-batch") {
    touchpad.moveTogether(point.contacts);
    return;
  }
  const pointer = asPointerEvent(point);
  if (point.target === "mic" && nativeMic) {
    if (point.phase === "cancel") nativeMic.up();
    else if (point.phase === "lost") nativeMic.up(pointer);
    else if (point.phase === "up") nativeMic.up(pointer, true);
    else nativeMic[point.phase](pointer);
  } else if (point.phase === "lost") touchpad.captureLost(pointer);
  else if (point.phase === "cancel") touchpad.cancel();
  else touchpad[point.phase](pointer);
}

const fileQuery = ref("");
const fileOrder = ref<FileOrder>("default");
const visibleFiles = computed(() => browseFiles(sharedFiles.value, fileQuery.value, fileOrder.value, locale.value));
const nativeChoices = ref("");
const nativeSheetRevision = ref(0);
const nativeScreenRevision = ref(0);
const nativeSheetOpen = computed(() => !!panel.value || !!nativeChoices.value || customEditing.value !== null);
watch([panel, nativeChoices, customEditing], () => { nativeSheetRevision.value++; }, { flush: "sync" });
if (nativeMic) watch(nativeMic.mode, (mode, previous) => {
  if (panel.value === "microphone" && (mode === "locked") !== (previous === "locked")) nativeSheetRevision.value++;
}, { flush: "sync" });
watch([plans.stage, plans.selected], () => { nativeSheetRevision.value++; }, { flush: "sync" });
watch(destination, () => { nativeScreenRevision.value++; }, { flush: "sync" });
const shortcutQuery = ref("");
const nativeHistoryOpen = ref(false);
const nativeDiagnosticsOpen = ref(false);
watch(panel, value => { if (value !== "connection") nativeDiagnosticsOpen.value = false; });
const editRevisions = { code: ref(0), draft: ref(0), "file-filter": ref(0), "shortcut-filter": ref(0), "application-filter": ref(0), "window-filter": ref(0), "plan-name": ref(0) };
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
  if (hardwareVolumeActive.value) for (const direction of ["up", "down"] as const) {
    nativeActions.set("hardware-volume-" + direction, value => {
      if (value === true) volumeRepeat.press(direction);
      else if (value === false) volumeRepeat.release(direction);
    });
  }
  const button = (id: string, label: string, callback: () => unknown, enabled = true, detail?: string): NativeControl => {
    if (enabled) nativeActions.set(id, callback);
    return { kind: "button", id, label, enabled, detail };
  };
  const key = (id: string, label: string, key: Extract<Command, { type: "key" }>["key"]) =>
    button(id, label, () => action({ type: "key", key }), canInput.value);
  const direction = (id: string, label: string, key: Direction): NativeControl => {
    if (canInput.value) nativeActions.set(id, value => {
      if (value === true) directionalRepeat.press(key);
      else if (value === false) directionalRepeat.release(key);
      else return directionalRepeat.tap(key);
    });
    return { kind: "button", id, label, enabled: canInput.value };
  };
  const command = (id: string, label: string, cmd: Command) => button(id, label, () => action(cmd), canInput.value);
  const toggle = (id: string, label: string, model: { value: boolean }, detail: string): NativeControl => {
    nativeActions.set(id, value => { model.value = value === true; });
    return { kind: "switch", id, label, value: model.value, detail };
  };
  const controls: NativeControl[] = [];
  const sheet: NativeControl[] = [];
  let sheetTitle = panel.value ? titles.value[panel.value] : "";
  let sheetIcon = "";
  const sheetActions: NativeControl[] = [];
  const sheetFooter: NativeControl[] = [];
  const choice = (id: string, label: string, model: { value: string }, options: { value: string; label: string; icon?: string; description?: string; group?: string }[], enabled = true) => {
    const selected = options.find(item => item.value === model.value);
    controls.push({ ...button(id, label, () => { shortcutQuery.value = ""; nativeChoices.value = id; }, enabled, selected?.label), icon: selected?.icon });
    if (nativeChoices.value !== id) return;
    sheetTitle = label;
    if (id.startsWith("icon-")) {
      const slot = slots[Number(id.slice(5))];
      const actionName = slot && shortcutLabel(slot.value);
      if (actionName && selected) {
        sheetTitle = actionName + " · " + selected.label;
        sheetIcon = selected.icon || "";
      }
    }
    const searchable = id.startsWith("slot-");
    if (searchable) sheet.push(row("choice-search",
      nativeEdit("shortcut-filter", shortcutQuery, t("搜索操作")),
      button("clear-shortcut-filter", t("清除"), () => { shortcutQuery.value = ""; }, !!shortcutQuery.value)));
    const groups = browseChoices(options, searchable ? shortcutQuery.value : "", locale.value);
    if (!groups.length) sheet.push(text("shortcut-empty", t("没有匹配的操作")));
    groups.forEach((group, index) => {
      if (group.label) sheet.push(heading("choice-group-" + index + "-heading", group.label));
      group.options.forEach(option => sheet.push({ ...button(id + option.value, option.label, () => {
        if (!enabled) return;
        model.value = option.value;
        nativeChoices.value = "";
      }, enabled, option.description), icon: option.icon, selected: option.value === model.value }));
    });
  };

  const slots = shortcutModels;
  const icons = [shortcutIconA, shortcutIconB, shortcutIconC];
  const iconNames = shortcutIconNames.map(name => t(shortcutIconLabels[name]));
  const micLabel = microphoneLocked.value ? t("持续收音") : t("麦克风");
  const english = locale.value === "en";
  const mic = { kind: "mic", id: "mic", label: english ? (microphoneLocked.value ? "Mic on" : "Mic") : micLabel,
    hint: microphoneLocked.value ? t("立即停止麦克风") : t("查看麦克风状态与操作"),
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
    else openPanel("microphone");
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
        direction("dpad-up", t("上"), "up"), direction("dpad-right", t("右"), "right"),
        direction("dpad-down", t("下"), "down"), direction("dpad-left", t("左"), "left"),
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
    const incoming = sharedSelection.selection.value;
    if (incoming.files.length || incoming.message) {
      const entries: NativeControl[] = [row("share-heading", heading("share-title", t("待发送 · {0}", incoming.files.length)),
        button("share-clear", t("清空"), sharedSelection.clear))];
      for (const file of incoming.files) {
        const name = file.name || t("文件");
        const detail = file.pending ? t("正在读取文件…") : file.error || formatBytes(file.size);
        entries.push(row("share-file-" + file.id,
          { kind: "column", id: "share-copy-" + file.id, children: [
            { ...button("share-name-" + file.id, name, () => { sharedFileDetail.value = file; openPanel("shared-file"); }), kind: "text" },
            text("share-info-" + file.id, detail)] },
          ...(file.error && !file.pending ? [{ ...button("share-retry-" + file.id, t("重新读取"), () => sharedSelection.retry(file), sharedSelection.canRetry(file)),
            icon: "refresh", iconOnly: true, status: t("重新读取 {0}", name) }] : []),
          { ...button("share-remove-" + file.id, t("移除 {0}", name), () => sharedSelection.remove(file)), icon: "close", iconOnly: true }));
      }
      if (incoming.message) entries.push(text("share-message", displayMessage(incoming.message)));
      if (incoming.files.length) {
        if (!canFiles.value) entries.push(text("share-hint", t("连接电脑并获得文件权限后，再确认发送。")));
        else if (transferBusy.value) entries.push(text("share-hint", t("当前传输结束后，可发送这些文件。")));
        entries.push(button("share-send", t("发送到 {0}", android?.getComputerName?.() || t("Windows 电脑")), sharedSelection.send, sharedSelection.canSend.value),
          button("share-computer", t("选择电脑"), () => android?.changeComputer()));
      }
      controls.push({ kind: "column", id: "share-card", children: entries });
    }
    if (folderReady.value) nativeActions.set("folder", () => openPanel("folder"));
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
    controls.push(button("upload", t("发送文件到 Windows"), chooseUploadFiles,
      canStartTransfer.value, t("单文件最大 2 GB")));
    if (transferMessage.value && !transferBusy.value) controls.push(text("transfer", displayMessage(transferMessage.value)));
    if (unfinishedDownloads.value.length) controls.push(button("unfinished", downloadRecoveryLabel(unfinishedDownloads.value), () => openPanel("unfinished"), !transferBusy.value));
    if (transferHistory.value.length) {
      controls.push({ ...button("history-toggle", t("最近传输"), () => { nativeHistoryOpen.value = !nativeHistoryOpen.value; }),
        icon: nativeHistoryOpen.value ? "up" : "down" });
      if (nativeHistoryOpen.value) controls.push(...transferHistory.value.map((entry, i) => text("history-" + i, entry)),
        button("clear-history", t("清空记录"), clearTransferHistory));
    }
    if (!canFiles.value) controls.push(text("files-status", t("连接电脑并获得文件权限后，即可开始传输。")));
    else controls.push(heading("shared", t("Windows 共享的文件")),
      button("reload", t("刷新"), loadFiles, canFiles.value && !transferBusy.value && !filesLoading.value));
    if (canFiles.value && (sharedFiles.value.length || fileQuery.value)) {
      controls.push(row("files-search", nativeEdit("file-filter", fileQuery, t("搜索文件名")),
        button("clear-file-filter", t("清除"), () => { fileQuery.value = ""; }, !!fileQuery.value)));
      choice("file-order", t("排序方式"), fileOrder, [
        { value: "default", label: t("默认顺序") }, { value: "name", label: t("名称 A–Z") }, { value: "largest", label: t("大小 从大到小") },
      ]);
      if (sharedFiles.value.length && !visibleFiles.value.length) controls.push(text("files-no-match", t("没有匹配的文件")));
    }
    if (canFiles.value && filesError.value) controls.push(text("files-status", displayMessage(filesError.value)),
      button("files-retry", filesLoading.value ? t("读取中…") : t("重试"), loadFiles, !transferBusy.value && !filesLoading.value));
    if (canFiles.value && !sharedFiles.value.length && (!filesError.value || filesLoading.value)) {
      controls.push({ kind: "text", id: "files-empty", icon: "folder", label: filesLoading.value ? t("正在读取共享文件…") : t("电脑文件会显示在这里"),
        detail: filesLoading.value ? t("请稍候。") : t("在电脑端选择要共享的文件，即可保存到手机。") });
    }
    if (canFiles.value) visibleFiles.value.forEach(file => controls.push(row("file-" + file.id,
      { kind: "text", id: "file-name-" + file.id, label: file.name, detail: formatBytes(file.size) },
      { ...button("save-" + file.id, t("保存"), () => downloadFile(file), folderReady.value && canStartTransfer.value),
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
    nativeActions.set("window-interaction", setWindowInteracting);
    if (canInput.value) controls.push(row("window-actions", text("window-actions-spacer", windowList.value.length
      ? windowLoading.value ? t("正在刷新…") : !windowListReady.value ? t("请刷新后重试") : "" : ""),
      { ...button("window-search", t(windowSearchShown.value ? "收起搜索" : "搜索"), toggleWindowSearch, !openingTaskView.value && !windowSwitching.value), icon: windowSearchShown.value ? "close" : "search", iconOnly: true },
      { ...button("window-refresh", windowLoading.value ? t("读取中…") : t("刷新"), () => void loadWindows(true), canInput.value && !windowLoading.value && !windowSwitching.value), icon: "refresh", iconOnly: true },
      { ...button("taskview", t("任务视图"), () => void openWindowTaskView(), !openingTaskView.value && !windowSwitching.value), icon: "taskview", iconOnly: true },
    ));
    if (canInput.value && windowSearchShown.value) controls.push(row("windows-search",
      { ...nativeEdit("window-filter", windowQuery, t("搜索窗口")), enabled: !windowSwitching.value && !openingTaskView.value },
      button("clear-window-filter", t("清除"), () => { windowQuery.value = ""; }, !!windowQuery.value && !windowSwitching.value && !openingTaskView.value)));
    if (!canInput.value) {
      controls.push(text("windows-offline", t("连接电脑并获得遥控权限后，可以切换窗口。")));
    } else {
      if (windowError.value && !windowList.value.length) {
        const unsupported = windowError.value.includes("窗口枚举不可用");
        controls.push(text("windows-error", unsupported ? t("这台电脑暂时无法提供窗口列表") : displayMessage(windowError.value)),
          row("window-recovery", ...(windowList.value.length ? [] : [button("window-retry", t("重试"), () => void loadWindows(true), !windowLoading.value)]),
            button("open-taskview", t("打开任务视图"), () => void openWindowTaskView(), !openingTaskView.value && !windowSwitching.value)));
      }
      if (windowLoading.value && !windowList.value.length) controls.push(text("windows-loading", t("正在读取窗口列表…")));
      if (!windowLoading.value && !windowList.value.length && !windowError.value) controls.push(text("windows-empty", t("电脑上没有可切换的窗口")));
      if (windowListReady.value && windowList.value.length && !visibleWindows.value.length) controls.push(text("windows-no-match", t("没有匹配的窗口")));
      visibleWindows.value.forEach(entry => {
        const id = "window-" + entry.hwnd;
        nativeActions.set(id, () => void switchWindow(entry));
        controls.push({
          kind: "window", id, label: entry.title,
          detail: windowSwitching.value === entry.hwnd ? t("正在切换…")
            : [entry.app, entry.minimized ? t("已最小化") : ""].filter(Boolean).join(" · "),
          icon: entry.icon ?? "monitor",
          enabled: canInput.value && windowListReady.value && !windowSwitching.value && !openingTaskView.value,
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
    controls.push(toggle("hardware-volume", t("音量键控制电脑"), hardwareVolume, t("仅在已连接的遥控页生效，按住可连续调节")),
      toggle("haptics", t("触觉反馈"), haptics, t("操作成功后轻触反馈")), heading("display", t("显示")));
    choice("language", t("语言"), languagePreference, languageOptions.value);
    choice("theme", t("外观"), theme, [{ value: "system", label: t("跟随系统") }, { value: "light", label: t("浅色") }, { value: "dark", label: t("深色") }]);
    controls.push(toggle("awake", t("保持屏幕常亮"), keepAwake, t("仅在前台连接电脑时生效")),
      toggle("alive", t("应用保活"), keepAlive, keepAliveFailed.value ? t("后台服务未能启动，请检查系统的后台运行限制")
        : backgroundProtocolUnsupported.value ? t("请同时更新手机和电脑端，以使用后台连接") : t("连接电脑时在后台保持运行，并显示常驻通知")));
    if (keepAlive.value && android?.openBackgroundSettings && (backgroundBatteryRestricted.value || keepAliveFailed.value))
      controls.push(button("background-settings", t("后台运行设置"), () => android.openBackgroundSettings?.(), true, t("允许羽连在息屏后继续连接电脑")));
    controls.push(button("magnify", t("鼠标放大镜"), toggleMagnifier, canInput.value && !lensPending.value, magnifier.value ? t("已开启") : t("已关闭")),
      heading("microphone-heading", t("手机麦克风")));
    choice("microphone-mode", t("收音模式"), microphoneMode, [
      { value: "standard", label: t("标准收音"), description: t("在电脑端手动调节麦克风增益") },
      { value: "voice", label: t("语音增强"), description: t("设备支持时自动调节音量并降低噪声") },
    ], !microphoneActive.value);
    if (microphoneActive.value) controls.push(text("microphone-mode-help", t("停止收音后可更改收音模式")));
    controls.push({ ...button("microphone-status", t("麦克风状态与帮助"), () => openPanel("microphone"), true,
      nativeMic?.lastIssue.value ? t("查看最近提示") : t("查看声音通路和配置")), icon: "mic" });
    controls.push(heading("shortcuts", t("快捷操作")));
    choice("shortcut-preset", t("快捷操作预设"), shortcutPreset, presetOptions());
    controls.push({ ...button("shortcut-plans", t("快捷方案"), openShortcutPlans, true, plans.currentName.value || t("保存和切换三键组合")), icon: "layers" });
    slots.forEach((slot, i) => {
      choice("slot-" + i, [t("快捷操作一"), t("快捷操作二"), t("快捷操作三")][i], slot,
        shortcutOptions(slot.value).filter(item => item.value === slot.value || !slots.some(other => other.value === item.value)));
      choice("icon-" + i, [t("图标一"), t("图标二"), t("图标三")][i], icons[i],
        shortcutIconNames.map((value, index) => ({ value, label: iconNames[index], icon: value })));
    });
    controls.push(text("shortcut-tip", t("在遥控页的“更多”中使用。")), heading("computer-help", t("电脑与帮助")), button("connection", t("当前电脑"), () => openPanel("connection"), true, settingsConnectionLabel.value),
      button("help", t("手势与操作指南"), () => openPanel("help")), button("privacy", t("隐私政策"), () => android?.showPrivacyPolicy?.()), button("shutdown", t("关闭电脑"), () => openPanel("shutdown"), canPower.value));
    controls.push({ ...text("settings-footnote", t("羽连 PlumeLink")), detail: t("让控制触手可及") });
  }

  if (panel.value === "shortcut-plans") {
    const stage = plans.stage.value;
    const chosen = plans.plan.value;
    const preview = (values: ReturnType<typeof currentShortcuts>) => values.forEach((slot, index) => sheet.push({
      ...text("plan-preview-" + index, shortcutLabel(slot.action)), detail: [t("快捷操作一"), t("快捷操作二"), t("快捷操作三")][index],
      kind: "window", icon: slot.icon, enabled: false,
    }));
    if (stage !== "list") sheetActions.push({ ...button("plan-back", t("返回"), () => plans.back()), icon: "left", iconOnly: true });
    if (stage === "list") {
      sheet.push({ ...heading("plans-builtins", t("内置方案")), tone: "neutral" });
      shortcutPresets.forEach(preset => {
        const value = builtinPlan("builtin:" + preset.value)!;
        sheet.push({ ...button("plan-" + preset.value, t(preset.label), () => plans.select(value.id), true,
          value.shortcuts.map(slot => shortcutLabel(slot.action)).join(" · ")), icon: value.shortcuts[1].icon,
          kind: "action-row", selected: matchingSlots(value.shortcuts, currentShortcuts()) });
      });
      sheet.push({ ...heading("plans-saved", t("已保存的方案")), tone: "neutral" });
      if (!shortcutPlans.value.length) sheet.push({ ...text("plans-empty", t("将当前三个快捷操作和图标保存为方案，方便下次切换。")), tone: "neutral" });
      shortcutPlans.value.forEach(saved => sheet.push({ ...button("plan-" + saved.id, saved.name, () => plans.select(saved.id), true,
        saved.shortcuts.map(slot => shortcutLabel(slot.action)).join(" · ")), icon: saved.shortcuts[1].icon,
        kind: "action-row", selected: matchingSlots(saved.shortcuts, currentShortcuts()) }));
      sheet.push({ ...button("plan-create", t("保存当前组合"), () => plans.saveCurrent(), shortcutPlans.value.length < maxShortcutPlans), kind: "action-row", disclosure: false, icon: "star" });
      if (shortcutPlans.value.length >= maxShortcutPlans) sheet.push(text("plans-limit", t("最多保存 8 个方案，请先删除不需要的方案")));
    } else if (stage === "save" || stage === "rename") {
      sheetTitle = stage === "save" ? t("保存当前组合") : t("重命名方案");
      sheet.push(nativeEdit("plan-name", plans.name, t("方案名称")));
      preview(stage === "save" ? plans.captured.value : chosen?.shortcuts || []);
      sheet.push(button("plan-save", t("保存"), () => plans.commit()));
    } else if (chosen) {
      sheetTitle = chosen.name;
      preview(chosen.shortcuts);
      if (stage === "delete") {
        sheet.push(text("plan-delete-hint", t("删除方案不会改变正在使用的三个快捷操作。")),
          button("plan-delete-confirm", t("删除方案"), () => plans.remove()), button("plan-delete-cancel", t("取消"), () => plans.back()));
      } else {
        sheet.push(button("plan-apply", t("应用方案"), () => { if (plans.apply()) leaveShortcutPlans(); }));
        if (!chosen.builtin) sheet.push(row("plan-manage", button("plan-rename", t("重命名方案"), () => plans.rename()), button("plan-delete", t("删除方案"), () => plans.requestDelete())));
      }
    }
    if (plans.error.value) sheet.push({ ...text("plan-error", displayMessage(plans.error.value)), kind: "notice", tone: "error" });
  } else if (panel.value === "microphone") {
    const value = online.value ? microphoneStatus.data.value : undefined;
    const recentIssue = nativeMic?.lastIssue.value || "";
    const computerIssue = microphoneStatus.error.value ? "" : value?.status.error || "";
    const issue = recentIssue || computerIssue;
    const warning = nativeMic?.warning.value || "";
    const allowed = remote.permissions.value.microphone && !microphoneStatus.denied.value;
    const guidance = microphoneStatus.error.value && online.value && allowed
      ? { title: t("状态暂时不可用"), help: t("请刷新状态，或在电脑端的「偏好设置 → 手机麦克风」检查。") }
      : microphoneGuidance(online.value, allowed, value, nativeMic?.phase.value || "idle", issue, warning, nativeMic?.permissionGranted.value !== false);
    sheetActions.push({ ...button("microphone-refresh", microphoneStatus.manualRefreshing.value ? t("读取中…") : t("刷新状态"), () => { void microphoneStatus.refresh(); }, online.value && remote.permissions.value.microphone && !microphoneStatus.loading.value && !microphoneStatus.manualRefreshing.value), icon: "refresh", iconOnly: true });
    sheet.push(heading("microphone-summary", guidance.title));
    if (microphoneStatus.manualRefreshing.value) sheet.push(text("microphone-refreshing", t("正在读取麦克风状态…")));
    if (recentIssue || warning) sheet.push(text("microphone-issue", displayMessage(recentIssue || warning)));
    if (computerIssue && computerIssue !== recentIssue) sheet.push(text("microphone-computer-error", t("电脑提示：{0}", displayMessage(computerIssue))));
    if (microphoneStatus.error.value) sheet.push({ kind: "notice", id: "microphone-read-error", tone: "error", label: displayMessage(microphoneStatus.error.value) });
    if (guidance.help) sheet.push(text("microphone-guidance", guidance.help));
    sheet.push(heading("microphone-phone-heading", t("手机收音")), text("microphone-phone", nativeMic?.recording.value
      ? t("{0} · 正在收音", nativeMic.duration.value) : nativeMic?.phase.value === "reconnecting" ? t("正在重连…")
        : nativeMic?.phase.value === "connecting" ? t("正在连接电脑…") : t("未在收音")));
    if (value?.allowed && !microphoneStatus.error.value) {
      sheet.push(heading("microphone-route-heading", t("声音去向")), text("microphone-output", value.configured ? value.output || t("已在电脑端选择音频设备") : t("未启用")));
      if (value.gainDb !== undefined) sheet.push(text("microphone-gain", t("电脑麦克风增益：+{0} dB", value.gainDb)));
      if (value.owned === true) {
        sheet.push({ ...button("microphone-details", microphoneDetails.value ? t("收起详细状态") : t("查看详细状态"), () => { microphoneDetails.value = !microphoneDetails.value; }),
          icon: microphoneDetails.value ? "down" : "next" });
        if (microphoneDetails.value) sheet.push(text("microphone-received", t("已接收：{0} 帧", value.status.receivedFrames)),
          text("microphone-buffer", t("播放缓冲：{0} ms", Math.round(value.status.queuedMs))),
          text("microphone-jitter", t("网络抖动：{0} ms", Math.round(value.status.jitterMs))),
          text("microphone-gaps", t("播放欠载：{0} 次 · 丢弃：{1} 帧", value.status.underruns, value.status.droppedFrames)));
      }
    }
    if (nativeMic?.lastIssue.value) sheet.push(button("microphone-dismiss-issue", t("清除最近提示"), nativeMic.dismissIssue));
    if (microphoneActive.value) sheetFooter.push(button("microphone-stop", t("立即停止麦克风"), () => nativeMic?.stop()));
    else if (microphoneRecoveryAllowed.value) sheetFooter.push(button("microphone-restart", t("重新开启持续收音"), restartMicrophone,
      !microphoneStatus.loading.value && !microphoneStatus.manualRefreshing.value));
    else if (microphoneStartAllowed.value) sheetFooter.push(button("microphone-start", t("开启持续收音"), startMicrophone,
      !microphoneStatus.loading.value && !microphoneStatus.manualRefreshing.value));
    sheet.push(button("microphone-back", t("返回遥控"), () => { closePanel(); navigate("remote"); }));
  } else if (panel.value === "shared-file" && sharedFileDetail.value) {
    const file = sharedFileDetail.value;
    sheetTitle = t("文件");
    sheet.push(text("share-full-name", file.name || t("文件")),
      text("share-full-size", file.pending ? t("正在读取文件…") : file.error || formatBytes(file.size)));
    if (sharedSelection.selection.value.message) sheet.push(text("share-read-message", displayMessage(sharedSelection.selection.value.message)));
    if (file.error && !file.pending) sheetFooter.push(button("share-file-retry", t("重新读取"), () => sharedSelection.retry(file), sharedSelection.canRetry(file)));
  } else if (panel.value === "unfinished") {
    if (unfinishedDownloads.value.some(file => file.pendingCommit)) sheetTitle = downloadRecoveryLabel(unfinishedDownloads.value);
    sheetActions.push({ ...button("unfinished-refresh", t("刷新"), () => android?.syncSession?.(), !transferBusy.value && !recoveryBusy.value), icon: "refresh", iconOnly: true });
    sheet.push(text("unfinished-help", unfinishedDownloads.value.some(file => file.pendingCommit)
      ? t("请先核对接收目录中的文件，再选择保留或删除。") : t("请核对文件后选择保留或删除。")));
    if (!unfinishedDownloads.value.length) sheet.push(text("unfinished-empty", t("没有未完成的文件")));
    if (recoveryMessage.value) sheet.push({ ...text("unfinished-error", displayMessage(recoveryMessage.value)), kind: "notice", tone: "error" });
    unfinishedDownloads.value.forEach(file => {
      const confirm = confirmUnfinished.value?.id === file.id && confirmUnfinished.value.version === file.version;
      const enabled = !transferBusy.value && !recoveryBusy.value;
      const expanded = confirm || expandedUnfinished.value?.id === file.id && expandedUnfinished.value.version === file.version;
      const actions = confirm ? [
        button(recoveryActionId("confirm-delete", file), t("确认删除"), () => { resolveUnfinishedDownload(file, false); confirmUnfinished.value = undefined; }, enabled && file.available),
        button(recoveryActionId("cancel-delete", file), t("取消"), () => { confirmUnfinished.value = undefined; }, enabled),
      ] : [
        button(recoveryActionId("keep", file), t("保留文件"), () => { resolveUnfinishedDownload(file, true); }, enabled),
        button(recoveryActionId("delete", file), t("删除"), () => { confirmUnfinished.value = { id: file.id, version: file.version }; }, enabled && file.available),
      ];
      sheet.push({ kind: "column", id: "unfinished-" + file.id, children: [
        { ...text("unfinished-name-" + file.id, file.name), maxLines: expanded ? undefined : 2 },
        text("unfinished-size-" + file.id, downloadRecoveryStatus(file)),
        ...(expanded && file.folder ? [text("unfinished-folder-" + file.id, t("位置：{0}", file.folder))] : []),
        ...(!confirm ? [button(recoveryActionId(expanded ? "collapse" : "expand", file), expanded ? t("收起详情") : t("查看详情"), () => {
          expandedUnfinished.value = expanded ? undefined : { id: file.id, version: file.version };
        })] : []),
        row("unfinished-actions-" + file.id, ...actions),
      ] });
    });
  } else if (panel.value === "folder") {
    sheetIcon = "folder";
    sheet.push(text("folder-path", androidFolder.value));
  } else if (panel.value === "pair") {
    sheet.push(heading("pair-name", android?.getComputerName?.() || t("Windows 电脑")), text("address", connectionAddress.value),
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
  } else if (panel.value === "applications") {
    sheetActions.push({ ...button("applications-refresh", t("刷新"), refreshApplications, canInput.value && !applicationsLoading.value && !applicationPending.value), icon: "refresh", iconOnly: true });
    if (applicationSearchShown.value) sheet.push(row("applications-search",
      { ...nativeEdit("application-filter", applicationQuery, t("搜索应用或文件夹")), enabled: canInput.value },
      button("clear-application-filter", t("清除"), () => { applicationQuery.value = ""; }, canInput.value && !!applicationQuery.value)));
    if (!canInput.value) sheet.push(text("applications-status", t("连接电脑并获得遥控权限后使用")));
    else if (applicationsLoading.value) sheet.push(text("applications-status", t("读取中…")));
    else if (!applications.value.length && !applicationsError.value) sheet.push(text("applications-empty", t("请先在电脑端的偏好设置中添加应用或文件夹")));
    if (applicationActionError.value || applicationsError.value) sheet.push({ ...text("applications-error", displayMessage(applicationActionError.value || applicationsError.value)), kind: "notice", tone: "error" });
    if (applicationsNotice.value) sheet.push({ ...text("applications-notice", applicationsNotice.value), kind: "notice", tone: "success" });
    if (applicationsReady.value && applications.value.length && !visibleApplications.value.length) sheet.push(text("applications-no-match", t("没有匹配的快捷入口")));
    visibleApplications.value.forEach(entry => sheet.push({ ...button("launch-" + entry.id, entry.name, () => launchApplication(entry), canInput.value && applicationsReady.value && !applicationPending.value && entry.available !== false,
      applicationPending.value === entry.id ? applicationPendingDetail.value : entry.available === false ? unavailableApplication(entry) : undefined), kind: "window", icon: entry.icon || (entry.kind === "folder" ? "folder" : undefined) }));
  } else if (panel.value === "more") {
    sheet.push(row("more-keys", key("esc", "Esc", "escape"), command("desktop", t("显示桌面"), { type: "desktop" }), key("backspace", t("删除"), "backspace")),
      row("more-actions", command("scroll-up", t("向上滚动"), { type: "scroll", dy: -120 }), command("scroll-down", t("向下滚动"), { type: "scroll", dy: 120 }), button("windows", t("切换窗口"), () => navigate("windows"), canInput.value)),
      row("more-slots", ...slots.map((slot, i) => ({ ...button("run-" + i, shortcutLabel(slot.value), () => runShortcut(slot.value), canInput.value), icon: icons[i].value }))));
    sheet.push({ ...button("shortcut-plans-more", t("快捷方案"), openShortcutPlans, true,
      plans.currentName.value || presetOptions().find(item => item.value === shortcutPreset.value)?.label), kind: "action-row", groupStart: true, icon: "layers" });
    sheet.push({ ...button("microphone-status-more", t("麦克风状态与帮助"), () => openPanel("microphone")), kind: "action-row", icon: "mic" });
    if (feedback.value) sheet.push(text("more-feedback", feedback.value));
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
      text("pc-status", displayMessage(message.value)), text("address", connectionAddress.value),
    ] }, ...(!online.value && !remote.usbExpired.value && !connectionRecovery.available.value ? [button("reconnect", status.value === "unpaired" ? t("输入配对码") : t("重新连接"), () => {
      if (status.value === "unpaired") openPanel("pair");
      else remote.resume();
    })] : []));
    if (connectionRecovery.available.value) {
      sheet.push({ ...button("connection-find", connectionRecovery.busy.value ? t("正在查找这台电脑…") : t("查找并重新连接"),
        connectionRecovery.recover, !connectionRecovery.busy.value), icon: "refresh" });
      if (connectionRecovery.error.value) sheet.push(text("connection-find-error", displayMessage(connectionRecovery.error.value)));
    }
    sheet.push({ kind: "column", id: "connection-metrics", children: [
      row("quality-row", { kind: "text", id: "quality", label: t("网络质量"), detail: remote.qualityLabel.value },
        text("connection-latency", remote.latency.value === null ? "—" : remote.latency.value + " ms")),
      { kind: "text", id: "permission", label: t("控制权限"),
        detail: !online.value ? t("等待连接") : (hasControl.value ? t("可以控制") : t("当前由{0}主控", remote.primaryName.value || t("另一台设备"))) + (remote.adminGranted.value ? t(" · 管理员控制已开启") : "") },
    ] },
      { ...button("diagnostics", t("连接详情"), () => { nativeDiagnosticsOpen.value = !nativeDiagnosticsOpen.value; }),
        icon: nativeDiagnosticsOpen.value ? "down" : "next" });
    sheetFooter.push(button("change", t("管理与切换电脑"), () => android?.changeComputer()));
    if (nativeDiagnosticsOpen.value) {
      const permission = (allowed: boolean) => allowed ? t("允许") : t("已关闭");
      sheet.push(text("transport", t("QFastTransmission · 发送间隔") + " " + remote.transportStats.value.intervalMs + " " + t("ms · 电脑排队") + " " + remote.transportStats.value.queueMs + " ms"),
        text("permissions", [t("权限：遥控"), permission(remote.permissions.value.input), t("· 文字"), permission(remote.permissions.value.text), t("· 文件"), permission(remote.permissions.value.files)].join(" ")));
    }
  } else if (panel.value === "help") {
    sheet.push(heading("one", t("一根手指")), text("one-detail", t("滑动移动指针，轻点单击，连续轻点双击。按住片刻后移动可拖拽，松手结束。")),
      heading("two", t("两根手指")), text("two-detail", t("上下或左右滑动滚动页面，轻点右键，捏合或张开缩放。")),
      heading("shortcuts-help", t("更多快捷操作")), text("shortcuts-detail", t("在设置中选择三个快捷操作，再到遥控页的“更多”中使用。")),
      heading("direction-help", t("方向遥控")), text("direction-detail", t("按住方向键连续移动，松手或移出按键区域即停止；确认键只执行一次。")),
      heading("input-help", t("输入与播放")), text("input-detail", t("先在电脑上选中输入位置，再发送文字。播放按钮发送空格键，前后按钮发送方向键，效果取决于当前应用。")));
  }
  if (sheetTitle && feedback.value) {
    const message = displayMessage(feedback.value);
    const showsMessage = (items: NativeControl[]): boolean => items.some(item => item.label?.includes(message)
      || item.detail?.includes(message) || item.children && showsMessage(item.children));
    if (!showsMessage(sheet)) sheet.push(text("sheet-feedback", message));
  }
  if (customEditing.value !== null) {
    sheet.length = 0; sheetActions.length = 0; sheetFooter.length = 0;
    sheetTitle = t("自定义按键"); sheetIcon = "keyboard";
    sheetActions.push(button("custom-key-save", t("保存"), saveCustomShortcut));
    sheet.push(text("custom-key-preview", chordLabel(customDraft.value)),
      { kind: "strip", id: "custom-modifiers", children: keyboardModifiers.map(modifier => ({ ...button("custom-mod-" + modifier, modifier === "win" ? "Win" : modifier[0].toUpperCase() + modifier.slice(1), () => toggleCustomModifier(modifier)), selected: customDraft.value.modifiers.includes(modifier) })) },
      { kind: "strip", id: "custom-key-groups", children: keyGroups.map(group => ({ ...button("custom-group-" + group.value, t(group.label), () => { customGroup.value = group.value; }), selected: customGroup.value === group.value })) });
    const keys = keyGroups.find(group => group.value === customGroup.value)?.keys || [];
    for (let index = 0; index < keys.length; index += 4) {
      const cells: NativeControl[] = keys.slice(index, index + 4).map(key => ({ ...button("custom-key-" + key, keyLabel(key), () => { customDraft.value.key = key; customError.value = ""; }), selected: customDraft.value.key === key }));
      while (cells.length < 4) cells.push(text("custom-empty-" + index + "-" + cells.length, ""));
      sheet.push(row("custom-key-row-" + index, ...cells));
    }
    if (customError.value) sheet.push({ ...text("custom-key-error", customError.value), kind: "notice", tone: "error" });
  }
  nativeActions.set("dismiss", () => { if (customEditing.value !== null) closeCustomShortcut(); else if (nativeChoices.value) nativeChoices.value = ""; else closePanel(); });
  nativeActions.set("dismiss-feedback", dismissFeedback);
  const dismissAction = bindSheetActions(nativeActions, [...sheet, ...sheetActions, ...sheetFooter], nativeSheetRevision.value);
  const keepDialPosition = immersive.value && destination.value === "remote" && controlMode.value === "dpad";
  const nav = ((immersive.value && !keepDialPosition) || destination.value === "keyboard" || destination.value === "windows") ? [] : ([
    ["remote", t("遥控")], ["files", t("文件")], ["settings", t("设置")],
  ] as const).map(([id, label]) => button("nav-" + id, label, () => navigate(id), destination.value !== id));
  const connection = { ...button("connection-chip", connectionLabel.value, () => openPanel(status.value === "unpaired" && !remote.usbExpired.value ? "pair" : "connection")),
    status: online.value ? remote.quality.value : status.value,
    hint: remote.connectionBadgeLabel.value,
    detail: online.value && remote.latency.value !== null ? remote.latency.value + " ms" : "" };
  const screenActions = bindScreenActions(nativeActions, [connection, ...controls, ...nav], nativeScreenRevision.value);
  android?.renderNativeUi?.(JSON.stringify({
    destination: destination.value, side: padOnRight.value, title: screenTitle.value, sessionStatus: status.value, fileTransferAllowed: canFiles.value,
    subtitle: android?.getComputerName?.(), connection, controls, nav, keepDialPosition, feedback: displayMessage(feedback.value),
    dark: darkAppearance.value, microphone, hardwareVolume: hardwareVolumeActive.value,
    sheetKey: customEditing.value !== null ? "custom-key-editor" : nativeChoices.value || (panel.value === "shortcut-plans" ? "shortcut-plans-" + plans.stage.value : panel.value), sheetTitle, sheetIcon, sheetActions, sheetFooter, sheet,
    dismissAction, screenActions,
  }));
});

onMounted(() => {
  refreshKeepAliveState();
  window.addEventListener("plumelink-keepalive", refreshKeepAliveState);
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
  if (status.value === "unpaired" && !remote.token()) openPanel("pair");
  document.addEventListener("visibilitychange", visibility);
  window.addEventListener("pagehide", pagehide);
  window.addEventListener("pageshow", pageshow);
  window.addEventListener("plumelink-native-confirm", nativeConfirm);
});

onUnmounted(() => {
  window.removeEventListener("plumelink-keepalive", refreshKeepAliveState);
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

<template></template>
