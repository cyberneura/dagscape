<script lang="ts">
  import { api, type FileChange, type Signature, type SubmoduleChange, type SubmoduleCommit } from "./api";
  import { chipClass, lineClass, statusClass } from "./diffStyle";
  import { formatDate, formatDateTime } from "./format";

  interface Props {
    oid: string;
    file: FileChange;
  }
  let { oid, file }: Props = $props();

  // Rendering every line of a large update at once makes the pane sluggish, so past this many
  // lines the files start collapsed and are opened one by one.
  const AUTO_EXPAND_LINES = 1500;

  let change = $state<SubmoduleChange | null>(null);
  let error = $state<string | null>(null);
  let expanded = $state<boolean[]>([]);
  let openError = $state<string | null>(null);

  $effect(() => {
    const id = oid;
    const f = file;
    change = null;
    error = null;
    openError = null;
    let cancelled = false;
    api
      .getSubmoduleChange(id, f.path, f.old_path)
      .then((c) => {
        if (cancelled) return;
        const total = c.files.reduce((n, file) => n + file.lines.length, 0);
        expanded = c.files.map(() => total <= AUTO_EXPAND_LINES);
        change = c;
      })
      .catch((e) => {
        if (!cancelled) error = String(e);
      });
    return () => {
      cancelled = true;
    };
  });

  const sides = $derived(
    change
      ? [
          { label: "Before", oid: change.old_oid, commit: change.old },
          { label: "After", oid: change.new_oid, commit: change.new },
        ].filter((s): s is { label: string; oid: string; commit: SubmoduleCommit | null } => s.oid !== null)
      : [],
  );

  function person(sig: Signature): string {
    return `${sig.name} <${sig.email}>`;
  }

  function plural(n: number, word: string): string {
    return `${n} ${word}${n === 1 ? "" : "s"}`;
  }

  function movement(c: SubmoduleChange): string | null {
    if (c.ahead === null || c.behind === null) return null;
    if (c.ahead === 0 && c.behind === 0) return null;
    const parts = [];
    if (c.ahead > 0) parts.push(`${plural(c.ahead, "commit")} added`);
    // The old commit is not an ancestor of the new one: the pointer moved back or to another branch.
    if (c.behind > 0) parts.push(`${plural(c.behind, "commit")} left behind`);
    return parts.join(", ");
  }

  async function openInNewWindow() {
    openError = null;
    try {
      await api.openSubmoduleWindow(oid, file.path, file.old_path);
    } catch (e) {
      openError = String(e);
    }
  }
</script>

