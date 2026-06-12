interface Props {
  repoColors: Map<string, string>;
}

export default function Legend({ repoColors }: Props) {
  const entries = Array.from(repoColors.entries());
  if (entries.length === 0) return null;
  return (
    <div className="space-y-1 p-4">
      <div className="text-xs font-semibold uppercase tracking-wide text-slate-500">
        Repos
      </div>
      <ul className="space-y-1">
        {entries.map(([repo, color]) => (
          <li key={repo} className="flex items-center gap-2 text-sm text-slate-300">
            <span
              className="inline-block h-3 w-3 rounded-sm"
              style={{ backgroundColor: color }}
            />
            <span className="truncate font-mono">{repo}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
