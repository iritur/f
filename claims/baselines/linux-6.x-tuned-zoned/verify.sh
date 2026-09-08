#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Ask this machine, one setting at a time, whether it is the baseline
# `claims/0016-write-amplification.toml` compares against.
#
#   F_ZONED_DEV=/dev/vdb F_MOUNT=/mnt/baseline ./verify.sh
#
# Exits 0 when every setting `apply.sh` makes is in force, and non-zero naming
# each one that is not. Needs no privilege: everything it reads is world
# readable, so it runs from a job, from a cron entry, and from whoever is
# standing in front of the machine wondering why a ratio moved.
#
# It is the half of the directory `A-04` runs — *re-tune every claim's baseline,
# or the tuned-Linux comparison quietly becomes a stock-Linux comparison*.
#
# It checks three things `apply.sh` does not, and each is a copy of a fact that
# is also written down somewhere else:
#
#   1. that `claims/0016-write-amplification.toml` still points at this
#      directory. A baseline nothing cites is a directory of notes.
#   2. that `comparison.conf`'s workload still agrees with the constants in
#      `zone/tests/cycle.rs`. The workload is the half of this baseline most
#      likely to drift without anybody noticing, because it drifts when a *test*
#      is edited rather than when a baseline is.
#   3. that the emulator, if one is reachable, is new enough to present a zoned
#      virtio-blk at all.
#
# On a machine with no zoned device — which is every machine this project has
# had — it prints a full report saying so and exits non-zero. That is the
# intended output there, not a failure of the script.

set -euo pipefail

PROG=$(basename "$0")
# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

for arg in "$@"; do
	case $arg in
	-h | --help)
		sed -n '3,30p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) die "unknown option: $arg" ;;
	esac
done

zoned_preflight
zoned_header "verify"

# The value sysfs currently holds for a bracketed multiple-choice file —
# `[mq-deadline] none` — and the whole line for everything else.
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
	bad "kernel in range" "$(uname -r), want >= $(conf_get device.conf kernel_min) and < $(conf_get device.conf kernel_below)"
	note "Not a drifted machine — a different baseline. See the README's reversal condition."
fi

# ---------------------------------------------------------------------------
printf '\nthe device\n'

if [ "$DEVICE_PRESENT" -eq 0 ]; then
	bad "$ZONED_DEV" "not a block device on this machine"
	note "Every check below that reads the device is skipped rather than reported as a"
	note "drifted value, because a missing file and a wrong value are different findings"
	note "and only one of them is about tuning."
else
	queue=/sys/block/$DEVNAME/queue

	if [ -r "$queue/zoned" ]; then
		expect "queue/zoned" "$(conf_get device.conf model)" "$(cat "$queue/zoned")"
	else
		bad "queue/zoned" "$queue/zoned is unreadable"
	fi

	# The zone size, as the block layer reports it: `chunk_sectors` in 512-byte
	# sectors, whatever the device's own logical block turns out to be.
	if [ -r "$queue/chunk_sectors" ]; then
		expect "zone size (bytes)" "$(conf_get device.conf zone_size_bytes)" \
			"$(($(cat "$queue/chunk_sectors") * 512))"
	else
		bad "zone size" "$queue/chunk_sectors is unreadable"
	fi

	if [ -r "$queue/nr_zones" ]; then
		expect "zones" "$(conf_get device.conf zones)" "$(cat "$queue/nr_zones")"
	else
		bad "zones" "$queue/nr_zones is unreadable"
	fi

	for name in logical_block_size max_open_zones max_active_zones; do
		case $name in
		logical_block_size) want=$(conf_get device.conf logical_block_bytes) ;;
		max_open_zones) want=$(conf_get device.conf max_open_zones) ;;
		max_active_zones) want=$(conf_get device.conf max_active_zones) ;;
		esac
		if [ -r "$queue/$name" ]; then
			expect "queue/$name" "$want" "$(cat "$queue/$name")"
		else
			bad "queue/$name" "unreadable"
		fi
	done

	# The conventional zones the baseline's metadata lives in. There is no sysfs
	# attribute for the count, so this needs `blkzone`; a machine without it is
	# unchecked rather than drifted, and says so.
	if command -v blkzone >/dev/null 2>&1; then
		report=$(blkzone report "$ZONED_DEV" 2>/dev/null || true)
		conv=$(printf '%s\n' "$report" | grep -c 'CONVENTIONAL' || true)
		expect "conventional zones" "$(conf_get device.conf conventional_zones)" "$conv"

		# The one geometry row sysfs cannot answer. A zone whose capacity is
		# below its size is a legitimate device and a different denominator —
		# every zone carries a hole neither mapping chose — so `device.conf`
		# fixes them equal and this is where that is asked of a real device.
		# `blkzone` prints capacity in 512-byte sectors as `cap 0x…`.
		cap=$(printf '%s\n' "$report" |
			sed -n 's/.*cap \(0x[0-9a-f]*\).*/\1/p' | sort -u | head -n 1)
		if [ -n "$cap" ]; then
			expect "zone capacity (bytes)" "$(conf_get device.conf zone_capacity_bytes)" \
				"$((cap * 512))"
		else
			note "blkzone reported no zone capacity, so it was not checked."
		fi
	else
		note "blkzone is not installed, so the conventional zone count was not checked."
		note "It is the region f2fs keeps its checkpoint, NAT and SIT in; a device with"
		note "fewer than device.conf names is one mkfs.f2fs will refuse or crowd."
	fi
