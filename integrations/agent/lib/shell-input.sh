#!/usr/bin/env bash
# Remove heredoc bodies before checking shell command scope.
strip_heredocs() {
  awk '
    BEGIN { in_heredoc = 0; tag = ""; tag_tab = "" }
    {
      if (in_heredoc) {
        line = $0
        if (line == tag || line == tag_tab) { in_heredoc = 0; tag = ""; tag_tab = "" }
        next
      }
      if (match($0, /<<-?[ \t]*"?'\''?[A-Za-z_][A-Za-z0-9_]*"?'\''?/)) {
        m = substr($0, RSTART, RLENGTH)
        # Strip leading `<<`, optional `-`, quotes, and surrounding whitespace.
        gsub(/^<<-?[ \t]*"?'\''?/, "", m)
        gsub(/"?'\''?$/, "", m)
        tag = m
        tag_tab = "\t" m
        in_heredoc = 1
        print
        next
      }
      print
    }
  '
}
