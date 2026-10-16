import type { Row } from "./api";

export const ROW_H = 24;
export const HEADER_H = 24;
export const MAX_LANE_W = 14;
export const MIN_LANE_W = 8;
export const LEFT_PAD = 12;
/// この幅までは間隔を詰めて収める。それでも収まらない分はグラフ列で切れる (ヘッダの境界をドラッグして広げられる)
export const TARGET_GRAPH_W = 520;

const PALETTE = [
  "#4a90e2",
  "#e0609a",
  "#3fb8a8",
  "#e8a33d",
  "#7b61ff",
  "#5cb85c",
  "#e05d5d",
  "#4fb0d6",
  "#c44dc4",
  "#8fa84f",
  "#d9c23a",
  "#b07a4a",
];

export function laneColor(index: number): string {
  return PALETTE[index % PALETTE.length];
}

/// レーンが多い時は間隔を詰める (ただし読める最小幅まで)
export function laneWidth(maxLanes: number): number {
  const fit = (TARGET_GRAPH_W - LEFT_PAD - 4) / Math.max(maxLanes, 1);
  return Math.min(MAX_LANE_W, Math.max(MIN_LANE_W, fit));
}

export function laneX(lane: number, laneW: number): number {
  return LEFT_PAD + lane * laneW;
}

export const MIN_GRAPH_W = 64;

export function graphWidth(maxLanes: number): number {
  return Math.max(LEFT_PAD + maxLanes * laneWidth(maxLanes) + 4, MIN_GRAPH_W);
}

export interface DrawOptions {
  getRow(index: number): Row | undefined;
  /// 描く行の範囲 (両端含む)。線は行の間にあるので、可視範囲より 1 行ずつ広く渡す
  first: number;
  last: number;
  scrollTop: number;
  laneW: number;
  headOid: string | null;
  background: string;
}

export function drawGraph(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  o: DrawOptions,
): void {
  ctx.clearRect(0, 0, width, height);
  const lineW = o.laneW < 8 ? 1.5 : 2;
  const radius = Math.max(2, Math.min(4.5, o.laneW * 0.4));
  ctx.lineWidth = lineW;
  ctx.lineCap = "round";

  for (let r = o.first; r <= o.last; r++) {
    const row = o.getRow(r);
    if (!row) continue;
    const y0 = r * ROW_H - o.scrollTop + ROW_H / 2;
    const y1 = y0 + ROW_H;
    for (const s of row.segments) {
      const x0 = laneX(s.from, o.laneW);
      const x1 = laneX(s.to, o.laneW);
      ctx.strokeStyle = laneColor(s.color);
      ctx.beginPath();
      ctx.moveTo(x0, y0);
      if (x0 === x1) {
        ctx.lineTo(x1, y1);
      } else {
        ctx.bezierCurveTo(x0, y0 + ROW_H * 0.6, x1, y1 - ROW_H * 0.6, x1, y1);
      }
      ctx.stroke();
    }
  }

  for (let r = o.first; r <= o.last; r++) {
    const row = o.getRow(r);
    if (!row) continue;
    const x = laneX(row.lane, o.laneW);
    const y = r * ROW_H - o.scrollTop + ROW_H / 2;
    const color = laneColor(row.color);
    const isMerge = row.parents.length > 1;
    const isHead = row.oid === o.headOid;
    ctx.beginPath();
    ctx.arc(x, y, isHead ? radius + 1 : radius, 0, Math.PI * 2);
    ctx.fillStyle = isMerge ? o.background : color;
    ctx.fill();
    ctx.lineWidth = lineW;
    ctx.strokeStyle = color;
    ctx.stroke();
    if (isHead) {
      ctx.beginPath();
      ctx.arc(x, y, Math.max(1, radius - 2.5), 0, Math.PI * 2);
      ctx.fillStyle = o.background;
      ctx.fill();
    }
  }
}
