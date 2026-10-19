import { Menu } from "@tauri-apps/api/menu";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { RefKind, RemoteInfo, Tracking } from "./api";

export interface LinkTarget {
  name: string;
  kind: RefKind;
  tracking: Tracking | null;
}

export class GitHub {
  private readonly webByRemote: Map<string, string>;
  private readonly defaultWeb: string | null;

  constructor(remotes: RemoteInfo[]) {
    this.webByRemote = new Map(
      remotes.filter((r) => r.web_url).map((r) => [r.name, r.web_url as string]),
    );
    this.defaultWeb = this.webByRemote.get("origin") ?? this.webByRemote.values().next().value ?? null;
  }

  get available(): boolean {
    return this.defaultWeb !== null;
  }

  refUrl(ref: LinkTarget, oid: string): string | null {
    if (ref.tracking) {
      const web = this.webByRemote.get(ref.tracking.remote);
      return web ? `${web}/tree/${encodePath(ref.tracking.branch)}` : null;
    }
    if (!this.defaultWeb) return null;
    switch (ref.kind) {
      case "tag":
        return `${this.defaultWeb}/releases/tag/${encodePath(ref.name)}`;
      case "head":
        return this.commitUrl(oid);
      case "branch":
      case "remote":
        return null;
    }
  }

  commitUrl(oid: string): string | null {
    return this.defaultWeb && `${this.defaultWeb}/commit/${oid}`;
  }

  fileUrl(oid: string, path: string, line?: number | null): string | null {
    if (!this.defaultWeb) return null;
    return `${this.defaultWeb}/blob/${oid}/${encodePath(path)}${line ? `#L${line}` : ""}`;
  }

  // GitHub はコミット画面の各ファイルに diff-<パスの SHA-256> のアンカーを付けている
  async diffUrl(oid: string, path: string, line?: { side: "L" | "R"; no: number }): Promise<string | null> {
    const base = this.commitUrl(oid);
    if (!base) return null;
    const hash = await sha256Hex(path);
    if (!hash) return base;
    return `${base}#diff-${hash}${line ? `${line.side}${line.no}` : ""}`;
  }
}

function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/");
}

async function sha256Hex(text: string): Promise<string | null> {
  // crypto.subtle は secure context でしか使えない
  if (!globalThis.crypto?.subtle) return null;
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export interface LinkItem {
  text: string;
  url: string | null | Promise<string | null>;
}

// popup() は表示した時点で resolve するので、選ばれる前に close() すると action が届かない。
// 次に開く時に前のメニューを閉じて、Rust 側に残るのを 1 つまでにする
let lastMenu: Menu | null = null;

// URL が 1 つも無ければ webview 既定のメニューに任せる。
// 既定のメニューを置き換えると Copy が消えるので、文字列を選択中なら足す
export async function showLinkMenu(e: MouseEvent, items: LinkItem[]): Promise<void> {
  const links = items.filter((i) => i.url !== null);
  if (links.length === 0) return;
  e.preventDefault();
  e.stopPropagation();
  const hasSelection = (window.getSelection()?.toString() ?? "") !== "";
  const menu = await Menu.new({
    items: [
      ...(hasSelection ? [{ item: "Copy" as const }, { item: "Separator" as const }] : []),
      ...links.map((link) => ({
        text: link.text,
        action: async () => {
          const url = await link.url;
          if (url) await openUrl(url);
        },
      })),
    ],
  });
  const previous = lastMenu;
  lastMenu = menu;
  await previous?.close();
  await menu.popup();
}
