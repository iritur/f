#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# What a red scheduled run says, and whether it should say anything.
#
# This is the half of the alarm that reaches no network. It is handed a table of
# job results, a commit, a link and — when there was a failure — the failed
# jobs' logs, and it writes three files: a one-word decision, a title, and a body.
# The workflow step around it is what opens, comments on or closes an issue with
# those three files; that half needs a token and cannot be run here.
#
# ## Why this is a file rather than thirty lines of YAML in five workflows
#
# Because a mechanism nobody can exercise is the one that is broken when it is
# needed, and `docs/postmortem/0002` is this repository's record of exactly that:
# the job built to reach a person was itself broken for five nights, and the
# breakage was invisible because nothing but a nightly ever ran it. Everything
# decidable without a token is decidable on a laptop, so it is here, and
# `xtask`'s `alarm_script` tests run it against fixtures inside `cargo xtask
# test`. What is left in the workflow is three `gh` calls and one `jq`
# expression.
#
# ## What it does not decide
#
# Whether an issue already exists. That is a search, the search is a network
# call, and a script that guessed would be guessing about the one thing the
# caller can simply look up. This prints `red`; the caller looks for the title
# and either opens or comments.
#
# ## Inputs, all through the environment
#
#   ALARM_JOBS      path to a `name<TAB>result` table, one job per line
#   ALARM_WORKFLOW  the workflow's name, e.g. `nightly`
#   ALARM_SHA       the commit the run tested
#   ALARM_BRANCH    the default branch's name, e.g. `main`
#   ALARM_HEAD      that branch's head when the alarm ran, or empty if unread
#   ALARM_RUN_URL   a link to the run
#   ALARM_LOG       path to the failed jobs' logs, or a path that does not exist.
#                   Each line `job<TAB>id<TAB>text` — the shape `gh run view
#                   --log-failed` prints and the alarm jobs write — so that what
#                   the body quotes can name the job it came from. An untagged
#                   line is read as text and its job as unknown.
#   ALARM_OUT       directory to write `decision`, `title` and `body.md` into
#
# Nothing here reads a clock, a random source or an environment variable not on
# that list. That is not ceremony: the body this writes is compared against a
# fixture in a test, and a body carrying the time of day is one no test can
# compare. RFC 0004 is the rule and this is the shell-shaped half of it.
#
# *What would reverse this:* a notification route that is not an issue — a chat
# webhook, a mail — would want the same decision and a different body. The
# decision and the body are already separate files for that reason; a second
# renderer goes beside this one and the decision does not move.

set -u

jobs=${ALARM_JOBS:?ALARM_JOBS names the job table and was not set}
workflow=${ALARM_WORKFLOW:-unknown}
sha=${ALARM_SHA:-}
branch=${ALARM_BRANCH:-main}
head=${ALARM_HEAD:-}
run_url=${ALARM_RUN_URL:-}
log=${ALARM_LOG:-/nonexistent}
out=${ALARM_OUT:-.}

if [ ! -f "$jobs" ]; then
	echo "alarm: $jobs is not a file. The job table is what this decides from, so" >&2
	echo "alarm: there is nothing to decide and saying so is the only honest answer." >&2
	echo "alarm: The run's own red cross is all that is left." >&2
	exit 2
fi

mkdir -p "$out"

# The three result classes, and `skipped` is deliberately not one of them.
#
# A job skipped by its own `if:` did not fail and did not pass — `maintain.yml`
# skips `diagnose` on every day nothing moved, which is that file working. A
# mechanism that treated a skip as red would be red most days, and a mechanism
# that is red most days is the muting A-06 names.
#
# `cancelled` is the one that needs an argument. A cancelled run asserts
# nothing: somebody pressed the button, or a newer run superseded it. So a run
# with cancellations and no failures decides `nothing` — it does not open, and
# it does not *close*, because closing on a run that proved nothing is how a
# real finding disappears. A run with both reports the failures: a matrix
# cancelling its siblings after one of them failed is still a failure.
failed=$(awk -F'\t' '$2 == "failure" { print $1 }' "$jobs" | sort)
cancelled=$(awk -F'\t' '$2 == "cancelled" { print $1 }' "$jobs" | sort)
succeeded=$(awk -F'\t' '$2 == "success" { n++ } END { print n + 0 }' "$jobs")
total=$(awk 'NF { n++ } END { print n + 0 }' "$jobs")

