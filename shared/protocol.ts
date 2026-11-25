import { z } from "zod";
import { keyboardKeys, keyboardModifiers } from "./keyboard";
export { keyboardKeys, keyboardModifiers } from "./keyboard";
export const commandSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("media"), action: z.enum(["playpause", "previous", "next", "stop"]) }).strict(),
  z.object({ type: z.literal("launch"), id: z.string().regex(/^[a-f0-9]{32}$/), request: z.string().regex(/^[a-f0-9]{32}$/).optional() }).strict(),
  z.object({ type: z.literal("motion"), dx: z.number().int().min(-500).max(500),
    dy: z.number().int().min(-500).max(500), scroll: z.number().int().min(-600).max(600) }).strict(),
  z
    .object({
      type: z.literal("move"),
      dx: z.number().int().min(-500).max(500),
      dy: z.number().int().min(-500).max(500),
    })
    .strict(),
  z
    .object({
      type: z.literal("click"),
      button: z.enum(["left", "right", "middle"]),
      count: z.union([z.literal(1), z.literal(2)]),
    })
    .strict(),
  z.object({ type: z.literal("buttonstate"), button: z.enum(["left", "right", "middle"]), down: z.boolean() }).strict(),
  z
    .object({
      type: z.literal("scroll"),
      dx: z.number().int().min(-600).max(600).optional(),
      dy: z.number().int().min(-600).max(600),
    })
    .strict(),
  z
    .object({
      type: z.literal("key"),
      key: z.enum(keyboardKeys),
    })
    .strict(),
  z
    .object({
      type: z.literal("combo"),
      modifiers: z
        .array(z.enum(keyboardModifiers))
        .min(1)
        .max(4)
        .refine((list) => new Set(list).size === list.length),
      key: z.enum(keyboardKeys),
    })
    .strict(),
  z
    .object({
      type: z.literal("volume"),
      action: z.enum(["up", "down", "mute"]),
    })
    .strict(),
  z
    .object({
      type: z.literal("switch"),
      action: z.enum(["next", "previous", "confirm", "cancel"]),
    })
    .strict(),
  z.object({ type: z.literal("desktop") }).strict(),
  z.object({ type: z.literal("taskview") }).strict(),
  z.object({ type: z.literal("appswitch"), direction: z.enum(["left", "right"]) }).strict(),
  z.object({ type: z.literal("virtualdesktop"), direction: z.enum(["left", "right"]) }).strict(),
  z.object({ type: z.literal("notifications") }).strict(),
  z.object({ type: z.literal("zoom"), direction: z.enum(["in", "out"]) }).strict(),
  z
    .object({
      type: z.literal("text"),
      text: z
        .string()
        .min(1)
        .max(1000)
        .refine((s) => !/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(s)),
    })
    .strict(),
  z.object({ type: z.literal("magnifier"), enabled: z.boolean() }).strict(),
  z.object({ type: z.literal("shutdown"), confirmed: z.literal(true) }).strict(),
  z.object({ type: z.literal("release") }).strict(),
  z.object({ type: z.literal("list-windows") }).strict(),
  z.object({ type: z.literal("switch-to-window"), hwnd: z.number().int().nonnegative() }).strict(),
]);
export type Command = z.infer<typeof commandSchema>;
export const clientSchema = z.discriminatedUnion("kind", [
  z
    .object({
      kind: z.literal("auth"),
      token: z.string().regex(/^[a-f0-9]{64}$/),
    })
    .strict(),
  z.object({ kind: z.literal("ping") }).strict(),
  z.object({ kind: z.literal("qftOpen") }).strict(),
  z.object({ kind: z.literal("qftStage"), id: z.number().int().min(1).max(2147483647), command: commandSchema }).strict(),
]);
export type ReceiveAction = "cancel" | "clear" | "open";
export interface ReceivedFile {
  taskId: string;
  name: string;
  size: number;
  receivedBytes: number;
  state: "receiving" | "saving" | "stopping" | "completed" | "stopped";
  deviceName?: string;
  error?: string;
}
export interface DesktopState {
  receivedFiles?: ReceivedFile[];
  lanRunning?: boolean;
  microphone?: {
    active: boolean; deviceName: string; endpointName: string; error: string;
    droppedFrames: number; receivedFrames: number; receivedSignalFrames: number;
    renderedSamples: number; renderedSignalSamples: number; inputLevel: number; outputLevel: number;
    gainClippedSamples: number; clipping: boolean;
  };
  discovery: { active: boolean; error: string };
  admin: { active: boolean };
  halo: { enabled: boolean; size: "small" | "medium" | "large" };
  autostart: boolean;
  trusted: boolean;
  pairedCount: number;
  devices: {
    id: string;
    name: string;
    online: boolean;
    permissions: { admin: boolean; input: boolean; text: boolean; files: boolean; power: boolean; microphone: boolean };
    pairedAt: number;
    lastSeen: number;
  }[];
  controlMode: "free" | "primary";
  primaryDevice: string | null;
  controlRequest: { id: string; name: string } | null;
  running: boolean;
  connected: boolean;
  connectedCount: number;
  recentController: string | null;
  code: string;
  codeRemaining: number;
  pairingIntervalMinutes: number;
  addresses: { name: string; address: string; virtual: boolean }[];
  selected: string;
  port: number;
  error: string;
  receivePath: string;
  sharedFiles: { id: number; name: string; size: number }[];
}
