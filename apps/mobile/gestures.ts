/** Pointer-event gesture recognizer for the remote touch surface. */
export interface Point { x: number; y: number }
export interface TouchContact extends Point { pointerId: number }
export type GestureKind = "move" | "scroll" | "zoom" | "tap" | "righttap";

export class TouchpadGesture {
  private points = new Map<number, Point>();
  private origins = new Map<number, Point>();
  private maxContacts = 0;
  private started = 0;
  private groupStarted = 0;
  private maxTravel = 0;
  private groupStart: Point = { x: 0, y: 0 };
  private previousCenter: Point = { x: 0, y: 0 };
  private startSpan = 0;
  private previousSpan = 0;
  private zoomRemainder = 0;
  private mode: "undecided" | "scroll" | "zoom" | "ignored" = "undecided";

  constructor(private emit: (kind: GestureKind, x: number, y: number) => void) {}
  get count() { return this.points.size; }

  hasFinalMovement(id: number, point: Point) {
    const previous = this.points.get(id);
    return this.maxContacts === 1 && previous !== undefined &&
      (point.x !== previous.x || point.y !== previous.y);
  }

  down(id: number, p: Point, now: number, excludedContacts = 0) {
    if (this.points.has(id)) return;
    if (this.points.size === 0) {
      this.started = now;
      this.maxContacts = 0;
      this.maxTravel = 0;
    }
    this.points.set(id, p);
    this.origins.set(id, p);
    if (this.points.size + excludedContacts > 2 || this.maxContacts > 2) {
      this.maxContacts = Math.max(this.maxContacts, this.points.size + excludedContacts);
      this.mode = "ignored";
      return;
    }
    this.maxContacts = Math.max(this.maxContacts, this.points.size);
    this.groupStarted = now;
    this.groupStart = this.centroid();
    this.previousCenter = this.groupStart;
    this.startSpan = this.span();
    this.previousSpan = this.startSpan;
    this.zoomRemainder = 0;
    this.mode = "undecided";
  }

  move(id: number, p: Point) {
    const prev = this.updatePosition(id, p);
    if (!prev) return;
    if (this.points.size === 1) {
      if (this.maxContacts === 1) this.emit("move", p.x - prev.x, p.y - prev.y);
      return;
    }
    this.movePair();
  }

  moveTogether(contacts: readonly TouchContact[]) {
    if (this.points.size <= 1) {
      for (const point of contacts) this.move(point.pointerId, point);
      return;
    }
    let updated = false;
    for (const point of contacts) {
      if (this.updatePosition(point.pointerId, point)) updated = true;
    }
    if (!updated) return;
    this.movePair();
  }

  private updatePosition(id: number, point: Point) {
    const previous = this.points.get(id);
    if (!previous) return;
    this.points.set(id, point);
    const origin = this.origins.get(id)!;
    this.maxTravel = Math.max(this.maxTravel, Math.hypot(point.x - origin.x, point.y - origin.y));
    return previous;
  }

  private movePair() {
    if (this.maxContacts >= 3) return;
    if (this.points.size === 2 && this.maxContacts === 2) {
      const center = this.centroid();
      const span = this.span();
      const centerTravel = Math.hypot(center.x - this.groupStart.x, center.y - this.groupStart.y);
      const spanTravel = Math.abs(span - this.startSpan);
      if (this.mode === "undecided") {
        if (spanTravel >= 16 && spanTravel > centerTravel * 1.4) {
          this.mode = "zoom";
          this.previousSpan = this.startSpan;
        } else if (centerTravel >= 12 && centerTravel > spanTravel * 0.7) {
          this.mode = "scroll";
          this.previousCenter = this.groupStart;
        }
      }
      if (this.mode === "scroll") {
        this.emit("scroll", center.x - this.previousCenter.x, center.y - this.previousCenter.y);
      } else if (this.mode === "zoom") {
        this.zoomRemainder += span - this.previousSpan;
        let steps = 0;
        while (Math.abs(this.zoomRemainder) >= 28 && steps++ < 4) {
          const direction = Math.sign(this.zoomRemainder);
          this.emit("zoom", direction, 0);
          this.zoomRemainder -= direction * 28;
        }
        this.zoomRemainder = Math.max(-27, Math.min(27, this.zoomRemainder));
      }
      this.previousCenter = center;
      this.previousSpan = span;
    }
  }

  up(id: number, now: number) {
    if (!this.points.has(id)) return;
    this.points.delete(id);
    this.origins.delete(id);
    if (this.points.size !== 0) return;
    if (this.mode === "undecided" && this.maxTravel < 9) {
      if (this.maxContacts === 1 && now - this.started < 280) this.emit("tap", 0, 0);
      if (this.maxContacts === 2 && now - this.groupStarted < 320) this.emit("righttap", 0, 0);
    }
  }

  lost(id: number) {
    if (this.points.delete(id)) {
      this.origins.delete(id);
      this.mode = "ignored";
      return true;
    }
    return false;
  }
  cancel() {
    this.points.clear();
    this.origins.clear();
    this.maxContacts = 0;
    this.mode = "undecided";
  }
  private centroid(): Point {
    let x = 0, y = 0;
    for (const point of this.points.values()) { x += point.x; y += point.y; }
    return { x: x / this.points.size, y: y / this.points.size };
  }
  private span(): number {
    const [a, b] = [...this.points.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  }
}

/** Velocity-adaptive pointer acceleration balancing pixel accuracy with fast traversal. */
export function pointerGain(dx: number, dy: number, dtMs: number): number {
  const dt = Math.max(4, Math.min(80, dtMs));
  const speed = Math.hypot(dx, dy) / dt;
  if (speed <= 0.15) return 0.65 + (speed / 0.15) * 0.35;
  return Math.min(2.6, 1 + Math.pow(speed - 0.15, 0.78) * 1.45);
}

export class MotionBuffer {
  x = 0;
  y = 0;
  scroll = 0;
  constructor(private readonly scrollStep = 120) {}
  add(x: number, y: number) {
    this.x = Math.max(-500, Math.min(500, this.x + x));
    this.y = Math.max(-500, Math.min(500, this.y + y));
  }
  hasPending() {
    return (
      Math.trunc(this.x) !== 0 ||
      Math.trunc(this.y) !== 0 ||
      Math.trunc(this.scroll / this.scrollStep) !== 0
    );
  }
  drain() {
    const dx = Math.trunc(this.x), dy = Math.trunc(this.y),
      scroll = Math.trunc(this.scroll / this.scrollStep) * this.scrollStep || 0;
    this.x -= dx;
    this.y -= dy;
    this.scroll -= scroll;
    return { dx, dy, scroll: Math.max(-600, Math.min(600, scroll)) };
  }
  clear() { this.x = 0; this.y = 0; this.scroll = 0; }
}
