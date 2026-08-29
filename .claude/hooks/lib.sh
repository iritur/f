#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Shared plumbing for the hooks in this directory.
#
# A hook reads one JSON object on stdin and says yes or no by its exit code:
# 0 lets the tool call through, 2 blocks it and hands stderr back to the agent
# as the reason. Anything else is a broken hook and is reported as such rather
# than silently allowing the call.
#
# The JSON is read with sed rather than a parser because a hook runs on every
# matching tool call and has to stay under a few milliseconds. The consequences
# are stated where they matter: `json_field` stops at the first unescaped quote,
# so it is used only for short scalar fields such as `file_path`. Anything that
# has to look at file *content* greps the whole payload instead, which cannot be
# fooled by escaping and costs nothing.

set -u

# Read the hook payload from stdin, once, into a single line.
hook_payload() {
	tr '\n' ' '
}

# json_field <key> <payload> — the first string value for <key>.
# Backslashes are normalised to forward slashes so that a Windows path in an
# escaped JSON string compares against the same prefixes as a POSIX one.
json_field() {
	printf '%s' "$2" |
		sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" |
		sed 's|\\\\|/|g; s|\\|/|g' |
		head -n 1
}

# The repository root. CLAUDE_PROJECT_DIR is set by Claude Code; the fallback
# keeps these scripts runnable by hand, which is how they get tested.
project_dir() {
	if [ -n "${CLAUDE_PROJECT_DIR:-}" ]; then
		printf '%s' "$CLAUDE_PROJECT_DIR"
	else
		( cd "$(dirname "$0")/../.." && pwd )
	fi
}

# Path relative to the repository root, with forward slashes, no leading `./`.
relative_path() {
	root=$(project_dir | sed 's|\\|/|g')
	printf '%s' "${1#"$root"/}"
}

# Block the tool call. Everything after the first line should say what to do
# instead: the agent reads this, and a refusal with no route out of it gets
# worked around rather than obeyed.
deny() {
	printf '%s\n' "$@" >&2
	exit 2
}
