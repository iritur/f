# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# What `apply.sh` and `verify.sh` both have to know: how to read the five data
# files, how to turn a key into the sysfs path it names, and how to say `[ok]`.
#
# Sourced rather than copied, for the reason the sibling directory's `lib.sh`
# gives and this directory has more of: five data files and two scripts is ten
# chances for one of them to read a setting the other does not, and the day they
# disagree `apply.sh` sets something `verify.sh` never checks and the machine
# passes.
#
# It sets nothing and reads nothing on being sourced. Both callers run
# `zoned_preflight` when they want the device resolved, so that `--help` works
# on a machine with no zoned device at all — which is every machine this project
# has had so far.

# ---------------------------------------------------------------------------
# Where the data files are: beside this file, whatever the working directory.
BASELINE_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# Drift counters. `verify.sh` exits on them; `apply.sh` uses the same printers
# so that its output and the verifier's read the same way.
BASELINE_OK=0
BASELINE_BAD=0
BASELINE_DRIFT=()

ok() { BASELINE_OK=$((BASELINE_OK + 1)); printf '  [ok]  %-38s %s\n' "$1" "${2-}"; }
bad() {
	BASELINE_BAD=$((BASELINE_BAD + 1))
	BASELINE_DRIFT+=("$1")
	printf '  [--]  %-38s %s\n' "$1" "${2-}"
}
note() { printf '        %s\n' "$*"; }
die() { printf '%s: %s\n' "${PROG:-baseline}" "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# `key = value` out of one of the data files.
#
# The same reader as the sibling directory's, and the same caveat: it reads the
# shape the files beside it are written in and is not a parser for anything more
# general. A key that is absent is an error at the call site rather than an empty
# string here, because every caller is asking a question whose blank answer would
# look like a passing check.
conf_get() {
	local file=$1 key=$2 value
	value=$(sed -n "s/^[[:space:]]*${key}[[:space:]]*=[[:space:]]*\(.*\)$/\1/p" \
		"$BASELINE_DIR/$file" | sed 's/[[:space:]]*#.*$//; s/[[:space:]]*$//' | head -n 1)
	[ -n "$value" ] || die "$file has no \`$key\` — the file and the script disagree"
	printf '%s\n' "$value"
}

# Every `key value` pair in sysctl.conf, one per line, comments removed.
sysctl_pairs() {
	sed 's/#.*$//' "$BASELINE_DIR/sysctl.conf" |
		sed -n 's/^[[:space:]]*\([a-z0-9_.]*\)[[:space:]]*=[[:space:]]*\(.*[^[:space:]]\)[[:space:]]*$/\1 \2/p'
}

# Every `key value` pair in sysfs.conf, one per line, comments removed.
sysfs_pairs() {
	sed 's/#.*$//' "$BASELINE_DIR/sysfs.conf" |
		sed -n 's/^[[:space:]]*\([a-z0-9_]*\)[[:space:]]*=[[:space:]]*\(.*[^[:space:]]\)[[:space:]]*$/\1 \2/p'
}

# The path a sysfs.conf key names.
#
# The key *is* the path, which is the point: `queue_scheduler` is
# `/sys/block/<dev>/queue/scheduler` and `f2fs_gc_idle` is
# `/sys/fs/f2fs/<dev>/gc_idle`, so the setting is written down once and the path
# is derived. Writing the path in the data file and the name in the script — or
# the other way round — is two copies of one fact, and this directory's whole
# argument is about what two copies of one fact do over time.
sysfs_path() {
	[ -n "${DEVNAME-}" ] || die "sysfs_path called before zoned_preflight"
	case $1 in
	queue_*) printf '/sys/block/%s/queue/%s\n' "$DEVNAME" "${1#queue_}" ;;
	f2fs_*) printf '/sys/fs/f2fs/%s/%s\n' "$DEVNAME" "${1#f2fs_}" ;;
	*) die "sysfs.conf key \`$1\` names no tree — see lib.sh" ;;
	esac
}

