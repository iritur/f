#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Ask this machine, one setting at a time, whether it is still the baseline.
#
#   F_MEASURED_CPUS=4-15 F_HOUSEKEEPING_CPUS=0-3 ./verify.sh
#
# Exits 0 when every setting `apply.sh` makes is in force, and non-zero naming
# each one that is not. Needs no privilege: everything it reads is world
# readable, so it can run from a job, from a cron entry, and from whoever is
# standing in front of the machine wondering why a number moved.
#
# This is the half of the directory that `A-04` runs — *re-tune every claim's
# baseline, or the tuned-Linux comparison quietly becomes a stock-Linux
# comparison* — and the reason it exists separately from `apply.sh` is that
# applying and checking fail differently. A machine drifts by a distribution
# upgrade re-enabling irqbalance, by a firmware update resetting a governor, by
# somebody debugging something at 2 a.m. and putting back four settings out of
# five. None of those is a failure of `apply.sh`; all of them are a number that
# is no longer a comparison, and a script that only applies would report them
# as a successful run.
#
# It checks one thing `apply.sh` does not: that the prose copy of the kernel
# command line in `claims/runner-class-A.md` still says what `cmdline.txt`
# says, when the checkout is reachable from here. Two copies of one list is the
# decay this directory was written against, and the only defence against it is
# a check that reads both.

set -euo pipefail

PROG=$(basename "$0")
# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

for arg in "$@"; do
	case $arg in
	-h | --help)
		sed -n '3,28p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) die "unknown option: $arg" ;;
	esac
done

baseline_preflight
baseline_header "verify"

# The value sysfs currently holds for a bracketed multiple-choice file —
# `always [madvise] never` — and the whole line for everything else.
sysfs_choice() {
	local text
	text=$(cat "$1")
	case $text in
	*'['*) printf '%s\n' "$text" | sed 's/.*\[\([^]]*\)\].*/\1/' ;;
	*) printf '%s\n' "$text" ;;
	esac
}

expect() {
	local what=$1 want=$2 have=$3
	if [ "$want" = "$have" ]; then
		ok "$what" "$have"
	else
		bad "$what" "want $want, have $have"
	fi
}

# ---------------------------------------------------------------------------
printf 'kernel version\n'

read -r major minor patch <<<"$(kernel_triple)"
if kernel_in_range "$major" "$minor" "$patch"; then
	ok "kernel in range" "$(uname -r)"
else
	bad "kernel in range" "$(uname -r) is outside \
[$(conf_get baseline.conf kernel_min), $(conf_get baseline.conf kernel_below))"
	note "A kernel outside the range is not a drifted machine, it is a different"
	note "baseline. The README's reversal condition says what to do: a new versioned"
	note "directory beside this one, never a widened range inside it."
fi

# ---------------------------------------------------------------------------
printf '\nkernel command line\n'

running=" $(tr -s ' \t' '  ' </proc/cmdline) "
while read -r token; do
	[ -n "$token" ] || continue
	case "$running" in
	*" $token "*) ok "cmdline $token" ;;
	*) bad "cmdline $token" "not in /proc/cmdline" ;;
	esac
done < <(cmdline_tokens)

# The kernel's own answer about which cores it isolated, rather than the
# parameter that asked it to. A typo in `isolcpus=` is accepted silently at
# boot and leaves an empty isolated set, which is the one failure here that
# produces a machine that looks configured and measures nothing.
isolated=/sys/devices/system/cpu/isolated
if [ -r "$isolated" ]; then
	expect "cores the kernel isolated" "$(cpu_expand "$MEASURED")" \
		"$(cpu_expand "$(cat "$isolated")")"
else
	bad "cores the kernel isolated" "$isolated is not readable"
fi

# ---------------------------------------------------------------------------
printf '\nsysctl\n'

while read -r key value; do
	[ -n "$key" ] || continue
	path=/proc/sys/${key//./\/}
	if [ ! -r "$path" ]; then
		if sysctl_optional "$key"; then
			ok "$key" "absent — the feature is not built into this kernel"
		else
			bad "$key" "no such sysctl on this kernel"
		fi
		continue
	fi
	expect "$key" "$value" "$(tr -d '\t\n' <"$path")"
done < <(sysctl_pairs)

# ---------------------------------------------------------------------------
printf '\ncpufreq\n'

governor=$(conf_get baseline.conf governor)
policies=(/sys/devices/system/cpu/cpufreq/policy*/scaling_governor)
if [ ! -e "${policies[0]}" ]; then
	bad "cpufreq governor" "no cpufreq policy in sysfs"
else
	for policy in "${policies[@]}"; do
		expect "governor $(basename "$(dirname "$policy")")" "$governor" "$(cat "$policy")"
	done
fi

# ---------------------------------------------------------------------------
printf '\ntransparent huge pages\n'

thp=/sys/kernel/mm/transparent_hugepage
if [ -d "$thp" ]; then
	expect "thp enabled" "$(conf_get baseline.conf thp_enabled)" "$(sysfs_choice "$thp/enabled")"
	expect "thp defrag" "$(conf_get baseline.conf thp_defrag)" "$(sysfs_choice "$thp/defrag")"
	expect "khugepaged defrag" "$(conf_get baseline.conf khugepaged_defrag)" \
		"$(cat "$thp/khugepaged/defrag")"
else
	bad "transparent huge pages" "$thp does not exist"
fi

# ---------------------------------------------------------------------------
printf '\nhuge page pool\n'

# The size and the count come out of `cmdline.txt` rather than out of a second
# constant here: `hugepagesz=1G hugepages=16` is already a statement of both,
# and a check with its own copy of a number checks that the copy is intact.
want_size=$(cmdline_tokens | sed -n 's/^hugepagesz=\(.*\)$/\1/p' | head -n 1)
want_count=$(cmdline_tokens | sed -n 's/^hugepages=\(.*\)$/\1/p' | head -n 1)
case $want_size in
*G) want_kib=$(( ${want_size%G} * 1024 * 1024 )) ;;
*M) want_kib=$(( ${want_size%M} * 1024 )) ;;
*) want_kib=0 ;;
esac
expect "hugepage size, KiB" "$want_kib" \
	"$(awk '/^Hugepagesize:/ { print $2 }' /proc/meminfo)"
