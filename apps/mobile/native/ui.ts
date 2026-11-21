import type { TouchContact } from "../gestures";

export type NativeControl = {
  kind: "button" | "text" | "heading" | "row" | "column" | "strip" | "switch" | "slider" | "pad" | "dpad" | "mic" | "edit" | "progress" | "window";
  id: string;
  label?: string;
  detail?: string;
  icon?: string;
  iconOnly?: boolean;
  hint?: string;
  status?: string;
  selected?: boolean;
  value?: string | number | boolean;
  revision?: number;
  enabled?: boolean;
  children?: NativeControl[];
};

export const text = (id: string, label: string): NativeControl => ({ kind: "text", id, label });
export const heading = (id: string, label: string): NativeControl => ({ kind: "heading", id, label });
export const row = (id: string, ...children: NativeControl[]): NativeControl => ({ kind: "row", id, children });

export type NativePointer = {
  phase: "down" | "move" | "up" | "cancel" | "lost";
  target: "pad" | "mic";
  pointerId: number;
  x: number;
  y: number;
  width: number;
  primary: boolean;
};

export type NativePointerMoveBatch = { target: "pad"; phase: "move-batch"; contacts: TouchContact[] };

export function asPointerEvent(point: NativePointer): PointerEvent {
  return {
    pointerType: "touch",
    pointerId: point.pointerId,
    button: 0,
    isPrimary: point.primary,
    clientX: point.x,
    clientY: point.y,
    currentTarget: {
      setPointerCapture() {},
      getBoundingClientRect: () => ({ left: 0, right: point.width, width: point.width }),
    },
  } as unknown as PointerEvent;
}
