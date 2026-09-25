//! Rendering of the `exchange` state for `comemory sync`: the TTY lines of
//! `--action status`'s `exchange` block and the one-line summary of a run's
//! `exchange` leg. Their `--json` shapes are the domain structs themselves.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::domains::sync::drain::report::Report;
use crate::domains::sync::drain::status::ExchangeStatus;

/// The TTY lines of the `exchange` status block.
pub(crate) fn exchange_status_lines(x: &ExchangeStatus) -> Vec<String> {
    let nonzero = |held: &BTreeMap<String, i64>| {
        held.iter()
            .filter(|(_, n)| **n > 0)
            .fold(String::new(), |mut line, (reason, n)| {
                let _ = write!(line, " {reason}={n}");
                line
            })
    };
    let mut lines = vec![
        format!(
            "exchange: {} · {} · network={}{}",
            x.protocol.as_deref().unwrap_or("not negotiated"),
            x.coverage_reason.as_deref().map_or_else(
                || x.coverage.clone().unwrap_or_default(),
                |why| format!("partial ({why})")
            ),
            x.network,
            x.last_error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        ),
        format!(
            "exchange cursor: {} of {} · caught_up={}",
            x.applied_sequence,
            x.upstream_head
                .map_or_else(|| "?".to_string(), |h| h.to_string()),
            x.caught_up
        ),
        format!(
            "exchange outbox: pending={} retryable={} rejected={}{}",
            x.outbox.pending,
            x.outbox.retryable,
            x.outbox.rejected,
            nonzero(&x.outbox.held)
        ),
        format!("exchange pull held:{}", nonzero(&x.pull.held)),
    ];
    if let Some(at) = x.pull.stalled_at {
        lines.push(format!(
            "exchange pull stalled before {at}: {}",
            x.pull.stall_reason.as_deref().unwrap_or("unknown")
        ));
    }
    lines
}

/// One line summarizing the `exchange` leg of a run.
pub(crate) fn exchange_run_line(x: &Report) -> String {
    format!(
        "exchange: {} · pushed={} pulled={} held={} settled={} rejected={} · end={} more={}{} · network={}",
        x.protocol.as_deref().unwrap_or("not negotiated"),
        x.pushed,
        x.pulled,
        x.held,
        x.settled,
        x.rejected,
        serde_json::to_value(x.end)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        x.more,
        if x.rebootstrapped {
            " rebootstrapped"
        } else {
            ""
        },
        x.network
    )
}
