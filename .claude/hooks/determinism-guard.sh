#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# PreToolUse on Write and Edit: catch a determinism violation at the keystroke
# rather than at `cargo xtask lint-determinism` twenty minutes later.
#
# This is the same policy as the lint and deliberately not the same code: the
# lint is authoritative, walks the whole tree, and owns the allow-list. This is
# a fast approximation that runs on one payload and exists only to shorten the
# feedback loop. If the two ever disagree, the lint is right.
#
# It does not read DETERMINISM_ALLOW. Instead it exempts the two paths that
# allow-list currently covers, and says so — an agent working inside bench/ gets
# no warning, and an agent that has genuinely earned a new allow-list entry gets
# a warning it can answer by editing the lint, which is the reviewable diff the
# policy wants anyway.

set -u
. "$(dirname "$0")/lib.sh"

payload=$(hook_payload)
file=$(json_field file_path "$payload")
case "$file" in
*.rs) ;;
*) exit 0 ;;
esac

rel=$(relative_path "$file")
case "$rel" in
bench/* | kernel/src/arch/x86_64/mod.rs | xtask/*) exit 0 ;;
esac

found=""
check() {
	if printf '%s' "$payload" | grep -Eq "$1"; then
		found="$found
  $2"
	fi
}

check 'rdtsc' 'rdtsc — read time through f_env::Env'
check 'SystemTime::now' 'SystemTime::now — read time through f_env::Env'
check 'Instant::now' 'Instant::now — read time through f_env::Env'
check 'thread_rng' 'thread_rng — draw randomness from f_env::Env'
check 'HashMap::new|HashMap::with_capacity' 'HashMap — iteration order is seeded per process; use BTreeMap'
check 'HashSet::new|HashSet::with_capacity' 'HashSet — iteration order is seeded per process; use BTreeSet'

if [ -n "$found" ]; then
	deny "Blocked: this edit introduces a direct source of nondeterminism in $rel.$found" \
		"" \
		"The contract is (seed, commit) -> byte-identical execution, and every other layer of the test" \
		"apparatus rests on it. RFC 0004, and .claude/skills/determinism-review/." \
		"" \
		"If this call site genuinely cannot go through Env, it needs an entry in DETERMINISM_ALLOW in" \
		"xtask/src/main.rs with a stated reason and a revisit condition — which is a reviewable diff," \
		"and is the point. Propose it; do not add it silently in the same edit as the code."
fi

exit 0
