<script lang="ts">
  import {
    api,
    type CommitDetail,
    type DiffLine,
    type FileChange,
    type FileDiff,
    type RefLabel,
    type Signature,
  } from "./api";
  import { chipClass, lineClass, statusClass } from "./diffStyle";
  import { formatDateTime } from "./format";
  import FilePreview from "./FilePreview.svelte";
  import SubmoduleDetail from "./SubmoduleDetail.svelte";
  import { previewType } from "./filePreview";
  import { showLinkMenu, type GitHub, type LinkItem } from "./github";
  import { clampSize, dragResize, keyResize, type ResizeBounds } from "./resize";

  interface Props {
    oid: string | null;
    refs: RefLabel[];
    github: GitHub;
    /** Width of the file list, in px. Bindable so the shell can restore and persist it. */
    fileListWidth?: number;
    onfilelistresized?: (width: number) => void;
    /**
     * Height of the commit header (id, refs, author, message), in px. null fits it to the content
     * with the message capped, as before it could be resized. Bindable like fileListWidth.
     */
    headerHeight?: number | null;
    onheaderresized?: (height: number | null) => void;
    onjump: (oid: string) => void;
  }
  let {
    oid,
    refs,
    github,
    fileListWidth = $bindable(320),
    onfilelistresized,
    headerHeight = $bindable(null),
    onheaderresized,
    onjump,
  }: Props = $props();
  let panes = $state<HTMLDivElement | null>(null);
  let panesWidth = $state(0);
  // Leave room for the diff whatever the window (or the sidebar) does.
  const fileListBounds: ResizeBounds = {
    min: 120,
    max: () => Math.max(120, (panesWidth || window.innerWidth) - 200),
  };

  // Re-clamp when the room changes: a width restored from the store on a smaller window, or the
  // sidebar being widened. ResizeObserver rather than the window's resize, because the pane's
  // width also moves when the sidebar does.
  $effect(() => {
    const el = panes;
    if (!el) return;
    const observer = new ResizeObserver(() => {
      panesWidth = el.clientWidth;
      const clamped = clampSize(fileListWidth, fileListBounds);
      if (clamped !== fileListWidth) fileListWidth = clamped;
    });
    observer.observe(el);
    return () => observer.disconnect();
  });

  function startFileListResize(e: PointerEvent) {
    dragResize(e, {
      ...fileListBounds,
      axis: "x",
      start: fileListWidth,
      onchange: (w) => (fileListWidth = w),
      ondone: (w) => onfilelistresized?.(w),
    });
  }

  function onFileListKey(e: KeyboardEvent) {
    const w = keyResize(e, {
      ...fileListBounds,
      axis: "x",
      current: fileListWidth,
      onchange: (v) => (fileListWidth = v),
    });
    if (w === null) return;
    e.preventDefault();
    e.stopPropagation();
    onfilelistresized?.(w);
  }

  let root = $state<HTMLDivElement | null>(null);
  let rootHeight = $state(0);
  let header = $state<HTMLDivElement | null>(null);
  // Leave room for a few rows of the file list / diff below the header.
  const headerBounds: ResizeBounds = {
    min: 40,
    max: () => Math.max(40, (rootHeight || window.innerHeight) - 80),
  };

  $effect(() => {
    const el = root;
    if (!el) return;
    const observer = new ResizeObserver(() => (rootHeight = el.clientHeight));
    observer.observe(el);
    return () => observer.disconnect();
  });

  // Re-clamp both when the detail pane gets shorter and when the shell restores a saved height
  // (the store is read asynchronously, possibly after the first resize notification).
  $effect(() => {
    if (headerHeight === null || !rootHeight) return;
    const clamped = clampSize(headerHeight, headerBounds);
    if (clamped !== headerHeight) headerHeight = clamped;
  });

  // While the header is still auto-sized, the first drag or key step starts from its rendered height.
  function currentHeaderHeight(): number {
    return headerHeight ?? header?.offsetHeight ?? headerBounds.min;
  }

  function startHeaderResize(e: PointerEvent) {
    dragResize(e, {
      ...headerBounds,
      axis: "y",
      start: currentHeaderHeight(),
      onchange: (h) => (headerHeight = h),
      ondone: (h) => onheaderresized?.(h),
    });
  }

  function onHeaderKey(e: KeyboardEvent) {
    const h = keyResize(e, {
      ...headerBounds,
      axis: "y",
      current: currentHeaderHeight(),
      onchange: (v) => (headerHeight = v),
    });
    if (h === null) return;
    e.preventDefault();
    e.stopPropagation();
    onheaderresized?.(h);
  }

  // Double-click goes back to fitting the content.
  function resetHeaderHeight() {
    headerHeight = null;
    onheaderresized?.(null);
  }

  let detail = $state<CommitDetail | null>(null);
  let error = $state<string | null>(null);
  let selectedFile = $state<FileChange | null>(null);
  let diff = $state<FileDiff | null>(null);
  let diffError = $state<string | null>(null);

  $effect(() => {
    const id = oid;
    detail = null;
    selectedFile = null;
    error = null;
    if (!id) return;
    let cancelled = false;
    api
      .getCommit(id)
      .then((d) => {
        if (cancelled) return;
        detail = d;
        selectedFile = d.files[0] ?? null;
      })
      .catch((e) => {
        if (!cancelled) error = String(e);
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const id = oid;
    const file = selectedFile;
    diff = null;
    diffError = null;
    if (!id || !file) return;
    let cancelled = false;
    api
      .getFileDiff(id, file.path, file.old_path)
      .then((d) => {
        if (!cancelled) diff = d;
      })
      .catch((e) => {
        if (!cancelled) diffError = String(e);
      });
    return () => {
      cancelled = true;
    };
  });

  // 削除されたファイル・削除行は、このコミットには無いので第 1 親の版を開く
  function fileLinks(d: CommitDetail, file: FileChange, line?: DiffLine): LinkItem[] {
    const parent = d.parents[0] ?? null;
    const oldPath = file.old_path ?? file.path;
    let blob: string | null;
    let diffLine: { side: "L" | "R"; no: number } | undefined;
    if (line?.kind === "del" && line.old_no !== null) {
      blob = parent && github.fileUrl(parent, oldPath, line.old_no);
      diffLine = { side: "L", no: line.old_no };
    } else if (line && line.kind !== "hunk" && line.new_no !== null) {
      blob = github.fileUrl(d.oid, file.path, line.new_no);
      diffLine = { side: "R", no: line.new_no };
    } else {
      blob = file.status === "D" ? parent && github.fileUrl(parent, oldPath) : github.fileUrl(d.oid, file.path);
    }
    const at = diffLine ? ` at Line ${diffLine.no}` : "";
    return [
      { text: `Open File${at} on GitHub`, url: blob },
      { text: `Open Diff${at} on GitHub`, url: github.available ? github.diffUrl(d.oid, file.path, diffLine) : null },
    ];
  }

  function person(sig: Signature): string {
    return `${sig.name} <${sig.email}>`;
  }
</script>

<div class="flex h-full flex-col text-[13px]" bind:this={root}>
  {#if !oid}
    <div class="p-4 text-neutral-500">Select a commit to see its details.</div>
  {:else if error}
    <div class="p-4 text-red-500">{error}</div>
  {:else if !detail}
    <div class="p-4 text-neutral-500">Loading…</div>
  {:else}
    <div
      class="selectable shrink-0 px-3 py-2 {headerHeight === null ? '' : 'overflow-auto'}"
      style={headerHeight === null ? undefined : `height:${headerHeight}px`}
      bind:this={header}
    >
      <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span
          class="font-mono text-xs text-neutral-500 dark:text-neutral-400"
          role="note"
          oncontextmenu={(e) =>
            detail && showLinkMenu(e, [{ text: "Open Commit on GitHub", url: github.commitUrl(detail.oid) }])}
          >{detail.oid}</span
        >
        {#each refs as ref (ref.kind + ref.name)}
          <span
            class="chip {chipClass(ref.kind)}"
            role="note"
            oncontextmenu={(e) =>
              detail && showLinkMenu(e, [{ text: `Open ${ref.name} on GitHub`, url: github.refUrl(ref, detail.oid) }])}
            >{ref.name}</span
          >
        {/each}
      </div>
      <!--
        Two label / value pairs per row once the pane is wide enough (a container query, because the
        sidebar takes its share of the window): when and from what on the left, by whom on the right.
        The second pair's label carries extra left padding so it does not read as part of the first
        pair's value, and the first value is capped (fit-content) so an octopus merge's parents wrap
        instead of squeezing the names. "Committed at" is the committer date; the author date differs after a rebase,
        cherry-pick or amend, and is then shown next to the author rather than dropped.
      -->
      <div class="@container mt-1">
        <div
          class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs text-neutral-600 @xl:grid-cols-[auto_fit-content(45%)_auto_minmax(0,1fr)] dark:text-neutral-300"
        >
          <span class="whitespace-nowrap text-neutral-400">Committed at</span>
          <span class="whitespace-nowrap">{formatDateTime(detail.committer.time)}</span>
          <span class="text-neutral-400 @xl:pl-8">Author</span>
          <span>
            {person(detail.author)}
            {#if detail.author.time !== detail.committer.time}
              <span class="whitespace-nowrap text-neutral-400">· authored {formatDateTime(detail.author.time)}</span>
            {/if}
          </span>
          <span class="text-neutral-400">Parents</span>
          <span>
            {#each detail.parents as parent (parent)}
              <button
                class="mr-2 cursor-pointer font-mono text-sky-600 hover:underline dark:text-sky-400"
                onclick={() => onjump(parent)}
                oncontextmenu={(e) => showLinkMenu(e, [{ text: "Open Commit on GitHub", url: github.commitUrl(parent) }])}
                >{parent.slice(0, 7)}</button
              >
            {:else}
              <span class="text-neutral-400">(root commit)</span>
            {/each}
          </span>
          <span class="text-neutral-400 @xl:pl-8">Committer</span>
          <span>{person(detail.committer)}</span>
        </div>
      </div>
      <pre
        class="mt-2 font-sans text-[13px] whitespace-pre-wrap {headerHeight === null
          ? 'max-h-40 overflow-auto'
          : ''}">{detail.message.trim()}</pre>
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <!-- A focusable separator is the WAI-ARIA window-splitter pattern; Svelte's a11y rules only know the static kind. -->
    <div
      class="h-1.5 shrink-0 cursor-row-resize touch-none bg-neutral-200 hover:bg-blue-400 focus:bg-blue-400 focus:outline-none dark:bg-neutral-700"
      onpointerdown={startHeaderResize}
      onkeydown={onHeaderKey}
      ondblclick={resetHeaderHeight}
      role="separator"
      aria-orientation="horizontal"
      aria-label="Resize commit message"
      aria-valuenow={headerHeight ?? header?.offsetHeight}
      aria-valuemin={headerBounds.min}
      aria-valuemax={headerBounds.max()}
      tabindex="0"
      title="Drag to resize, double-click to fit the message"
    ></div>
    <div class="flex min-h-0 flex-1" bind:this={panes}>
      <div class="shrink-0 overflow-auto py-1" style="width:{fileListWidth}px">
        {#each detail.files as file (file.path)}
          <button
            class="flex w-full items-center gap-2 px-2 py-0.5 text-left {selectedFile === file
              ? 'bg-blue-500/25'
              : 'hover:bg-neutral-500/10'}"
            onclick={() => (selectedFile = file)}
            oncontextmenu={(e) => detail && showLinkMenu(e, fileLinks(detail, file))}
            title={file.old_path ? `${file.old_path} → ${file.path}` : file.path}
          >
            <span class="w-3 shrink-0 font-mono text-xs {statusClass(file.status)}">{file.status}</span>
            <!-- Paths are told apart by their tail (the file name), so the ellipsis goes at the start. -->
            <span class="truncate-start min-w-0 flex-1"
              ><span
                >{#if file.old_path}<span class="text-neutral-400">{file.old_path} → </span>{/if}{file.path}</span
              ></span
            >
            {#if file.binary}
              <span class="shrink-0 text-[11px] text-neutral-400">bin</span>
            {:else}
              <span class="shrink-0 font-mono text-[11px] text-emerald-600 dark:text-emerald-400">+{file.additions}</span>
              <span class="shrink-0 font-mono text-[11px] text-red-500 dark:text-red-400">-{file.deletions}</span>
            {/if}
          </button>
        {:else}
          <div class="px-2 py-1 text-neutral-500">
            {detail.diff_error ? `Diff unavailable: ${detail.diff_error}` : "No file changes"}
          </div>
        {/each}
      </div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <!-- A focusable separator is the WAI-ARIA window-splitter pattern; Svelte's a11y rules only know the static kind. -->
      <div
        class="w-1.5 shrink-0 cursor-col-resize touch-none bg-neutral-200 hover:bg-blue-400 focus:bg-blue-400 focus:outline-none dark:bg-neutral-700"
        onpointerdown={startFileListResize}
        onkeydown={onFileListKey}
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize file list"
        aria-valuenow={fileListWidth}
        aria-valuemin={fileListBounds.min}
        aria-valuemax={fileListBounds.max()}
        tabindex="0"
      ></div>
      <div class="selectable min-w-0 flex-1 overflow-auto font-mono text-xs leading-5">
        {#if selectedFile && !selectedFile.submodule}
          {#key selectedFile}
            <FilePreview oid={detail.oid} file={selectedFile} {onjump} />
          {/key}
        {/if}
        {#if diffError}
          <div class="p-2 text-red-500">{diffError}</div>
        {:else if diff}
          {#if diff.binary && !(selectedFile && previewType(selectedFile.path))}
            <div class="p-2 text-neutral-500">Binary file</div>
          {/if}
          <table class="w-full border-collapse">
            <tbody>
              {#each diff.lines as line, i (i)}
                <tr
                  class={lineClass(line.kind)}
                  oncontextmenu={(e) => detail && selectedFile && showLinkMenu(e, fileLinks(detail, selectedFile, line))}
                >
                  <td class="w-12 pr-1 text-right text-neutral-400 select-none">{line.old_no ?? ""}</td>
                  <td class="w-12 pr-2 text-right text-neutral-400 select-none">{line.new_no ?? ""}</td>
                  <td class="px-2 whitespace-pre"
                    >{line.kind === "add" ? "+" : line.kind === "del" ? "-" : " "}{line.text}</td
                  >
                </tr>
              {/each}
            </tbody>
          </table>
          {#if diff.truncated}
            <div class="p-2 text-amber-600">Diff truncated (too many lines)</div>
          {/if}
        {:else if selectedFile}
          <div class="p-2 text-neutral-500">Loading…</div>
        {/if}
        <!-- Below the two "Subproject commit" lines: what those two hashes are, and what lies between them. -->
        {#if selectedFile?.submodule}
          {#key selectedFile}
            <SubmoduleDetail oid={detail.oid} file={selectedFile} />
          {/key}
        {/if}
      </div>
    </div>
  {/if}
</div>
