import { useEffect, useRef } from "react";
import Graph from "graphology";
import Sigma from "sigma";
import forceAtlas2 from "graphology-layout-forceatlas2";
import type { CodeGraph, Filters } from "../types";
import { EDGE_COLORS } from "../colors";

interface Props {
  graph: CodeGraph;
  repoColors: Map<string, string>;
  filters: Filters;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  /** Bumped to re-center the camera on `searchTarget`. */
  searchNonce: number;
  searchTarget: string | null;
  /** Bumped to re-run the ForceAtlas2 layout. */
  layoutNonce: number;
}

const DIM_NODE = "#3a4250";

export default function GraphCanvas({
  graph,
  repoColors,
  filters,
  selectedId,
  onSelect,
  searchNonce,
  searchTarget,
  layoutNonce,
}: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const rendererRef = useRef<Sigma | null>(null);
  const graphRef = useRef<Graph | null>(null);
  const filtersRef = useRef(filters);
  const selectedRef = useRef<string | null>(selectedId);
  const hoodRef = useRef<Set<string> | null>(null);
  const rafRef = useRef<number | null>(null);

  // Build the graph + renderer whenever the data or coloring changes.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const g = new Graph({ type: "directed", multi: true });
    const maxRank = graph.nodes.reduce((m, n) => Math.max(m, n.rank), 0) || 1;
    const maxWeight = graph.edges.reduce((m, e) => Math.max(m, e.weight), 1);
    const n = graph.nodes.length || 1;

    graph.nodes.forEach((node, i) => {
      const a = (2 * Math.PI * i) / n;
      g.addNode(node.id, {
        label: node.label,
        repo: node.repo,
        x: Math.cos(a),
        y: Math.sin(a),
        size: 3 + 17 * (node.rank / maxRank),
        color: repoColors.get(node.repo) ?? "#5b8def",
      });
    });

    graph.edges.forEach((e, i) => {
      if (!g.hasNode(e.src) || !g.hasNode(e.dst)) return;
      const isImport = e.rel === "imports";
      const size = isImport
        ? 1
        : 1 + 5 * ((e.weight - 1) / (maxWeight - 1 || 1));
      try {
        g.addDirectedEdgeWithKey("e" + i, e.src, e.dst, {
          rel: e.rel,
          weight: e.weight,
          type: isImport ? "arrow" : "line",
          color: EDGE_COLORS[e.rel] ?? "#888888",
          size,
        });
      } catch {
        // Duplicate key / self-loop — adds no signal, skip.
      }
    });

    const renderer = new Sigma(g, container, {
      defaultEdgeType: "line",
      renderEdgeLabels: false,
      labelRenderedSizeThreshold: 6,
      nodeReducer: (node, data) => {
        const res = { ...data };
        const hood = hoodRef.current;
        if (selectedRef.current && hood && !hood.has(node)) {
          res.color = DIM_NODE;
          res.label = "";
        }
        return res;
      },
      edgeReducer: (edge, data) => {
        const res = { ...data };
        const f = filtersRef.current;
        const rel = data.rel as string;
        if (!f[rel as "imports" | "co_changed"]) {
          res.hidden = true;
          return res;
        }
        if (rel === "co_changed" && (data.weight as number) < f.minWeight) {
          res.hidden = true;
          return res;
        }
        const sel = selectedRef.current;
        if (sel) {
          const ext = g.extremities(edge);
          if (ext[0] !== sel && ext[1] !== sel) res.hidden = true;
        }
        return res;
      },
    });

    renderer.on("clickNode", ({ node }) => onSelect(node));
    renderer.on("clickStage", () => onSelect(null));

    rendererRef.current = renderer;
    graphRef.current = g;
    runLayout(g, rafRef);

    return () => {
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
      renderer.kill();
      rendererRef.current = null;
      graphRef.current = null;
    };
  }, [graph, repoColors, onSelect]);

  // Mutable interaction state: refresh without rebuilding the renderer.
  useEffect(() => {
    filtersRef.current = filters;
    rendererRef.current?.refresh();
  }, [filters]);

  useEffect(() => {
    selectedRef.current = selectedId;
    const g = graphRef.current;
    if (selectedId && g && g.hasNode(selectedId)) {
      const hood = new Set<string>(g.neighbors(selectedId));
      hood.add(selectedId);
      hoodRef.current = hood;
    } else {
      hoodRef.current = null;
    }
    rendererRef.current?.refresh();
  }, [selectedId]);

  // Center the camera on a search hit.
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer || !searchTarget) return;
    const g = graphRef.current;
    if (!g || !g.hasNode(searchTarget)) return;
    const pos = renderer.getNodeDisplayData(searchTarget);
    if (pos) {
      renderer.getCamera().animate(
        { x: pos.x, y: pos.y, ratio: 0.35 },
        { duration: 500 },
      );
    }
  }, [searchNonce, searchTarget]);

  // Re-run the layout on demand.
  useEffect(() => {
    if (layoutNonce === 0) return;
    const g = graphRef.current;
    if (g) runLayout(g, rafRef);
  }, [layoutNonce]);

  return <div ref={containerRef} className="h-full w-full" />;
}

/** Animate ForceAtlas2 over a fixed frame budget on requestAnimationFrame. */
function runLayout(g: Graph, rafRef: { current: number | null }) {
  if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
  const settings = forceAtlas2.inferSettings(g);
  let frames = 0;
  const MAX_FRAMES = 600;
  const step = () => {
    forceAtlas2.assign(g, { iterations: 1, settings });
    frames += 1;
    if (frames < MAX_FRAMES) {
      rafRef.current = requestAnimationFrame(step);
    } else {
      rafRef.current = null;
    }
  };
  rafRef.current = requestAnimationFrame(step);
}
