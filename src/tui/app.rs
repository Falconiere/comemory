//! Pure UI state for the read-only explorer.
//!
//! [`App`] owns the query box, the active tab, the current page of hits for
//! each index, and the selection. [`App::apply`] is the single state
//! transition: it maps an [`Action`] onto the state and returns the [`Effect`]
//! the event loop must carry out (re-search, semantic enrich, accept, quit, or
//! just redraw). Nothing here performs IO — the loop and the DB-worker do.

use crate::retrieval::code_rerank::CodeReranked;
use crate::retrieval::pipeline::PageWindow;
use crate::retrieval::rerank::Reranked;

/// Which index the explorer is currently searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The memory index (`pipeline::search`).
    Memory,
    /// The code-symbol index (`code_rerank::rerank_code`).
    Code,
}

impl Tab {
    /// The other tab.
    pub fn toggled(self) -> Tab {
        match self {
            Tab::Memory => Tab::Code,
            Tab::Code => Tab::Memory,
        }
    }

    /// Human label for the status/tab bar.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Memory => "memory",
            Tab::Code => "code",
        }
    }
}

/// A user intent decoded from a key press (see [`crate::tui::event`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Append a character to the query.
    InsertChar(char),
    /// Delete the last query character.
    Backspace,
    /// Clear the whole query.
    ClearQuery,
    /// Move the selection up one row.
    SelectUp,
    /// Move the selection down one row.
    SelectDown,
    /// Advance to the next page of the bounded window.
    PageNext,
    /// Retreat to the previous page.
    PagePrev,
    /// Switch between the Memory and Code tabs.
    SwitchTab,
    /// Request Memory-tab semantic enrichment.
    Semantic,
    /// Surface the selected id on the status line.
    CopyId,
    /// Accept the selection (print it to stdout on exit).
    Accept,
    /// Quit without printing.
    Quit,
    /// No-op (unbound key).
    Noop,
}

/// What the event loop should do after [`App::apply`] mutates the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Re-render only; no new query.
    Redraw,
    /// Run a fresh lexical search for the active tab/query/window.
    Search,
    /// Embed the query and re-run the Memory leg with the vector.
    Semantic,
    /// Accept the current selection and exit.
    Accept,
    /// Exit without a selection.
    Quit,
}

/// The explorer's full UI state. Pure data + transitions; the loop owns IO.
pub struct App {
    /// Current query box contents.
    pub query: String,
    /// Active index tab.
    pub tab: Tab,
    /// Current page of memory hits.
    pub memory_hits: Vec<Reranked>,
    /// Current page of code hits.
    pub code_hits: Vec<CodeReranked>,
    /// Index of the selected row within the active tab's hits.
    pub selected: usize,
    /// Offset/limit page of the bounded ranked window.
    pub window: PageWindow,
    /// Whether more in-window results exist beyond this page.
    pub has_more: bool,
    /// Whether the last Memory query ran with a semantic vector.
    pub enriched: bool,
    /// Transient hint / error line.
    pub status: String,
    /// Optional repo filter forwarded to both legs.
    pub repo: Option<String>,
    /// Generation counter: incremented on every dispatched search so the loop
    /// can discard stale DB-worker responses.
    pub seq: u64,
    /// Whether an embed command is configured. Gates Memory-tab `Ctrl-S`: with
    /// no embedder, semantic enrich is a no-op that must NOT bump `seq` (else it
    /// would discard an in-flight lexical response).
    pub has_embedder: bool,
}

impl App {
    /// Build the initial state with an optional seed query and the page size
    /// (`retrieval.top_k`) for the bounded window.
    pub fn new(repo: Option<String>, query: Option<String>, page_size: usize) -> App {
        App {
            query: query.unwrap_or_default(),
            tab: Tab::Memory,
            memory_hits: Vec::new(),
            code_hits: Vec::new(),
            selected: 0,
            window: PageWindow {
                offset: 0,
                limit: page_size,
            },
            has_more: false,
            enriched: false,
            status: String::new(),
            repo,
            seq: 0,
            has_embedder: false,
        }
    }

    /// Number of hits in the active tab.
    pub fn active_len(&self) -> usize {
        match self.tab {
            Tab::Memory => self.memory_hits.len(),
            Tab::Code => self.code_hits.len(),
        }
    }

