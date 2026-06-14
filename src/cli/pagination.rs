//! Reusable clap pagination flags shared across subcommands. Flatten this into
//! a command's `Args` with `#[command(flatten)]` so every paginated command
//! exposes an identical `--limit` / `--offset` pair and feeds the same window
//! into [`crate::output::page::Page::from_slice`] (Binding Rule 1).

use clap::Args as ClapArgs;

/// `--limit` / `--offset` window flags. `--limit` defaults to 50; `--limit 0`
/// means "all" (no slicing). `--offset` defaults to 0.
#[derive(ClapArgs, Debug, Clone, Copy)]
pub struct PaginationArgs {
    /// Maximum number of results to return. `0` means "all" (no limit).
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Number of leading results to skip before the window starts.
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
}
