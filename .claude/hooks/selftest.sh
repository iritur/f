#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The hooks are the part of the agent configuration that does not depend on
# being read, which makes them the part most worth testing — and the part most
# likely to rot silently, because a hook that stops firing looks exactly like a
# hook that has nothing to complain about.
#
# Each case feeds one hook the payload shape Claude Code sends and asserts the
# exit code: 0 allows, 2 blocks. Run by `cargo xtask lint`'s CI job.
#
#   bash .claude/hooks/selftest.sh

set -u
cd "$(dirname "$0")"

fail=0
n=0

# case <expected-exit> <script> <payload>
case_() {
	n=$((n + 1))
	got=$(printf '%s' "$3" | bash "./$2" 2>&1 >/dev/null)
	code=$?
	if [ "$code" != "$1" ]; then
		printf 'FAIL  %-22s expected exit %s, got %s\n' "$2" "$1" "$code"
		printf '      payload: %s\n' "$3"
		[ -n "$got" ] && printf '      said:    %s\n' "$(printf '%s' "$got" | head -n 1)"
		fail=$((fail + 1))
	else
		printf 'ok    %-22s exit %s\n' "$2" "$code"
	fi
}

echo "protected paths"
case_ 2 protected-paths.sh '{"tool_name":"Edit","tool_input":{"file_path":"third_party/driver.c"}}'
case_ 2 protected-paths.sh '{"tool_name":"Write","tool_input":{"file_path":"rust-toolchain.toml"}}'
case_ 2 protected-paths.sh '{"tool_name":"Edit","tool_input":{"file_path":"Cargo.lock"}}'
case_ 2 protected-paths.sh '{"tool_name":"Write","tool_input":{"file_path":"target/debug/x"}}'
case_ 0 protected-paths.sh '{"tool_name":"Edit","tool_input":{"file_path":"ring/src/lib.rs"}}'
case_ 0 protected-paths.sh '{"tool_name":"Write","tool_input":{"file_path":"docs/rfc/0012-x.md"}}'

echo
echo "credentials"
case_ 2 no-credentials.sh '{"tool_input":{"content":"-----BEGIN RSA PRIVATE KEY-----"}}'
case_ 2 no-credentials.sh '{"tool_input":{"content":"AKIAIOSFODNN7EXAMPLE"}}'
case_ 2 no-credentials.sh '{"tool_input":{"content":"password = \"correcthorsebattery\""}}'
case_ 0 no-credentials.sh '{"tool_input":{"content":"let token = env::var(\"F_TOKEN\")?;"}}'
case_ 0 no-credentials.sh '{"tool_input":{"content":"// the token is passed by value"}}'

echo
echo "determinism"
case_ 2 determinism-guard.sh '{"tool_input":{"file_path":"ring/src/lib.rs","new_string":"HashMap::new()"}}'
case_ 2 determinism-guard.sh '{"tool_input":{"file_path":"env/src/sim.rs","new_string":"Instant::now()"}}'
case_ 0 determinism-guard.sh '{"tool_input":{"file_path":"bench/src/lib.rs","new_string":"Instant::now()"}}'
case_ 0 determinism-guard.sh '{"tool_input":{"file_path":"ring/src/lib.rs","new_string":"BTreeMap::new()"}}'
case_ 0 determinism-guard.sh '{"tool_input":{"file_path":"docs/rfc/0004.md","new_string":"HashMap::new()"}}'

echo
echo "tests hold"
case_ 2 tests-hold.sh '{"tool_input":{"file_path":"ring/src/lib.rs","new_string":"#[ignore]\nfn t(){}"}}'
case_ 2 tests-hold.sh '{"tool_input":{"file_path":"abi/src/lib.rs","new_string":"assert!(true);"}}'
case_ 2 tests-hold.sh '{"tool_input":{"file_path":"ring/tests/litmus.rs","new_string":"const ITERS: u32 = 10;"}}'
case_ 0 tests-hold.sh '{"tool_input":{"file_path":"ring/src/lib.rs","new_string":"assert_eq!(a, b);"}}'
case_ 0 tests-hold.sh '{"tool_input":{"file_path":"README.md","new_string":"#[ignore] is not allowed"}}'

echo
echo "release gate"
case_ 2 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"cargo publish"}}'
case_ 2 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}'
case_ 2 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"git reset --hard HEAD~3"}}'
case_ 2 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"git commit --no-verify -m x"}}'
case_ 2 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"git push origin --tags"}}'
case_ 0 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"cargo xtask verify"}}'
case_ 0 release-gate.sh '{"tool_name":"Bash","tool_input":{"command":"git push origin feature-branch"}}'

echo
if [ "$fail" = 0 ]; then
	printf '%s cases, all green\n' "$n"
	exit 0
fi
printf '%s of %s cases failed\n' "$fail" "$n"
exit 1