if [ "$total" -eq 0 ]; then
	echo "alarm: the job table is empty, which cannot be right — an alarm job has" >&2
	echo "alarm: every other job in its file in \`needs:\`, so an empty table means" >&2
	echo "alarm: the table was built wrong rather than that nothing ran." >&2
	exit 2
fi

# And a run in which nothing *succeeded* asserts nothing either. The count above
# is of rows, not of anything having run, so a table of every job `skipped`
# decided `green` — and green closes every open thread. That is the refusal just
# above reached with a table that is not empty: a run reporting no measurement
# must not be the thing that closes a finding. Latent today, because every
# workflow's first job runs unconditionally; it is here so that stays true of
# the alarm and not only of the workflows.
if [ -n "$failed" ]; then
	decision=red
elif [ -n "$cancelled" ] || [ "$succeeded" -eq 0 ]; then
	decision=nothing
else
	decision=green
fi

printf '%s\n' "$decision" > "$out/decision"

# The title is the signature, and it is sorted for a reason that is the whole of
# the second-failure question. The same set of jobs failing on thirty
# consecutive nights has to produce one thread with thirty comments rather than
# thirty issues, and the only thing the caller can match on is the title — so
# the title must not depend on the order the jobs happened to finish in. A
# *different* set of jobs is a different title and a different thread, which is
# right: `rollback` red and `sweep` red are two findings and one of them is
# already fixed on a branch about as often as not.
#
# Three names and a count, because a title is read in a notification list. The
# body carries all of them.
if [ "$decision" = red ]; then
	shown=$(printf '%s\n' "$failed" | head -3 | paste -sd, - | sed 's/,/, /g')
	count=$(printf '%s\n' "$failed" | wc -l | tr -d ' ')
	if [ "$count" -gt 3 ]; then
		title="$workflow is red: $shown, and $((count - 3)) more"
	else
		title="$workflow is red: $shown"
	fi
else
	# The prefix a green run searches for, so that what closes an issue and what
	# opens one cannot spell the signature two different ways.
	title="$workflow is red:"
fi
printf '%s\n' "$title" > "$out/title"

# The first failing line. Every line of the log carries its job and a second
# field before the text, tab-separated (see ALARM_LOG above), and the runner's
# own timestamp after them, so the part worth reading starts after the second
# tab and the stamp — and when there is no tab, the line is already the text.
#
# The patterns are this tree's own failure vocabulary rather than a general one:
# `xtask` prints `FAIL:` and `xtask:` for a refusal, `cargo` prints `error:` and
# `warning:` (which is a failure here, because clippy runs with `-D warnings`),
# and the runner prints `Process completed with exit code` when nothing above
# matched. That last one is the fallback and is deliberately last: it names no
# defect, so a run where it is the headline is a run whose failure this tree has
# no vocabulary for, and that is worth seeing rather than hiding.
#
# The excerpt is taken from the **same job** as the headline, and both say
# which. The log is every failed job's log one after another, so with two
# failures the headline was the first job's line and the excerpt the second
# job's last forty, under a label that said *the failing job* — one job's
# failure and another's context handed over as one. When the log does not say
# which job a line is from, that is said instead of guessed.
strip='s/^[^\t]*\t[^\t]*\t//; s/^[0-9][0-9-]*T[0-9:.]*Z //'
headline=
headline_job=
excerpt=
if [ -f "$log" ]; then
	at=$(sed "$strip" "$log" |
		grep -n -m1 -E '^(FAIL|xtask: |error(\[E[0-9]+\])?:|warning:|thread .* panicked|Error:)' |
		cut -d: -f1 || true)
	if [ -z "$at" ]; then
		at=$(sed "$strip" "$log" | grep -n -m1 -E 'Process completed with exit code' |
			cut -d: -f1 || true)
	fi
	if [ -n "$at" ]; then
		headline=$(sed "$strip" "$log" | sed -n "${at}p")
		headline_job=$(sed -n "${at}p" "$log" | awk -F'\t' 'NF >= 3 { print $1 }')
	fi
	if [ -n "$headline_job" ]; then
		excerpt=$(awk -F'\t' -v job="$headline_job" 'NF >= 3 && $1 == job' "$log" |
			sed "$strip" | tail -40)
	else
		excerpt=$(sed "$strip" "$log" | tail -40)
	fi
