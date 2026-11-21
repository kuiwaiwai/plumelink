<script setup lang="ts">
import { computed } from "vue";
import { homepage } from "../../../package.json";
import { t, languageOptions, languagePreference } from "../../../shared/i18n";

import { ChevronRight, Monitor, ScanSearch, CircleHelp, Power } from "@lucide/vue";
import SettingSwitch from "./SettingSwitch.vue";
import SettingChoice from "./SettingChoice.vue";
import { shortcutIcons, shortcutIconNames } from "../shortcut-icons";
const sensitivity = defineModel<number>('sensitivity', { required: true });
const keepAwake = defineModel<boolean>('keepAwake', { required: true });
const keepAlive = defineModel<boolean>('keepAlive', { required: true });
const haptics = defineModel<boolean>('haptics', { required: true });
const longPressDrag = defineModel<boolean>('longPressDrag', { required: true });
const scrollDirection = defineModel<string>('scrollDirection', { required: true });
const theme = defineModel<string>('theme', { required: true });
const responseMode = defineModel<string>('responseMode', { required: true });
const shortcutA = defineModel<string>('shortcutA', { required: true });
const shortcutB = defineModel<string>('shortcutB', { required: true });
const shortcutC = defineModel<string>('shortcutC', { required: true });
const shortcutIconA = defineModel<string>('shortcutIconA', { required: true });
const shortcutIconB = defineModel<string>('shortcutIconB', { required: true });
const shortcutIconC = defineModel<string>('shortcutIconC', { required: true });
defineProps<{ native: boolean; canInput: boolean; canPower: boolean; magnifier: boolean; lensPending: boolean; connectionLabel: string }>();
defineEmits<{ connection: []; magnify: []; help: []; shutdown: [] }>();
const themes = computed(() => ([{ value: "system", label: t('跟随系统') }, { value: "light", label: t('浅色') }, { value: "dark", label: t('深色') }]));
const responses = computed(() => ([{ value: "balanced", label: t('均衡'), description: t('兼顾响应与耗电，适合日常使用') }, { value: "responsive", label: t('低延迟'), description: t('提高采样频率，耗电略有增加') }]));
const scrollDirections = computed(() => [
  { value: "follow", label: t('与手指方向一致') },
  { value: "opposite", label: t('与手指方向相反') },
]);
const shortcuts = computed(() => ([{value:"mute",label:t('静音')},{value:"magnifier",label:t('鼠标放大镜')},{value:"taskview",label:t('任务视图')},{value:"notifications",label:t('通知中心')},{value:"previousapp",label:t('上一个应用')},{value:"nextapp",label:t('下一个应用')},{value:"virtualleft",label:t('上一个桌面')},{value:"virtualright",label:t('下一个桌面')}]));
const shortcutChoices = (current: string) => shortcuts.value.filter(option => option.value === current || ![shortcutA.value, shortcutB.value, shortcutC.value].includes(option.value));
const iconNames: Record<string, string> = { star: '星标', bolt: '闪电', bell: '铃铛', grid: '方格', monitor: '电脑', pointer: '指针', volume: '音量', search: '搜索', switch: '切换', left: '向左箭头', right: '向右箭头', layers: '图层', keyboard: '键盘', command: '快捷键', mic: '麦克风图标', gamepad: '游戏手柄', settings: '设置图标', activity: '活动', heart: '爱心', idea: '灯泡' };
const iconOptions = computed(() => shortcutIconNames.map(value => ({ value, label: t(iconNames[value] as Parameters<typeof t>[0]), icon: shortcutIcons[value] })));
</script>
<template>
  <div class="settings-screen page-scroll">
    <section class="settings-section"><h2>{{ t('触控与反馈') }}</h2><div class="settings-list">
      <div class="sensitivity-row"><label for="sensitivity">{{ t('指针灵敏度') }} <output>{{ sensitivity.toFixed(1) }}×</output></label><input id="sensitivity" v-model.number="sensitivity" type="range" min="0.5" max="3" step="0.1" :style="{ '--range-progress': `${(sensitivity - .5) / 2.5 * 100}%` }" :aria-valuetext="t('{0} 倍', sensitivity.toFixed(1))" /><div><span>{{ t('精准') }}</span><span>{{ t('快速') }}</span></div></div>
      <SettingChoice id="response-mode" v-model="responseMode" :label="t('触控响应')" :options="responses" />
      <SettingSwitch v-model="longPressDrag" :label="t('长按拖拽')" :description="t('按住片刻后移动，松手结束')" />
      <SettingChoice id="scroll-direction" v-model="scrollDirection" :label="t('滚动逻辑')" :options="scrollDirections" />
      <SettingSwitch v-model="haptics" :label="t('触觉反馈')" :description="t('操作成功后轻触反馈')" />
    </div></section>
    <section class="settings-section"><h2>{{ t('显示') }}</h2><div class="settings-list">
      <SettingChoice id="language" v-model="languagePreference" :label="t('语言')" :options="languageOptions" />
      <SettingChoice id="theme" v-model="theme" :label="t('外观')" :options="themes" />
      <SettingSwitch v-model="keepAwake" :label="t('保持屏幕常亮')" :description="t('仅在前台连接电脑时生效')" />
      <SettingSwitch v-if="native" v-model="keepAlive" :label="t('应用保活')" :description="t('连接电脑时在后台保持运行，并显示常驻通知')" />
      <button class="setting-row" :disabled="!canInput || lensPending" :aria-pressed="magnifier" @click="$emit('magnify')"><ScanSearch aria-hidden="true" /><span class="row-copy"><strong>{{ t('鼠标放大镜') }}</strong><small>{{ t('放大电脑上的指针附近区域') }}</small></span><span class="row-value">{{ magnifier ? t('已开启') : t('已关闭') }}</span></button>
    </div></section>
    <section class="settings-section"><h2>{{ t('快捷操作') }}</h2><div class="shortcut-settings"><div class="settings-list shortcut-pair"><SettingChoice id="shortcut-a" v-model="shortcutA" :label="t('快捷操作一')" :options="shortcutChoices(shortcutA)" /><SettingChoice id="shortcut-icon-a" v-model="shortcutIconA" :label="t('图标一')" :options="iconOptions" /></div><div class="settings-list shortcut-pair"><SettingChoice id="shortcut-b" v-model="shortcutB" :label="t('快捷操作二')" :options="shortcutChoices(shortcutB)" /><SettingChoice id="shortcut-icon-b" v-model="shortcutIconB" :label="t('图标二')" :options="iconOptions" /></div><div class="settings-list shortcut-pair"><SettingChoice id="shortcut-c" v-model="shortcutC" :label="t('快捷操作三')" :options="shortcutChoices(shortcutC)" /><SettingChoice id="shortcut-icon-c" v-model="shortcutIconC" :label="t('图标三')" :options="iconOptions" /></div></div><p>{{ t('在遥控页的“更多”中使用。') }}</p></section>
    <section class="settings-section"><h2>{{ t('电脑与帮助') }}</h2><div class="settings-list">
      <button class="setting-row" @click="$emit('connection')"><Monitor aria-hidden="true" /><span class="row-copy"><strong>{{ t('当前电脑') }}</strong><small>{{ connectionLabel }}</small></span><ChevronRight aria-hidden="true" /></button>
      <button class="setting-row" @click="$emit('help')"><CircleHelp aria-hidden="true" /><span class="row-copy"><strong>{{ t('手势与操作指南') }}</strong></span><ChevronRight aria-hidden="true" /></button>
      <button class="setting-row danger-text" :disabled="!canPower" @click="$emit('shutdown')"><Power aria-hidden="true" /><span class="row-copy"><strong>{{ t('关闭电脑') }}</strong></span><ChevronRight aria-hidden="true" /></button>
    </div></section>
    <p class="app-footnote"><a :href="homepage" target="_blank" rel="noopener noreferrer">{{ t('羽连 PlumeLink') }}</a> · {{ t('让控制触手可及') }}</p>
  </div>
</template>
