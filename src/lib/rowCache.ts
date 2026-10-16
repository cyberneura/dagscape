import { api, type Row } from "./api";

const CHUNK = 256;
// 10 万行を全部スクロールしても WebView に全履歴が溜まらないよう、使っていないチャンクから捨てる
const MAX_CHUNKS = 48;

/// 行をチャンク単位で IPC 取得してキャッシュする。表示側は可視範囲だけ ensure すればよい。
export class RowCache {
  private chunks = new Map<number, Row[]>();
  private pending = new Map<number, Promise<void>>();
  private lastUsed = new Map<number, number>();
  private clock = 0;

  constructor(readonly total: number) {}

  get(index: number): Row | undefined {
    if (index < 0 || index >= this.total) return undefined;
    const c = Math.floor(index / CHUNK);
    const chunk = this.chunks.get(c);
    if (chunk) this.lastUsed.set(c, ++this.clock);
    return chunk?.[index % CHUNK];
  }

  /// [start, end) を使えるようにする。既に揃っていれば何も取らない。
  async ensure(start: number, end: number): Promise<void> {
    start = Math.max(0, start);
    end = Math.min(this.total, end);
    if (start >= end) return;
    const waits: Promise<void>[] = [];
    for (let c = Math.floor(start / CHUNK); c <= Math.floor((end - 1) / CHUNK); c++) {
      if (this.chunks.has(c)) continue;
      let p = this.pending.get(c);
      if (!p) {
        p = api
          .getRows(c * CHUNK, (c + 1) * CHUNK)
          .then((rows) => {
            this.chunks.set(c, rows);
          })
          .finally(() => {
            this.pending.delete(c);
          });
        this.pending.set(c, p);
      }
      waits.push(p);
    }
    await Promise.all(waits);
    this.evict(Math.floor(start / CHUNK), Math.floor((end - 1) / CHUNK));
  }

  /// [keepFrom, keepTo] のチャンク以外を、最後に読まれたのが古い順に捨てる
  private evict(keepFrom: number, keepTo: number): void {
    if (this.chunks.size <= MAX_CHUNKS) return;
    const candidates = [...this.chunks.keys()]
      .filter((c) => c < keepFrom || c > keepTo)
      .sort((a, b) => (this.lastUsed.get(a) ?? 0) - (this.lastUsed.get(b) ?? 0));
    for (const c of candidates) {
      if (this.chunks.size <= MAX_CHUNKS) break;
      this.chunks.delete(c);
      this.lastUsed.delete(c);
    }
  }
}
