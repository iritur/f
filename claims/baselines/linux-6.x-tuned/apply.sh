#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Put this machine into the configuration `claims/0001-ring-submit-latency.toml`
# names as `linux-6.x-tuned`, and say what it could not do.
#
#   sudo F_MEASURED_CPUS=4-15 F_HOUSEKEEPING_CPUS=0-3 ./apply.sh
#   ./apply.sh --dry-run        every write it would make, and no writes
#
# Idempotent, and idempotent by construction rather than by checking first:
# every write is an absolute value, never an increment and never an append, so
# the second run and the tenth produce the state the first one did. The one
# thing it does not write is the kernel command line — see below.
#
# Exit codes, because a caller in a job needs the difference:
#
#   0  the machine is the baseline
#   1  something could not be applied
#   2  every run-time setting is applied and the kernel command line is not.
#      A reboot away from the baseline, and this is the expected result of the
#      first run on a new machine.
#
# What it deliberately does not do:
#
#   **It never edits a bootloader.** Everything in `cmdline.txt` needs a reboot
#   to take effect and a mistake there is a machine that does not come back. So
#   the command line is a *check* that prints the tokens missing and the line to
#   add; a stranger applies it with their own eyes on it. This is the one place
#   where an unattended script would be worth less than a slow one.
#
#   **It never grants a capability to a binary.** SQPOLL's `SQ_AFF` needs
#   CAP_SYS_NICE on the workload; the `setcap` line is printed, because a script
#   that guesses which binary is the workload has granted a capability to the
#   wrong one.
#
#   **It never turns a mitigation off.** `mitigations=auto` is in `cmdline.txt`
#   with the argument beside it.
#
# `verify.sh` beside this file checks everything this sets, and is what `A-04`
# runs to find out whether the machine is still the machine.

set -euo pipefail

PROG=$(basename "$0")
# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

DRY=0
for arg in "$@"; do
	case $arg in
	--dry-run) DRY=1 ;;
	-h | --help)
		sed -n '3,39p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) die "unknown option: $arg" ;;
	esac
done

baseline_preflight
baseline_header "apply"

if [ "$DRY" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
	die "this writes to /proc and /sys — re-run with sudo, or with --dry-run"
fi

# One verb for every write, so that "what did it change" is one grep away in a
# transcript, and so that --dry-run is a single branch rather than a branch per
# setting.
set_value() {
	local what=$1 path=$2 value=$3
	if [ "$DRY" -eq 1 ]; then
		printf '  [w]   %-38s %s <- %s\n' "$what" "$path" "$value"
		return 0
	fi
	if [ ! -w "$path" ]; then
		bad "$what" "$path is not writable"
		return 0
	fi
	printf '%s' "$value" >"$path"
	ok "$what" "$value"
}

# ---------------------------------------------------------------------------
printf 'kernel version\n'

read -r major minor patch <<<"$(kernel_triple)"
if kernel_in_range "$major" "$minor" "$patch"; then
	ok "kernel in range" "$(uname -r)"
else
	die "$(uname -r) is outside [$(conf_get baseline.conf kernel_min), $(conf_get baseline.conf kernel_below)).
baseline.conf says why the range is a range; the README says what to do about a
kernel outside it, which is a new versioned directory and not a wider range here."
fi

# ---------------------------------------------------------------------------
printf '\nkernel command line — checked, never edited\n'

running=" $(tr -s ' \t' '  ' </proc/cmdline) "
missing=()
while read -r token; do
	[ -n "$token" ] || continue
	case "$running" in
	*" $token "*) ;;
	*) missing+=("$token") ;;
	esac
done < <(cmdline_tokens)

reboot_owed=0
if [ "${#missing[@]}" -eq 0 ]; then
	ok "kernel command line" "every token in cmdline.txt is live"
else
	reboot_owed=1
	bad "kernel command line" "${#missing[@]} token(s) missing"
	for token in "${missing[@]}"; do note "missing  $token"; done
	note ""
	note "Add them, on one line, to GRUB_CMDLINE_LINUX_DEFAULT in /etc/default/grub,"
	note "then grub-mkconfig -o /boot/grub/grub.cfg and reboot:"
	note ""
	note "  ${missing[*]}"
	note ""
	note "Read it before pasting it. This is the only step here that can leave the"
	note "machine unbootable, which is why it is a message and not a write."
fi

# ---------------------------------------------------------------------------
printf '\nsysctl\n'

