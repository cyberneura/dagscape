<script lang="ts">
  import type { RefLabel } from "./api";
  import type { RowCache } from "./rowCache";
  import { HEADER_H, MIN_GRAPH_W, ROW_H, TARGET_GRAPH_W, drawGraph, graphWidth, laneWidth } from "./graph";
  import { formatDate } from "./format";

  interface Props {
    cache: RowCache;
    maxLanes: number;
    selected: number | null;
    hits: Set<number>;
    headOid: string | null;
    onselect: (index: number) => void;
  }
  let { cache, maxLanes, selected, hits, headOid, onselect }: Props = $props();

  let scroller = $state<HTMLDivElement | undefined>();
  let canvas = $state<HTMLCanvasElement | undefined>();
  let scrollTop = $state(0);
  // 横スクロールした時に、外側に置いた Canvas を行と同じだけずらす
  let scrollLeft = $state(0);
  let height = $state(0);
  // 行の取得が終わるたびに増やして、行と Canvas を描き直す
  let tick = $state(0);

  const total = $derived(cache.total);
  // グラフ全体を描くのに要る幅。列幅はこれを上限に、既定 TARGET_GRAPH_W で切り、ドラッグで広げられる
  const naturalW = $derived(graphWidth(maxLanes));
  let graphColW = $state(0);
  const graphW = $derived(
    Math.min(naturalW, graphColW > 0 ? Math.max(graphColW, MIN_GRAPH_W) : TARGET_GRAPH_W),
  );
  const laneW = $derived(laneWidth(maxLanes));
  // 行の部分の高さ (ヘッダを除く)
  const viewH = $derived(Math.max(0, height - HEADER_H));
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW_H)));
  const last = $derived(Math.min(total - 1, Math.ceil((scrollTop + viewH) / ROW_H)));
  const rows = $derived.by(() => {
    tick;
    const out: { i: number; row: ReturnType<RowCache["get"]> }[] = [];
    for (let i = first; i <= last; i++) out.push({ i, row: cache.get(i) });
    return out;
  });

  $effect(() => {
    const c = cache;
    const f = first;
    const l = last;
    if (c.total === 0 || f > l) return;
    let cancelled = false;
    // 線は行の間にあるので、前後 1 行ずつ余分に取る
    c.ensure(f - 1, l + 2).then(() => {
      if (!cancelled) tick++;
    });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    if (!scroller) return;
    const el = scroller;
    const ro = new ResizeObserver(() => {
      height = el.clientHeight;
    });
    ro.observe(el);
    height = el.clientHeight;
    return () => ro.disconnect();
  });

  $effect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => tick++;
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  });

  $effect(() => {
    tick;
    headOid;
    if (!canvas || !scroller) return;
    const dpr = window.devicePixelRatio || 1;
    const w = graphW;
    const h = viewH;
    const pw = Math.round(w * dpr);
    const ph = Math.round(h * dpr);
    if (canvas.width !== pw || canvas.height !== ph) {
      canvas.width = pw;
      canvas.height = ph;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    drawGraph(ctx, w, h, {
      getRow: (i) => cache.get(i),
      first: Math.max(0, first - 1),
      last: Math.min(total - 1, last + 1),
      scrollTop,
      laneW,
      headOid,
      background: getComputedStyle(scroller).backgroundColor,
    });
  });

  export function ensureVisible(index: number): void {
    if (!scroller) return;
    const top = index * ROW_H;
    const bottom = top + ROW_H;
    const visibleH = scroller.clientHeight - HEADER_H;
    if (top < scroller.scrollTop) {
      scroller.scrollTop = top;
    } else if (bottom > scroller.scrollTop + visibleH) {
      scroller.scrollTop = bottom - visibleH;
    }
  }

  export function scrollToCenter(index: number): void {
    if (!scroller) return;
    const visibleH = scroller.clientHeight - HEADER_H;
    scroller.scrollTop = Math.max(0, index * ROW_H - visibleH / 2 + ROW_H / 2);
  }

  export function getScrollTop(): number {
    return scroller?.scrollTop ?? 0;
  }

  export function setScrollTop(value: number): void {
    if (!scroller) return;
    scroller.scrollTop = value;
    scrollTop = scroller.scrollTop;
  }

  export function pageSize(): number {
    return Math.max(1, Math.floor(viewH / ROW_H) - 1);
  }

  function startColumnResize(e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = graphW;
    const move = (ev: PointerEvent) => {
      graphColW = Math.max(MIN_GRAPH_W, Math.min(naturalW, startW + ev.clientX - startX));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function chipClass(ref: RefLabel): string {
    switch (ref.kind) {
      case "head":
        return "bg-neutral-800 text-white dark:bg-neutral-200 dark:text-black";
      case "branch":
        return ref.is_head
          ? "bg-emerald-600 font-bold text-white ring-2 ring-emerald-300 dark:ring-emerald-400"
          : "bg-emerald-600/85 text-white";
      case "remote":
        return "bg-sky-600/85 text-white";
      case "tag":
        return "bg-amber-400 text-black";
    }
  }
</script>

<div class="relative h-full w-full">
  <div
    class="absolute inset-0 overflow-auto bg-white dark:bg-neutral-900"
    bind:this={scroller}
    onscroll={() => {
      if (!scroller) return;
      scrollTop = scroller.scrollTop;
      scrollLeft = scroller.scrollLeft;
    }}
    role="listbox"
    tabindex="-1"
    aria-label="Commits"
  >
    <div class="relative" style="height:{HEADER_H + total * ROW_H}px">
      <div
        class="sticky top-0 z-10 flex items-center border-b border-neutral-200 bg-neutral-50 text-[11px] text-neutral-500 uppercase dark:border-neutral-700 dark:bg-neutral-800"
        style="height:{HEADER_H}px"
      >
        <div class="shrink-0 px-2" style="width:{graphW - 4}px">Graph</div>
        <div
          class="h-full w-1 shrink-0 cursor-col-resize hover:bg-blue-400"
          onpointerdown={startColumnResize}
          role="separator"
          aria-orientation="vertical"
          title="Drag to resize the graph column"
        ></div>
        <div class="flex-1">Subject</div>
        <div class="w-36 shrink-0">Author</div>
        <div class="w-32 shrink-0">Date</div>
        <div class="w-16 shrink-0">SHA</div>
      </div>
      {#each rows as { i, row } (i)}
        <!-- キーボード操作 (矢印 / PageUp / Home 等) は +page.svelte が window で受けている -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="absolute right-0 left-0 flex items-center text-[13px] {i === selected
            ? 'bg-blue-500/25'
            : hits.has(i)
              ? 'bg-amber-400/25'
              : 'hover:bg-neutral-500/10'}"
          style="top:{HEADER_H + i * ROW_H}px;height:{ROW_H}px"
          role="option"
          aria-selected={i === selected}
          tabindex="-1"
          onclick={() => onselect(i)}
        >
          <div class="shrink-0" style="width:{graphW}px"></div>
          {#if row}
            <div class="flex min-w-0 flex-1 items-center gap-1 pr-2">
              {#each row.refs as ref (ref.kind + ref.name)}
                <span class="chip {chipClass(ref)}" title={ref.name}>{ref.name}</span>
              {/each}
              <span class="truncate">{row.summary}</span>
            </div>
            <div class="w-36 shrink-0 truncate pr-2 text-neutral-500 dark:text-neutral-400">
              {row.author}
            </div>
            <div class="w-32 shrink-0 pr-2 text-neutral-500 tabular-nums dark:text-neutral-400">
              {formatDate(row.time)}
            </div>
            <div class="w-16 shrink-0 font-mono text-xs text-neutral-400 dark:text-neutral-500">
              {row.short}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </div>
  <canvas
    bind:this={canvas}
    class="pointer-events-none absolute"
    style="left:{-scrollLeft}px;top:{HEADER_H}px;width:{graphW}px;height:{viewH}px"
  ></canvas>
</div>
