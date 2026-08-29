#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# PreToolUse on Write and Edit: keep credentials out of the diff.
#
# This greps the whole hook payload rather than extracting the content field,
# because extraction can be defeated by escaping and grepping cannot. The cost
# is that a credential-shaped string in a *file path* also trips it, which has
# never happened and would be worth noticing anyway.
#
# The patterns are the shapes that are unambiguous. A hook that guesses is a
# hook people disable, so anything genuinely ambiguous — a variable named
# `secret`, a base64 blob — is left to REVIEW.md pass 3.

set -u
. "$(dirname "$0")/lib.sh"

payload=$(hook_payload)

# `--` because a pattern may start with a dash and grep would read it as an
# option.
check() {
	if printf '%s' "$payload" | grep -Eq -- "$1"; then
		printf '%s\n' \
			"Blocked: this edit looks like it contains $2." \
			"Credentials do not go in the tree, in tests, or in fixtures. If this is a real secret it" \
			"belongs in the environment or in the CI secret store; if it is a placeholder, make it" \
			"obviously fake — REDACTED, or a value too short to be real." \
			"If this is a false positive, say what the string is and let a person decide." >&2
		exit 2
	fi
}

check '-----BEGIN [A-Z ]*PRIVATE KEY-----' 'a private key'
check 'AKIA[0-9A-Z]{16}' 'an AWS access key id'
check 'gh[pousr]_[A-Za-z0-9]{20,}' 'a GitHub token'
check 'sk-ant-[A-Za-z0-9_-]{20,}' 'an Anthropic API key'
check 'xox[abprs]-[A-Za-z0-9-]{10,}' 'a Slack token'
check '(password|passwd|secret|api_key|apikey|token)[[:space:]]*[:=][[:space:]]*[^A-Za-z0-9]{0,3}[A-Za-z0-9/+_-]{12,}' \
	'a hardcoded credential'

exit 0
