#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Put this guest into the configuration `claims/0016-write-amplification.toml`
# names as `linux-6.x-tuned-zoned`, and say what it could not do.
#
#   ./apply.sh --dry-run                      every write it would make, and none
#   sudo F_ZONED_DEV=/dev/vdb F_MOUNT=/mnt/baseline ./apply.sh
#   sudo F_ZONED_DEV=/dev/vdb F_MOUNT=/mnt/baseline ./apply.sh --format
#
# Idempotent by construction rather than by checking first: every write is an
# absolute value, never an increment and never an append. The exception is
# `--format`, which is not idempotent in any useful sense and is the reason it is
# a flag.
#
# Exit codes, because a caller in a job needs the difference:
#
#   0  the guest is the baseline
#   1  something could not be applied
#   2  every knob is applied and there is no filesystem. A `--format` away from
#      the baseline, and the expected result of the first run on a new guest.
#
# What it deliberately does not do:
#
#   **It never runs mkfs without `--format`.** `F_ZONED_DEV` names a block
#   device and `mkfs` destroys it. The sibling directory's scripts can afford to
#   apply everything they check; this one cannot, and a flag that has to be typed
#   is the whole of the defence.
#
#   **It never creates the device.** `device.conf` says the geometry comes from
#   the host's `null_blk` and the emulator's zoned virtio-blk, both of which are
#   outside this guest. What this script does instead is *refuse* a device whose
#   geometry is not the one `device.conf` names — because a baseline applied to
#   the wrong device is worse than one not applied at all: it produces a number.
#
#   **It never turns a barrier off.** `nobarrier` and `compress` are checked for
#   their absence by `verify.sh`, with the argument in `filesystem.conf`.
#
# `verify.sh` beside this file checks everything this sets.

set -euo pipefail

PROG=$(basename "$0")
# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

DRY=0
FORMAT=0
for arg in "$@"; do
	case $arg in
	--dry-run) DRY=1 ;;
	--format) FORMAT=1 ;;
	-h | --help)
		sed -n '3,37p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*) die "unknown option: $arg" ;;
	esac
done

zoned_preflight
zoned_header "apply"

