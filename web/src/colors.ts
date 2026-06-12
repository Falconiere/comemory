import type { GNode } from "./types";

/** Qualitative palette cycled across repos for stable per-repo coloring. */
const PALETTE = [
  "#5b8def",
  "#e0793b",
  "#3fb37f",
  "#c061d0",
  "#d9b13b",
  "#3fb8c0",
  "#e0607e",
  "#8a7be0",
  "#7fae3f",
  "#d06a9e",
];

/** Edge colors by relation kind (mirrors the static DOT/HTML renderers). */
export const EDGE_COLORS: Record<string, string> = {
  imports: "#3367d6",
  co_changed: "#d9730d",
};

/**
 * Stable map of repo label → color. Repos are sorted so the assignment is
 * deterministic regardless of node iteration order.
 */
export function repoColorMap(nodes: GNode[]): Map<string, string> {
  const repos = Array.from(new Set(nodes.map((n) => n.repo))).sort();
  const map = new Map<string, string>();
  repos.forEach((repo, i) => map.set(repo, PALETTE[i % PALETTE.length]));
  return map;
}
