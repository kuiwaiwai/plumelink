export type MicMode = "idle" | "holding" | "locked";
/** Time-based interaction, independent of animation frame rate and native capture readiness. */
export class MicrophoneGesture {
  mode: MicMode = "idle";
  progress = 0;
  intent: "start" | "lock" | "stop" | null = null;
  private pressed = false;
  private began = 0;
  private tapX = 0;
  private tapMoved = false;
  private y = 0;
  private anchor = { x: 0, y: 0 };
  private dwell: number | null = null;
  constructor(private emit: (event: "start" | "stop" | "lock") => void) {}
  restoreLocked() {
    this.mode = "locked";
    this.pressed = false;
    this.intent = null;
    this.progress = 0;
    this.dwell = null;
  }
  down(x: number, y: number, now: number) {
    if (this.pressed) return;
    this.pressed = true;
    this.began = now;
    this.tapX = x;
    this.tapMoved = false;
    this.y = y;
    this.anchor = { x, y };
    this.dwell = null;
    this.intent = this.mode === "locked" ? "stop" : "start";
    this.progress = 0;
  }
  move(x: number, y: number, now: number) {
    if (!this.pressed) return;
    if (Math.hypot(x - this.tapX, y - this.y) > 12) this.tapMoved = true;
    if (this.intent === "stop") return;
    if (this.y - y < 64) {
      this.dwell = null;
      this.intent = this.mode === "holding" ? null : "start";
      this.progress = 0;
      return;
    }
    if (
      this.dwell === null ||
      Math.hypot(x - this.anchor.x, y - this.anchor.y) > 14
    ) {
      this.anchor = { x, y };
      this.dwell = now;
    }
    this.intent = "lock";
  }
  tick(now: number) {
    if (!this.pressed) return;
    if (this.mode === "idle" && now - this.began >= 220) {
      this.mode = "holding";
      this.emit("start");
      if (this.intent === "start") this.intent = null;
    }
    if (this.intent === "stop") {
      this.progress = Math.min(1, (now - this.began) / 2000);
      if (this.progress === 1) {
        this.cancel();
      }
    } else if (
      this.intent === "lock" &&
      this.dwell !== null &&
      this.mode === "holding"
    ) {
      this.progress = Math.min(1, (now - this.dwell) / 1000);
      if (this.progress === 1) {
        this.mode = "locked";
        this.pressed = false;
        this.intent = null;
        this.progress = 0;
        this.emit("lock");
      }
    } else if (this.intent === "start")
      this.progress = Math.min(1, (now - this.began) / 220);
  }
  up(releasedAt?: number) {
    const tap = this.pressed && !this.tapMoved && releasedAt !== undefined && releasedAt - this.began < 220;
    if (tap && this.mode === "locked") { this.cancel(); return; }
    this.pressed = false;
    this.intent = null;
    this.progress = 0;
    this.dwell = null;
    if (this.mode === "holding") {
      this.mode = "idle";
      this.emit("stop");
    }
  }
  cancel() {
    const active = this.mode !== "idle";
    this.mode = "idle";
    this.pressed = false;
    this.intent = null;
    this.progress = 0;
    this.dwell = null;
    if (active) this.emit("stop");
  }
}
