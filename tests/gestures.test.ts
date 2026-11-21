import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { computed, ref } from "vue";
import { useTouchpad } from "../apps/mobile/composables/useTouchpad";
import type { Command } from "../shared/protocol";
import { TouchpadGesture, MotionBuffer, pointerGain } from "../apps/mobile/gestures";
type PadOptions = { longPressDrag?: boolean };
function touchpadEnv(options: PadOptions = {}) {
  const commands: Command[] = [];
  const motion: { dx: number; dy: number; scroll: number }[] = [];
  const pad = useTouchpad({
    canInput: computed(() => true),
    online: computed(() => true),
    panelOpen: computed(() => false),
    sensitivity: ref(1),
    longPressDrag: ref(options.longPressDrag ?? false),
    scrollDirection: ref("follow"),
    sendCommand: command => commands.push(command),
    sendMotion: sample => motion.push(sample),
    clearRemoteMotion: () => {},
    onMotionSent: () => {},
  });
  return { pad, commands, motion };
}
function touch(pointerId: number, x: number, y: number) {
  return {
    pointerType: "touch", pointerId, button: 0, isPrimary: pointerId === 1,
    clientX: x, clientY: y, width: 0, height: 0,
    currentTarget: {
      setPointerCapture() {},
      getBoundingClientRect: () => ({ left: 0, right: 400, top: 0, bottom: 400, width: 400, height: 400 }),
    },
  } as unknown as PointerEvent;
}

describe("touchpad", () => {
  it("distinguishes a tap from a drag and long press", () => {
    const events: unknown[] = [];
    const g = new TouchpadGesture((...e) => events.push(e));
    g.down(1, { x: 0, y: 0 }, 0);
    g.up(1, 100);
    expect(events).toEqual([["tap", 0, 0]]);
    events.length = 0;
    g.down(1, { x: 0, y: 0 }, 200);
    g.move(1, { x: 20, y: 2 });
    g.up(1, 250);
    expect(events).toEqual([["move", 20, 2]]);
    events.length = 0;
    g.down(1, { x: 0, y: 0 }, 300);
    g.up(1, 800);
    expect(events).toEqual([]);
  });
  it("two fingers scroll and never become a stray click after lifting one", () => {
    const events: unknown[] = [];
    const g = new TouchpadGesture((...e) => events.push(e));
    g.down(1, { x: 0, y: 0 }, 0);
    g.down(2, { x: 20, y: 0 }, 5);
    g.move(1, { x: 0, y: 16 });
    g.move(2, { x: 20, y: 16 });
    g.up(2, 100);
    g.move(1, { x: 0, y: 21 });
    g.up(1, 120);
    expect(events).toEqual([["scroll", 0, 16]]);
  });
  it("classifies a native two-finger frame after updating both contacts", () => {
    const events: unknown[] = [];
    const gesture = new TouchpadGesture((...event) => events.push(event));
    gesture.down(1, { x: 0, y: 0 }, 0);
    gesture.down(2, { x: 100, y: 0 }, 5);
    gesture.moveTogether([{ pointerId: 1, x: 61, y: 0 }, { pointerId: 2, x: 161, y: 0 }]);
    gesture.moveTogether([{ pointerId: 2, x: 201, y: 0 }, { pointerId: 1, x: 101, y: 0 }]);
    gesture.up(2, 100);
    gesture.up(1, 105);
    expect(events).toEqual([["scroll", 61, 0], ["scroll", 40, 0]]);
  });
  it("cancellation loses old fingers and clears fractional motion", () => {
    const events: unknown[] = [];
    const g = new TouchpadGesture((...e) => events.push(e));
    g.down(1, { x: 0, y: 0 }, 0);
    g.cancel();
    g.up(1, 100);
    expect(events).toEqual([]);
    const m = new MotionBuffer();
    m.add(0.4, 0.5);
    expect(m.drain().dx).toBe(0);
    m.add(0.8, 0.7);
    expect(m.drain()).toEqual({ dx: 1, dy: 1, scroll: 0 });
    m.add(900, 900);
    m.clear();
    expect(m.drain()).toEqual({ dx: 0, dy: 0, scroll: 0 });
  });
});

