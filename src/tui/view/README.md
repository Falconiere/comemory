# tui/view/

**What belongs here:** pure ratatui widgets. Every function is a pure render
from `&App` into a frame region — no state, no IO — so the layout is
snapshot-testable against a `TestBackend`.

**What does NOT belong here:** state mutation or key handling. Those live one
level up in `tui::app` and `tui::event`; `view/` only reads `&App` and draws.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `layout.rs` | `render` | Top-level frame layout: search bar, results/preview split, status line |
| `list.rs` | `render` | The results-list widget: the active tab's hits, selection highlighted |
| `preview.rs` | `render` | The preview-pane widget: the selected row's detail, wrapped in a border |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/tui/view.rs` (`pub mod
<name>;`) and callers import concrete paths.
