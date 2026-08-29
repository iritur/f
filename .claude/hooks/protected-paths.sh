#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# PreToolUse on Write, Edit and NotebookEdit.
#
# Four kinds of path are not the agent's to edit, for four different reasons:
#
#   third_party/        imported verbatim. Editing it in place destroys the
#                       property that makes the licence boundary checkable — that
#                       what is in there is what was imported. A change here is
#                       a new import commit. LICENSING.md, RFC 0003.
#   target/             build output, including the .profraw files coverage
#                       writes. Editing it edits nothing.
#   rust-toolchain.toml a bump invalidates every claim in claims/ and requires a
#                       full re-run. It is a decision, not a fix for a build
#                       error. See claims/README.md.
#   Cargo.lock          resolved by cargo. Hand-editing it produces a lockfile
#                       that does not correspond to any solve.
#
# Read access is untouched: all four are worth reading and often need to be.

set -u
. "$(dirname "$0")/lib.sh"

payload=$(hook_payload)
file=$(json_field file_path "$payload")
[ -n "$file" ] || exit 0

rel=$(relative_path "$file")

case "$rel" in
third_party/*)
	deny "Blocked: $rel is imported source." \
		"third_party/ is imported verbatim and the licence boundary depends on it staying that way." \
		"A change to imported code is a new import commit that touches nothing else, plus the deny.toml" \
		"and LICENSING.md entries that go with it. See RFC 0003 and .claude/skills/licence-boundary/."
	;;
target/* | */target/* | *.profraw)
	deny "Blocked: $rel is build output." \
		"Nothing here is a source of truth. If a build artefact is wrong, the input that produced it is" \
		"what to change; if it is stale, 'cargo clean' is what to run."
	;;
rust-toolchain.toml)
	deny "Blocked: $rel pins the toolchain." \
		"A toolchain bump invalidates every claim in claims/ and requires a full re-run, so it is a" \
		"decision a person makes deliberately — never a fix for a build error. If a build needs a newer" \
		"toolchain, say so and stop. claims/README.md, and .claude/skills/claims-registry/."
	;;
Cargo.lock)
	deny "Blocked: $rel is resolved by cargo, not written by hand." \
		"Run the cargo command that produces the change you want — 'cargo update -p <crate>' for one" \
		"dependency, or a build for a new one — and commit the result."
	;;
esac

exit 0
