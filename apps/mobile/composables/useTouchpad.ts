import { ref, type ComputedRef, type Ref } from "vue";
import type { Command } from "../../../shared/protocol";
import { MotionBuffer, TouchpadGesture, pointerGain, type Point, type TouchContact } from "../gestures";

type TouchpadOptions = {
  canInput: ComputedRef<boolean>;
  online: ComputedRef<boolean>;
  panelOpen: ComputedRef<boolean>;
  sensitivity: Ref<number>;
  longPressDrag: Ref<boolean>;
  scrollDirection: Ref<string>;
  sendCommand: (command: Command) => void;
  sendMotion: (sample: { dx: number; dy: number; scroll: number }) => void;
  clearRemoteMotion: () => void;
  onMotionSent: () => void;
};

export function useTouchpad(options: TouchpadOptions) {
  const touching = ref(false);
  const dragLocked = ref(false);
  const motion = new MotionBuffer(15);
  const rejectedTouchIds = new Set<number>();

  let tapTimer: ReturnType<typeof setTimeout> | undefined;
  let longPressTimer: ReturnType<typeof setTimeout> | undefined;
  let pressStart = { x: 0, y: 0 };
  let ticker: number | undefined;
  let lastMoveAt = 0;
  let lastScrollAt = 0;
  let scrollVelocity = 0;
  let hScroll = 0;
  let lastInertiaAt = 0;
  let lastTapPoint = { x: 0, y: 0, t: 0 };
  let suppressTap = false;
  let doubleArmed = false;
  // A pending first tap can still become a single click.
  let doublePending = false;
  let doubleStart = { x: 0, y: 0 };

  function sendSingleTap() {
    options.sendCommand({ type: "click", button: "left", count: 1 });
  }

  function tap() {
    lastTapPoint.t = performance.now();
    tapTimer = setTimeout(() => {
      tapTimer = undefined;
      sendSingleTap();
    }, 280);
  }

  const gesture = new TouchpadGesture((kind, x, y) => {
    if (!options.canInput.value || (kind === "tap" && suppressTap)) return;
    const now = performance.now();
    if (kind === "move") {
      const dt = lastMoveAt ? now - lastMoveAt : 16;
      lastMoveAt = now;
      const gain = pointerGain(x, y, dt) * options.sensitivity.value;
      motion.add(x * gain, y * gain);
    } else if (kind === "scroll") {
      const dt = lastScrollAt ? Math.max(4, Math.min(80, now - lastScrollAt)) : 16;
      lastScrollAt = now;
      const direction = options.scrollDirection.value === "follow" ? -4 : 4;
      const step = y * direction;
      motion.scroll = Math.max(-600, Math.min(600, motion.scroll + step));
      scrollVelocity = scrollVelocity * 0.55 + (step / dt) * 0.45;
      hScroll = Math.max(-600, Math.min(600, hScroll + x * direction));
    } else if (kind === "zoom") {
      options.sendCommand({ type: "zoom", direction: x > 0 ? "in" : "out" });
    } else if (kind === "righttap") {
      options.sendCommand({ type: "click", button: "right", count: 1 });
    } else if (kind === "tap") {
      tap();
    }
  });

  function startTicker() {
    if (ticker !== undefined || !options.online.value || document.hidden) return;
    ticker = requestAnimationFrame(frameNow => {
      ticker = undefined;
      if (Math.abs(scrollVelocity) > 0.05 && gesture.count === 0) {
        const dt = Math.max(4, Math.min(48, frameNow - lastInertiaAt));
        lastInertiaAt = frameNow;
        motion.scroll = Math.max(-600, Math.min(600, motion.scroll + scrollVelocity * dt));
        scrollVelocity *= Math.pow(0.9, dt / 16);
        if (Math.abs(scrollVelocity) <= 0.05) scrollVelocity = 0;
      }
      const sample = motion.drain();
      if (sample.dx || sample.dy || sample.scroll) {
        options.onMotionSent();
        options.sendMotion(sample);
      }
      const hStep = Math.trunc(hScroll / 15) * 15 || 0;
      if (hStep) {
        hScroll -= hStep;
        options.onMotionSent();
        options.sendCommand({ type: "scroll", dx: Math.max(-600, Math.min(600, hStep)), dy: 0 });
      }
      if (motion.hasPending() || Math.abs(scrollVelocity) > 0.05 || Math.abs(hScroll) >= 15) {
        startTicker();
      }
    });
  }

  function down(e: PointerEvent) {
    if (!options.canInput.value || options.panelOpen.value || (e.pointerType === "mouse" && e.button !== 0)) return;
    const el = e.currentTarget as HTMLElement;
    if (e.pointerType === "touch") {
      const contactWide = (e.width ?? 0) > 48 || (e.height ?? 0) > 48;
      const rect = el.getBoundingClientRect();
      const edgeGrip =
        gesture.count === 0 &&
        rect.width > 120 &&
        (e.clientX < rect.left + 10 || e.clientX > rect.right - 10);
      if (contactWide || edgeGrip) {
        rejectedTouchIds.add(e.pointerId);
        return;
      }
    }
    rejectedTouchIds.delete(e.pointerId);
    el.setPointerCapture(e.pointerId);
    touching.value = true;
    scrollVelocity = 0;
    if (gesture.count === 0) {
      lastMoveAt = performance.now();
    }
    const now = performance.now();
    const secondContact = gesture.count > 0;
    if (secondContact) {
      // Preserve the preceding tap when a second contact cancels double-tap detection.
      const restoreTap = doublePending;
      doubleArmed = false;
      doublePending = false;
      lastTapPoint = { x: 0, y: 0, t: 0 };
      if (restoreTap) sendSingleTap();
      clearTimeout(longPressTimer);
      if (dragLocked.value) {
        dragLocked.value = false;
        options.sendCommand({ type: "buttonstate", button: "left", down: false });
      }
    } else {
      if (tapTimer !== undefined) {
        clearTimeout(tapTimer);
        tapTimer = undefined;
        if (now - lastTapPoint.t < 280 && Math.hypot(e.clientX - lastTapPoint.x, e.clientY - lastTapPoint.y) < 40) {
          doubleArmed = true;
          doublePending = true;
          doubleStart = { x: e.clientX, y: e.clientY };
        } else {
          sendSingleTap();
        }
      }
      lastTapPoint = { x: e.clientX, y: e.clientY, t: now };
    }
    if (!secondContact && e.isPrimary && options.longPressDrag.value) {
      pressStart = { x: e.clientX, y: e.clientY };
      clearTimeout(longPressTimer);
      longPressTimer = setTimeout(() => {
        if (doublePending) {
          sendSingleTap();
          doublePending = false;
        }
        dragLocked.value = true;
        options.sendCommand({ type: "buttonstate", button: "left", down: true });
      }, 430);
    }
    gesture.down(e.pointerId, { x: e.clientX, y: e.clientY }, now, rejectedTouchIds.size);
  }

  function cancelPendingDrag(point: Point) {
    if (!dragLocked.value && Math.hypot(point.x - pressStart.x, point.y - pressStart.y) > 8) {
      clearTimeout(longPressTimer);
    }
  }

  function beginDoubleDrag(point: Point) {
    if (!doubleArmed || dragLocked.value) return;
    if (Math.hypot(point.x - doubleStart.x, point.y - doubleStart.y) <= 8) return;
    clearTimeout(longPressTimer);
    clearTimeout(tapTimer);
    tapTimer = undefined;
    if (doublePending) {
      doublePending = false;
      options.sendCommand({ type: "click", button: "left", count: 1 });
    }
    dragLocked.value = true;
    options.sendCommand({ type: "buttonstate", button: "left", down: true });
  }

  function move(e: PointerEvent) {
    if (rejectedTouchIds.has(e.pointerId)) return;
    beginDoubleDrag({ x: e.clientX, y: e.clientY });
    cancelPendingDrag({ x: e.clientX, y: e.clientY });
    const samples = typeof e.getCoalescedEvents === "function" ? e.getCoalescedEvents() : undefined;
    if (samples && samples.length > 0) {
      for (const sample of samples) gesture.move(e.pointerId, { x: sample.clientX, y: sample.clientY });
    } else {
      gesture.move(e.pointerId, { x: e.clientX, y: e.clientY });
    }
    if (motion.hasPending() || Math.abs(hScroll) >= 15) startTicker();
  }

  function moveTogether(contacts: readonly TouchContact[]) {
    const accepted = contacts.filter(point => !rejectedTouchIds.has(point.pointerId));
    for (const point of accepted) {
      beginDoubleDrag(point);
      cancelPendingDrag(point);
    }
    gesture.moveTogether(accepted);
    if (motion.hasPending() || Math.abs(hScroll) >= 15) startTicker();
  }

  function up(e: PointerEvent) {
    if (rejectedTouchIds.delete(e.pointerId)) return;
    clearTimeout(longPressTimer);
    // A fast swipe may report its last movement only on pointerup.
    const point = { x: e.clientX, y: e.clientY };
    if (gesture.hasFinalMovement(e.pointerId, point)) {
      beginDoubleDrag(point);
      gesture.move(e.pointerId, point);
      if (motion.hasPending()) startTicker();
    }
    const now = performance.now();
    const wasDouble = doubleArmed;
    doubleArmed = false;
    doublePending = false;
    suppressTap = wasDouble;
    try {
      gesture.up(e.pointerId, now);
    } finally {
      suppressTap = false;
    }
    if (wasDouble) {
      if (dragLocked.value) {
        dragLocked.value = false;
        options.sendCommand({ type: "buttonstate", button: "left", down: false });
      } else if (gesture.count === 0) {
        options.sendCommand({ type: "click", button: "left", count: 2 });
      }
    }
    if (dragLocked.value) {
      dragLocked.value = false;
      options.sendCommand({ type: "buttonstate", button: "left", down: false });
    }
    touching.value = gesture.count > 0;
    if (gesture.count === 0 && (now - lastScrollAt < 90 && Math.abs(scrollVelocity) >= 0.15)) {
      lastInertiaAt = now;
      startTicker();
    } else if (gesture.count === 0) {
      scrollVelocity = 0;
    }
  }

  function captureLost(e: PointerEvent) {
    if (rejectedTouchIds.delete(e.pointerId)) return;
    if (!gesture.lost(e.pointerId)) return;
    touching.value = gesture.count > 0;
    doubleArmed = false;
    doublePending = false;
    suppressTap = false;
    clearTimeout(longPressTimer);
    clearTimeout(tapTimer);
    tapTimer = undefined;
    if (dragLocked.value) {
      dragLocked.value = false;
      options.sendCommand({ type: "buttonstate", button: "left", down: false });
    }
  }

  function reset() {
    options.clearRemoteMotion();
    if (ticker !== undefined) cancelAnimationFrame(ticker);
    ticker = undefined;
    scrollVelocity = 0;
    hScroll = 0;
    doubleArmed = false;
    doublePending = false;
    suppressTap = false;
    lastTapPoint = { x: 0, y: 0, t: 0 };
    rejectedTouchIds.clear();
    gesture.cancel();
    motion.clear();
    clearTimeout(tapTimer);
    clearTimeout(longPressTimer);
    if (dragLocked.value && options.online.value) {
      options.sendCommand({ type: "buttonstate", button: "left", down: false });
    }
    dragLocked.value = false;
    tapTimer = undefined;
    touching.value = false;
  }

  function cancel() {
    reset();
    if (options.canInput.value) options.sendCommand({ type: "release" });
  }

  return {
    touching,
    dragLocked,
    drainMotion: () => motion.drain(),
    clearMotion: () => motion.clear(),
    down,
    move,
    moveTogether,
    up,
    captureLost,
    reset,
    cancel,
  };
}
