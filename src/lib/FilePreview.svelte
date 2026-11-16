<script lang="ts">
  import { api, type BlobSideName, type FileChange, type FileInfo } from "./api";
  import { formatBytes, previewType } from "./filePreview";
  import { formatDate } from "./format";

  interface Props {
    oid: string;
    file: FileChange;
    onjump: (oid: string) => void;
  }
  let { oid, file, onjump }: Props = $props();

  interface Preview {
    side: BlobSideName;
    url: string | null;
    error: string | null;
    /** Natural size of an image or a video, once it has loaded. */
    dims: string | null;
  }

  let info = $state<FileInfo | null>(null);
  let infoError = $state<string | null>(null);
  let previews = $state<Preview[]>([]);

  const type = $derived(previewType(file.path));

  $effect(() => {
    const id = oid;
    const f = file;
    info = null;
    infoError = null;
    let cancelled = false;
    api
      .getFileInfo(id, f.path, f.old_path)
      .then((i) => {
        if (!cancelled) info = i;
      })
      .catch((e) => {
        if (!cancelled) infoError = String(e);
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const id = oid;
    const f = file;
    const t = type;
    const urls: string[] = [];
    let cancelled = false;
    // Before / after side by side only for images; one player or document is enough for the rest.
    const sides: BlobSideName[] =
      f.status === "A" ? ["new"] : f.status === "D" ? ["old"] : t?.kind === "image" ? ["old", "new"] : ["new"];
    previews = t ? sides.map((side) => ({ side, url: null, error: null, dims: null })) : [];
    if (t) {
      sides.forEach((side, i) => {
        api
          .getBlob(id, f.path, f.old_path, side)
          .then((buf) => {
            if (cancelled) return;
            const url = URL.createObjectURL(new Blob([buf], { type: t.mime }));
            urls.push(url);
            previews[i].url = url;
          })
          .catch((e) => {
            if (!cancelled) previews[i].error = String(e);
          });
      });
    }
    return () => {
      cancelled = true;
      for (const url of urls) URL.revokeObjectURL(url);
    };
  });

  function sizeText(i: FileInfo): string {
    if (i.old && i.new && i.old.oid !== i.new.oid) return `${formatBytes(i.old.size)} → ${formatBytes(i.new.size)}`;
    const side = i.new ?? i.old;
    return side ? formatBytes(side.size) : "—";
  }

  function sideLabel(side: BlobSideName): string {
    return side === "old" ? "Before" : "After";
  }
</script>

<div class="border-b border-neutral-200 px-3 py-2 font-sans text-xs dark:border-neutral-700">
  {#if infoError}
    <div class="text-red-500">{infoError}</div>
  {:else if !info}
    <div class="text-neutral-500">Loading file info…</div>
  {:else}
    <div class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-neutral-600 dark:text-neutral-300">
      <span class="text-neutral-400">Size</span>
      <span>{sizeText(info)}</span>
      <span class="text-neutral-400">Added</span>
      <span>
        {#if info.added}
          {formatDate(info.added.time)}
          {#if info.added.oid === info.modified.oid}
            <span class="text-neutral-400">(this commit)</span>
          {:else}
            <button
              class="cursor-pointer font-mono text-sky-600 hover:underline dark:text-sky-400"
              onclick={() => info?.added && onjump(info.added.oid)}>{info.added.short}</button
            >
          {/if}
        {:else}
          <span class="text-neutral-400">unknown (history too long or incomplete)</span>
        {/if}
      </span>
      <span class="text-neutral-400">{file.status === "D" ? "Deleted" : "Modified"}</span>
      <span>{formatDate(info.modified.time)} <span class="text-neutral-400">(this commit)</span></span>
    </div>
  {/if}

  {#if type && previews.length > 0}
    <div class="mt-2 flex flex-wrap items-start gap-3">
      {#each previews as preview (preview.side)}
        <figure class="min-w-0 {type.kind === 'pdf' ? 'w-full' : ''}">
          {#if previews.length > 1}
            <figcaption class="mb-1 text-neutral-400">{sideLabel(preview.side)}</figcaption>
          {/if}
          {#if preview.error}
            <div class="text-red-500">{preview.error}</div>
          {:else if !preview.url}
            <div class="text-neutral-500">Loading preview…</div>
          {:else if type.kind === "image"}
            <img
              onerror={() => (preview.error = "This image cannot be displayed")}
              class="checkerboard max-h-80 max-w-full object-contain"
              src={preview.url}
              alt={file.path}
              onload={(e) => {
                const img = e.currentTarget as HTMLImageElement;
                preview.dims = `${img.naturalWidth} × ${img.naturalHeight}`;
              }}
            />
          {:else if type.kind === "video"}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video
              class="max-h-80 max-w-full"
              src={preview.url}
              controls
              preload="metadata"
              onerror={() => (preview.error = "This video cannot be played here")}
              onloadedmetadata={(e) => {
                const v = e.currentTarget as HTMLVideoElement;
                preview.dims = `${v.videoWidth} × ${v.videoHeight}`;
              }}
            ></video>
          {:else if type.kind === "audio"}
            <audio
              src={preview.url}
              controls
              preload="metadata"
              onerror={() => (preview.error = "This audio cannot be played here")}
            ></audio>
          {:else if type.kind === "pdf"}
            <!-- The Blob is typed application/pdf, so the webview opens its PDF viewer and never parses it as HTML. -->
            <iframe class="h-[32rem] w-full bg-white" src={preview.url} title={file.path}></iframe>
          {/if}
          {#if preview.dims}
            <div class="mt-1 text-neutral-400">{preview.dims}</div>
          {/if}
        </figure>
      {/each}
    </div>
  {/if}
</div>
