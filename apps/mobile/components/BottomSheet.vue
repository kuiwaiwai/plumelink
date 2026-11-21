<script setup lang="ts">
import { t } from "../../../shared/i18n";

import { nextTick, onBeforeUnmount, ref, watch, useId } from "vue";

const props = defineProps<{ open: boolean; title: string }>();
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement>();
const titleId = useId();
let returnFocus: HTMLElement | null = null;
let previousOverflow = "";
let backdropDown: { x: number; y: number; id: number } | null = null;
let backdropReady = false;
function outside(e: PointerEvent) {
  const r = dialog.value?.querySelector(".sheet-surface")?.getBoundingClientRect();
  return !!r && (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom);
}
function backdropStart(e: PointerEvent) { backdropReady = false; backdropDown = outside(e) ? { x: e.clientX, y: e.clientY, id: e.pointerId } : null; }
function backdropEnd(e: PointerEvent) {
  const start = backdropDown; backdropDown = null;
  backdropReady = !!start && start.id === e.pointerId && outside(e) && Math.hypot(e.clientX - start.x, e.clientY - start.y) < 10;
}
function backdropClick(e: MouseEvent) {
  e.stopPropagation();
  if (backdropReady && outside(e as PointerEvent)) {
    e.preventDefault();
    emit("close");
  }
  backdropReady = false;
}
let locked = false;
const dragOffset = ref(0);
const dragging = ref(false);
const expanded = ref(false);
let swipe: { y: number; id: number } | null = null;
let ignoreHandleClick = false;
function handleClick() {
  if (ignoreHandleClick) { ignoreHandleClick = false; return; }
  expanded.value = !expanded.value;
}
function dragStart(event: PointerEvent) {
  if (!event.isPrimary || event.button !== 0 || swipe) return;
  const handle = event.currentTarget as HTMLElement;
  const currentOffset = handle.getBoundingClientRect().top - dialog.value!.getBoundingClientRect().top;
  swipe = { y: event.clientY - currentOffset, id: event.pointerId };
  dragOffset.value = Math.max(0, currentOffset);
  dragging.value = true;
  handle.setPointerCapture(event.pointerId);
}
function dragMove(event: PointerEvent) {
  if (swipe?.id === event.pointerId) dragOffset.value = Math.max(0, event.clientY - swipe.y);
}
function resetDrag() {
  swipe = null;
  dragging.value = false;
  dragOffset.value = 0;
}
function dragEnd(event: PointerEvent) {
  if (swipe?.id !== event.pointerId) return;
  const distance = event.clientY - swipe.y;
  resetDrag();
  ignoreHandleClick = Math.abs(distance) >= 10;
  if (distance <= -60) expanded.value = true;
  else if (distance >= 60) {
    if (expanded.value) expanded.value = false;
    else emit("close");
  }
}
function dragCancel(event: PointerEvent) {
  if (swipe?.id === event.pointerId) resetDrag();
}

function viewport() {
  const vv = window.visualViewport;
  // Follow the visible area when the software keyboard opens. Do not constrain pinch zoom.
  const height = vv && vv.scale === 1 ? vv.height : window.innerHeight;
  const bottom = vv && vv.scale === 1 ? Math.max(0, window.innerHeight - vv.height - vv.offsetTop) : 0;
  dialog.value?.style.setProperty("--sheet-height", `${Math.max(120, height - 12)}px`);
  dialog.value?.style.setProperty("--sheet-default-height", `${Math.max(120, Math.min(height - 12, Math.round(height * .88)))}px`);
  dialog.value?.style.setProperty("--sheet-bottom", `${bottom}px`);
}
function unlock() {
  window.visualViewport?.removeEventListener("resize", viewport);
  window.visualViewport?.removeEventListener("scroll", viewport);
  window.removeEventListener("resize", viewport);
  if (locked) document.body.style.overflow = previousOverflow;
  locked = false;
}
watch(() => props.open, async (open) => {
  if (open) {
    returnFocus = document.activeElement as HTMLElement;
    previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    locked = true;
    viewport();
    dialog.value?.showModal();
    await nextTick();
    if (!props.open) return;
    // A clearly focusable field remains available if iOS declines keyboard activation.
    dialog.value?.querySelector<HTMLElement>("[data-initial-focus]")?.focus({ preventScroll: true });
    window.visualViewport?.addEventListener("resize", viewport);
    window.visualViewport?.addEventListener("scroll", viewport);
    window.addEventListener("resize", viewport);
  } else {
    resetDrag();
    expanded.value = false;
    backdropReady = false;
    dialog.value?.close();
    unlock();
    if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
  }
}, { flush: "post" });
onBeforeUnmount(() => { dialog.value?.close(); unlock(); });
</script>

<template>
  <dialog ref="dialog" class="bottom-sheet" :aria-labelledby="titleId"
    @cancel.prevent="emit('close')"
    @pointerdown.stop="backdropStart" @pointerup.stop="backdropEnd" @pointercancel="backdropDown = null" @click.stop="backdropClick">
    <div class="sheet-surface" :class="{ dragging, expanded }" :style="{ '--drag-offset': dragOffset + 'px' }">
      <button type="button" class="sheet-drag-area" :aria-label="expanded ? t('收起抽屉') : t('展开抽屉')" :aria-expanded="expanded"
        @click="handleClick" @pointerdown.prevent="dragStart" @pointermove="dragMove"
        @pointerup="dragEnd" @pointercancel="dragCancel" @lostpointercapture="dragCancel"><span class="sheet-grabber" aria-hidden="true"></span></button>
      <header class="sheet-header"><h2 :id="titleId">{{ title }}</h2></header>
      <div class="sheet-content"><slot /></div>
    </div>
  </dialog>
</template>
