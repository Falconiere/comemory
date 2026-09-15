#!/usr/bin/env bash
# shellcheck shell=bash
# Stable project-skills entry point for hooks and the standalone wrapper.

PS_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)" || {
  printf 'project-skills: cannot resolve library directory\n' >&2
  return 1
}

# shellcheck source=repo-scope.sh
# shellcheck disable=SC1091 # Resolved at runtime from this library's directory.
. "$PS_LIB_DIR/repo-scope.sh"
# shellcheck source=project-skills-foundation.sh
# shellcheck disable=SC1091 # Resolved at runtime from this library's directory.
. "$PS_LIB_DIR/project-skills-foundation.sh"
# shellcheck source=project-skills-commands.sh
# shellcheck disable=SC1091 # Resolved at runtime from this library's directory.
. "$PS_LIB_DIR/project-skills-commands.sh"
# shellcheck source=project-skills-curation.sh
# shellcheck disable=SC1091 # Resolved at runtime from this library's directory.
. "$PS_LIB_DIR/project-skills-curation.sh"
