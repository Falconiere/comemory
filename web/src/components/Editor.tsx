import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { oneDark } from "@codemirror/theme-one-dark";
import { rust } from "@codemirror/lang-rust";
import { python } from "@codemirror/lang-python";
import { javascript } from "@codemirror/lang-javascript";
import { go } from "@codemirror/lang-go";
import type { Extension } from "@codemirror/state";
import type { FileView } from "../types";

interface Props {
  view: FileView;
  onClose: () => void;
}

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

export default function Editor({ view, onClose }: Props) {
  const extensions = useMemo(() => langExtension(view.lang), [view.lang]);

  return (
    <div className="flex h-full flex-col bg-slate-900 text-slate-100">
      <div className="flex items-center gap-3 border-b border-slate-700 px-4 py-2">
        <span className="truncate font-mono text-sm" title={view.path}>
          {view.path}
        </span>
        <span className="rounded bg-slate-700 px-1.5 py-0.5 text-xs uppercase">
          {view.lang}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <button
            className="rounded px-2 py-1 text-sm text-slate-400 hover:text-slate-100"
            onClick={onClose}
            title="Close editor"
          >
            ✕
          </button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <CodeMirror
          value={view.contents}
          height="100%"
          theme={oneDark}
          extensions={extensions}
          editable={false}
          readOnly
        />
      </div>
    </div>
  );
}
