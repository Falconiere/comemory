import type { Filters } from "../types";

interface Props {
  filters: Filters;
  onFilters: (f: Filters) => void;
  search: string;
  onSearch: (s: string) => void;
  onSearchSubmit: () => void;
  onRelayout: () => void;
  dark: boolean;
  onToggleDark: () => void;
}

export default function Toolbar({
  filters,
  onFilters,
  search,
  onSearch,
  onSearchSubmit,
  onRelayout,
  dark,
  onToggleDark,
}: Props) {
  return (
    <div className="flex flex-wrap items-center gap-4 border-b border-slate-700 bg-slate-900 px-4 py-2 text-sm text-slate-200">
      <span className="font-semibold text-slate-100">comemory</span>

      <form
        className="flex items-center gap-1"
        onSubmit={(e) => {
          e.preventDefault();
          onSearchSubmit();
        }}
      >
        <input
          className="w-56 rounded border border-slate-600 bg-slate-800 px-2 py-1 text-slate-100 placeholder:text-slate-500"
          placeholder="search files…"
          value={search}
          onChange={(e) => onSearch(e.target.value)}
        />
        <button className="rounded bg-slate-700 px-2 py-1 hover:bg-slate-600" type="submit">
          go
        </button>
      </form>

      <label className="flex items-center gap-1">
        <input
          type="checkbox"
          checked={filters.imports}
          onChange={(e) => onFilters({ ...filters, imports: e.target.checked })}
        />
        <span className="inline-block h-2 w-2 rounded-sm" style={{ background: "#3367d6" }} />
        imports
      </label>
      <label className="flex items-center gap-1">
        <input
          type="checkbox"
          checked={filters.co_changed}
          onChange={(e) => onFilters({ ...filters, co_changed: e.target.checked })}
        />
        <span className="inline-block h-2 w-2 rounded-sm" style={{ background: "#d9730d" }} />
        co-changed
      </label>

      <label className="flex items-center gap-2">
        min-weight
        <input
          type="range"
          min={1}
          max={10}
          value={filters.minWeight}
          onChange={(e) =>
            onFilters({ ...filters, minWeight: Number(e.target.value) })
          }
        />
        <span className="w-4 text-center font-mono">{filters.minWeight}</span>
      </label>

      <button className="rounded bg-slate-700 px-2 py-1 hover:bg-slate-600" onClick={onRelayout}>
        re-layout
      </button>

      <button
        className="ml-auto rounded bg-slate-700 px-2 py-1 hover:bg-slate-600"
        onClick={onToggleDark}
        title="Toggle theme"
      >
        {dark ? "☾" : "☀"}
      </button>
    </div>
  );
}
