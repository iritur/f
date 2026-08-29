#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# PostToolUse on Write and Edit: format the file that was just written.
#
# The policy job runs `cargo fmt --all -- --check`, so an unformatted file is a
# red build. Formatting here means the agent never spends a cycle on it, and —
# more usefully — never reformats a neighbouring file it did not touch, which is
# what happens when formatting is deferred to a whole-tree run at the end.
#
# rustfmt picks up rustfmt.toml from the file's ancestors, so this gets
# `use_small_heuristics = "Max"` without being told. Never exits non-zero: a
# formatter that blocks the loop over its own failure is worse than an
# unformatted file, and the check in CI is the actual gate.

set -u
. "$(dirname "$0")/lib.sh"

payload=$(hook_payload)
file=$(json_field file_path "$payload")
case "$file" in
*.rs) ;;
*) exit 0 ;;
esac
[ -f "$file" ] || exit 0

command -v rustfmt >/dev/null 2>&1 || exit 0
rustfmt --edition 2024 "$file" >/dev/null 2>&1 || true
exit 0
