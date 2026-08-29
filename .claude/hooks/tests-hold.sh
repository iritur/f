#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# PreToolUse on Write and Edit: an agent fixing a failure may not weaken the
# thing that detected it.
#
# This is the hook the test stage exists for. Given a red test and a hard bug,
# making the test agree is a shorter path than making the code right, and it is
# indistinguishable from success at the point where anybody looks. So the escape
# routes are closed mechanically: `#[ignore]`, a deleted assertion left as
# `assert!(true)`, a shrunk iteration count in a litmus or stress test.
#
# It is deliberately narrow. Legitimately deleting a test happens, and this hook
# blocks it — the answer is to say so and let a person decide, which takes one
# sentence and leaves a record.

set -u
. "$(dirname "$0")/lib.sh"

payload=$(hook_payload)
file=$(json_field file_path "$payload")
case "$file" in
*.rs) ;;
*) exit 0 ;;
esac

if printf '%s' "$payload" | grep -Eq '#\[ignore'; then
	deny "Blocked: this edit adds #[ignore]." \
		"An ignored test is a deleted test that still reports green. If the test is wrong, fix the test" \
		"and say why in the diff. If the test is right and the code is not, the code is the work." \
		"If it must be disabled, a person decides that and it goes in TODO.md with an exit condition."
fi

if printf '%s' "$payload" | grep -Eq 'assert(_eq)?!\([[:space:]]*true[[:space:]]*[,)]'; then
	deny "Blocked: assert!(true) asserts nothing." \
		"This is what a deleted assertion looks like afterwards. Restore what was being checked, or" \
		"delete the test outright and say so — a test that passes unconditionally is worse than no test," \
		"because it occupies the place where somebody would notice one is missing."
fi

# The litmus tests are stress tests: their repeat count *is* their sensitivity.
# Lowering it makes an ordering bug stop reproducing without making it stop
# existing. CONTRIBUTING.md, and .claude/skills/memory-ordering/.
case "$file" in
*litmus*)
	if printf '%s' "$payload" | grep -Eq '(iterations|repeat|ROUNDS|ITERS)[[:space:]]*[:=]'; then
		deny "Blocked: this edit changes a repeat count in a litmus test." \
			"A litmus test's iteration count is its sensitivity. Lowering it stops an ordering bug" \
			"reproducing without stopping it existing, and x86 total store order hides the whole class" \
			"already. Raising it is fine — say which you are doing and why, and a person can wave it" \
			"through. See .claude/skills/memory-ordering/."
	fi
	;;
esac

exit 0