fi
if [ -z "$headline" ]; then
	headline="(the log could not be read — open the run)"
fi

body="$out/body.md"

if [ "$decision" = green ]; then
	{
		echo "This workflow is green again, at \`$sha\`."
		echo
		echo "Closed by the run below rather than by a person, because an issue that"
		echo "outlives its defect is worse than no issue: a reader who learns to ignore a"
		echo "stale one has been trained to ignore the mechanism. If this closed something"
		echo "that is not actually fixed, the next red run opens a thread with the same"
		echo "title."
		echo
		echo "Run: $run_url"
	} > "$body"
	exit 0
fi

if [ "$decision" = nothing ]; then
	{
		if [ -n "$cancelled" ]; then
			echo "Cancelled, so this run asserts nothing: no issue opened and none closed."
		else
			echo "Every job was skipped and no job in this run succeeded, so it asserts"
			echo "nothing: no issue opened and none closed."
		fi
		echo
		echo "Run: $run_url"
	} > "$body"
	exit 0
fi

{
	echo "The scheduled workflow \`$workflow\` failed at \`$sha\`."
	echo
	if [ -n "$headline_job" ]; then
		echo "The first failing line, from \`$headline_job\`:"
		echo
	elif [ "$count" -gt 1 ]; then
		echo "The first failing line in the $count failed jobs' logs, concatenated — the"
		echo "log does not say which job it came from:"
		echo
	fi
	echo '```'
	echo "$headline"
	echo '```'
	echo
	echo "| job | result |"
	echo "|---|---|"
	awk -F'\t' 'NF { printf "| `%s` | %s |\n", $1, $2 }' "$jobs"
	echo
	# The paragraph this whole mechanism is for. `docs/postmortem/0003` is a red
	# nightly whose repair had landed on a branch the day before, so the schedule
	# reported a defect nobody could connect to its fix — and the thing that
	# would have connected them is the two lines below. A notification carrying
	# only the failure would reproduce that incident with a faster delivery.
	if [ -n "$head" ] && [ -n "$sha" ] && [ "$head" != "$sha" ]; then
		echo "**\`$branch\` has moved since this run started.** It was at \`$head\` when this"
		echo "was written and the run tested \`$sha\`, so **the repair may already exist**."
		echo "Read the range before filing this as new:"
		echo
		echo '```'
		echo "git log --oneline $sha..$branch"
		echo '```'
		echo
		echo "That is \`docs/postmortem/0003\` exactly: a nightly reported a defect whose fix"
		echo "had been committed the day before, on a branch, and nothing connected the two"
		echo "for a day."
	elif [ -n "$head" ]; then
		echo "\`$branch\` is at \`$head\`, which is the commit this run tested, so this is not"
		echo "\`docs/postmortem/0003\`'s shape: there is no later commit that could already be"
		echo "the repair."
	else
		echo "The head of \`$branch\` could not be read, so whether a repair already exists on"
		echo "it is the first thing to check by hand — that is \`docs/postmortem/0003\`."
	fi
	echo
	if [ -n "$excerpt" ]; then
		if [ -n "$headline_job" ]; then
			echo "<details><summary>the last 40 lines of $headline_job, the job the line above is from</summary>"
		elif [ "$count" -gt 1 ]; then
			echo "<details><summary>the last 40 lines of the failed jobs' logs, concatenated — possibly not the job the line above is from</summary>"
		else
			echo "<details><summary>the last 40 lines of the failing job</summary>"
		fi
		echo
		echo '```'
		echo "$excerpt"
		echo '```'
		echo
		echo "</details>"
		echo
	fi
	echo "Run: $run_url"
	echo
	echo "Opened by \`ops/alarm.sh\` from \`.github/workflows/$workflow.yml\`. A green run of"
	echo "the same workflow closes it. Muting it is a reviewable diff:"
	echo "\`cargo xtask lint-schedules\` refuses a scheduled workflow whose alarm job does"
	echo "not watch every job in its file."
} > "$body"