    /// The selected row's id: a memory id, or a `repo:path:symbol` code id.
    pub fn selected_id(&self) -> Option<String> {
        match self.tab {
            Tab::Memory => self
                .memory_hits
                .get(self.selected)
                .map(|r| r.memory_id.clone()),
            Tab::Code => self
                .code_hits
                .get(self.selected)
                .map(|c| format!("{}:{}:{}", c.repo, c.path, c.symbol)),
        }
    }

    /// Replace the memory page (DB-worker response landed); clamp selection.
    pub fn set_memory_hits(&mut self, hits: Vec<Reranked>, has_more: bool) {
        self.memory_hits = hits;
        self.has_more = has_more;
        self.clamp_selection();
    }

    /// Replace the code page (DB-worker response landed); clamp selection.
    pub fn set_code_hits(&mut self, hits: Vec<CodeReranked>, has_more: bool) {
        self.code_hits = hits;
        self.has_more = has_more;
        self.clamp_selection();
    }

    /// Apply an action and report the [`Effect`] the loop must carry out.
    pub fn apply(&mut self, action: Action) -> Effect {
        match action {
            Action::InsertChar(c) => {
                self.query.push(c);
                self.on_query_change()
            }
            Action::Backspace => {
                self.query.pop();
                self.on_query_change()
            }
            Action::ClearQuery => {
                self.query.clear();
                self.on_query_change()
            }
            Action::SelectUp => self.select_up(),
            Action::SelectDown => self.select_down(),
            Action::PageNext => self.page_next(),
            Action::PagePrev => self.page_prev(),
            Action::SwitchTab => self.switch_tab(),
            Action::Semantic => self.semantic(),
            Action::CopyId => self.copy_id(),
            Action::Accept => Effect::Accept,
            Action::Quit => Effect::Quit,
            Action::Noop => Effect::Redraw,
        }
    }

    /// Move the selection up one row (saturating at the top).
    fn select_up(&mut self) -> Effect {
        self.selected = self.selected.saturating_sub(1);
        Effect::Redraw
    }

    /// Move the selection down one row, clamped to the active tab's length.
    fn select_down(&mut self) -> Effect {
        let max = self.active_len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
        Effect::Redraw
    }

    /// Advance to the next page when more in-window results exist.
    fn page_next(&mut self) -> Effect {
        if !self.has_more {
            return Effect::Redraw;
        }
        self.window.offset = self.window.offset.saturating_add(self.page_size());
        self.selected = 0;
        self.bump();
        Effect::Search
    }

    /// Retreat to the previous page (no-op on the first page).
    fn page_prev(&mut self) -> Effect {
        if self.window.offset == 0 {
            return Effect::Redraw;
        }
        self.window.offset = self.window.offset.saturating_sub(self.page_size());
        self.selected = 0;
        self.bump();
        Effect::Search
    }

    /// Switch tabs and dispatch a fresh search for the new index.
    fn switch_tab(&mut self) -> Effect {
        self.tab = self.tab.toggled();
        self.selected = 0;
        self.window.offset = 0;
        self.enriched = false;
        self.bump();
        Effect::Search
    }

    /// Request semantic enrichment — Memory tab only; a hint on Code.
    fn semantic(&mut self) -> Effect {
        match (self.tab, self.has_embedder) {
            (Tab::Memory, true) => {
                self.bump();
                Effect::Semantic
            }
            (Tab::Memory, false) => {
                self.status = "no embed command configured (set --embed-cmd or COMEMORY_EMBED_CMD)"
                    .to_string();
                Effect::Redraw
            }
            (Tab::Code, _) => {
                self.status = "semantic search is Memory-tab only".to_string();
                Effect::Redraw
            }
        }
    }

    /// Surface the selected row's id on the status line.
    fn copy_id(&mut self) -> Effect {
        self.status = match self.selected_id() {
            Some(id) => format!("id: {id}"),
            None => "no selection".to_string(),
        };
        Effect::Redraw
    }

    /// Effective page size (the window limit, never zero for paging math).
    fn page_size(&self) -> usize {
        self.window.limit.max(1)
    }

    /// Bump the generation counter so older worker responses are ignored.
    fn bump(&mut self) {
        self.seq = self.seq.saturating_add(1);
    }

    /// Shared reset for any query edit: rewind to the first page, drop the
    /// semantic flag, and dispatch a fresh lexical search.
    fn on_query_change(&mut self) -> Effect {
        self.selected = 0;
        self.window.offset = 0;
        self.enriched = false;
        self.bump();
        Effect::Search
    }

    /// Keep the selection within the active tab's bounds after a page swap.
    fn clamp_selection(&mut self) {
        let max = self.active_len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }
}
