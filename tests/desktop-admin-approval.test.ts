import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { desktop } from "../apps/desktop-ui/bridge";
import type { DesktopState } from "../shared/protocol";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => {
  vi.useRealTimers();
  vi.mocked(invoke).mockReset();
});

it("waits for local UAC approval beyond the ordinary IPC timeout", async () => {
  vi.useFakeTimers();
  let approve!: (state: DesktopState) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise(resolve => { approve = resolve; }));
  const result = desktop.admin("start");
  const approved = { admin: { active: true } } as DesktopState;
  const assertion = expect(result).resolves.toBe(approved);
  await vi.advanceTimersByTimeAsync(30_000);
  approve(approved);
  await assertion;
});

it("surfaces a cancelled approval without retrying it", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("UAC cancelled"));
  await expect(desktop.admin("start")).rejects.toThrow("UAC cancelled");
  expect(invoke).toHaveBeenCalledTimes(1);
});
