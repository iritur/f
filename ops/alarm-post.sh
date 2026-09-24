#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The half of the alarm that reaches the network, and the only half a real
# failure can exercise.
#
# `ops/alarm.sh` has already written `alarm/decision`, `alarm/title` and
# `alarm/body.md` from the job table. This opens, comments on or closes an issue
# with them, and does nothing else — no formatting, no decision, no reading of
# what failed. The split is deliberate and it is the whole reason the decision
# and the message are testable at all: everything a fixture can decide is
# decided in a file `cargo xtask test` runs, and what is left here is three `gh`
# calls whose behaviour is GitHub's rather than this repository's.
#
# ## Why a title match and not a label
#
# Because a label has to exist before it can be used, `gh issue create --label`
# fails on a repository that has not got one, and a mechanism whose identity
# depends on a piece of repository configuration is a mechanism that silently
# becomes two mechanisms when somebody renames the label. The title is the
# signature — `ops/alarm.sh` writes it and sorts it so the same failure set
# spells the same title on every run — and it is also the thing a person reads.
# A label is still applied when one exists, because it is useful for filtering;
# nothing depends on it.
#
# ## What closes an issue
#
# A green run of the same workflow, and it closes *every* open thread whose
# title this workflow could have written. That is broader than "the one it
# opened" and it is deliberate: a green run means every job in the file passed,
# so any open `<workflow> is red:` thread is about a failure that is no longer
# happening. A stale issue trains a reader to ignore the mechanism, which is the
# failure A-06 names in the other direction.
#
# *What would reverse this:* the day a failure is intermittent enough that a
# green run does not mean the defect is gone, closing on green is wrong and the
# thread should instead be commented on and left open. Nothing here has seen
# that yet; `docs/postmortem/0002` and `0003` are both deterministic failures
# that stayed red until somebody fixed them.

set -eu

out=${ALARM_OUT:-alarm}
decision=$(cat "$out/decision")
title=$(cat "$out/title")
body="$out/body.md"

# `--search "<title> in:title"` is a full-text search and matches more than the
# exact string, so every result is filtered on equality here. Without that, a
# thread titled `nightly is red: cut` would be found by a search for
# `nightly is red: cut, miri` and the second finding would be filed as a comment
# on the first.
open_with_title() {
	gh issue list --state open --limit 100 --search "\"$1\" in:title" --json number,title \
		--jq "[.[] | select(.title == \"$1\")] | .[].number"
}

# And the green case wants the other relation: every thread this workflow could
# have opened, which is a prefix match rather than an equality one.
open_with_prefix() {
	gh issue list --state open --limit 100 --search "\"$1\" in:title" --json number,title \
		--jq "[.[] | select(.title | startswith(\"$1\"))] | .[].number"
}

case "$decision" in
red)
	existing=$(open_with_title "$title")
	if [ -n "$existing" ]; then
		# One thread per failure set. A job that fails for thirty consecutive
		# nights should produce one issue with thirty comments, not thirty
		# issues nobody closes — thirty issues is how a mechanism becomes
		# muted without anybody choosing to mute it.
		for n in $existing; do
			gh issue comment "$n" --body-file "$body"
			echo "alarm: commented on #$n"
		done
	else
		gh issue create --title "$title" --body-file "$body" --label schedule ||
			gh issue create --title "$title" --body-file "$body"
		echo "alarm: opened \"$title\""
	fi
	;;
green)
	closed=0
	for n in $(open_with_prefix "$title"); do
		gh issue comment "$n" --body-file "$body"
		gh issue close "$n"
		echo "alarm: closed #$n"
		closed=$((closed + 1))
	done
	echo "alarm: $closed stale thread(s) closed by a green run"
	;;
*)
	echo "alarm: decision is \`$decision\`, so nothing is opened and nothing is closed"
	;;
esac
