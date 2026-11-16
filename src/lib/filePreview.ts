// What the detail pane can show of a file besides its diff, decided by the extension (the bytes
// come from the repository, so the webview is told the type rather than sniffing it).

export type PreviewKind = "image" | "video" | "audio" | "pdf";

// A Map, not an object literal: "x.constructor" or "x.__proto__" must not resolve to an inherited
// property and reach the renderer with no MIME type.
const TYPES = new Map<string, [PreviewKind, string]>(Object.entries({
  png: ["image", "image/png"],
  jpg: ["image", "image/jpeg"],
  jpeg: ["image", "image/jpeg"],
  gif: ["image", "image/gif"],
  webp: ["image", "image/webp"],
  avif: ["image", "image/avif"],
  bmp: ["image", "image/bmp"],
  ico: ["image", "image/x-icon"],
  // Shown through <img>, where an SVG's scripts do not run.
  svg: ["image", "image/svg+xml"],
  mp4: ["video", "video/mp4"],
  m4v: ["video", "video/mp4"],
  mov: ["video", "video/quicktime"],
  webm: ["video", "video/webm"],
  ogv: ["video", "video/ogg"],
  mp3: ["audio", "audio/mpeg"],
  m4a: ["audio", "audio/mp4"],
  aac: ["audio", "audio/aac"],
  wav: ["audio", "audio/wav"],
  ogg: ["audio", "audio/ogg"],
  oga: ["audio", "audio/ogg"],
  opus: ["audio", "audio/ogg"],
  flac: ["audio", "audio/flac"],
  pdf: ["pdf", "application/pdf"],
} satisfies Record<string, [PreviewKind, string]>));

export function previewType(path: string): { kind: PreviewKind; mime: string } | null {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  const hit = TYPES.get(name.slice(dot + 1).toLowerCase());
  return hit ? { kind: hit[0], mime: hit[1] } : null;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