it("capture loss only removes its own pointer and never resumes mouse movement", () => {
  const events: unknown[] = [];
  const g = new TouchpadGesture((...e) => events.push(e));
  g.down(1,{x:0,y:0},0); g.down(2,{x:20,y:0},1);
  g.up(2,100); g.lost(2);
  g.down(3,{x:20,y:0},110); g.move(1,{x:0,y:16}); g.move(3,{x:20,y:16});
  expect(events).toEqual([["scroll",0,16]]);
  g.lost(3); g.move(1,{x:0,y:20}); g.up(1,120);
  expect(events).toHaveLength(1);
});
it("recognizes two-finger tap as right click without a left click", () => {
  const events: unknown[] = [];
  const g = new TouchpadGesture((...e) => events.push(e));
  g.down(1, {x:0,y:0}, 0); g.down(2, {x:30,y:0}, 10);
  g.up(1, 90); g.up(2, 100);
  expect(events).toEqual([["righttap",0,0]]);
});
it("recognizes pinch and spread, not scrolling, and suppresses taps", () => {
  const events: unknown[] = [];
  const g = new TouchpadGesture((...e) => events.push(e));
  g.down(1, {x:0,y:0}, 0); g.down(2, {x:100,y:0}, 0);
  g.move(1, {x:-35,y:0}); g.move(2, {x:135,y:0});
  g.up(1, 100); g.up(2, 100);
  expect(events).toEqual([["zoom",1,0],["zoom",1,0]]);
  events.length = 0;
  g.down(1, {x:0,y:0}, 200); g.down(2, {x:100,y:0}, 200);
  g.move(1, {x:35,y:0}); g.move(2, {x:65,y:0});
  g.up(1, 300); g.up(2, 300);
  expect(events).toEqual([["zoom",-1,0],["zoom",-1,0]]);
});
it("accumulates small scroll deltas into complete detents, clears remainder on cancel", () => {
  const m = new MotionBuffer();
  for(let i=0;i<5;i++) { m.scroll += 20; expect(m.drain().scroll).toBe(0); }
  m.scroll += 40; expect(m.drain().scroll).toBe(120);
  m.clear(); m.scroll = -119; expect(m.drain().scroll).toBe(0);
  m.scroll -= 1; expect(m.drain().scroll).toBe(-120);
  expect(m.drain().scroll).toBe(0);
});

it("supports sub-notch smooth scroll, re-entry anchor reset, and velocity pointer gain", () => {
  const smooth = new MotionBuffer(15);
  smooth.scroll = 22;
  expect(smooth.hasPending()).toBe(true);
  expect(smooth.drain().scroll).toBe(15);
  expect(smooth.hasPending()).toBe(false);
  smooth.scroll += 8;
  expect(smooth.drain().scroll).toBe(15);

  const events: unknown[] = [];
  const g = new TouchpadGesture((...e) => events.push(e));
  g.down(1, { x: 0, y: 0 }, 0);
  g.down(2, { x: 20, y: 0 }, 5);
  g.move(1, { x: 0, y: 16 });
  g.move(2, { x: 20, y: 16 });
  g.up(2, 50);
  g.move(1, { x: 0, y: 80 });
  events.length = 0;
  g.down(3, { x: 40, y: 80 }, 80);
  g.move(1, { x: 0, y: 95 });
  g.move(3, { x: 40, y: 95 });
  expect(events).toEqual([["scroll", 0, 15]]);

  expect(pointerGain(0.5, 0, 20)).toBeLessThan(1);
  expect(pointerGain(40, 0, 16)).toBeGreaterThan(2);
});

it("emits horizontal and vertical scroll deltas but ignores a three-finger tap", () => {
  const events: unknown[] = [];
  const g = new TouchpadGesture((...e) => events.push(e));
  g.down(1, { x: 0, y: 0 }, 0);
  g.down(2, { x: 20, y: 0 }, 5);
  g.move(1, { x: 18, y: 12 });
  g.move(2, { x: 38, y: 12 });
  g.up(1, 50);
  g.up(2, 60);
  expect(events).toEqual([["scroll", 18, 12]]);

  events.length = 0;
  g.down(1, { x: 0, y: 0 }, 100);
  g.down(2, { x: 30, y: 0 }, 110);
  g.down(3, { x: 60, y: 0 }, 120);
  g.up(1, 180);
  g.up(2, 190);
  g.up(3, 200);
  expect(events).toEqual([]);
});

