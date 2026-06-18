import { useEffect, useMemo, useReducer, useRef } from "react";
import ForceGraph3D from "react-force-graph-3d";
import type { Object3D } from "three";
import type { CodeGraph, Filters, GNode } from "../types";
import { EDGE_COLORS } from "../colors";
import { buildLinks, endpointId, maxRank, type GLink } from "./graph/links";
import { makeNodeThreeObject, neighborIds, topRankIds } from "./graph/labels";
import { flyTo, type FgHandle } from "./graph/camera";

/** Accessor type `react-force-graph-3d` expects for `nodeThreeObject`. */
type NodeObjectAccessor = (node: GNode) => Object3D;

interface Props {
  graph: CodeGraph;
  repoColors: Map<string, string>;
  filters: Filters;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  /** Bumped to fly the camera to `searchTarget`. */
  searchNonce: number;
  searchTarget: string | null;
  /** Bumped to reheat the force simulation. */
  layoutNonce: number;
}

/** Background of the 3D scene (matches the surrounding `bg-slate-950`). */
const BACKGROUND = "#020617";
/** Color for links not incident to the current selection. */
const DIM_LINK = "rgba(90,100,120,0.12)";
/** Extra value range mapped onto the highest-rank node sphere. */
const SIZE_SPREAD = 8;

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
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const fgRef = useRef<FgHandle | undefined>(undefined);
  const size = useElementSize(wrapRef);

  const links = useMemo(() => buildLinks(graph, filters), [graph, filters]);
  const data = useMemo(
    () => ({ nodes: graph.nodes, links }),
    [graph.nodes, links],
  );

  const topN = useMemo(() => topRankIds(graph), [graph]);
  const hood = useMemo(
    () => neighborIds(graph, selectedId),
    [graph, selectedId],
  );
  const rankCeil = useMemo(() => maxRank(graph), [graph]);

  // The persistent-label accessor; `false` for ungated nodes. Cast once to the
  // library's `Object3D`-only accessor type (it treats a falsy return as "no
  // custom object", a contract its types don't express).
  const nodeThreeObject = useMemo(
    () => makeNodeThreeObject(topN, hood) as unknown as NodeObjectAccessor,
    [topN, hood],
  );

  // Recompute persistent labels whenever the gating sets change.
  useEffect(() => {
    fgRef.current?.refresh();
  }, [nodeThreeObject]);

  // Fly the camera to a search hit.
  useEffect(() => {
    const fg = fgRef.current;
    if (fg && searchTarget) flyTo(fg, graph.nodes, searchTarget);
  }, [searchNonce, searchTarget, graph.nodes]);

  // Reheat the force simulation on demand.
  useEffect(() => {
    if (layoutNonce !== 0) fgRef.current?.d3ReheatSimulation();
  }, [layoutNonce]);

  return (
    <div ref={wrapRef} className="h-full w-full">
      <ForceGraph3D<GNode, GLink>
        ref={fgRef}
        width={size.width}
        height={size.height}
        graphData={data}
        backgroundColor={BACKGROUND}
        nodeId="id"
        nodeLabel={(n) => n.label}
        nodeVal={(n) => 1 + (n.rank / rankCeil) * SIZE_SPREAD}
        nodeColor={(n) =>
          selectedId && !hood.has(n.id)
            ? "#334155"
            : (repoColors.get(n.repo) ?? "#5b8def")
        }
        nodeThreeObjectExtend
        nodeThreeObject={nodeThreeObject}
        linkColor={(l) =>
          selectedId &&
          endpointId(l.source) !== selectedId &&
          endpointId(l.target) !== selectedId
            ? DIM_LINK
            : (EDGE_COLORS[l.rel] ?? "#888888")
        }
        linkWidth={(l) => (l.rel === "co_changed" ? Math.min(l.weight, 6) : 1)}
        linkDirectionalArrowLength={3}
        linkDirectionalArrowRelPos={1}
        onNodeClick={(n) => onSelect(n.id)}
        onBackgroundClick={() => onSelect(null)}
      />
    </div>
  );
}

interface Size {
  width: number;
  height: number;
}

/**
 * Track an element's content-box size via `ResizeObserver` so the canvas fills
 * its container instead of the window. The observer is torn down on unmount.
 */
function useElementSize(ref: React.RefObject<HTMLElement | null>): Size {
  const sizeRef = useRef<Size>({ width: 0, height: 0 });
  const [, render] = useReducer((c: number) => c + 1, 0);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const apply = (width: number, height: number) => {
      if (width === sizeRef.current.width && height === sizeRef.current.height)
        return;
      sizeRef.current = { width, height };
      render();
    };
    const ro = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect;
      if (box) apply(box.width, box.height);
    });
    ro.observe(el);
    apply(el.clientWidth, el.clientHeight);
    return () => ro.disconnect();
  }, [ref]);

  return sizeRef.current;
}