<div class="border-t border-neutral-200 px-3 py-2 font-sans text-xs dark:border-neutral-700">
  {#if error}
    <div class="text-red-500">{error}</div>
  {:else if !change}
    <div class="text-neutral-500">Loading submodule…</div>
  {:else}
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
      <span class="font-semibold text-neutral-700 dark:text-neutral-200">Submodule</span>
      {#if change.repo_path}
        <span class="min-w-0 break-all text-neutral-500">{change.repo_path}</span>
        <button
          class="btn shrink-0 cursor-pointer select-none"
          onclick={openInNewWindow}
          title="Open this submodule's repository in a new Dagscape window">Open in New Window</button
        >
      {:else}
        <span class="text-neutral-500"
          >Not checked out here, so only the hashes are known. Run
          <code class="font-mono">git submodule update --init</code> to see the commits and the diff.</span
        >
      {/if}
    </div>
    {#if openError}
      <div class="mt-1 text-red-500">{openError}</div>
    {/if}

    {#if change.repo_path}
      <div class="@container mt-2">
        <div class="grid grid-cols-1 gap-3 @4xl:grid-cols-2">
          {#each sides as side (side.label)}
            <section class="min-w-0 rounded border border-neutral-200 p-2 dark:border-neutral-700">
              <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span class="text-neutral-400">{side.label}</span>
                <span class="font-mono text-neutral-600 dark:text-neutral-300">{side.oid.slice(0, 7)}</span>
                {#each side.commit?.refs ?? [] as ref (ref.kind + ref.name)}
                  <span class="chip {chipClass(ref.kind)}">{ref.name}</span>
                {/each}
              </div>
              {#if side.commit}
                {@const c = side.commit}
                <div class="mt-1 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-neutral-600 dark:text-neutral-300">
                  <span class="whitespace-nowrap text-neutral-400">Committed at</span>
                  <span class="whitespace-nowrap">{formatDateTime(c.committer.time)}</span>
                  <span class="text-neutral-400">Author</span>
                  <span>
                    {person(c.author)}
                    {#if c.author.time !== c.committer.time}
                      <span class="whitespace-nowrap text-neutral-400">· authored {formatDateTime(c.author.time)}</span>
                    {/if}
                  </span>
                  <span class="text-neutral-400">Committer</span>
                  <span>{person(c.committer)}</span>
                  {#if c.contained_in.length > 0}
                    <span class="text-neutral-400">Branches</span>
                    <span class="break-words"
                      >{c.contained_in.join(", ")}{#if c.contained_in_truncated}, …{/if}</span
                    >
                  {/if}
                </div>
                <pre class="mt-1.5 max-h-32 overflow-auto font-sans whitespace-pre-wrap">{c.message.trim()}</pre>
              {:else}
                <div class="mt-1 text-neutral-500">
                  This commit is not in the local submodule repository. Fetch the submodule to see its details.
                </div>
              {/if}
            </section>
          {/each}
        </div>
      </div>

      {#if movement(change)}
        <div class="mt-3 font-semibold text-neutral-700 dark:text-neutral-200">{movement(change)}</div>
      {/if}
      {#if change.commits.length > 0}
        <table class="mt-1 w-full border-collapse">
          <tbody>
            {#each change.commits as commit (commit.oid)}
              <tr>
                <td class="w-16 pr-3 font-mono whitespace-nowrap text-neutral-500">{commit.short}</td>
                <td class="pr-3">{commit.summary}</td>
                <td class="w-px pr-3 whitespace-nowrap text-neutral-500">{commit.author}</td>
                <td class="w-px whitespace-nowrap text-neutral-500">{formatDate(commit.time)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if change.commits_truncated}
          <div class="mt-1 text-amber-600">Only the newest {change.commits.length} commits are listed</div>
        {/if}
      {/if}

      {#if change.diff_error}
        <div class="mt-3 text-red-500">Diff unavailable: {change.diff_error}</div>
      {/if}
      {#if change.files.length > 0}
        <div class="mt-3 font-semibold text-neutral-700 dark:text-neutral-200">
          {plural(change.files.length, "file")} changed in the submodule
        </div>
        <!-- Keyed by position: a type change (file → symlink) is a deletion plus an addition of the same path. -->
        {#each change.files as f, i (i)}
          <div class="mt-1 rounded border border-neutral-200 dark:border-neutral-700">
            <button
              class="flex w-full cursor-pointer items-center gap-2 px-2 py-0.5 text-left select-none hover:bg-neutral-500/10"
              onclick={() => (expanded[i] = !expanded[i])}
              aria-expanded={expanded[i]}
            >
              <span class="w-3 shrink-0 text-neutral-400">{expanded[i] ? "▾" : "▸"}</span>
              <span class="w-3 shrink-0 font-mono {statusClass(f.status)}">{f.status}</span>
              <span class="min-w-0 flex-1 break-all"
                >{#if f.old_path}<span class="text-neutral-400">{f.old_path} → </span>{/if}{f.path}</span
              >
              {#if f.binary}
                <span class="shrink-0 text-[11px] text-neutral-400">bin</span>
              {:else}
                <span class="shrink-0 font-mono text-[11px] text-emerald-600 dark:text-emerald-400">+{f.additions}</span>
                <span class="shrink-0 font-mono text-[11px] text-red-500 dark:text-red-400">-{f.deletions}</span>
              {/if}
            </button>
            {#if expanded[i] && f.lines.length > 0}
              <div class="overflow-x-auto border-t border-neutral-200 font-mono leading-5 dark:border-neutral-700">
                <table class="w-full border-collapse">
                  <tbody>
                    {#each f.lines as line, j (j)}
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
              </div>
            {/if}
          </div>
        {/each}
        {#if change.diff_truncated}
          <div class="mt-1 text-amber-600">Diff truncated (too many files or lines)</div>
        {/if}
      {/if}
    {/if}
  {/if}
</div>
