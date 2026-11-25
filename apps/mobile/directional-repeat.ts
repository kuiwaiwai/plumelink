export type Direction = "up" | "right" | "down" | "left";
const INITIAL_DELAY_MS = 400;
const REPEAT_INTERVAL_MS = 100;

export class DirectionalRepeat<T extends Direction = Direction> {
  private generation = 0;
  private held: T | undefined;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly options: {
    enabled: () => boolean;
    send: (direction: T, repeating: boolean) => Promise<boolean>;
  }) {}

  press(direction: T) {
    this.stop();
    if (!this.options.enabled()) return;
    this.held = direction;
    void this.send(this.generation, Date.now() + INITIAL_DELAY_MS);
  }

  release(direction: T) {
    if (this.held === direction) this.stop();
  }

  tap(direction: T) {
    this.stop();
    if (this.options.enabled()) return this.options.send(direction, false);
  }

  stop() {
    this.generation++;
    clearTimeout(this.timer);
    this.timer = undefined;
    this.held = undefined;
  }

  private async send(generation: number, firstRepeatAt?: number) {
    const direction = this.held;
    if (!direction || generation !== this.generation) return;
    if (!this.options.enabled()) { this.stop(); return; }
    let accepted = false;
    try { accepted = await this.options.send(direction, firstRepeatAt === undefined); } catch {}
    if (generation !== this.generation) return;
    if (!accepted || !this.options.enabled()) { this.stop(); return; }
    const delay = firstRepeatAt === undefined ? REPEAT_INTERVAL_MS : Math.max(0, firstRepeatAt - Date.now());
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.send(generation);
    }, delay);
  }
}
