import type { ForceGraphMethods, NodeObject, LinkObject } from "react-force-graph-3d";
import type { GLink } from "./links";
import type { GNode } from "../../types";

/** Camera-fly duration in milliseconds. */
const FLY_MS = 1000;
/** Distance the camera is offset from the focused node along its view ray. */
const FLY_DISTANCE = 120;

/** The imperative ForceGraph3D handle, generic over our node/link shapes. */
export type FgHandle = ForceGraphMethods<
  NodeObject<GNode>,
  LinkObject<GNode, GLink>
>;

/**
 * Fly the camera to `nodeId`. `react-force-graph-3d` mutates the very node
 * objects we pass it with live `x/y/z`, so we read coordinates straight off
 * `nodes`. Once the node is positioned we orbit the camera out along its view
 * ray and look at it; before coordinates exist (first paint) we fit the whole
 * graph instead.
 */
export function flyTo(
  fg: FgHandle,
  nodes: NodeObject<GNode>[],
  nodeId: string,
): void {
  const node = nodes.find((n) => n.id === nodeId);
  if (!node || node.x === undefined) {
    fg.zoomToFit(FLY_MS);
    return;
  }
  const { x = 0, y = 0, z = 0 } = node;
  const dist = Math.hypot(x, y, z) || 1;
  const ratio = 1 + FLY_DISTANCE / dist;
  fg.cameraPosition(
    { x: x * ratio, y: y * ratio, z: z * ratio },
    { x, y, z },
    FLY_MS,
  );
}
