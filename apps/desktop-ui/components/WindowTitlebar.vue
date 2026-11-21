<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../../../shared/i18n";
import brand from "../assets/brand.svg";

const emit = defineEmits<{ error: [message: string] }>();
const maximized = ref(false);
const pending = ref(false);
let disposed = false;
let stateRequest = 0;
let resizeFrame = 0;

async function withTimeout<T>(operation: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error("Window operation timeout")), 5000);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function refreshState() {
  const request = ++stateRequest;
  const value = await withTimeout(getCurrentWindow().isMaximized());
  if (!disposed && request === stateRequest) maximized.value = value;
}

function reportError() {
  if (!disposed) emit("error", t("窗口操作失败，请稍后重试"));
}

async function control(action: "hide" | "toggleMaximize" | "close") {
  if (pending.value) return;
  pending.value = true;
  try {
    await withTimeout(getCurrentWindow()[action]());
    if (action === "toggleMaximize") await refreshState();
  } catch {
    reportError();
  } finally {
    pending.value = false;
  }
}

function drag(event: MouseEvent) {
  if (event.button !== 0) return;
  if (event.detail === 2) void control("toggleMaximize");
  else void getCurrentWindow().startDragging().catch(reportError);
}

function onResize() {
  if (resizeFrame) return;
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = 0;
    void refreshState().catch(reportError);
  });
}

onMounted(() => {
  onResize();
  window.addEventListener("resize", onResize);
  window.addEventListener("focus", onResize);
});

onUnmounted(() => {
  disposed = true;
  cancelAnimationFrame(resizeFrame);
  window.removeEventListener("resize", onResize);
  window.removeEventListener("focus", onResize);
});
</script>

<template>
  <header class="window-titlebar" :aria-label="t('应用标题栏')">
    <div class="window-drag-region" @mousedown="drag">
      <img :src="brand" class="titlebar-brand" alt="" draggable="false" />
      <span>{{ t('羽连 PlumeLink') }}</span>
    </div>
    <div class="window-controls" role="group" :aria-label="t('窗口控制')" :aria-busy="pending">
      <button :disabled="pending" :aria-label="t('最小化')" :title="t('最小化到托盘')" @click="control('hide')">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8h10" /></svg>
      </button>
      <button :disabled="pending" :aria-label="maximized ? t('还原窗口') : t('最大化')" :title="maximized ? t('还原窗口') : t('最大化')" @click="control('toggleMaximize')">
        <svg v-if="maximized" viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 5.5v-3h8v8h-3M2.5 5.5h8v8h-8z" /></svg>
        <svg v-else viewBox="0 0 16 16" aria-hidden="true"><path d="M3 3h10v10H3z" /></svg>
      </button>
      <button class="window-close" :disabled="pending" :aria-label="t('关闭窗口')" :title="t('关闭到托盘，保持遥控运行')" @click="control('close')">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="m3.5 3.5 9 9m0-9-9 9" /></svg>
      </button>
    </div>
  </header>
</template>