# ---------------------------------------------------------------------------
# Every line of every data file is either a setting one of the readers above
# emits, or it is an error here.
#
# R04 aimed at this directory rather than at the machine, and it is the check
# the sibling directory acquired after the fact: a setting misspelled here is a
# setting `apply.sh` never applies and `verify.sh` never checks, and both would
# still exit 0. Five data files make that likelier rather than less likely, which
# is why the key lists live here — in the one place that knows the whole of what
# the format is — rather than being discovered by whoever calls `conf_get`.
#
# One key in this list is read by nobody, and it is named rather than left to be
# discovered: `backing`. Every other row describes something a running guest can
# be asked about — sysfs answers the geometry, `blkzone` answers the capacity and
# the conventional count, `verify.sh` answers the emulator and this file answers
# the kernel range. `backing` describes what stands behind the device on the
# *host*, which is outside every machine these scripts run on, so it is data for
# a reader rather than for a script. A key nothing reads is ordinarily the
# failure this check exists to catch; this one is the exception, and an exception
# with no sentence beside it is how a list of them starts.
DEVICE_KEYS="model zone_size_bytes zone_capacity_bytes zones conventional_zones \
logical_block_bytes max_open_zones max_active_zones backing qemu_min kernel_min kernel_below"

FILESYSTEM_KEYS="fs mkfs_opts mount_opts scheduler"

COMPARISON_KEYS="object_min_bytes object_max_bytes seed device_fills retention_keep_every \
durability overprovision_percent barriers compression counting_boundary counted_nodes \
counter_read_after quiescence"

