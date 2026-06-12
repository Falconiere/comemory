import { useCallback, useEffect, useMemo, useState } from "react";
import GraphCanvas from "./components/GraphCanvas";
import DetailPanel from "./components/DetailPanel";
import Legend from "./components/Legend";
import Toolbar from "./components/Toolbar";
import Editor from "./components/Editor";
import { getFile, getGraph, getHealth } from "./api";
import { repoColorMap } from "./colors";
import type { CodeGraph, FileView, Filters, Health } from "./types";

interface OpenFile {
  nodeId: string;
  view: FileView;
}

export default function App() {
  const [graph, setGraph] = useState<CodeGraph | null>(null);
  const [health, setHealth] = useState<Health | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filters, setFilters] = useState<Filters>({
    imports: true,
    co_changed: true,
    minWeight: 1,
  });
  const [search, setSearch] = useState("");
  const [searchTarget, setSearchTarget] = useState<string | null>(null);
  const [searchNonce, setSearchNonce] = useState(0);
  const [layoutNonce, setLayoutNonce] = useState(0);
  const [openFile, setOpenFile] = useState<OpenFile | null>(null);
  const [dark, setDark] = useState(() => {
    return localStorage.getItem("comemory-theme") !== "light";
  });

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    localStorage.setItem("comemory-theme", dark ? "dark" : "light");
  }, [dark]);

  useEffect(() => {
    getGraph().then(setGraph).catch((e) => setError(String(e)));
    getHealth().then(setHealth).catch(() => undefined);
  }, []);

  const repoColors = useMemo(
    () => (graph ? repoColorMap(graph.nodes) : new Map<string, string>()),
    [graph],
  );

  const selectedNode = useMemo(
    () => graph?.nodes.find((n) => n.id === selectedId) ?? null,
    [graph, selectedId],
  );

  const onSelect = useCallback((id: string | null) => setSelectedId(id), []);

  const onSearchSubmit = useCallback(() => {
    if (!graph || !search.trim()) return;
    const q = search.trim().toLowerCase();
    const hit = graph.nodes.find((n) => n.label.toLowerCase().includes(q));
    if (hit) {
      setSelectedId(hit.id);
      setSearchTarget(hit.id);
      setSearchNonce((n) => n + 1);
      setNotice(null);
    } else {
      setNotice(`no file matching "${search.trim()}"`);
    }
  }, [graph, search]);

  const viewSource = useCallback(async (id: string) => {
    try {
      const view = await getFile(id);
      setOpenFile({ nodeId: id, view });
      setNotice(null);
    } catch (e) {
      setNotice(String(e));
    }
  }, []);

  const reloadFile = useCallback(async () => {
    if (!openFile) return;
    try {
      const view = await getFile(openFile.nodeId);
      setOpenFile({ nodeId: openFile.nodeId, view });
    } catch (e) {
      setNotice(String(e));
    }
  }, [openFile]);

  return (
    <div className="flex h-screen flex-col bg-slate-950 text-slate-100">
      <Toolbar
        filters={filters}
        onFilters={setFilters}
        search={search}
        onSearch={setSearch}
        onSearchSubmit={onSearchSubmit}
        onRelayout={() => setLayoutNonce((n) => n + 1)}
        dark={dark}
        onToggleDark={() => setDark((d) => !d)}
      />
      {notice && (
        <div className="bg-amber-900/50 px-4 py-1 text-sm text-amber-200">{notice}</div>
      )}

      <div className="flex min-h-0 flex-1">
        <div className="relative min-w-0 flex-1 bg-slate-950">
          {error ? (
            <div className="flex h-full items-center justify-center p-6 text-center text-red-300">
              {error}
            </div>
          ) : graph ? (
            <GraphCanvas
              graph={graph}
              repoColors={repoColors}
              filters={filters}
              selectedId={selectedId}
              onSelect={onSelect}
              searchNonce={searchNonce}
              searchTarget={searchTarget}
              layoutNonce={layoutNonce}
            />
          ) : (
            <div className="flex h-full items-center justify-center text-slate-500">
              loading graph…
            </div>
          )}
          {graph && (
            <div className="pointer-events-none absolute bottom-3 left-3 rounded bg-slate-900/80 px-2 py-1 text-xs text-slate-400">
              {graph.nodes.length} files · {graph.edges.length} edges · node size = PageRank
            </div>
          )}
        </div>

        {openFile ? (
          <div className="w-1/2 border-l border-slate-700">
            <Editor
              nodeId={openFile.nodeId}
              view={openFile.view}
              serverReadOnly={health?.read_only ?? false}
              onClose={() => setOpenFile(null)}
              onReload={() => void reloadFile()}
            />
          </div>
        ) : (
          <aside className="w-80 overflow-auto border-l border-slate-700 bg-slate-900">
            <DetailPanel
              node={selectedNode}
              color={selectedNode ? repoColors.get(selectedNode.repo) : undefined}
              onViewSource={(id) => void viewSource(id)}
            />
            <div className="border-t border-slate-800" />
            <Legend repoColors={repoColors} />
          </aside>
        )}
      </div>
    </div>
  );
}
