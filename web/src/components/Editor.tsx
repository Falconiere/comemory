import { useEffect, useMemo, useRef, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { oneDark } from "@codemirror/theme-one-dark";
import { rust } from "@codemirror/lang-rust";
import { python } from "@codemirror/lang-python";
import { javascript } from "@codemirror/lang-javascript";
import { go } from "@codemirror/lang-go";
import type { Extension } from "@codemirror/state";
import type { FileView } from "../types";
import { putFile } from "../api";

interface Props {
  nodeId: string;
  view: FileView;
  serverReadOnly: boolean;
  onClose: () => void;
  onReload: () => void;
}

type Status =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "saved" }
  | { kind: "conflict"; currentOid: string }
  | { kind: "error"; message: string };

function langExtension(lang: string): Extension[] {
  switch (lang) {
    case "rust":
      return [rust()];
    case "python":
      return [python()];
    case "javascript":
      return [javascript()];
    case "typescript":
      return [javascript({ typescript: true })];
    case "go":
      return [go()];
    default:
      return [];
  }
}

export default function Editor({
  nodeId,
  view,
  serverReadOnly,
  onClose,
  onReload,
}: Props) {
  const [value, setValue] = useState(view.contents);
  const [baseOid, setBaseOid] = useState(view.blob_oid);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const savedRef = useRef(view.contents);

  // Reset when a new file (or a reload of the same file) arrives.
  useEffect(() => {
    setValue(view.contents);
    setBaseOid(view.blob_oid);
    savedRef.current = view.contents;
    setStatus({ kind: "idle" });
  }, [view]);

  const dirty = value !== savedRef.current;
  const extensions = useMemo(() => langExtension(view.lang), [view.lang]);

  async function save() {
    if (serverReadOnly || !dirty) return;
    setStatus({ kind: "saving" });
    try {
      const res = await putFile(nodeId, value, baseOid);
      if (res.conflict) {
        setStatus({ kind: "conflict", currentOid: res.current_oid });
        return;
      }
      setBaseOid(res.blob_oid);
      savedRef.current = value;
      setStatus({ kind: "saved" });
    } catch (e) {
      setStatus({ kind: "error", message: String(e) });
    }
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
    }
  }

  return (
    <div
      className="flex h-full flex-col bg-slate-900 text-slate-100"
      onKeyDown={onKeyDown}
    >
      <div className="flex items-center gap-3 border-b border-slate-700 px-4 py-2">
        <span className="truncate font-mono text-sm" title={view.path}>
          {view.path}
        </span>
        <span className="rounded bg-slate-700 px-1.5 py-0.5 text-xs uppercase">
          {view.lang}
        </span>
        {dirty && <span className="text-amber-400" title="unsaved changes">●</span>}
        <div className="ml-auto flex items-center gap-2">
          {!serverReadOnly && (
            <button
              className="rounded bg-blue-600 px-3 py-1 text-sm font-medium hover:bg-blue-500 disabled:opacity-40"
              onClick={() => void save()}
              disabled={!dirty || status.kind === "saving"}
            >
              {status.kind === "saving" ? "Saving…" : "Save"}
            </button>
          )}
          <button
            className="rounded px-2 py-1 text-sm text-slate-400 hover:text-slate-100"
            onClick={onClose}
            title="Close editor"
          >
            ✕
          </button>
        </div>
      </div>

      <StatusBar status={status} serverReadOnly={serverReadOnly} onReload={onReload} />

      <div className="min-h-0 flex-1 overflow-auto">
        <CodeMirror
          value={value}
          height="100%"
          theme={oneDark}
          extensions={extensions}
          editable={!serverReadOnly}
          onChange={setValue}
        />
      </div>
    </div>
  );
}

function StatusBar({
  status,
  serverReadOnly,
  onReload,
}: {
  status: Status;
  serverReadOnly: boolean;
  onReload: () => void;
}) {
  if (serverReadOnly) {
    return (
      <div className="bg-slate-800 px-4 py-1 text-xs text-slate-400">
        read-only server — start without <code>--read-only</code> to edit
      </div>
    );
  }
  if (status.kind === "saved") {
    return <div className="bg-emerald-900/60 px-4 py-1 text-xs text-emerald-300">saved</div>;
  }
  if (status.kind === "error") {
    return (
      <div className="bg-red-900/60 px-4 py-1 text-xs text-red-300">{status.message}</div>
    );
  }
  if (status.kind === "conflict") {
    return (
      <div className="flex items-center gap-2 bg-amber-900/60 px-4 py-1 text-xs text-amber-200">
        file changed on disk since you opened it.
        <button className="underline hover:text-amber-100" onClick={onReload}>
          reload
        </button>
      </div>
    );
  }
  return null;
}
