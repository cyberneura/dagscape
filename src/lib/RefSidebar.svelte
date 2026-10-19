<script lang="ts">
  import type { RefInfo } from "./api";
  import { showLinkMenu, type GitHub } from "./github";

  interface Props {
    refs: RefInfo[];
    activeOid: string | null;
    github: GitHub;
    onjump: (oid: string) => void;
  }
  let { refs, activeOid, github, onjump }: Props = $props();

  const branches = $derived(refs.filter((r) => r.kind === "branch"));
  const tags = $derived(
    refs
      .filter((r) => r.kind === "tag")
      .slice()
      .sort((a, b) => b.name.localeCompare(a.name, undefined, { numeric: true })),
  );
  const remotes = $derived.by(() => {
    const groups = new Map<string, RefInfo[]>();
    for (const r of refs) {
      if (r.kind !== "remote") continue;
      const slash = r.name.indexOf("/");
      const remote = slash < 0 ? r.name : r.name.slice(0, slash);
      const name = slash < 0 ? r.name : r.name.slice(slash + 1);
      const list = groups.get(remote) ?? [];
      list.push({ ...r, name });
      groups.set(remote, list);
    }
    return [...groups.entries()];
  });
</script>

{#snippet group(title: string, items: RefInfo[])}
  <details open>
    <summary
      class="sticky top-0 bg-neutral-100 px-2 py-1 text-[11px] font-semibold tracking-wide text-neutral-500 uppercase dark:bg-neutral-800"
    >
      {title} <span class="font-normal">({items.length})</span>
    </summary>
    {#each items as r (r.full_name)}
      <button
        class="flex w-full items-center px-2 py-0.5 text-left {r.oid === activeOid
          ? 'bg-blue-500/25'
          : 'hover:bg-neutral-500/10'} {r.is_head ? 'font-bold' : ''}"
        title={r.full_name}
        onclick={() => onjump(r.oid)}
        oncontextmenu={(e) => showLinkMenu(e, [{ text: "Open on GitHub", url: github.refUrl(r, r.oid) }])}
      >
        <span class="truncate">{r.name}</span>
      </button>
    {/each}
  </details>
{/snippet}

<div class="h-full overflow-auto text-[12px]">
  {@render group("Branches", branches)}
  {#each remotes as [name, items] (name)}
    {@render group(name, items)}
  {/each}
  {@render group("Tags", tags)}
</div>