fi

# ---------------------------------------------------------------------------
printf '\nthe runtime knobs\n'

if [ "$DEVICE_PRESENT" -eq 0 ]; then
	note "skipped: every path under sysfs.conf is derived from the device name."
else
	while read -r key want; do
		path=$(sysfs_path "$key")
		if [ -r "$path" ]; then
			expect "$key" "$want" "$(sysfs_choice "$path")"
		else
			bad "$key" "$path is unreadable"
			case $key in
			f2fs_*)
				note "An f2fs knob with no file is usually a device with no f2fs mounted on"
				note "it, rather than a kernel without the knob."
				;;
			esac
		fi
	done < <(sysfs_pairs)
fi

# ---------------------------------------------------------------------------
printf '\nthe filesystem\n'

want_fs=$(conf_get filesystem.conf fs)
mounted=$(awk -v m="$MOUNT" '$2 == m { print $3 " " $4 }' /proc/mounts | head -n 1)

if [ -z "$mounted" ]; then
	bad "$MOUNT" "nothing is mounted there"
	note "apply.sh --format creates and mounts it. It will not do so without that flag,"
	note "because a baseline script that silently runs mkfs on a named block device is a"
	note "footgun this directory is not willing to ship."
else
	have_fs=${mounted%% *}
	have_opts=${mounted#* }
	expect "filesystem type" "$want_fs" "$have_fs"

	IFS=, read -r -a wanted <<<"$(conf_get filesystem.conf mount_opts)"
	for opt in "${wanted[@]}"; do
		case ",$have_opts," in
		*",$opt,"*) ok "mount $opt" ;;
		*) bad "mount $opt" "not in $have_opts" ;;
		esac
	done

	# The two that must be absent. Each would let the baseline promise less than
	# F promises, which is the cheapest way there is to win this ratio
	# dishonestly — so their absence is checked rather than assumed, in the one
	# place a machine can be asked.
	for forbidden in compress nobarrier; do
		case ",$have_opts," in
		*",$forbidden"*) bad "mount without $forbidden" "$have_opts" ;;
		*) ok "mount without $forbidden" ;;
		esac
	done
fi

# ---------------------------------------------------------------------------
printf '\nsysctls\n'

