<script lang="ts">
  import {
    api,
    type CommitDetail,
    type DiffLine,
    type FileChange,
    type FileDiff,
    type RefLabel,
  } from "./api";
  import { formatDateTime } from "./format";

  interface Props {
    oid: string | null;
    refs: RefLabel[];
    onjump: (oid: string) => void;
  }
  let { oid, refs, onjump }: Props = $props();

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

  const committerDiffers = $derived(
    detail !== null &&
      (detail.author.email !== detail.committer.email ||
        detail.author.time !== detail.committer.time),
  );

  function statusClass(status: string): string {
    switch (status) {
      case "A":
        return "text-emerald-600 dark:text-emerald-400";
      case "D":
        return "text-red-600 dark:text-red-400";
      case "R":
      case "C":
        return "text-sky-600 dark:text-sky-400";
      default:
        return "text-amber-600 dark:text-amber-400";
    }
  }

  function lineClass(kind: DiffLine["kind"]): string {
    switch (kind) {
      case "add":
        return "bg-emerald-500/15 text-emerald-800 dark:text-emerald-200";
      case "del":
        return "bg-red-500/15 text-red-800 dark:text-red-200";
      case "hunk":
        return "bg-sky-500/10 text-sky-700 dark:text-sky-300";
      default:
        return "";
    }
  }

  function chipClass(ref: RefLabel): string {
    switch (ref.kind) {
      case "head":
        return "bg-neutral-800 text-white dark:bg-neutral-200 dark:text-black";
      case "branch":
        return "bg-emerald-600/85 text-white";
      case "remote":
        return "bg-sky-600/85 text-white";
      case "tag":
        return "bg-amber-400 text-black";
    }
  }

  function signatureText(sig: CommitDetail["author"]): string {
    return `${sig.name} <${sig.email}> · ${formatDateTime(sig.time)}`;
  }
</script>

<div class="flex h-full flex-col text-[13px]">
  {#if !oid}
    <div class="p-4 text-neutral-500">Select a commit to see its details.</div>
  {:else if error}
    <div class="p-4 text-red-500">{error}</div>
  {:else if !detail}
    <div class="p-4 text-neutral-500">Loading…</div>
  {:else}
    <div
      class="selectable shrink-0 border-b border-neutral-200 px-3 py-2 dark:border-neutral-700"
    >
      <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span class="font-mono text-xs text-neutral-500 dark:text-neutral-400">{detail.oid}</span>
        {#each refs as ref (ref.kind + ref.name)}
          <span class="chip {chipClass(ref)}">{ref.name}</span>
        {/each}
      </div>
      <div
        class="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs text-neutral-600 dark:text-neutral-300"
      >
        <span class="text-neutral-400">Author</span>
        <span>{signatureText(detail.author)}</span>
        {#if committerDiffers}
          <span class="text-neutral-400">Committer</span>
          <span>{signatureText(detail.committer)}</span>
        {/if}
        <span class="text-neutral-400">Parents</span>
        <span>
          {#each detail.parents as parent (parent)}
            <button
              class="mr-2 cursor-pointer font-mono text-sky-600 hover:underline dark:text-sky-400"
              onclick={() => onjump(parent)}>{parent.slice(0, 7)}</button
            >
          {:else}
            <span class="text-neutral-400">(root commit)</span>
          {/each}
        </span>
      </div>
      <pre
        class="mt-2 max-h-40 overflow-auto font-sans text-[13px] whitespace-pre-wrap">{detail.message.trim()}</pre>
    </div>
    <div class="flex min-h-0 flex-1">
      <div
        class="w-80 shrink-0 overflow-auto border-r border-neutral-200 py-1 dark:border-neutral-700"
      >
        {#each detail.files as file (file.path)}
          <button
            class="flex w-full items-center gap-2 px-2 py-0.5 text-left {selectedFile === file
              ? 'bg-blue-500/25'
              : 'hover:bg-neutral-500/10'}"
            onclick={() => (selectedFile = file)}
            title={file.old_path ? `${file.old_path} → ${file.path}` : file.path}
          >
            <span class="w-3 shrink-0 font-mono text-xs {statusClass(file.status)}">{file.status}</span>
            <span class="min-w-0 flex-1 truncate">
              {#if file.old_path}<span class="text-neutral-400">{file.old_path} → </span>{/if}{file.path}
            </span>
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
      <div class="selectable min-w-0 flex-1 overflow-auto font-mono text-xs leading-5">
        {#if diffError}
          <div class="p-2 text-red-500">{diffError}</div>
        {:else if diff}
          {#if diff.binary}
            <div class="p-2 text-neutral-500">Binary file</div>
          {/if}
          <table class="w-full border-collapse">
            <tbody>
              {#each diff.lines as line, i (i)}
                <tr class={lineClass(line.kind)}>
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
      </div>
    </div>
  {/if}
</div>
