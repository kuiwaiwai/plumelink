<script setup lang="ts">
import { t } from "../../../shared/i18n";

import { MousePointer2, Move, Mouse } from "@lucide/vue";
defineProps<{ enabled: boolean; unavailableReason: string; touching: boolean; dragging: boolean }>();
defineEmits<{ down: [event: PointerEvent]; move: [event: PointerEvent]; up: [event: PointerEvent]; cancel: []; lost: [event: PointerEvent]; click: [button: "left" | "right" | "middle"] }>();
</script>
<template>
  <section class="pad-area" :aria-label="t('鼠标操作')">
    <div class="touchpad" :class="{ touching, dragging, unavailable: !enabled }" role="application" :aria-disabled="!enabled"
      :aria-label="t('鼠标触控板：单指移动和点按，双指滚动与缩放，长按拖拽')"
      @pointerdown.prevent="$emit('down', $event)" @pointermove.prevent="$emit('move', $event)" @pointerup.prevent="$emit('up', $event)"
      @pointercancel="$emit('cancel')" @lostpointercapture="$emit('lost', $event)" @contextmenu.prevent>
      <div class="touchpad-topline"><span><Mouse aria-hidden="true" />{{ t('触控板') }}</span><span v-if="enabled">{{ dragging ? t('拖拽中') : t('可操作') }}</span></div>
      <div class="touchpad-guide"><Move v-if="dragging" aria-hidden="true" /><MousePointer2 v-else aria-hidden="true" /><strong>{{ !enabled ? unavailableReason : dragging ? t('松手结束拖拽') : t('滑动，移动指针') }}</strong></div>
      <p v-if="enabled && !dragging" class="touchpad-hint">{{ t('轻点单击') }}<span>·</span>{{ t('双指滚动') }}<span>·</span>{{ t('长按拖拽') }}</p>
    </div>
    <div class="mouse-buttons"><button :disabled="!enabled" @click="$emit('click', 'left')">{{ t('左键') }}</button><span aria-hidden="true"></span><button class="mouse-middle" :disabled="!enabled" @click="$emit('click', 'middle')">{{ t('中键') }}</button><span aria-hidden="true"></span><button :disabled="!enabled" @click="$emit('click', 'right')">{{ t('右键') }}</button></div>
  </section>
</template>
