<script setup lang="ts">
import { displayMessage, t } from "../../../shared/i18n";

import { watch } from "vue";
import { Mic, MicOff, LockKeyhole, ArrowUp, X } from "@lucide/vue";
import {
  useMicrophone,
  type MicrophoneProps,
} from "../composables/useMicrophone";
const props = defineProps<MicrophoneProps>();
const emit = defineEmits<{
  notice: [message: string, durationMs?: number];
  locked: [value: boolean];
  active: [value: boolean];
}>();
const {
  mode,
  intent,
  progress,
  phase,
  warning,
  level,
  recording,
  expanded,
  label,
  duration,
  down,
  move,
  up,
  stop,
  keyboard,
  lockedSession,
} = useMicrophone(props, (message, durationMs) => emit("notice", message, durationMs));
watch(mode, (value) => {
  emit("locked", value === "locked" || lockedSession.value);
  emit("active", value !== "idle");
});
</script>
<template>
  <div
    class="mic-control"
    :class="{
      expanded,
      recording,
      locked: mode === 'locked' || lockedSession,
      stopping: intent === 'stop',
    }"
  >
    <Teleport defer to="#microphone-status">
      <Transition name="mic-panel">
        <div
          v-if="mode === 'locked' || lockedSession"
          class="mic-compact"
          :class="{ stopping: intent === 'stop' }"
          :style="{ '--progress': progress }"
        >
          <MicOff v-if="warning" aria-hidden="true" /><Mic v-else aria-hidden="true" /><span class="mic-compact-copy" :title="displayMessage(warning)"
            ><strong>{{ phase === "reconnecting" ? t('正在重连…') : phase === "connecting" ? t('连接中') : warning ? t('收音受限') : t('收音中') }}</strong
            ><time>{{ duration }}</time></span
          >
          <span v-if="warning" class="sr-only" role="status">{{ displayMessage(warning) }}</span>
          <span
            class="mic-compact-level"
            role="meter"
            :aria-label="t('实时麦克风音量')"
            :aria-valuenow="Math.round(level * 100)"
            aria-valuemin="0"
            aria-valuemax="100"
            ><i
              v-for="weight in [0.5, 1, 0.7]"
              :key="weight"
              :style="{ transform: `scaleY(${0.15 + level * weight * 0.85})` }"
            ></i
          ></span>
          <button
            class="mic-compact-stop"
            :aria-label="t('立即停止麦克风')"
            @click="stop"
          >
            <X aria-hidden="true" /><svg
              class="mic-progress"
              viewBox="0 0 48 48"
              aria-hidden="true"
            >
              <circle cx="24" cy="24" r="21" />
            </svg>
          </button>
        </div>
      </Transition>
    </Teleport>
    <Transition name="mic-panel">
      <div v-if="expanded" class="mic-capsule" role="status" aria-live="polite">
        <div
          class="mic-lock-target"
          :class="{ dwelling: intent === 'lock' }"
          :style="{ '--progress': progress }"
        >
          <LockKeyhole v-if="intent === 'lock'" /><ArrowUp v-else /><svg
            class="mic-progress"
            viewBox="0 0 48 48"
            aria-hidden="true"
          >
            <circle cx="24" cy="24" r="21" />
          </svg>
        </div>
        <div class="mic-live-copy">
          <strong>{{ label }}</strong
          ><small>{{
            displayMessage(warning) || (recording ? t('{0} · 正在收音', duration) : t('连接成功后开始收音'))
          }}</small>
        </div>
        <div
          class="mic-meter"
          :aria-label="t('实时麦克风音量')"
          role="meter"
          :aria-valuenow="Math.round(level * 100)"
          aria-valuemin="0"
          aria-valuemax="100"
        >
          <i
            v-for="(weight, index) in [0.45, 0.75, 1, 0.65, 0.9, 0.55, 0.35]"
            :key="index"
            :style="{ transform: `scaleY(${0.12 + level * weight * 0.88})` }"
          ></i>
        </div>
      </div>
    </Transition>
    <button
      class="mic-button"
      :disabled="mode === 'locked' ? !allowed : !enabled"
      :aria-label="
        mode === 'locked' ? t('立即停止麦克风') : t('按住说话，上滑锁定')
      "
      :aria-pressed="mode === 'locked'"
      aria-describedby="mic-instruction"
      :style="{ '--progress': progress, '--level': level }"
      @pointerdown.prevent="down"
      @pointermove.prevent="move"
      @pointerup.prevent="up($event, true)"
      @pointercancel="stop"
      @lostpointercapture="up"
      @contextmenu.prevent
      @keydown="keyboard($event, true)"
      @keyup="keyboard($event, false)"
      @blur="up()"
    >
      <span class="mic-symbol"
        ><MicOff v-if="intent === 'stop'" aria-hidden="true" /><Mic
          v-else
          aria-hidden="true" /><LockKeyhole
          v-if="mode === 'locked'"
          class="mic-lock-badge"
          aria-hidden="true" /><svg
          class="mic-progress"
          viewBox="0 0 48 48"
          aria-hidden="true"
        >
          <circle cx="24" cy="24" r="21" /></svg
      ></span>
      <span>{{ mode === "locked" ? t('持续收音') : t('麦克风') }}</span>
    </button>
    <span id="mic-instruction" class="sr-only"
      >{{ t('按住说话，松手停止；上滑停留 1 秒锁定。锁定后可从通知中停止。') }}</span
    >
  </div>
</template>