# One data file whose keys come from a fixed list.
conf_keys_check() {
	local file=$1 allowed=$2 line key rejected=""
	while IFS= read -r line; do
		[ -n "${line//[[:space:]]/}" ] || continue
		if [[ $line =~ ^[[:space:]]*([a-z0-9_.]+)[[:space:]]*=[[:space:]]*[^[:space:]] ]]; then
			key=${BASH_REMATCH[1]}
			case " $allowed " in
			*" $key "*) ;;
			*) rejected="$rejected
  $file: \`$key\` is not a key any script reads" ;;
			esac
		else
			rejected="$rejected
  $file: $line"
		fi
	done < <(sed 's/#.*$//' "$BASELINE_DIR/$file")
	printf '%s' "$rejected"
}

zoned_data_check() {
	local rejected="" line key

	rejected="$rejected$(conf_keys_check device.conf "$DEVICE_KEYS")"
	rejected="$rejected$(conf_keys_check filesystem.conf "$FILESYSTEM_KEYS")"
	rejected="$rejected$(conf_keys_check comparison.conf "$COMPARISON_KEYS")"

	# sysctl.conf: every surviving line is `key = value` in the character class
	# `sysctl_pairs` matches, or the pair silently is not a pair.
	while IFS= read -r line; do
		[ -n "${line//[[:space:]]/}" ] || continue
		[[ $line =~ ^[[:space:]]*[a-z0-9_.]+[[:space:]]*=[[:space:]]*[^[:space:]] ]] ||
			rejected="$rejected
  sysctl.conf: $line"
	done < <(sed 's/#.*$//' "$BASELINE_DIR/sysctl.conf")

	# sysfs.conf: the shape, and a key `sysfs_path` can turn into a path. A key
	# in neither namespace is a setting with nowhere to be written, which reads
	# in the file exactly like one that has somewhere.
	while IFS= read -r line; do
		[ -n "${line//[[:space:]]/}" ] || continue
		if [[ $line =~ ^[[:space:]]*([a-z0-9_]+)[[:space:]]*=[[:space:]]*[^[:space:]] ]]; then
			key=${BASH_REMATCH[1]}
			case $key in
			queue_* | f2fs_*) ;;
			*) rejected="$rejected
  sysfs.conf: \`$key\` is in neither the queue nor the f2fs namespace" ;;
			esac
		else
			rejected="$rejected
  sysfs.conf: $line"
		fi
	done < <(sed 's/#.*$//' "$BASELINE_DIR/sysfs.conf")

	[ -z "$rejected" ] || die "these lines are in the data files and no script reads them:$rejected

A line neither reader understands is a setting nothing applies and nothing
checks, and both scripts would still exit 0."
}

# ---------------------------------------------------------------------------
# The kernel version, as `major minor patch`, and whether it is in the range
# `device.conf` names. The same two functions as the sibling directory's, over a
# different file, because the range belongs beside the device it is a range for.
kernel_triple() {
	local release=${1:-$(uname -r)}
	printf '%s\n' "${release%%-*}" | awk -F. '{ printf "%d %d %d\n", $1, $2, $3 }'
}

kernel_in_range() {
	local have min below
	have=$(printf '%03d%03d%03d' "$@")
	# shellcheck disable=SC2046  # the triple is three words, deliberately
	min=$(printf '%03d%03d%03d' $(kernel_triple "$(conf_get device.conf kernel_min)"))
	# shellcheck disable=SC2046
	below=$(printf '%03d%03d%03d' $(kernel_triple "$(conf_get device.conf kernel_below)"))
	[ "$have" -ge "$min" ] && [ "$have" -lt "$below" ]
}

# ---------------------------------------------------------------------------
# Resolve the device and the mount point, and refuse every way of getting them
# wrong.
#
# There is no default and there will not be one, and the argument is sharper
# here than it is for the sibling directory's CPU lists: `apply.sh --format`
# runs `mkfs` on whatever this names. A default would be a filesystem created on
# whichever block device the author of this file happened to have.
#
# Unlike the sibling, this does *not* die when the device is absent or is not
# zoned. A machine with no zoned device is the ordinary state of this project
# today, and the useful output there is the full report saying so rather than
# one line of refusal — so absence is drift, counted and named, and the checks
# that depend on it are skipped with a note.
zoned_preflight() {
	ZONED_DEV=${F_ZONED_DEV-}
	MOUNT=${F_MOUNT-}

	[ -n "$ZONED_DEV" ] || die "F_ZONED_DEV is unset — see the README, 'Applying it'"
	[ -n "$MOUNT" ] || die "F_MOUNT is unset — see the README"

	case $ZONED_DEV in
	/dev/*) ;;
	*) die "F_ZONED_DEV=$ZONED_DEV is not under /dev — this is a block device, not an image" ;;
	esac

	DEVNAME=$(basename "$ZONED_DEV")

	# Whether the rest of the run has a device to ask. Set once, read by both
	# scripts, so that "the device is missing" is reported once rather than
	# thirty times as thirty unreadable files.
	if [ -b "$ZONED_DEV" ]; then
		DEVICE_PRESENT=1
	else
		DEVICE_PRESENT=0
	fi

	zoned_data_check
}

# The block both scripts print first, so that an output pasted into an issue
# says which machine, which device and which baseline it came from.
zoned_header() {
	printf '%s — %s\n\n' "$1" "$(basename "$BASELINE_DIR")"
	printf '  kernel        %s\n' "$(uname -r)"
	printf '  device        %s\n' "$ZONED_DEV"
	printf '  mount point   %s\n' "$MOUNT"
	printf '  filesystem    %s\n\n' "$(conf_get filesystem.conf fs)"
}

# ---------------------------------------------------------------------------
# The number a Rust `const` holds, evaluated.
#
# `zone/tests/cycle.rs` writes `128 * 1024` where `comparison.conf` writes
# `131072`, and the two are the same number written for two readers. This turns
# the first into the second so that `verify.sh` can compare them, because two
# copies of one number in two files is precisely the decay `E1-D06` exists to
# stop — and the workload half of this baseline is the half most likely to move
# without anybody noticing, since it moves when a *test* is edited.
#
# It evaluates only what these constants are written as: integers, and products
# of integers. Anything else is refused rather than guessed at.
rust_const() {
	local file=$1 name=$2 rhs
	rhs=$(sed -n "s/^const ${name}: [a-z0-9]* = \(.*\);$/\1/p" "$file" | head -n 1)
	[ -n "$rhs" ] || return 1
	[[ $rhs =~ ^[0-9\ \*]+$ ]] || return 1
	printf '%s\n' "$((rhs))"
}
