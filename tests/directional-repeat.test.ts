import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DirectionalRepeat } from "../apps/mobile/directional-repeat";

const controllers: DirectionalRepeat[] = [];
function setup(send = vi.fn(async (_direction: string) => true), enabled = () => true) {
  const repeat = new DirectionalRepeat({ send, enabled });
  controllers.push(repeat);
  return { repeat, send };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => { controllers.splice(0).forEach(item => item.stop()); vi.useRealTimers(); });

it("sends a quick press once without a release click or later repeat", async () => {
  const { repeat, send } = setup();
  repeat.press("up");
  repeat.release("up");
  await vi.advanceTimersByTimeAsync(1000);
  expect(send.mock.calls.map(([direction]) => [direction])).toEqual([["up"]]);
});

it("starts repeating after 400 ms and stops immediately on release", async () => {
  const { repeat, send } = setup();
  repeat.press("right");
  await vi.advanceTimersByTimeAsync(399);
  expect(send).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(201);
  expect(send).toHaveBeenCalledTimes(4);
  repeat.release("right");
  await vi.advanceTimersByTimeAsync(2000);
  expect(send).toHaveBeenCalledTimes(4);
});

it("marks timed repeats so haptics can stay limited to the initial press", async () => {
  const send = vi.fn(async (_direction: string, _repeating: boolean) => true);
  const repeat = new DirectionalRepeat({ send, enabled: () => true }); controllers.push(repeat);
  repeat.press("up");
  await vi.advanceTimersByTimeAsync(500);
  expect(send.mock.calls).toEqual([["up", false], ["up", true], ["up", true]]);
});

it("never queues repeats behind a slow acknowledgement or after release", async () => {
  let confirm!: (accepted: boolean) => void;
  const { repeat, send } = setup(vi.fn(() => new Promise<boolean>(resolve => { confirm = resolve; })));
  repeat.press("down");
  await vi.advanceTimersByTimeAsync(3000);
  expect(send).toHaveBeenCalledTimes(1);
  repeat.release("down");
  confirm(true);
  await vi.advanceTimersByTimeAsync(1000);
  expect(send).toHaveBeenCalledTimes(1);
});

it("does not let a previous acknowledgement restart a new direction", async () => {
  let confirm!: (accepted: boolean) => void;
  const send = vi.fn(async (_direction: string) => true).mockImplementationOnce(() => new Promise<boolean>(resolve => { confirm = resolve; }));
  const { repeat } = setup(send);
  repeat.press("left"); repeat.press("up");
  confirm(true);
  await vi.advanceTimersByTimeAsync(500);
  expect(send.mock.calls.map(([direction]) => [direction])).toEqual([["left"], ["up"], ["up"], ["up"]]);
});

it("ignores an unrelated direction release and treats accessible taps as single actions", async () => {
  const { repeat, send } = setup();
  repeat.press("up"); repeat.release("down");
  await vi.advanceTimersByTimeAsync(400);
  await repeat.tap("left");
  await vi.advanceTimersByTimeAsync(1000);
  expect(send.mock.calls.map(([direction]) => [direction])).toEqual([["up"], ["up"], ["left"]]);
});

it.each([false, "reject"])("stops on a failed or uncertain result (%s)", async result => {
  const send = vi.fn(async (_direction: string) => { if (result === "reject") throw Error("unknown result"); return false; });
  const { repeat } = setup(send);
  repeat.press("up");
  await vi.advanceTimersByTimeAsync(2000);
  expect(send).toHaveBeenCalledTimes(1);
});

it("stops when permission is lost and does not restart when it returns", async () => {
  let enabled = true;
  const { repeat, send } = setup(undefined, () => enabled);
  repeat.press("up"); enabled = false;
  await vi.advanceTimersByTimeAsync(500);
  enabled = true;
  await vi.advanceTimersByTimeAsync(1000);
  expect(send).toHaveBeenCalledTimes(1);
});

it("does not start while disabled and cancels a pending repeat on reset", async () => {
  let enabled = false;
  const { repeat, send } = setup(undefined, () => enabled);
  repeat.press("up"); await repeat.tap("left");
  expect(send).not.toHaveBeenCalled();
  enabled = true; repeat.press("up");
  await vi.advanceTimersByTimeAsync(300);
  repeat.stop();
  await vi.advanceTimersByTimeAsync(1000);
  expect(send).toHaveBeenCalledTimes(1);
});