while read -r key want; do
	path=/proc/sys/${key//./\/}
	if [ -r "$path" ]; then
		expect "$key" "$want" "$(cat "$path")"
	else
		bad "$key" "the kernel does not have it"
	fi
done < <(sysctl_pairs)

# ---------------------------------------------------------------------------
printf '\nthe emulator\n'

if command -v qemu-system-x86_64 >/dev/null 2>&1; then
	have_qemu=$(qemu-system-x86_64 --version | sed -n 's/^QEMU emulator version \([0-9.]*\).*/\1/p')
	want_qemu=$(conf_get device.conf qemu_min)
	if [ "$(printf '%s\n%s\n' "$want_qemu" "$have_qemu" | sort -V | head -n 1)" = "$want_qemu" ]; then
		ok "qemu >= $want_qemu" "$have_qemu"
	else
		bad "qemu >= $want_qemu" "$have_qemu"
		note "Below $want_qemu there is no zoned virtio-blk to present, so neither half of"
		note "claims/0016 can be run whatever this machine's tuning says. docker/README.md"
		note "names the one-line image base change."
	fi
else
	note "no qemu on PATH, so the emulator was not checked. This matters only on the"
	note "host that runs the two guests; inside one of them it is expected."
fi

# ---------------------------------------------------------------------------
printf '\nthe other copies\n'

# 1. The claim still points here. Two directories and one claim is the ordinary
#    state after a re-tune, and the failure it produces is silent: the new
#    directory is verified and the old one is compared against.
#    The row and not the file: a first draft of this check grepped the whole
#    claim for the directory's name and passed against a *sentence* in
#    `[baseline] notes` that named the directory as owed. A check that a
#    paragraph mentions a path is not a check that the claim compares against it,
#    and it passed before the claim had been edited at all — which is the exact
#    class of green this directory exists to make impossible.
claim=$BASELINE_DIR/../../0016-write-amplification.toml
if [ -r "$claim" ]; then
	if grep -Eq "^path[[:space:]]*=[[:space:]]*\"claims/baselines/$(basename "$BASELINE_DIR")\"" "$claim"; then
		ok "claims/0016 [baseline] path" "names this directory"
	else
		bad "claims/0016 [baseline] path" "does not name this directory"
		note "Either the claim moved to a re-tuned baseline and this one is now historical,"
		note "or somebody verified a directory nothing compares against."
	fi
else
	note "no checkout reachable from here, so the claim was not checked: $claim"
fi

# 2. The workload still agrees with the modelled cycle it was taken from.
cycle=$BASELINE_DIR/../../../zone/tests/cycle.rs
if [ -r "$cycle" ]; then
	shared=0
	for pair in \
		"OBJECT_MIN_BYTES object_min_bytes" \
		"OBJECT_MAX_BYTES object_max_bytes" \
		"KEEP_EVERY retention_keep_every" \
		"DEFAULT_SEED seed"; do
		set -- $pair
		if have=$(rust_const "$cycle" "$1"); then
			expect "cycle.rs $1" "$(conf_get comparison.conf "$2")" "$have"
			shared=$((shared + 1))
		else
			bad "cycle.rs $1" "not found, or not an integer this reader evaluates"
		fi
	done
	[ "$shared" -eq 0 ] || note "$shared workload constant(s) are written in two files and compared here."
	note "BLOCK_BYTES is deliberately NOT one of them: the modelled cycle uses 512 and"
	note "device.conf uses 4096, and the difference is argued in device.conf rather than"
	note "being a drift. It is also why the modelled fill term of 1.0111 is not the fill"
	note "term this device would produce."
else
	note "no checkout reachable from here, so the workload constants were not checked."
fi

# ---------------------------------------------------------------------------
printf '\n  %d in force, %d drifted\n' "$BASELINE_OK" "$BASELINE_BAD"

if [ "$BASELINE_BAD" -eq 0 ]; then
	printf '\n  This machine is the baseline claims/0016 names.\n'
	exit 0
fi

printf '\n  drifted:\n'
for what in "${BASELINE_DRIFT[@]}"; do printf '    %s\n' "$what"; done
printf '\n  A ratio taken here is not a comparison against linux-6.x-tuned-zoned. Run\n'
printf '  ./apply.sh, or record the drift with the number and say which it was.\n'
exit 1
