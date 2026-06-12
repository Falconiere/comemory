import type { GNode } from "../types";

interface Props {
  node: GNode | null;
  color: string | undefined;
  onViewSource: (id: string) => void;
}

export default function DetailPanel({ node, color, onViewSource }: Props) {
  if (!node) {
    return (
      <div className="p-4 text-sm text-slate-400">
        Click a file node to see its details and source.
      </div>
    );
  }
  return (
    <div className="space-y-3 p-4 text-sm">
      <div className="flex items-center gap-2">
        <span
          className="inline-block h-3 w-3 rounded-sm"
          style={{ backgroundColor: color ?? "#5b8def" }}
        />
        <span className="break-all font-mono text-slate-100">{node.label}</span>
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-slate-300">
        <dt className="text-slate-500">repo</dt>
        <dd className="font-mono">{node.repo}</dd>
        <dt className="text-slate-500">rank</dt>
        <dd className="font-mono">{node.rank.toFixed(4)}</dd>
        <dt className="text-slate-500">symbols</dt>
        <dd className="font-mono">{node.symbols}</dd>
      </dl>
      <button
        className="w-full rounded bg-blue-600 px-3 py-1.5 font-medium text-white hover:bg-blue-500"
        onClick={() => onViewSource(node.id)}
      >
        View source
      </button>
    </div>
  );
}