while read -r key value; do
	[ -n "$key" ] || continue
	path=/proc/sys/${key//./\/}
	if [ ! -e "$path" ]; then
		if sysctl_optional "$key"; then
			ok "$key" "absent — the feature is not built into this kernel"
		else
			bad "$key" "no such sysctl on this kernel"
		fi
		continue
	fi
	set_value "$key" "$path" "$value"
done < <(sysctl_pairs)

# ---------------------------------------------------------------------------
printf '\ncpufreq\n'

governor=$(conf_get baseline.conf governor)
policies=(/sys/devices/system/cpu/cpufreq/policy*/scaling_governor)
if [ ! -e "${policies[0]}" ]; then
	bad "cpufreq governor" "no cpufreq policy in sysfs"
	note "intel_pstate=disable is in cmdline.txt and hands frequency to acpi-cpufreq."
	note "With intel_pstate still driving, there is nothing here to set and the"
	note "governor named in baseline.conf is not the one in force."
else
	for policy in "${policies[@]}"; do
		set_value "governor $(basename "$(dirname "$policy")")" "$policy" "$governor"
	done
fi

# ---------------------------------------------------------------------------
printf '\ntransparent huge pages — the baseline half only, see baseline.conf\n'

thp=/sys/kernel/mm/transparent_hugepage
if [ -d "$thp" ]; then
	set_value "thp enabled" "$thp/enabled" "$(conf_get baseline.conf thp_enabled)"
	set_value "thp defrag" "$thp/defrag" "$(conf_get baseline.conf thp_defrag)"
	set_value "khugepaged defrag" "$thp/khugepaged/defrag" \
		"$(conf_get baseline.conf khugepaged_defrag)"
else
	bad "transparent huge pages" "$thp does not exist — CONFIG_TRANSPARENT_HUGEPAGE is off"
fi

# ---------------------------------------------------------------------------
printf '\ninterrupts\n'

if [ "$(conf_get baseline.conf irqbalance)" = "off" ]; then
	if command -v systemctl >/dev/null 2>&1 &&
		systemctl list-unit-files irqbalance.service >/dev/null 2>&1; then
		if [ "$DRY" -eq 1 ]; then
			printf '  [w]   %-38s %s\n' "irqbalance" "systemctl disable --now irqbalance"
		elif systemctl disable --now irqbalance.service >/dev/null 2>&1 &&
			! systemctl is-active --quiet irqbalance.service; then
			ok "irqbalance" "stopped and disabled"
		else
			bad "irqbalance" "still active"
		fi
	else
		ok "irqbalance" "not installed"
	fi
fi

# Every interrupt whose affinity a script may set, set to the housekeeping set.
# The ones that refuse are the managed ones — a driver owns their affinity and
# the write returns EIO — and `isolcpus=managed_irq` in cmdline.txt is what
# keeps *those* off the measured cores. Two mechanisms, because there are two
# kinds of interrupt and neither mechanism covers the other kind.
moved=0
refused=0
for irq in /proc/irq/[0-9]*; do
	[ -w "$irq/smp_affinity_list" ] || continue
	if [ "$DRY" -eq 1 ]; then
		moved=$((moved + 1))
	elif printf '%s' "$HOUSEKEEPING" >"$irq/smp_affinity_list" 2>/dev/null; then
		moved=$((moved + 1))
	else
		refused=$((refused + 1))
	fi
done
ok "irq affinity" "$moved on $HOUSEKEEPING, $refused managed by a driver"

# ---------------------------------------------------------------------------
printf '\nio_uring and the workload policy — stated, not started\n'

note "io_uring is enabled by kernel.io_uring_disabled = 0 above, and SQPOLL with it."
note "Pinning the poll thread (IORING_SETUP_SQ_AFF) needs CAP_SYS_NICE on the workload:"
note ""
note "  setcap cap_sys_nice,cap_ipc_lock+ep <the baseline workload binary>"
note ""
note "Launch it under the policy baseline.conf chose, on a measured core:"
note ""
note "  chrt --fifo $(conf_get baseline.conf sched_priority) \\"
note "    taskset --cpu-list <one core from $MEASURED> <the baseline workload binary>"
note ""
note "Neither is run here: this script was not told which binary is the workload,"
note "and a capability granted to a guess is a capability granted to the wrong file."

# ---------------------------------------------------------------------------
printf '\n  %d applied, %d outstanding\n' "$BASELINE_OK" "$BASELINE_BAD"

if [ "$DRY" -eq 1 ]; then
	printf '\n  dry run: nothing was written.\n'
	exit 0
fi

if [ "$BASELINE_BAD" -eq 0 ]; then
	printf '\n  Now run ./verify.sh, which asks the same questions from the other side.\n'
	exit 0
fi

if [ "$reboot_owed" -eq 1 ] && [ "$BASELINE_BAD" -eq 1 ]; then
	printf '\n  Every run-time setting is applied and the kernel command line is not.\n'
	printf '  Reboot with the tokens above, then run ./verify.sh.\n'
	exit 2
fi

printf '\n  outstanding: %s\n' "${BASELINE_DRIFT[*]}"
printf '  This machine is not the baseline, and nothing measured on it is a comparison.\n'
exit 1
