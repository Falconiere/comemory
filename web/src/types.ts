export interface GNode {
  id: string;
  label: string;
  repo: string;
  rank: number;
  symbols: number;
}

export interface GEdge {
  src: string;
  dst: string;
  rel: string;
  weight: number;
}

export interface CodeGraph {
  nodes: GNode[];
  edges: GEdge[];
}

export interface FileView {
  path: string;
  lang: string;
  contents: string;
  blob_oid: string;
}

export interface Health {
  read_only: boolean;
  version: string;
}

export interface Filters {
  imports: boolean;
  co_changed: boolean;
  minWeight: number;
}

export type SaveResult =
  | { conflict: false; blob_oid: string }
  | { conflict: true; current_oid: string };
