// Resizing of a pane from its separator, shared by the three separators (sidebar | list,
// list / detail, file list | diff): a pointer drag, and the arrow keys for keyboard users
// (WAI-ARIA window splitter: the separator is focusable and carries value / min / max).

export interface ResizeBounds {
  min: number;
  /** Read at every step: the window may be resized mid-drag. */
  max: () => number;
}

export interface DragResizeOptions extends ResizeBounds {
  axis: "x" | "y";
  /** Size when the drag started. */
  start: number;
  /** For a handle whose pane grows when the pointer moves towards the origin (the detail pane). */
  invert?: boolean;
  onchange: (size: number) => void;
  /** Called once with the final size; the place to persist it. */
  ondone?: (size: number) => void;
}

export function clampSize(size: number, bounds: ResizeBounds): number {
  return Math.max(bounds.min, Math.min(bounds.max(), size));
}

/**
 * Starts a drag from a pointerdown on the separator. The pointer is captured by the handle, so
 * the drag survives the pointer leaving it and ends on any release, cancel, or loss of capture
 * (a window losing focus mid-drag would otherwise keep resizing when the pointer came back).
 */
export function dragResize(e: PointerEvent, options: DragResizeOptions): void {
  const handle = e.currentTarget as HTMLElement | null;
  if (!handle) return;
  const pointerId = e.pointerId;
  const origin = options.axis === "x" ? e.clientX : e.clientY;
  let size = options.start;
  let done = false;
  const move = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    const now = options.axis === "x" ? ev.clientX : ev.clientY;
    const delta = options.invert ? origin - now : now - origin;
    size = clampSize(options.start + delta, options);
    options.onchange(size);
  };
  const finish = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId || done) return;
    done = true;
    handle.removeEventListener("pointermove", move);
    handle.removeEventListener("pointerup", finish);
    handle.removeEventListener("pointercancel", finish);
    handle.removeEventListener("lostpointercapture", finish);
    if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
    options.ondone?.(size);
  };
  handle.addEventListener("pointermove", move);
  handle.addEventListener("pointerup", finish);
  handle.addEventListener("pointercancel", finish);
  handle.addEventListener("lostpointercapture", finish);
  handle.setPointerCapture(pointerId);
  e.preventDefault();
}

/** Step for one arrow-key press, in px. */
export const KEY_STEP = 16;

export interface KeyResizeOptions extends ResizeBounds {
  axis: "x" | "y";
  current: number;
  /** The key that makes the pane *smaller* is the one pointing into it (see `invert`). */
  invert?: boolean;
  onchange: (size: number) => void;
}

/**
 * Arrow keys on a focused separator. Returns the new size, or null when the key was not one of
 * ours -- the caller stops the event only then, so the list's own navigation keeps the rest.
 */
export function keyResize(e: KeyboardEvent, options: KeyResizeOptions): number | null {
  const [grow, shrink] =
    options.axis === "x" ? ["ArrowRight", "ArrowLeft"] : ["ArrowDown", "ArrowUp"];
  let delta: number;
  if (e.key === grow) delta = KEY_STEP;
  else if (e.key === shrink) delta = -KEY_STEP;
  else if (e.key === "Home") delta = -Infinity;
  else if (e.key === "End") delta = Infinity;
  else return null;
  if (options.invert) delta = -delta;
  const size = clampSize(options.current + delta, options);
  options.onchange(size);
  return size;
}
