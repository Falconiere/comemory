import type { CodeGraph, FileView, Health, SaveResult, SearchResult } from "./types";

/** The per-session token injected into the page `<meta>` by the server. */
function token(): string {
  const meta = document.querySelector('meta[name="comemory-token"]');
  return meta?.getAttribute("content") ?? "";
}

function authHeaders(extra?: Record<string, string>): Record<string, string> {
  return { "X-Comemory-Token": token(), ...(extra ?? {}) };
}

async function ok(res: Response): Promise<Response> {
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`${res.status} ${res.statusText}: ${body}`);
  }
  return res;
}

export async function getHealth(): Promise<Health> {
  const res = await ok(await fetch("/api/health", { headers: authHeaders() }));
  return res.json();
}

export async function getGraph(): Promise<CodeGraph> {
  const res = await ok(await fetch("/api/graph", { headers: authHeaders() }));
  return res.json();
}

export async function searchFiles(q: string, k?: number): Promise<SearchResult> {
  const params = new URLSearchParams({ q });
  if (k != null) params.set("k", String(k));
  const url = `/api/search?${params.toString()}`;
  const res = await ok(await fetch(url, { headers: authHeaders() }));
  return res.json();
}

export async function getFile(id: string): Promise<FileView> {
  const url = `/api/file?id=${encodeURIComponent(id)}`;
  const res = await ok(await fetch(url, { headers: authHeaders() }));
  return res.json();
}

export async function putFile(
  id: string,
  contents: string,
  ifMatch: string,
): Promise<SaveResult> {
  const url = `/api/file?id=${encodeURIComponent(id)}`;
  const res = await fetch(url, {
    method: "PUT",
    headers: authHeaders({
      "Content-Type": "text/plain; charset=utf-8",
      "If-Match": ifMatch,
    }),
    body: contents,
  });
  if (res.status === 409) {
    const body = await res.json();
    return { conflict: true, current_oid: body.current_oid };
  }
  await ok(res);
  const body = await res.json();
  return { conflict: false, blob_oid: body.blob_oid };
}
