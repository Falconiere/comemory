import type { CodeGraph, Filters } from "../../types";

/**
 * A force-graph link: `source`/`target` are node ids that
 * `react-force-graph-3d` resolves to live node objects after the first tick.
 */
export interface GLink {
  source: string;
  target: string;
  rel: string;
  weight: number;
}

/**
 * Map every {@link CodeGraph} edge to a {@link GLink}, dropping edges whose
 * relation toggle is off and `co_changed` edges below `minWeight`. Mirrors the
 * old Sigma `edgeReducer` hide rules so the visible-edge set stays identical.
 */
export function buildLinks(graph: CodeGraph, filters: Filters): GLink[] {
  const links: GLink[] = [];
  for (const e of graph.edges) {
    if (e.rel === "imports" && !filters.imports) continue;
    if (e.rel === "co_changed") {
      if (!filters.co_changed) continue;
      if (e.weight < filters.minWeight) continue;
    }
    links.push({ source: e.src, target: e.dst, rel: e.rel, weight: e.weight });
  }
  return links;
}

/** Largest node `rank` in the graph (≥ 1) for size normalization. */
export function maxRank(graph: CodeGraph): number {
  return graph.nodes.reduce((m, n) => Math.max(m, n.rank), 0) || 1;
}

/**
 * Resolve a link endpoint to its node id. Before the first layout tick
 * `react-force-graph-3d` keeps `source`/`target` as id strings; afterwards it
 * replaces them with live node objects. Accept either shape.
 */
export function endpointId(end: string | { id?: string }): string {
  return typeof end === "string" ? end : (end.id ?? "");
}
