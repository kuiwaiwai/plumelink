import { t } from "../../shared/i18n";
import type { Command } from "../../shared/protocol";
export type Motion = { dx: number; dy: number; scroll: number };
import wasmUrl from "./generated/qft.wasm?url";

export interface Kernel {
  memory: WebAssembly.Memory;
  qft_input_ptr(): number;
  qft_output_ptr(): number;
  qft_init(
    start: number,
    now: number,
    interval: number,
    leaseMs: number,
    motionTtlMs: number,
    actionTtlMs: number,
  ): number;
  qft_set_interval(interval: number): number;
  qft_offer(
    x: number,
    y: number,
    z: number,
    captured: number,
    now: number,
  ): number;
  qft_action(argument: number, captured: number, now: number): number;
  qft_step(x: number, y: number, z: number, motionAt: number, argument: number, actionAt: number, now: number): number;
  qft_receive(now: number): number;
  qft_poll(now: number, writable: number): number;
  qft_next(now: number, writable: number): number;
  qft_renew(now: number): number;
  qft_cancel(now: number): number;
  qft_closed(): number;
  qft_outcome(): number;
  qft_metric(which: number): number;
}
export type QftCapability = {
  version: 2;
  frameBytes: 64;
  session: string;
  leaseMs: number;
  motionTtlMs: number;
  actionTtlMs: number;
  maxCatchup: 16384;
  adapter: "logical-pointer-v1";
};
export function supportsQft(value: unknown): value is QftCapability {
  if (!value || typeof value !== "object" || typeof WebAssembly !== "object")
    return false;
  const c = value as QftCapability;
  return (
    c.version === 2 &&
    c.frameBytes === 64 &&
    Number.isInteger(c.leaseMs) && c.leaseMs >= 5000 && c.leaseMs <= 60000 &&
    Number.isInteger(c.motionTtlMs) && c.motionTtlMs >= 120 && c.motionTtlMs <= c.leaseMs &&
    Number.isInteger(c.actionTtlMs) && c.actionTtlMs >= 200 && c.actionTtlMs <= c.leaseMs &&
    c.maxCatchup === 16384 &&
    c.adapter === "logical-pointer-v1" &&
    typeof c.session === "string" &&
    /^[1-9][0-9]{0,19}$/.test(c.session) &&
    (() => { try { return BigInt(c.session) <= 0xffffffffffffffffn; } catch { return false; } })()
  );
}
let compiled: Promise<WebAssembly.Module> | undefined;
async function compileKernel() {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 3000);
  try {
    const response = await fetch(wasmUrl, { signal: controller.signal });
    if (!response.ok) throw new Error(t('传输组件无法载入'));
    return WebAssembly.compile(await response.arrayBuffer());
  } finally {
    clearTimeout(timer);
  }
}
export async function loadQftKernel(): Promise<Kernel> {
  compiled ??= compileKernel()
    .catch((error) => {
      compiled = undefined;
      throw error;
    });
  return (await WebAssembly.instantiate(await compiled, {}))
    .exports as unknown as Kernel;
}
type MotionJob = { motion: Motion; at: number };
type ActionJob = {
  command: Command;
  at: number;
  resolve(): void;
  reject(error: Error): void;
  stagedPayload?: string;
  stagedLength?: number;
};
type Job = MotionJob | ActionJob;
type Socket = Pick<
  WebSocket,
  "send" | "bufferedAmount" | "readyState" | "close"
>;
const nowUs = () => Math.floor(performance.now() * 1000);
const sharedEncoder = new TextEncoder();

/** Scheduling/transport glue only. Admission, pacing, retries and outcomes use Rust. */
export class QftTransport {
  private timer: ReturnType<typeof setTimeout> | undefined;
  private openedAt = 0;
  private initialized = false;
  private stopped = false;
  private renewing = false;
  private lease = 1;
  private cancelling = false;
  private stageId = 0;
  private textSentAt: number[] = [];
  private queue: Job[] = [];
  private action: ActionJob | undefined;
  private closingAt: number | undefined;
  private cachedMemBuffer: ArrayBufferLike | undefined;
  private cachedInputView: Uint8Array | undefined;
  private cachedOutputView: Uint8Array | undefined;
  constructor(
    private kernel: Kernel,
    private socket: Socket,
    private capability: QftCapability,
    private interval: number,
    private ready: () => void,
    private fault: (message: string) => void,
  ) {}