describe("touchpad pointer semantics", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("document", { hidden: true });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("sends a single click for one tap", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    expect(commands).toEqual([]);
    vi.advanceTimersByTime(300);
    expect(commands).toEqual([{ type: "click", button: "left", count: 1 }]);
  });

  it("moves without clicking when the only displacement arrives on lift", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 160, 100));
    vi.advanceTimersByTime(500);
    expect(commands).toEqual([]);
    expect(pad.drainMotion().dx).toBeGreaterThan(0);
  });

  it("accounts for the last drag segment exactly once before release", () => {
    const { pad, commands } = touchpadEnv({ longPressDrag: true });
    pad.down(touch(1, 100, 100));
    vi.advanceTimersByTime(500);
    pad.move(touch(1, 130, 100));
    pad.drainMotion();
    pad.up(touch(1, 160, 100));
    expect(commands).toEqual([
      { type: "buttonstate", button: "left", down: true },
      { type: "buttonstate", button: "left", down: false },
    ]);
    expect(pad.drainMotion().dx).toBeGreaterThan(0);
    expect(pad.drainMotion().dx).toBe(0);
  });

  it("recognizes a double tap as one click with count 2", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    expect(commands).toEqual([{ type: "click", button: "left", count: 2 }]);
    vi.advanceTimersByTime(400);
    expect(commands).toHaveLength(1);
  });

  it("keeps quick taps far apart as two single clicks", () => {
    const { pad, commands } = touchpadEnv();
    for (const x of [100, 240]) {
      pad.down(touch(1, x, 100));
      pad.up(touch(1, x, 100));
      vi.advanceTimersByTime(30);
    }
    vi.advanceTimersByTime(300);
    expect(commands).toEqual(Array(2).fill({ type: "click", button: "left", count: 1 }));
  });

  it("allows implicit capture release after a tap", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    pad.captureLost(touch(1, 100, 100));
    vi.advanceTimersByTime(300);
    expect(commands).toEqual([{ type: "click", button: "left", count: 1 }]);
  });

  it("starts a drag when the second tap moves", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    pad.down(touch(1, 100, 100));
    pad.move(touch(1, 140, 100));
    pad.up(touch(1, 140, 100));
    expect(commands).toEqual([
      { type: "click", button: "left", count: 1 },
      { type: "buttonstate", button: "left", down: true },
      { type: "buttonstate", button: "left", down: false },
    ]);
  });

  it("starts a drag from the native moveTogether path", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    pad.down(touch(1, 100, 100));
    pad.moveTogether([{ pointerId: 1, x: 140, y: 100 }]);
    pad.up(touch(1, 140, 100));
    expect(commands).toEqual([
      { type: "click", button: "left", count: 1 },
      { type: "buttonstate", button: "left", down: true },
      { type: "buttonstate", button: "left", down: false },
    ]);
  });

  it("treats two fingers landing one after another as a right click, not a double click", () => {
    const { pad, commands } = touchpadEnv();
    pad.down(touch(1, 100, 100));
    pad.up(touch(1, 100, 100));
    pad.down(touch(1, 100, 100));
    pad.down(touch(2, 130, 100));
    pad.up(touch(1, 100, 100));
    pad.up(touch(2, 130, 100));
    expect(commands).toEqual([
      { type: "click", button: "left", count: 1 },
      { type: "click", button: "right", count: 1 },
    ]);
  });

  it.each([3, 4])("ignores %i contacts through staggered lift and accepts the next tap", fingers => {
    const { pad, commands } = touchpadEnv({ longPressDrag: true });
    for (const firstX of [4, 80]) {
      const points = Array.from({ length: fingers }, (_, i) => ({
        pointerId: i + 1, x: firstX + i * 45, y: 120,
      }));
      for (const p of points) pad.down(touch(p.pointerId, p.x, p.y));
      vi.advanceTimersByTime(500);
      for (const offset of [70, -50]) pad.moveTogether(points.map(p => ({ ...p, x: p.x + offset })));
      while (points.length) {
        const p = points.pop()!;
        pad.up(touch(p.pointerId, p.x - 50, p.y));
        pad.moveTogether(points.map(q => ({ ...q, y: q.y + 35 })));
      }
      expect(pad.drainMotion()).toEqual({ dx: 0, dy: 0, scroll: 0 });
    }
    pad.down(touch(1, 90, 100));
    pad.up(touch(1, 90, 100));
    vi.advanceTimersByTime(300);
    expect(commands).toEqual([{ type: "click", button: "left", count: 1 }]);
  });
});
