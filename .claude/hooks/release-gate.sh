#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# PreToolUse on Bash: the approval gate.
#
# This repository has no production service, so "production" here means the
# operations that leave it and cannot be taken back: a pushed tag, a published
# crate, a pushed image, a rewritten history on a shared branch. Those are the
# release boundary, and the gate is that an agent does not cross it on its own
# authority.
#
# The route through is an environment variable naming the authorisation:
#
#   F_RELEASE_AUTHORIZATION="release 0.1.0, approved by <name>, <date>"
#
# It is deliberately not a flag the agent can pass and not a file it can write.
# A person exports it in the shell where the release is being made, which is the
# same person who will be asked about it afterwards. Every block is printed with
# the command it stopped, so the transcript carries the record.
#
# Rewriting history and deleting refs have no route through at all: there is no
# authorisation under which an agent should run them, and the correct escalation
# is a human at a terminal.

set -u
. "$(dirname "$0")/lib.sh"

payload=$(hook_payload)
cmd=$(json_field command "$payload")
[ -n "$cmd" ] || exit 0

authorised=${F_RELEASE_AUTHORIZATION:-}

gate() {
	if [ -z "$authorised" ]; then
		deny "Blocked: $1." \
			"" \
			"  $cmd" \
			"" \
			"This crosses the release boundary and needs a named authorisation. A person exports" \
			"F_RELEASE_AUTHORIZATION in the shell where the release is made:" \
			"" \
			"  export F_RELEASE_AUTHORIZATION=\"release <version>, approved by <name>, <date>\"" \
			"" \
			"Ask for it. Do not set it yourself — a gate the gated party can open is a log entry, not a" \
			"gate. docs/sdlc.md, stage 5."
	fi
	printf 'release gate: allowed under authorisation "%s"\n' "$authorised" >&2
}

case "$cmd" in
*"git push"*--force* | *"git push"*-f\ * | *"git push --force-with-lease"*)
	deny "Blocked: force-push." \
		"" \
		"  $cmd" \
		"" \
		"There is no authorisation under which an agent rewrites a shared branch. If history has to" \
		"change, a person does it at a terminal, having read what is about to be lost."
	;;
*"git reset --hard"* | *"git clean -"*d*f* | *"git branch -D"* | *"git push"*--delete*)
	deny "Blocked: this destroys work that has no other copy." \
		"" \
		"  $cmd" \
		"" \
		"Say what you wanted to discard and let a person run it. If the goal is a clean tree, 'git" \
		"stash' keeps the option of being wrong about that."
	;;
*"cargo publish"*)
	gate "cargo publish sends this crate to a registry"
	;;
*"git push"*tag* | *"git push"*--tags* | *"git tag"*-s* | *"git tag"*-a*)
	gate "pushing a tag is what makes a release exist"
	;;
*"docker push"* | *"podman push"*)
	gate "pushing an image publishes it"
	;;
*--no-verify*)
	deny "Blocked: --no-verify skips the hooks that are the point." \
		"" \
		"  $cmd" \
		"" \
		"If a pre-commit hook is failing, the failure is the finding. Fix it, or say what is wrong with" \
		"the hook."
	;;
esac

exit 0
