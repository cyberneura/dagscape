<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { load as loadStore, type Store } from "@tauri-apps/plugin-store";
  import { clampSize, dragResize, keyResize, type ResizeBounds } from "$lib/resize";
  import { api, type RefLabel, type RepoSummary } from "$lib/api";
  import { RowCache } from "$lib/rowCache";
  import { GitHub } from "$lib/github";
  import CommitList from "$lib/CommitList.svelte";
  import CommitDetail from "$lib/CommitDetail.svelte";
  import RefSidebar from "$lib/RefSidebar.svelte";

  const MAX_RECENT = 10;

  let store: Store | null = null;
  let summary = $state<RepoSummary | null>(null);
  let cache = $state.raw<RowCache>(new RowCache(0));
  let selected = $state<number | null>(null);
  let selectedOid = $state<string | null>(null);
  let selectedRefs = $state<RefLabel[]>([]);
  let includeRemotes = $state(true);
  let recent = $state<string[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let query = $state("");
  let hits = $state<number[]>([]);
  let hitPos = $state(0);
  // Pane sizes, in px. Restored from the store once it is loaded; saved when a drag ends.
  let detailHeight = $state(320);
  let sidebarWidth = $state(224);
  let fileListWidth = $state(320);
  let showSidebar = $state(true);
  let list = $state<CommitList | undefined>();
  let searchInput = $state<HTMLInputElement | undefined>();
  // open / reload が重なった時に、古い方の結果を捨てるための世代番号
  let generation = 0;
  // 連続した選択 / 検索で、遅れて返った古い結果を捨てるための番号
  let selectSeq = 0;
  let searchSeq = 0;
  // open の読込中は旧リポジトリの watcher / Reload ボタンからの reload を受けない。
  // 受けると reload が先に終わって summary が旧リポジトリ、Rust 側 session が新リポジトリになる
  let opening = false;

  const hitSet = $derived(new Set(hits));
  const github = $derived(new GitHub(summary?.remotes ?? []));
  const status = $derived(
    loading
      ? "Loading…"
      : summary
        ? `${summary.total.toLocaleString()} commits · ${summary.load_ms} ms`
        : "",
  );

  function applySummary(s: RepoSummary) {
    summary = s;
    cache = new RowCache(s.total);
    selected = null;
    selectedOid = null;
    selectedRefs = [];
    hits = [];
    hitPos = 0;
  }

  async function rememberRecent(path: string) {
    recent = [path, ...recent.filter((p) => p !== path)].slice(0, MAX_RECENT);
    await store?.set("recent", recent);
  }

  async function forgetRecent(path: string) {
    if (!recent.includes(path)) return;
    recent = recent.filter((p) => p !== path);
    await store?.set("recent", recent);
  }

  async function openRepo(path: string) {
    const gen = ++generation;
    loading = true;
    opening = true;
    error = null;
    try {
      const s = await api.openRepo(path, includeRemotes);
      if (gen !== generation) return;
      applySummary(s);
      await rememberRecent(s.path);
      await getCurrentWindow().setTitle(`${s.name} — Dagscape`);
      await selectOid(s.head?.oid ?? null, true);
    } catch (e) {
      if (gen !== generation) return;
      error = String(e);
      await forgetRecent(path);
    } finally {
      if (gen === generation) {
        loading = false;
        opening = false;
      }
    }
  }

  async function reload() {
    if (!summary || opening) return;
    const gen = ++generation;
    loading = true;
    const keepOid = selectedOid;
    const keepScroll = list?.getScrollTop() ?? 0;
    const keepQuery = query.trim();
    try {
      const s = await api.reloadRepo(includeRemotes);
      if (gen !== generation) return;
      applySummary(s);
      await tick();
      list?.setScrollTop(keepScroll);
      if (keepOid) await selectOid(keepOid, false);
      if (keepQuery) await runSearch(false);
    } catch (e) {
      if (gen === generation) error = String(e);
    } finally {
      if (gen === generation) loading = false;
    }
  }

  async function chooseRepo() {
    const dir = await openDialog({ directory: true, multiple: false, title: "Open Git Repository" });
    if (typeof dir === "string") await openRepo(dir);
  }

  async function toggleRemotes() {
    includeRemotes = !includeRemotes;
    await store?.set("includeRemotes", includeRemotes);
    await reload();
  }

  async function selectIndex(index: number | null) {
    const seq = ++selectSeq;
    selected = index;
    if (index === null) {
      selectedOid = null;
      selectedRefs = [];
      return;
    }
    list?.ensureVisible(index);
    const c = cache;
    await c.ensure(index, index + 1);
    if (c !== cache || seq !== selectSeq) return;
    const row = c.get(index);
    selectedOid = row?.oid ?? null;
    selectedRefs = row?.refs ?? [];
  }

  async function selectOid(oid: string | null, center: boolean) {
    if (!oid) return;
    const index = await api.rowIndexOf(oid);
    if (index === null) return;
    if (center) list?.scrollToCenter(index);
    await selectIndex(index);
  }

  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  function onQueryInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => runSearch(true), 200);
  }

  async function runSearch(jump: boolean) {
    if (!summary) return;
    const seq = ++searchSeq;
    const q = query.trim();
    if (!q) {
      hits = [];
      hitPos = 0;
      return;
    }
    const gen = generation;
    const found = await api.searchCommits(q);
    if (gen !== generation || seq !== searchSeq) return;
    hits = found;
    hitPos = 0;
    if (jump && found.length > 0) {
      list?.scrollToCenter(found[0]);
      await selectIndex(found[0]);
    }
  }

  function stepHit(delta: number) {
    if (hits.length === 0) return;
    hitPos = (hitPos + delta + hits.length) % hits.length;
    list?.scrollToCenter(hits[hitPos]);
    selectIndex(hits[hitPos]);
  }

  function focusSearch() {
    searchInput?.focus();
    searchInput?.select();
  }

  function onMenu(id: string) {
    if (id === "open-repo") chooseRepo();
    else if (id === "reload-repo") reload();
    else if (id === "find") focusSearch();
  }

  function onKeydown(e: KeyboardEvent) {
    const target = e.target;
    const inInput = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
    if (inInput) {
      if (e.key === "Escape") {
        (target as HTMLElement).blur();
        query = "";
        hits = [];
        searchSeq++;
      } else if (e.key === "Enter") {
        e.preventDefault();
        stepHit(e.shiftKey ? -1 : 1);
      }
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === "f") {
      e.preventDefault();
      focusSearch();
      return;
    }
    if (!summary || summary.total === 0) return;
    const total = summary.total;
    const current = selected ?? -1;
    let next: number;
    switch (e.key) {
      case "ArrowDown":
      case "j":
        next = Math.min(total - 1, current + 1);
        break;
      case "ArrowUp":
      case "k":
        next = Math.max(0, current - 1);
        break;
      case "PageDown":
        next = Math.min(total - 1, current + (list?.pageSize() ?? 20));
        break;
      case "PageUp":
        next = Math.max(0, current - (list?.pageSize() ?? 20));
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = total - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    selectIndex(next);
  }

  // The same bounds for the drag, the keys, a restored value and a window that shrank, so a
  // size saved on a big screen cannot push the other panes off a small one. The window size is
  // state so the separators' aria-valuemax follows it.
  let windowWidth = $state(0);
  let windowHeight = $state(0);
  const detailBounds: ResizeBounds = { min: 120, max: () => windowHeight - 200 };
  const sidebarBounds: ResizeBounds = { min: 120, max: () => windowWidth - 400 };

  function startDetailResize(e: PointerEvent) {
    dragResize(e, {
      ...detailBounds,
      axis: "y",
      start: detailHeight,
      invert: true,
      onchange: (h) => (detailHeight = h),
      ondone: (h) => void store?.set("detailHeight", h),
    });
  }

  function startSidebarResize(e: PointerEvent) {
    dragResize(e, {
      ...sidebarBounds,
      axis: "x",
      start: sidebarWidth,
      onchange: (w) => (sidebarWidth = w),
      ondone: (w) => void store?.set("sidebarWidth", w),
    });
  }

  function onDetailKey(e: KeyboardEvent) {
    const h = keyResize(e, {
      ...detailBounds,
      axis: "y",
      current: detailHeight,
      invert: true,
      onchange: (v) => (detailHeight = v),
    });
    if (h === null) return;
    e.preventDefault();
    e.stopPropagation();
    void store?.set("detailHeight", h);
  }

  function onSidebarKey(e: KeyboardEvent) {
    const w = keyResize(e, {
      ...sidebarBounds,
      axis: "x",
      current: sidebarWidth,
      onchange: (v) => (sidebarWidth = v),
    });
    if (w === null) return;
    e.preventDefault();
    e.stopPropagation();
    void store?.set("sidebarWidth", w);
  }

  function saveFileListWidth(w: number) {
    void store?.set("fileListWidth", w);
  }

  function clampPanes() {
    windowWidth = window.innerWidth;
    windowHeight = window.innerHeight;
    detailHeight = clampSize(detailHeight, detailBounds);
    sidebarWidth = clampSize(sidebarWidth, sidebarBounds);
  }

  onMount(() => {
    const unlisteners: Array<() => void> = [];
    let disposed = false;
    clampPanes();
    (async () => {
      store = await loadStore("settings.json");
      includeRemotes = (await store.get<boolean>("includeRemotes")) ?? true;
      recent = (await store.get<string[]>("recent")) ?? [];
      detailHeight = (await store.get<number>("detailHeight")) ?? detailHeight;
      sidebarWidth = (await store.get<number>("sidebarWidth")) ?? sidebarWidth;
      fileListWidth = (await store.get<number>("fileListWidth")) ?? fileListWidth;
      clampPanes();
      unlisteners.push(await listen("repo-changed", () => reload()));
      unlisteners.push(await listen<string>("menu", (e) => onMenu(e.payload)));
      unlisteners.push(
        await getCurrentWebview().onDragDropEvent((e) => {
          if (e.payload.type === "drop" && e.payload.paths[0]) openRepo(e.payload.paths[0]);
        }),
      );
      if (disposed) return;
      const initial = await api.initialPath();
      if (initial) await openRepo(initial);
      else if (recent[0]) await openRepo(recent[0]);
    })();
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  });
</script>

<svelte:window onkeydown={onKeydown} onresize={clampPanes} />

<div class="flex h-screen flex-col">
  <div
    class="flex h-10 shrink-0 items-center gap-2 border-b border-neutral-300 bg-neutral-100 px-2 text-[12px] dark:border-neutral-700 dark:bg-neutral-800"
  >
    <button class="btn" onclick={() => (showSidebar = !showSidebar)} title="Toggle sidebar">☰</button>
    <button class="btn" onclick={chooseRepo} title="Open Repository (⌘O)">Open…</button>
    <button class="btn" onclick={reload} disabled={!summary} title="Reload (⌘R)">Reload</button>
    <label class="flex items-center gap-1">
      <input type="checkbox" checked={includeRemotes} onchange={toggleRemotes} />
      Remotes
    </label>
    <div class="min-w-0 flex-1 truncate text-neutral-600 dark:text-neutral-300" title={summary?.path}>
      {summary?.path ?? ""}
    </div>
    <div class="text-neutral-500 tabular-nums">{status}</div>
    <input
      bind:this={searchInput}
      bind:value={query}
      oninput={onQueryInput}
      placeholder="Search (⌘F)"
      class="selectable w-56 rounded border border-neutral-300 bg-white px-2 py-0.5 dark:border-neutral-600 dark:bg-neutral-900"
    />
    {#if query.trim()}
      <span class="w-14 text-right text-neutral-500 tabular-nums">
        {hits.length ? `${hitPos + 1}/${hits.length}` : "0"}
      </span>
      <button class="btn" onclick={() => stepHit(-1)} title="Previous (Shift+Enter)">↑</button>
      <button class="btn" onclick={() => stepHit(1)} title="Next (Enter)">↓</button>
    {/if}
  </div>

  {#if error}
    <div class="flex shrink-0 items-center gap-2 bg-red-600 px-3 py-1 text-xs text-white">
      <span class="flex-1 truncate">{error}</span>
      <button onclick={() => (error = null)}>✕</button>
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    {#if showSidebar}
      <div
        class="shrink-0 overflow-hidden bg-neutral-50 dark:bg-neutral-800/60"
        style="width:{sidebarWidth}px"
      >
        <RefSidebar
          refs={summary?.refs ?? []}
          activeOid={selectedOid}
          {github}
          onjump={(oid) => selectOid(oid, true)}
        />
      </div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <!-- A focusable separator is the WAI-ARIA window-splitter pattern; Svelte's a11y rules only know the static kind. -->
      <div
        class="w-1.5 shrink-0 cursor-col-resize touch-none bg-neutral-300 hover:bg-blue-400 focus:bg-blue-400 focus:outline-none dark:bg-neutral-700"
        onpointerdown={startSidebarResize}
        onkeydown={onSidebarKey}
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize sidebar"
        aria-valuenow={sidebarWidth}
        aria-valuemin={sidebarBounds.min}
        aria-valuemax={sidebarBounds.max()}
        tabindex="0"
      ></div>
    {/if}
    <div class="flex min-w-0 flex-1 flex-col">
      {#if !summary}
        <div class="flex flex-1 flex-col items-center justify-center gap-3 text-neutral-500">
          <div class="text-lg">Open a Git repository</div>
          <button class="btn" onclick={chooseRepo}>Open Repository… (⌘O)</button>
          <div class="text-xs">or drop a folder onto this window</div>
          {#if recent.length > 0}
            <div class="mt-4 w-[28rem] text-xs">
              <div class="mb-1 font-semibold uppercase">Recent</div>
              {#each recent as path (path)}
                <button
                  class="block w-full truncate rounded px-2 py-1 text-left hover:bg-neutral-500/10"
                  onclick={() => openRepo(path)}
                  title={path}>{path}</button
                >
              {/each}
            </div>
          {/if}
        </div>
      {:else}
        <div class="min-h-0 flex-1">
          <CommitList
            bind:this={list}
            {cache}
            maxLanes={summary.max_lanes}
            {selected}
            hits={hitSet}
            headOid={summary.head?.oid ?? null}
            {github}
            onselect={selectIndex}
          />
        </div>
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <!-- A focusable separator is the WAI-ARIA window-splitter pattern; Svelte's a11y rules only know the static kind. -->
        <div
          class="h-1.5 shrink-0 cursor-row-resize touch-none bg-neutral-300 hover:bg-blue-400 focus:bg-blue-400 focus:outline-none dark:bg-neutral-700"
          onpointerdown={startDetailResize}
          onkeydown={onDetailKey}
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize commit detail"
          aria-valuenow={detailHeight}
          aria-valuemin={detailBounds.min}
          aria-valuemax={detailBounds.max()}
          tabindex="0"
        ></div>
        <div class="shrink-0 bg-white dark:bg-neutral-900" style="height:{detailHeight}px">
          <CommitDetail
            oid={selectedOid}
            refs={selectedRefs}
            {github}
            bind:fileListWidth
            onfilelistresized={saveFileListWidth}
            onjump={(oid) => selectOid(oid, true)}
          />
        </div>
      {/if}
    </div>
  </div>
</div>