  private wasmViews() {
    const buffer = this.kernel.memory.buffer;
    if (buffer !== this.cachedMemBuffer || !this.cachedInputView || !this.cachedOutputView) {
      this.cachedMemBuffer = buffer;
      this.cachedInputView = new Uint8Array(buffer, this.kernel.qft_input_ptr(), 64);
      this.cachedOutputView = new Uint8Array(buffer, this.kernel.qft_output_ptr(), 64);
    }
    return { input: this.cachedInputView, output: this.cachedOutputView };
  }

  setIntervalUs(interval: number) {
    this.interval = interval;
    if (this.initialized && !this.stopped) this.kernel.qft_set_interval(interval);
  }

  start() {
    if (this.socket.readyState !== WebSocket.OPEN) {
      this.abort(t('连接中断，操作未发送'));
      return;
    }
    this.openedAt = nowUs();
    try {
      this.socket.send(JSON.stringify({ kind: "qftOpen" }));
    } catch {
      this.abort(t('连接中断，操作未发送'));
      return;
    }
    this.timer = setTimeout(() => this.abort(t('传输初始化超时')), 3000);
  }
  receive(bytes: ArrayBuffer) {
    if (this.stopped) return;
    if (bytes.byteLength !== 64) {
      this.abort(t('传输确认格式不匹配'));
      return;
    }
    const view = new DataView(bytes);
    if (
      bytes.byteLength !== 64 ||
      view.getUint32(0, false) !== 0x51465432 ||
      view.getBigUint64(8, true) !== BigInt(this.capability.session)
    ) {
      this.abort(t('传输确认格式不匹配'));
      return;
    }
    this.wasmViews().input.set(new Uint8Array(bytes));
    const now = nowUs();
    if (!this.initialized) {
      if (
        view.getUint8(4) !== 1 ||
        view.getUint32(16, true) !== 1 ||
        view.getUint32(48, true) !== 0 ||
        this.kernel.qft_init(
          this.openedAt,
          now,
          this.interval,
          this.capability.leaseMs,
          this.capability.motionTtlMs,
          this.capability.actionTtlMs,
        ) !== 0
      ) {
        this.abort(t('传输租约初始化失败'));
        return;
      }
      this.initialized = true;
      clearTimeout(this.timer);
      this.ready();
    } else {
      if (
        ![1, 6].includes(view.getUint8(4)) ||
        this.kernel.qft_receive(now) !== 0
      ) {
        this.abort(t('传输确认无效，已停止输入'));
        return;
      }
      if (view.getUint8(4) === 1 && view.getUint32(16, true) !== this.lease) {
        // A grant starts a new coordinate epoch. Release is state cleanup and
        // must survive the handoff; never redate motion or effectful actions.
        const previous = this.queue;
        this.queue = [];
        for (const job of previous) {
          if ("command" in job) {
            if (job.command.type === "release" || (job.command.type === "buttonstate" && !job.command.down))
              this.queue.push({ at: now, command: { type: "release" }, resolve: job.resolve, reject: job.reject });
            else
              job.reject(new Error(t('连接已重新同步，待发送操作已取消，请重试')));
          }
        }
        this.lease = view.getUint32(16, true);
        this.renewing = false;
      }
    }
    this.tick();
  }
  motion(motion: Motion, captured = nowUs()) {
    if (!this.initialized || this.stopped || this.cancelling) return;
    if (
      ![motion.dx, motion.dy, motion.scroll].every(
        (v) => Number.isSafeInteger(v) && Math.abs(v) <= 16384,
      )
    ) {
      this.abort(t('输入超出允许范围'));
      return;
    }
    const previous = this.queue[this.queue.length - 1];
    if (previous && "motion" in previous) {
      const next = {
        dx: previous.motion.dx + motion.dx,
        dy: previous.motion.dy + motion.dy,
        scroll: previous.motion.scroll + motion.scroll,
      };
      if (Object.values(next).some((v) => Math.abs(v) > 16384)) {
        this.abort(t('输入积压过多，请重新同步'));
        return;
      }
      previous.motion = next; // retain oldest capture time
    } else if (this.queue.length < 48)
      this.queue.push({ motion, at: captured });
    else {
      this.abort(t('输入队列已满，已停止输入'));
      return;
    }
    this.tick();
  }
  send(command: Command): Promise<void> {
    if (!this.initialized || this.stopped || this.cancelling)
      return Promise.reject(new Error(t('传输尚未就绪')));
    if (command.type === "release") this.rejectQueued(t('操作已取消'));
    if (
      command.type === "motion" ||
      command.type === "move" ||
      (command.type === "scroll" && (command.dx ?? 0) === 0)
    ) {
      this.motion(
        command.type === "scroll"
          ? { dx: 0, dy: 0, scroll: command.dy }
          : {
              dx: command.dx,
              dy: command.dy,
              scroll: command.type === "motion" ? command.scroll : 0,
            },
      );
      return Promise.resolve();
    }
    if (this.queue.length >= 48)
      return Promise.reject(new Error(t('操作队列已满')));
    return new Promise<void>((resolve, reject) => {
      this.queue.push({ command, at: nowUs(), resolve, reject });
      this.tick();
    });
  }
  cancel() {
    if (!this.initialized || this.stopped) return;
    this.cancelling = true;
    this.rejectQueued(t('操作已取消'));
    this.kernel.qft_cancel(nowUs());
    this.tick();
  }
  clearUnsent() {
    this.rejectQueued(t('待发送输入已取消'));
  }
  private rejectQueued(message: string) {
    for (const job of this.queue)
      if ("command" in job) job.reject(new Error(message));
    this.queue = [];
  }
  dispose() {
    this.stopped = true;
    clearTimeout(this.timer);
    this.rejectQueued(t('连接变化，操作未发送'));
    this.action?.reject(new Error(t('连接中断，操作结果可能未知；不会自动重发')));
    this.action = undefined;
  }
  private abort(message: string) {
    this.dispose();
    this.fault(message);
    if (this.socket.readyState < WebSocket.CLOSING) this.socket.close(4000, message);
  }
  snapshot() {
    return {
      intervalMs: this.kernel.qft_metric(1) / 1000,
      srttMs: Math.round(this.kernel.qft_metric(2) / 1000) || null,
      queueMs: Math.round(this.kernel.qft_metric(3) / 1000),
      sent: this.kernel.qft_metric(4),
      merged: this.kernel.qft_metric(5),
      expired: this.kernel.qft_metric(6),
      pressured: 0,
    };
  }
  private tick() {
    clearTimeout(this.timer);
    if (this.stopped || !this.initialized) return;
    const now = nowUs();
    const outcome = this.kernel.qft_outcome();
    if (outcome && this.action) {
      const action = this.action;
      this.action = undefined;
      if (outcome === 1) action.resolve();
      else
        action.reject(
          new Error(
            outcome === 2
              ? t('操作未执行')
              : t('操作结果未知，请确认电脑状态；不会自动重发'),
          ),
        );
    }
    if (this.socket.readyState !== WebSocket.OPEN) {
      this.abort(this.action ? t('连接中断，操作结果可能未知') : t('连接中断，操作未发送'));
      return;
    }
    if (this.kernel.qft_closed()) {
      this.rejectQueued(t('输入已过期，需重新同步'));
      this.closingAt ??= now;
      // Allow result-only reconciliation before dropping the authenticated channel.
      if (!this.action && now - this.closingAt >= 20_000) {
        this.abort(this.cancelling ? t('输入已释放') : t('输入已过期，正在重新同步'));
        return;
      }
    } else {
      // A later click must never execute against a silently discarded prefix.
      const head = this.queue[0];
      if (head && now - head.at >= ("motion" in head ? this.capability.motionTtlMs : this.capability.actionTtlMs) * 1000) {
        // The lift/click can arrive on the next tick, after this sample expires.
        this.kernel.qft_cancel(now);
        this.rejectQueued(t('操作等待过久，已取消'));
      }
      while (
        !this.action &&
        !this.renewing &&
        this.queue.length &&
        !this.kernel.qft_closed()
      ) {
        const first = this.queue[0];
        const second = this.queue[1];
        const prefix = "motion" in first && second && "command" in second ? first : undefined;
        const job = prefix ? second : first;
        if ("motion" in job) {
          const code = this.kernel.qft_offer(
            job.motion.dx,
            job.motion.dy,
            job.motion.scroll,
            job.at,
            now,
          );
          if (code === -1) break;
          if (code !== 0) {
            this.abort(t('运动未接纳，已停止输入'));
            return;
          }
          this.queue.shift();
        } else {
          if (job.command.type === "text") {
            // Mirror the desktop's four text submissions per ten-second window.
            this.textSentAt = this.textSentAt.filter(at => now - at < 10_000_000);
            if (this.textSentAt.length >= 4) {
              this.queue.splice(0, prefix ? 2 : 1);
              job.reject(new Error(t('文字发送过于频繁，请稍后再试')));
              this.rejectQueued(t('操作已取消'));
              break;
            }
          }
          const next = this.stageId + 1;
          if (next > 0x7fffffff) {
            this.abort(t('传输序号耗尽'));
            return;
          }
          if (!job.stagedPayload) {
            job.stagedPayload = JSON.stringify({
              kind: "qftStage",
              id: next,
              command: job.command,
            });
            job.stagedLength = sharedEncoder.encode(job.stagedPayload).length;
          }
          const staged = job.stagedPayload;
          if (this.socket.bufferedAmount + (job.stagedLength ?? 0) + 64 > 8192)
            break;
          const code = prefix
            ? this.kernel.qft_step(prefix.motion.dx, prefix.motion.dy, prefix.motion.scroll, prefix.at, next, job.at, now)
            : this.kernel.qft_action(next, job.at, now);
          if (code === -1) break;
          if (code !== 0) {
            this.abort(t('操作未接纳，已停止输入'));
            return;
          }
          this.stageId = next;
          this.action = job;
          this.queue.splice(0, prefix ? 2 : 1);
          try {
            this.socket.send(staged);
            if (job.command.type === "text") this.textSentAt.push(now);
          } catch {
            this.abort(t('连接中断，操作未发送'));
            return;
          }
        }
      }
      if (
        !this.action &&
        !this.renewing &&
        now >= this.kernel.qft_metric(0) - 3_000_000
      ) {
        const code = this.kernel.qft_renew(now);
        if (code === 0) this.renewing = true;
        else if (code !== -1) {
          this.abort(t('租约无法续期'));
          return;
        }
      }
    }
    const writable = this.socket.bufferedAmount + 64 <= 2048 ? 1 : 0;
    const due = this.kernel.qft_next(now, writable);
    if (due === -2) {
      this.abort(t('传输时钟异常'));
      return;
    }
    if (due >= 0 && due <= now) {
      const count = this.kernel.qft_poll(now, writable);
      if (count < 0) {
        this.abort(t('传输调度失败'));
        return;
      }
      if (count === 1) {
        const bytes = this.wasmViews().output.slice();
        try {
          this.socket.send(bytes.buffer);
        } catch {
          this.abort(t('连接中断，操作结果可能未知'));
          return;
        }
      }
    }
    let wake = this.kernel.qft_next(now, writable);
    if (!this.kernel.qft_closed()) {
      const renewAt = this.kernel.qft_metric(0) - 2_000_000;
      if (!this.renewing)
        wake = Math.min(
          wake < 0 ? Infinity : wake,
          Math.max(now + 25_000, renewAt),
        );
      if (this.queue[0])
        wake = Math.min(
          wake < 0 ? Infinity : wake,
          this.queue[0].at +
            ("motion" in this.queue[0]
              ? this.capability.motionTtlMs
              : this.capability.actionTtlMs) *
              1000,
        );
    } else wake = Math.min(wake < 0 ? Infinity : wake, now + 20_000);
    // WebSocket has no writable event. Probe only while it actually has queued bytes.
    if (!writable) wake = Math.min(wake < 0 ? Infinity : wake, now + 16_000);
    if (Number.isFinite(wake) && wake >= 0)
      this.timer = setTimeout(
        () => this.tick(),
        Math.max(1, Math.ceil((wake - now) / 1000)),
      );
  }
}
