# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# What `apply.sh` and `verify.sh` both have to know: how to read the three data
# files, how to expand the two CPU lists into them, and how to say `[ok]`.
#
# It is a sourced file rather than a copy in each script because two parsers
# for one format is the same decay this whole directory exists to prevent, one
# level down: the day they disagree, `apply.sh` sets something `verify.sh` does
# not check, and the machine passes.
#
# It sets nothing and reads nothing on being sourced. Both callers run
# `baseline_preflight` when they want the CPU lists resolved, and the reason
# that is a call rather than a side effect is that `--help` should work on a
# machine with neither variable set.

# ---------------------------------------------------------------------------
# Where the data files are: beside this file, whatever the working directory.
BASELINE_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# Drift counters. `verify.sh` exits on them; `apply.sh` uses the same printers
# so that its output and the verifier's read the same way.
BASELINE_OK=0
BASELINE_BAD=0
BASELINE_DRIFT=()

ok()   { BASELINE_OK=$((BASELINE_OK + 1)); printf '  [ok]  %-38s %s\n' "$1" "${2-}"; }
bad()  {
	BASELINE_BAD=$((BASELINE_BAD + 1))
	BASELINE_DRIFT+=("$1")
	printf '  [--]  %-38s %s\n' "$1" "${2-}"
}
note() { printf '        %s\n' "$*"; }
die()  { printf '%s: %s\n' "${PROG:-baseline}" "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# `key = value` out of one of the data files.
#
# Not a parser for anything more general: it reads the shape the three files
# beside it are written in, which is the same caveat `xtask` states about the
# claim registry. A key that is absent is an error at the call site rather than
# an empty string here, because every caller of this is asking a question whose
# blank answer would look like a passing check.
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

# Every token of the required kernel command line, expanded.
#
# Fails rather than substituting an empty string for a CPU list that was not
# given: `isolcpus=domain,managed_irq,` is a line the kernel accepts and
# ignores, which is the worst of the three possible outcomes.
cmdline_tokens() {
	[ -n "${MEASURED-}" ] && [ -n "${HOUSEKEEPING-}" ] ||
		die "cmdline_tokens called before baseline_preflight"
	sed 's/#.*$//' "$BASELINE_DIR/cmdline.txt" |
		sed -n 's/^[[:space:]]*\([^[:space:]].*[^[:space:]]\|[^[:space:]]\)[[:space:]]*$/\1/p' |
		sed "s/@MEASURED@/$MEASURED/g; s/@HOUSEKEEPING@/$HOUSEKEEPING/g"
}

# ---------------------------------------------------------------------------
# The three readers above emit what they recognise, and — until this function
# ran — said nothing about what they did not. That is the failure this whole
# directory was written against, one level down: a line these files hold and
# neither script understands is a setting `apply.sh` never applies and
# `verify.sh` never checks, and both of them still print green. `vm.SwapPiness
# = 0` with a capital letter, `net.core.busy_poll 50` with the `=` left out, a
# governor spelled `governer` — three edits somebody makes in a hurry, three
# machines that look configured and measure nothing.
#
# So every line of every data file is either a setting one of the readers
# emits, or it is an error here (R04). This is the only place that knows the
# whole of what the format is, which is why the keys `baseline.conf` may hold
# are listed here rather than discovered by whoever calls `conf_get`.
BASELINE_CONF_KEYS="kernel_min kernel_below governor sched_policy sched_priority \
thp_enabled thp_defrag khugepaged_defrag irqbalance sysctl_optional"

baseline_data_check() {
	local line key rejected=""

	# sysctl.conf: every surviving line is `key = value` in the character class
	# `sysctl_pairs` matches, or the pair silently is not a pair.
	while IFS= read -r line; do
		[ -n "${line//[[:space:]]/}" ] || continue
		[[ $line =~ ^[[:space:]]*[a-z0-9_.]+[[:space:]]*=[[:space:]]*[^[:space:]] ]] ||
			rejected="$rejected
  sysctl.conf: $line"
	done < <(sed 's/#.*$//' "$BASELINE_DIR/sysctl.conf")

	# baseline.conf: the same shape, and a key from the list above. An unknown
	# key here is the worse of the two failures — nothing reads it, so the
	# setting it was meant to be is simply not part of the baseline.
	while IFS= read -r line; do
		[ -n "${line//[[:space:]]/}" ] || continue
		if [[ $line =~ ^[[:space:]]*([a-z0-9_]+)[[:space:]]*=[[:space:]]*[^[:space:]] ]]; then
			key=${BASH_REMATCH[1]}
			case " $BASELINE_CONF_KEYS " in
			*" $key "*) ;;
			*) rejected="$rejected
  baseline.conf: \`$key\` is not a key any script reads" ;;
			esac
		else
			rejected="$rejected
  baseline.conf: $line"
		fi
	done < <(sed 's/#.*$//' "$BASELINE_DIR/baseline.conf")

	# cmdline.txt: a token with whitespace in it is two tokens that will never
	# match `/proc/cmdline` as one, and a token still holding an `@` is a
	# placeholder this file does not know how to expand — which would be
	# reported forever as a parameter the machine is missing.
	while IFS= read -r line; do
		[ -n "$line" ] || continue
		case $line in
		*[[:space:]]* | *@*) rejected="$rejected
  cmdline.txt: $line" ;;
		esac
	done < <(cmdline_tokens)

	[ -z "$rejected" ] || die "these lines are in the data files and no script reads them:$rejected

A line neither reader understands is a setting nothing applies and nothing
checks, and both scripts would still exit 0. Fix the line, or teach lib.sh the
shape — but not silently, which is what this file did before."
}

# Is this one of the two sysctls whose absence satisfies the baseline? The
# argument for each is in `baseline.conf` beside the list, and the default is
# the other way: a knob the kernel does not have is drift until somebody has
# written down why it is not.
sysctl_optional() {
	case " $(conf_get baseline.conf sysctl_optional) " in
	*" $1 "*) return 0 ;;
	*) return 1 ;;
	esac
}

# ---------------------------------------------------------------------------
# `4-7,12` -> `4 5 6 7 12`. Used to prove the two sets do not overlap, which is
# the one mistake in this whole procedure that produces a machine that looks
# configured and measures nothing.
cpu_expand() {
	local part lo hi i out=""
	local IFS=,
	for part in $1; do
		case $part in
		*-*)
			lo=${part%-*}
			hi=${part#*-}
			for ((i = lo; i <= hi; i++)); do out="$out $i"; done
			;;
		*) out="$out $part" ;;
		esac
	done
	printf '%s\n' "${out# }"
}

# ---------------------------------------------------------------------------
# The kernel version, as `major minor patch`, and whether it is in the range
# `baseline.conf` names.
kernel_triple() {
	local release=${1:-$(uname -r)}
	printf '%s\n' "${release%%-*}" | awk -F. '{ printf "%d %d %d\n", $1, $2, $3 }'
}

# 0 if `$1 $2 $3` (major minor patch) is >= min and < below.
kernel_in_range() {
	local have min below
	have=$(printf '%03d%03d%03d' "$@")
	# shellcheck disable=SC2046  # the triple is three words, deliberately
	min=$(printf '%03d%03d%03d' $(kernel_triple "$(conf_get baseline.conf kernel_min)"))
	below=$(printf '%03d%03d%03d' $(kernel_triple "$(conf_get baseline.conf kernel_below)"))
	[ "$have" -ge "$min" ] && [ "$have" -lt "$below" ]
}

# ---------------------------------------------------------------------------
# Resolve the two CPU lists, and refuse every way of getting them wrong.
#
# There is no default and there will not be one. CPU enumeration is the
# firmware's business: `claims/runner-class-A.md` writes `4-15` as an example
# and says in the same paragraph that it is wrong on most machines. A default
# here would be that example, silently applied, on a machine whose measured set
# is somebody else's housekeeping set.
baseline_preflight() {
	MEASURED=${F_MEASURED_CPUS-}
	HOUSEKEEPING=${F_HOUSEKEEPING_CPUS-}

	[ -n "$MEASURED" ] || die "F_MEASURED_CPUS is unset — see the README, 'Applying it'"
	[ -n "$HOUSEKEEPING" ] || die "F_HOUSEKEEPING_CPUS is unset — see the README"

	local m h overlap=""
	m=$(cpu_expand "$MEASURED")
	h=$(cpu_expand "$HOUSEKEEPING")
	local a b
	for a in $m; do
		for b in $h; do
			[ "$a" = "$b" ] && overlap="$overlap $a"
		done
	done
	[ -z "$overlap" ] || die "the two CPU sets overlap on:$overlap"

	# Both siblings of an SMT pair, or neither. RFC 0007's first component is a
	# physical core and not a hardware thread, and a measured set that holds one
	# sibling of a pair has already conceded the tail it was assembled to bound.
	local cpu siblings sib
	for cpu in $m; do
		siblings=/sys/devices/system/cpu/cpu$cpu/topology/thread_siblings_list
		[ -r "$siblings" ] || continue
		for sib in $(cpu_expand "$(cat "$siblings")"); do
			case " $m " in
			*" $sib "*) ;;
			*) die "cpu$cpu is measured and its SMT sibling cpu$sib is not — RFC 0007" ;;
			esac
		done
	done

	# Last, because it expands `cmdline.txt` and so needs the two lists above.
	# Both scripts get it by calling this, which is the point: a check only one
	# of them ran would be a check the other one's silence still passes.
	baseline_data_check
}

# The block `apply.sh` and `verify.sh` both print first, so that an output
# pasted into an issue says which machine and which baseline it came from.
baseline_header() {
	printf '%s — %s\n\n' "$1" "$(basename "$BASELINE_DIR")"
	printf '  kernel        %s\n' "$(uname -r)"
	printf '  measured      %s\n' "$MEASURED"
	printf '  housekeeping  %s\n\n' "$HOUSEKEEPING"
}