if [ "$DRY" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
	die "this writes to /proc, /sys and a block device — re-run with sudo, or with --dry-run"
fi

# One verb for every write, so that "what did it change" is one grep away in a
# transcript, and so that --dry-run is a single branch rather than one per
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

run_cmd() {
	local what=$1
	shift
	if [ "$DRY" -eq 1 ]; then
		printf '  [x]   %-38s %s\n' "$what" "$*"
		return 0
	fi
	if "$@"; then
		ok "$what" "$*"
	else
		bad "$what" "$* failed"
	fi
}

# ---------------------------------------------------------------------------
printf 'kernel version\n'

read -r major minor patch <<<"$(kernel_triple)"
if kernel_in_range "$major" "$minor" "$patch"; then
	ok "kernel in range" "$(uname -r)"
else
	die "kernel $(uname -r) is outside $(conf_get device.conf kernel_min)..$(conf_get device.conf kernel_below).
This is a different baseline rather than a drifted machine: see the README's
reversal condition before applying anything here."
fi

# ---------------------------------------------------------------------------
printf '\nthe device\n'

# The geometry is checked and never written, and it is checked *before* anything
# else because everything below either writes to this device or formats it. A
# guest whose device is not the device `device.conf` names is a guest this script
# must not touch: the failure it would otherwise produce is a fully applied
# baseline on the wrong geometry, which is a number nobody can tell from a good
# one.
if [ "$DEVICE_PRESENT" -eq 0 ] && [ "$DRY" -eq 0 ]; then
	die "$ZONED_DEV is not a block device.
device.conf says where the device comes from: a host null_blk in zoned mode,
presented to this guest as a zoned virtio-blk by a QEMU of at least
$(conf_get device.conf qemu_min). Neither half is inside this guest."
fi

queue=/sys/block/$DEVNAME/queue
geometry_bad=0
check_geometry() {
	local what=$1 want=$2 have=$3
	if [ "$want" = "$have" ]; then
		ok "$what" "$have"
	else
		bad "$what" "want $want, have $have"
		geometry_bad=1
	fi
}

# `--dry-run` has to work on a machine with no zoned device, because that is
# every machine this project has and because the point of the flag is to show
# what would be written to somebody who cannot yet run it. So the geometry is
# *listed* rather than checked there. It is not a weaker check: the run that
# writes anything is the run that still refuses.
if [ "$DEVICE_PRESENT" -eq 0 ]; then
	note "--dry-run on a machine with no $ZONED_DEV: the geometry below is what a real"
	note "run would require of the device before writing anything, and is listed rather"
	note "than checked."
	while read -r what key; do
		printf '  [?]   %-38s want %s\n' "$what" "$(conf_get device.conf "$key")"
	done <<-'GEOMETRY'
		queue/zoned model
		zone-size-bytes zone_size_bytes
		zones zones
		queue/logical_block_size logical_block_bytes
		queue/max_open_zones max_open_zones
		queue/max_active_zones max_active_zones
	GEOMETRY
else
	[ -r "$queue/zoned" ] ||
		die "$queue/zoned is unreadable — this kernel has no zoned block support"
	check_geometry "queue/zoned" "$(conf_get device.conf model)" "$(cat "$queue/zoned")"
	check_geometry "zone size (bytes)" "$(conf_get device.conf zone_size_bytes)" \
		"$(($(cat "$queue/chunk_sectors") * 512))"
	check_geometry "zones" "$(conf_get device.conf zones)" "$(cat "$queue/nr_zones")"
	check_geometry "queue/logical_block_size" "$(conf_get device.conf logical_block_bytes)" \
		"$(cat "$queue/logical_block_size")"
	check_geometry "queue/max_open_zones" "$(conf_get device.conf max_open_zones)" \
		"$(cat "$queue/max_open_zones")"
	check_geometry "queue/max_active_zones" "$(conf_get device.conf max_active_zones)" \
		"$(cat "$queue/max_active_zones")"
fi

if [ "$geometry_bad" -ne 0 ]; then
	die "$ZONED_DEV is not the device device.conf describes.
Nothing was written. Fix the device — the host's null_blk parameters and the
emulator's zoned virtio-blk arguments are what set every row above — or, if the
new geometry is deliberate, it is a new baseline directory beside this one and
not an edit to this file."
fi

# ---------------------------------------------------------------------------
printf '\nthe block queue\n'

# The scheduler goes on before the filesystem, because a zoned device without a
# zone-ordering scheduler is one f2fs may refuse to mount.
set_value "queue_scheduler" "$queue/scheduler" "$(conf_get filesystem.conf scheduler)"
set_value "queue_nr_requests" "$queue/nr_requests" "$(conf_get sysfs.conf queue_nr_requests)"

# ---------------------------------------------------------------------------
printf '\nthe filesystem\n'

formatted=0
if [ "$FORMAT" -eq 1 ]; then
	if mountpoint -q "$MOUNT" 2>/dev/null; then
		run_cmd "unmount" umount "$MOUNT"
	fi
	# shellcheck disable=SC2046  # mkfs_opts is a word list, deliberately
	run_cmd "mkfs" mkfs."$(conf_get filesystem.conf fs)" \
		$(conf_get filesystem.conf mkfs_opts) "$ZONED_DEV"
	run_cmd "mount point" mkdir -p "$MOUNT"
	run_cmd "mount" mount -t "$(conf_get filesystem.conf fs)" \
		-o "$(conf_get filesystem.conf mount_opts)" "$ZONED_DEV" "$MOUNT"
	formatted=1
elif mountpoint -q "$MOUNT" 2>/dev/null; then
	ok "already mounted" "$(awk -v m="$MOUNT" '$2 == m { print $3 }' /proc/mounts | head -n 1)"
	formatted=1
else
	note "nothing is mounted at $MOUNT and --format was not given, so no filesystem was"
	note "made. This script will not run mkfs on a named block device without being told"
	note "to in the same command line. Re-run with --format when $ZONED_DEV is expendable."
fi

# ---------------------------------------------------------------------------
printf '\nthe f2fs knobs\n'

if [ "$formatted" -eq 0 ] && [ "$DRY" -eq 0 ]; then
	note "skipped: /sys/fs/f2fs/$DEVNAME does not exist until the filesystem is mounted."
else
	while read -r key want; do
		case $key in
		f2fs_*) set_value "$key" "$(sysfs_path "$key")" "$want" ;;
		esac
	done < <(sysfs_pairs)
fi

# ---------------------------------------------------------------------------
printf '\nsysctls\n'

while read -r key want; do
	set_value "$key" "/proc/sys/${key//./\/}" "$want"
done < <(sysctl_pairs)

# ---------------------------------------------------------------------------
printf '\n  %d applied, %d not\n' "$BASELINE_OK" "$BASELINE_BAD"

if [ "$DRY" -eq 1 ]; then
	printf '\n  --dry-run: nothing was written.\n'
	exit 0
fi

if [ "$BASELINE_BAD" -ne 0 ]; then
	printf '\n  not applied:\n'
	for what in "${BASELINE_DRIFT[@]}"; do printf '    %s\n' "$what"; done
	exit 1
fi

if [ "$formatted" -eq 0 ]; then
	printf '\n  Every knob is in force and there is no filesystem. Re-run with --format\n'
	printf '  when %s is expendable, then ./verify.sh.\n' "$ZONED_DEV"
	exit 2
fi

printf '\n  This guest is the baseline claims/0016 names. Check it with ./verify.sh.\n'
exit 0
