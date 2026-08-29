#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Deterministic detection for the maintain stage.
#
# Reads the signals in ops/bands.yaml, and for each one compares the newest
# sample against the mean and standard deviation of the samples before it.
# Prints one line per signal and a final `max-band:` line, and always exits 0 —
# a detector that fails the job it runs in gets removed from the job.
#
# It contains no thresholds. A threshold is a number somebody chose once and
# nobody revisits; a band is a statement about this signal's own history, which
# is what makes it survive a hardware change and a workload change.
#
# What it is not: change-point detection. The claims registry says regression
# detection should be change-point rather than threshold, and this is neither —
# it is a z-score, which is the cheapest thing that is honest about variance.
# When there is enough history for change-point detection to mean anything,
# this file is where it goes, and the interface above it does not change.
#
# The YAML is read with awk, so it reads exactly the shape bands.yaml is written
# in: two-space list items under `signals:`, one `key: value` per line. It is not
# a YAML parser and does not pretend to be one.

set -u

root=$(cd "$(dirname "$0")/.." && pwd)
bands="$root/ops/bands.yaml"

[ -f "$bands" ] || {
	echo "detect: $bands not found"
	echo "max-band: 0"
	exit 0
}

max_band=0

# name<TAB>history<TAB>column, one signal per line.
signals=$(awk '
	/^signals:/          { in_signals = 1; next }
	/^[a-z]/             { in_signals = 0 }
	!in_signals          { next }
	/^[[:space:]]*-[[:space:]]*name:/ {
		if (name != "") print name "\t" history "\t" column
		name = $NF; history = ""; column = 2; next
	}
	/^[[:space:]]*history:/ { history = $NF; next }
	/^[[:space:]]*column:/  { column  = $NF; next }
	END { if (name != "") print name "\t" history "\t" column }
' "$bands")

printf 'detect: %s\n\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

while IFS=$(printf '\t') read -r name history column; do
	[ -n "$name" ] || continue
	path="$root/$history"

	if [ ! -f "$path" ]; then
		printf '  %-28s no history yet (%s)\n' "$name" "$history"
		continue
	fi

	# Column `column` of every non-comment, non-header line. The last row is the
	# sample under test; the rest are what it is being judged against.
	report=$(awk -F, -v col="$column" '
		/^#/ { next }
		NR == 1 && $col !~ /^-?[0-9.]+$/ { next }   # a header row
		{ v[++n] = $col + 0 }
		END {
			if (n < 8) { printf "short %d\n", n; exit }
			last = v[n]
			for (i = 1; i < n; i++) { s += v[i] }
			mean = s / (n - 1)
			for (i = 1; i < n; i++) { d = v[i] - mean; ss += d * d }
			sd = (n > 2) ? sqrt(ss / (n - 2)) : 0
			if (sd == 0) { printf "flat %.4f\n", last; exit }
			z = (last - mean) / sd
			if (z < 0) z = -z
			band = 0
			if (z >= 1) band = 1
			if (z >= 2) band = 2
			if (z >= 3) band = 3
			printf "ok %d %.4f %.4f %.4f %.2f\n", band, last, mean, sd, z
		}
	' "$path")

	case "$report" in
	short*)
		printf '  %-28s %s samples: too few to say anything\n' "$name" "${report#short }"
		;;
	flat*)
		printf '  %-28s no variance in history; value %s\n' "$name" "${report#flat }"
		;;
	ok*)
		set -- $report
		band=$2
		value=$3
		mean=$4
		sd=$5
		z=$6
		printf '  %-28s band %s   value %s  mean %s  sd %s  z %s\n' \
			"$name" "$band" "$value" "$mean" "$sd" "$z"
		[ "$band" -gt "$max_band" ] && max_band=$band
		;;
	*)
		printf '  %-28s unreadable history\n' "$name"
		;;
	esac
done <<EOF
$signals
EOF

printf '\n'
case "$max_band" in
0) printf 'nothing outside one standard deviation.\n' ;;
1) printf 'band 1: logged. Normal variation is not an event.\n' ;;
2) printf 'band 2: a read-only diagnosis is warranted. No diffs.\n' ;;
3) printf 'band 3: a pull request or a pre-approved runbook is permitted.\n' ;;
esac
printf 'max-band: %s\n' "$max_band"
exit 0