expect "hugepages reserved, pages" "$want_count" \
	"$(awk '/^HugePages_Total:/ { print $2 }' /proc/meminfo)"

swap=$(awk '/^SwapTotal:/ { print $2 }' /proc/meminfo)
expect "swap, KiB" "0" "$swap"

# ---------------------------------------------------------------------------
printf '\ninterrupts\n'

# Two questions, because `apply.sh` answers two — `systemctl disable --now` is
# a stop and an unmask of the next boot, and a machine that is stopped but
# still enabled passes the first and fails the measurement after the next
# reboot. Reported separately rather than folded together: "it is running" and
# "it will be running" need different repairs, and the second is the one a
# distribution upgrade re-introduces silently, which is the first drift this
# file's header names.
if command -v systemctl >/dev/null 2>&1; then
	if systemctl is-active --quiet irqbalance.service; then
		bad "irqbalance running" "it will undo every affinity below, on its own schedule"
	else
		ok "irqbalance running" "no"
	fi
	if systemctl is-enabled --quiet irqbalance.service 2>/dev/null; then
		bad "irqbalance enabled" "it starts at the next boot and undoes every affinity below"
	else
		ok "irqbalance enabled" "no"
	fi
else
	ok "irqbalance" "no systemctl on this machine"
fi

# An interrupt whose *effective* affinity includes a measured core is an
# interrupt that will be taken on a measured core. `smp_affinity_list` is what
# was requested; this is what the interrupt controller did with the request,
# and on a machine with more CPUs than the controller can address in one mask
# the two differ routinely.
stray=""
for irq in /proc/irq/[0-9]*; do
	mask=$irq/effective_affinity_list
	[ -r "$mask" ] || mask=$irq/smp_affinity_list
	[ -r "$mask" ] || continue
	for cpu in $(cpu_expand "$(cat "$mask" 2>/dev/null || echo)"); do
		case " $(cpu_expand "$MEASURED") " in
		*" $cpu "*) stray="$stray $(basename "$irq")" ;;
		esac
	done
done
if [ -z "$stray" ]; then
	ok "irq affinity" "no interrupt is routed to a measured core"
else
	bad "irq affinity" "routed to a measured core: \
$(printf '%s\n' $stray | sort -u -n | tr '\n' ' ')"
fi

# Delivered counts, as a note rather than as drift: `/proc/interrupts` counts
# since boot, so a non-zero column for a measured core is as likely to be the
# minute before `apply.sh` ran as it is to be now. It is here because it is the
# only evidence in this file about what actually happened rather than what is
# configured, and a reader chasing a tail should look at it.
note "interrupts delivered to measured cores, since boot — not drift, evidence:"
awk -v cpus="$(cpu_expand "$MEASURED")" '
	NR == 1 { for (i = 1; i <= NF; i++) column[$i] = i; next }
	{
		total = 0
		split(cpus, want, " ")
		for (w in want) { c = column["CPU" want[w]]; if (c != "") total += $(c + 1) }
		if (total > 0) printf "          %-12s %d\n", $1, total
	}
' /proc/interrupts || true

# ---------------------------------------------------------------------------
printf '\nthe other copy of the command line\n'

prose=$BASELINE_DIR/../../runner-class-A.md
if [ -r "$prose" ]; then
	drifted=""
	while read -r token; do
		[ -n "$token" ] || continue
		key=${token%%=*}
		grep -q -- "$key" "$prose" || drifted="$drifted $key"
	done < <(cmdline_tokens)
	if [ -z "$drifted" ]; then
		ok "claims/runner-class-A.md" "names every parameter cmdline.txt requires"
	else
		bad "claims/runner-class-A.md" "does not mention:$drifted"
		note "One of the two copies was edited and the other was not. cmdline.txt is the"
		note "copy that runs, so it is the one to trust and the prose is the one to fix —"
		note "unless the edit was deliberate, in which case it is a new baseline directory."
	fi
else
	note "no checkout reachable from here, so the prose copy was not checked."
	note "Run this from a clone when you want that comparison: $prose"
fi

# ---------------------------------------------------------------------------
printf '\n  %d in force, %d drifted\n' "$BASELINE_OK" "$BASELINE_BAD"

if [ "$BASELINE_BAD" -eq 0 ]; then
	printf '\n  This machine is the baseline claims/0001 names.\n'
	exit 0
fi

printf '\n  drifted:\n'
for what in "${BASELINE_DRIFT[@]}"; do printf '    %s\n' "$what"; done
printf '\n  A number taken here is not a comparison against linux-6.x-tuned. Run\n'
printf '  ./apply.sh, or record the drift with the number and say which it was.\n'
exit 1
