import SpriteText from "three-spritetext";
import type { Object3D } from "three";
import type { CodeGraph, GNode } from "../../types";

/** Number of highest-rank nodes that always show a persistent text label. */
export const TOP_N_LABELS = 30;

/**
 * Ids of the `TOP_N_LABELS` highest-rank nodes. Ties break on id so the set is
 * deterministic across renders.
 */
export function topRankIds(graph: CodeGraph, n = TOP_N_LABELS): Set<string> {
  const sorted = [...graph.nodes].sort(
    (a, b) => b.rank - a.rank || a.id.localeCompare(b.id),
  );
  return new Set(sorted.slice(0, n).map((node) => node.id));
}

/**
 * The selected node plus its direct neighbors (both edge directions). Empty
 * when nothing is selected. Filtered links are ignored on purpose — selection
 * highlights structural adjacency, not the currently-visible edge subset.
 */
export function neighborIds(
  graph: CodeGraph,
  selectedId: string | null,
): Set<string> {
  const hood = new Set<string>();
  if (!selectedId) return hood;
  hood.add(selectedId);
  for (const e of graph.edges) {
    if (e.src === selectedId) hood.add(e.dst);
    else if (e.dst === selectedId) hood.add(e.src);
  }
  return hood;
}

/**
 * Whether a node should carry a persistent text label: it is selected, a
 * neighbor of the selection, or among the top-N by rank. This gate keeps the
 * scene from rendering all 1000+ labels at once (they would overlap illegibly).
 */
export function shouldLabel(
  id: string,
  topN: Set<string>,
  hood: Set<string>,
): boolean {
  return topN.has(id) || hood.has(id);
}

/**
 * Build a `three-spritetext` label for a node, sized so selected/neighbor
 * labels read slightly larger than ambient top-N labels.
 */
export function labelSprite(node: GNode, emphasized: boolean): SpriteText {
  const sprite = new SpriteText(node.label);
  sprite.color = "#e2e8f0";
  sprite.textHeight = emphasized ? 6 : 4;
  sprite.backgroundColor = "rgba(15,23,42,0.7)";
  sprite.padding = 1;
  sprite.position.set(0, 8, 0);
  return sprite;
}

/**
 * `nodeThreeObject` accessor: render a {@link labelSprite} alongside the
 * default node sphere only for gated nodes, and `false` (no custom object)
 * for the rest. Returning `false` keeps all 1000+ ambient labels off-screen.
 */
export function makeNodeThreeObject(
  topN: Set<string>,
  hood: Set<string>,
): (node: GNode) => Object3D | false {
  return (node) =>
    shouldLabel(node.id, topN, hood)
      ? labelSprite(node, hood.has(node.id))
      : false;
}
