import { t } from "../../../shared/i18n";
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { MicrophoneGesture } from "../microphone-gesture";
import { androidBridge, type MicrophoneState } from "../native/bridge";

export type MicrophoneProps = {
  enabled: boolean;
  allowed: boolean;
  activePage: boolean;
};
export function useMicrophone(
  props: MicrophoneProps,
  notice: (message: string, durationMs?: number) => void,
) {
  const native = androidBridge();
  const mode = ref("idle"),
    intent = ref<string | null>(null),
    progress = ref(0),
    phase = ref("idle"),
    warning = ref(""),
    level = ref(0),
    elapsed = ref(0);
  const lockedSession = ref(false);
  const lastIssue = ref("");
  let dismissedIssue: { id: number; message: string } | undefined;
  function dismissIssue() {
    dismissedIssue = { id: request, message: lastIssue.value };
    lastIssue.value = "";
  }
  const permissionGranted = ref<boolean>();
  function syncPermission() {
    try { permissionGranted.value = native?.hasMicrophonePermission?.(); } catch {}
  }
  const nativeLocked = ref(false);
  let restorePending = true;
  let lockOnReady: number | undefined;
  let request = Date.now() % 1_000_000_000,
    pointer: number | null = null,
    frame = 0,
    clock: ReturnType<typeof setInterval> | undefined,
    began = 0;
  const recording = computed(() => phase.value === "live");
  const expanded = computed(
    () =>
      mode.value === "holding" ||
      (phase.value === "connecting" && mode.value !== "locked"),
  );
  const label = computed(() =>
    intent.value === "stop"
      ? t('继续按住，关闭麦克风')
      : intent.value === "lock"
        ? t('停留 1 秒，锁定收音')
        : phase.value === "connecting"
          ? t('正在连接电脑…')
          : mode.value === "locked"
            ? t('常态麦克风')
            : recording.value
              ? t('松手停止 · 上滑锁定')
              : t('手机麦克风'),
  );
  const duration = computed(
    () =>
      `${Math.floor(elapsed.value / 60)
        .toString()
        .padStart(2, "0")}:${(elapsed.value % 60).toString().padStart(2, "0")}`,
  );
  const gesture = new MicrophoneGesture((event) => {
    if (event === "start") {
      if (!native?.startMicrophone) {
        gesture.cancel();
        notice(t('请在 Android 应用中使用手机麦克风'));
        return;
      }
      lockOnReady = undefined;
      phase.value = "connecting";
      lastIssue.value = "";
      dismissedIssue = undefined;
      warning.value = "";
      restorePending = false;
      request++;
      native.startMicrophone(request);
    } else if (event === "stop") {
      restorePending = false;
      lockOnReady = undefined;
      lockedSession.value = false;
      native?.stopMicrophone?.(request);
      phase.value = "idle";
      warning.value = "";
      level.value = 0;
      clearInterval(clock);
    } else {
      lockedSession.value = true;
      nativeLocked.value = true;
      native?.lockMicrophone?.(request);
      native?.haptic?.();
    }
  });
  function sync() {
    mode.value = gesture.mode;
    intent.value = gesture.intent;
    progress.value = gesture.progress;
  }
  function animate() {
    gesture.tick(performance.now());
    sync();
    if (pointer !== null) frame = requestAnimationFrame(animate);
  }
  function down(e: PointerEvent) {
    if (
      !(mode.value === "locked" ? props.allowed : props.enabled) ||
      !props.activePage ||
      pointer !== null ||
      (e.pointerType === "mouse" && e.button !== 0)
    )
      return;
    pointer = e.pointerId;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture.down(e.clientX, e.clientY, performance.now());
    sync();
    frame = requestAnimationFrame(animate);
  }
  function move(e: PointerEvent) {
    if (pointer === e.pointerId) {
      gesture.move(e.clientX, e.clientY, performance.now());
      sync();
    }
  }
  function up(e?: PointerEvent, completed = false) {
    if (e && e.pointerId !== pointer) return;
    pointer = null;
    cancelAnimationFrame(frame);
    if (completed && e) gesture.move(e.clientX, e.clientY, performance.now());
    gesture.up(completed ? performance.now() : undefined);
    sync();
  }
  function stop() {
    lockOnReady = undefined;
    lockedSession.value = false;
    clearInterval(clock);
    clock = undefined;
    pointer = null;
    cancelAnimationFrame(frame);
    gesture.cancel();
    sync();
    nativeLocked.value = false;
  }
  function resetLocal(terminal = "idle") {
    restorePending = false;
    lockOnReady = undefined;
    lockedSession.value = false;
    nativeLocked.value = false;
    clearInterval(clock);
    clock = undefined;
    pointer = null;
    cancelAnimationFrame(frame);
    gesture.reset();
    phase.value = terminal;
    warning.value = "";
    level.value = 0;
    sync();
  }
  function beginLocked() {
    if (!props.allowed || !props.enabled || document.hidden || pointer !== null
      || !native?.startMicrophone || !native.lockMicrophone || gesture.mode !== "idle"
      || ["connecting", "reconnecting", "live"].includes(phase.value)) return false;
    resetLocal();
    request++;
    lockOnReady = request;
    lastIssue.value = "";
    dismissedIssue = undefined;
    phase.value = "connecting";
    lockedSession.value = true;
    gesture.restoreLocked();
    sync();
    native.startMicrophone(request);
    return true;
  }
  function startLocked() {
    syncPermission();
    return !lastIssue.value && beginLocked();
  }
  function restartLocked() {
    syncPermission();
    return permissionGranted.value === true && !!lastIssue.value && beginLocked();
  }
  function keyboard(e: KeyboardEvent, pressed: boolean) {
    if (![" ", "Enter"].includes(e.key) || e.repeat) return;
    e.preventDefault();
    if (
      pressed &&
      (mode.value === "locked" ? props.allowed : props.enabled) &&
      props.activePage
    ) {
      pointer = -1;
      gesture.down(0, 0, performance.now());
      frame = requestAnimationFrame(animate);
    } else if (!pressed && pointer === -1) up(undefined, true);
  }
  function nativeState(e: Event) {
    const value = (e as CustomEvent<unknown>).detail;
    if (!value || typeof value !== "object") return;
    const state = value as MicrophoneState;
    if (!Number.isSafeInteger(state.id) || typeof state.phase !== "string") return;
    if (state.locked !== undefined && typeof state.locked !== "boolean") return;
    if (!["idle", "connecting", "reconnecting", "live", "error", "permission"].includes(state.phase)) return;
    const isSnapshot = state.snapshot === true;
    const active = ["live", "connecting", "reconnecting"].includes(state.phase);
    if (isSnapshot && !restorePending && (state.id !== request || (gesture.mode === "idle" && active))) return;
    // A new page can restore the capture owned by the service.
    if (isSnapshot && restorePending && ["error", "permission"].includes(state.phase)) {
      restorePending = false;
      request = state.id;
    }
    if (isSnapshot && (state.locked === true || (lockedSession.value && state.phase === "live")) && active) {
      restorePending = false;
      if (state.locked) {
        lockedSession.value = true;
        nativeLocked.value = true;
      }
      request = state.id;
      if (gesture.mode !== "locked") {
        gesture.restoreLocked();
        sync();
      }
    } else if (isSnapshot && gesture.mode === "locked" && ["idle", "error"].includes(state.phase)) {
      request = state.id;
    }
    if (state.id !== request) return;
    if (lockOnReady === request && active) {
      lockOnReady = undefined;
      nativeLocked.value = true;
      native?.lockMicrophone?.(request);
    }
    if (state.phase !== phase.value || isSnapshot) syncPermission();
    if (["error", "permission"].includes(state.phase) && typeof state.message === "string") {
      const message = state.message.slice(0, 1024);
      if (dismissedIssue?.id !== state.id || dismissedIssue.message !== message) lastIssue.value = message;
    }
    if (state.phase === "live") { lastIssue.value = ""; dismissedIssue = undefined; }
    // Keep the lock through reconnect snapshots that omit its state.
    if (state.phase === "idle" && gesture.mode === "locked" && state.locked !== false) return;
    if (state.phase === "live" && gesture.mode === "idle") {
      native?.stopMicrophone?.(request);
      return;
    }
    const first = phase.value !== "live" && state.phase === "live";
    const nextWarning = state.phase === "live" && typeof state.message === "string" ? state.message : "";
    if (nextWarning && nextWarning !== warning.value) notice(nextWarning);
    warning.value = nextWarning;
    phase.value = state.phase;
    const nextLevel = typeof state.level === "number" && Number.isFinite(state.level) ? state.level : 0;
    level.value = Math.max(0, Math.min(1, nextLevel));
    if (first) {
      began = typeof state.began === "number" && Number.isFinite(state.began) ? state.began : Date.now();
      elapsed.value = Math.floor((Date.now() - began) / 1000);
      clearInterval(clock);
      clock = setInterval(
        () => (elapsed.value = Math.floor((Date.now() - began) / 1000)),
        1000,
      );
      native?.haptic?.();
    }
    if (state.phase === "error" || state.phase === "permission" ||
        (state.phase === "idle" && (gesture.mode !== "locked" || state.locked === false))) {
      resetLocal(state.phase);
      if (!isSnapshot && typeof state.message === "string" && state.message) {
        if (state.phase === "error" && state.message === "请在电脑端选择虚拟音频设备") {
          notice(t("请在电脑端打开「偏好设置 → 手机麦克风」，在「音频送往」中选择虚拟音频设备，再点击「保存音频设备」。"), 10000);
        } else {
          notice(state.message);
        }
      }
    }
  }
  function visibility() {
    if (document.hidden) background();
    else resume();
  }
  function background() {
    if (gesture.mode === "locked" && native?.lockMicrophone) up();
    else stop();
  }
  function resume() {
    syncPermission();
    native?.syncSession?.();
  }
  watch(
    () => [props.allowed, props.enabled, props.activePage],
    (_, previous) => {
      // Locked capture survives navigation, but still requires a connection and permission.
      if (!props.allowed || !props.enabled) {
        if (!lockedSession.value || nativeLocked.value) stop();
      }
      else if (!props.activePage && gesture.mode !== "locked") stop();
    },
  );

  onMounted(() => {
    syncPermission();
    window.addEventListener("plumelink-microphone", nativeState);
    window.addEventListener("pagehide", background);
    window.addEventListener("pageshow", resume);
    document.addEventListener("visibilitychange", visibility);
    native?.syncSession?.();
  });
  onUnmounted(() => {
    background();
    clearInterval(clock);
    window.removeEventListener("plumelink-microphone", nativeState);
    window.removeEventListener("pagehide", background);
    window.removeEventListener("pageshow", resume);
    document.removeEventListener("visibilitychange", visibility);
  });
  return {
    mode,
    intent,
    progress,
    phase,
    warning,
    lastIssue,
    dismissIssue,
    permissionGranted,
    level,
    recording,
    expanded,
    label,
    duration,
    down,
    move,
    up,
    stop,
    startLocked,
    restartLocked,
    keyboard,
    lockedSession,
  };
}
