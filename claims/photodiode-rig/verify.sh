#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Ask this directory, one number at a time, whether it still says what it says.
#
#   ./verify.sh
#   F_RIG_CAPTURE=/path/to/capture ./verify.sh
#
# Exits 0 when every number in `README.md` follows from the specification block
# in `README.md`, and non-zero naming each one that does not.
#
# It exists for the reason `claims/baselines/linux-6.x-tuned/verify.sh` exists:
# a document with two copies of one number and nothing comparing them ends up
# being neither. Here the copies are the requirement block, the budget table
# that is derived from it, the bill of materials, and the totals stated in
# prose. Four places, one arithmetic, and a reader who edits a sample interval
# has no way to know the error bar four screens down is now wrong.
#
# It checks three kinds of thing.
#
#   Arithmetic.  Every budget term recomputed from the requirement block; the
#                budget summed; the bill of materials re-added; the stated
#                error bar checked to be the sum rounded up, never down.
#   Decay.       `prices_read_on` against today. Prices are the part of the
#                document with the shortest half-life and the part nothing else
#                can notice. This goes red on its own.
#   The review.  `timestamp-review.md`'s needle set, run as a control against a
#                file that is known to read a clock, and against the rig's
#                capture toolchain if one has been built. The control is the
#                point: a search nobody has watched find something is a search
#                that proves nothing when it finds nothing.
#
# What it cannot check is the physical rig, and it says so on every green run
# rather than letting a green line stand for more than it is.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
README=$HERE/README.md
REVIEW=$HERE/timestamp-review.md
CONTROL=$HERE/../../input/src/stamp.rs

OK=0
BAD=0
OWED=0
declare -a FAILED=()

ok() { OK=$((OK + 1)); printf '  ok    %-42s %s\n' "$1" "${2-}"; }
bad() {
	BAD=$((BAD + 1))
	FAILED+=("$1")
	printf '  BAD   %-42s %s\n' "$1" "${2-}"
}
owed() { OWED=$((OWED + 1)); printf '  owed  %-42s %s\n' "$1" "${2-}"; }
note() { printf '        %s\n' "$1"; }

expect() {
	local what=$1 want=$2 have=$3
	if [ "$want" = "$have" ]; then ok "$what" "$have"; else bad "$what" "want $want, have $have"; fi
}

# --- the requirement block ------------------------------------------------
# One copy of every number this document imposes. Everything below is
# recomputed from it, so an edit here that is not carried through goes red
# rather than sitting in the file being wrong.

spec() {
	awk -v key="$1" '
		/<!-- spec:begin -->/ { f = 1; next }
		/<!-- spec:end -->/   { f = 0 }
		f && $1 == key        { print $2; found = 1; exit }
		END                   { if (!found) exit 3 }
	' "$README"
}

for key in digitiser_sample_interval_nanos capture_window_nanos timebase_accuracy_ppm \
	inter_channel_skew_nanos front_end_bandwidth_hertz rise_time_bandwidth_product_nanohertz \
	amplitude_hold_denominator optical_step_millivolts front_end_noise_millivolts \
	error_bar_half_width_nanos bom_total_usd prices_read_on prices_stale_after_days; do
	if ! value=$(spec "$key"); then
		printf '  BAD   %-42s %s\n' "spec $key" "not in the spec block"
		exit 1
	fi
	printf -v "SPEC_$key" '%s' "$value"
done

# shellcheck disable=SC2154
SAMPLE=$SPEC_digitiser_sample_interval_nanos
WINDOW=$SPEC_capture_window_nanos
PPM=$SPEC_timebase_accuracy_ppm
SKEW=$SPEC_inter_channel_skew_nanos
BANDWIDTH=$SPEC_front_end_bandwidth_hertz
RTBW=$SPEC_rise_time_bandwidth_product_nanohertz
HOLD=$SPEC_amplitude_hold_denominator
STEP_MV=$SPEC_optical_step_millivolts
NOISE_MV=$SPEC_front_end_noise_millivolts
STATED_BAR=$SPEC_error_bar_half_width_nanos
STATED_BOM=$SPEC_bom_total_usd
READ_ON=$SPEC_prices_read_on
STALE_AFTER=$SPEC_prices_stale_after_days

printf '\nthe error bar, recomputed from the requirement block\n'

RISE=$((RTBW / BANDWIDTH))
note "front_end_rise_time_nanos = $RTBW / $BANDWIDTH = $RISE"

declare -A WANT=(
	[sample_quantisation_nanos]=$SAMPLE
	[threshold_crossing_drift_nanos]=$((RISE / HOLD))
	[inter_channel_skew_nanos]=$SKEW
	[timebase_error_nanos]=$((WINDOW * PPM / 1000000))
	[amplitude_noise_nanos]=$((RISE * NOISE_MV / STEP_MV))
)

# The budget table, as the document states it.
budget_row() {
	awk -v term="$1" '
		/<!-- budget:begin -->/ { f = 1; next }
		/<!-- budget:end -->/   { f = 0 }
		f && index($0, "`" term "`") {
			n = split($0, c, "|"); gsub(/[ \t]/, "", c[3]); print c[3]; found = 1; exit
		}
		END { if (!found) exit 3 }
	' "$README"
}

SUM=0
for term in sample_quantisation_nanos threshold_crossing_drift_nanos \
	inter_channel_skew_nanos timebase_error_nanos amplitude_noise_nanos; do
	if ! have=$(budget_row "$term"); then
		bad "$term" "no row in the budget table"
		continue
	fi
	expect "$term" "${WANT[$term]}" "$have"
	SUM=$((SUM + have))
done

STATED_SUM=$(sed -n 's/^<!-- budget:sum \([0-9]*\) -->$/\1/p' "$README")
expect "budget sums to the stated sum" "$SUM" "${STATED_SUM:-absent}"

# Rounded up to the next whole microsecond, and never down: an error bar that
# is too small is wrong in the one direction that flatters the claim.
ROUNDED=$(((SUM + 999) / 1000 * 1000))
expect "error_bar_half_width_nanos" "$ROUNDED" "$STATED_BAR"
if [ "$STATED_BAR" -lt "$SUM" ]; then
	bad "error bar covers the budget" "$STATED_BAR < $SUM"
else
	ok "error bar covers the budget" "$STATED_BAR >= $SUM"
fi

# The one place the prose restates the number. Two copies, so they are compared
# — the same check `claims/baselines/linux-6.x-tuned/verify.sh` makes against
# `runner-class-A.md`'s prose copy of the kernel command line.
PROSE_US=$((STATED_BAR / 1000))
if grep -q "± *${PROSE_US} µs" "$README" && grep -q "± *${PROSE_US} µs" "$REVIEW"; then
	ok "prose says the same number" "±${PROSE_US} µs in both files"
else
	bad "prose says the same number" "±${PROSE_US} µs is not in both README.md and timestamp-review.md"
	note "One copy was edited and the other was not. The spec block is the copy that"
	note "computes, so it is the one to trust and the prose is the one to fix."
fi

# --- the bill of materials ------------------------------------------------
printf '\nthe bill of materials\n'

# A markdown row is `| a | b | ... | z |`, so splitting on the bar leaves an
# empty field past the trailing one. The three columns that matter are counted
# back from there: line total, unit price, quantity.
BOM=$(awk '
	/<!-- bom:begin -->/ { f = 1; next }
	/<!-- bom:end -->/   { f = 0 }
	f && /^\| *[0-9]+ *\|/ {
		n = split($0, c, "|")
		qty = c[n - 3]; unit = c[n - 2]; line = c[n - 1]; row = c[2]
		gsub(/[ \t,]/, "", qty); gsub(/[ \t,]/, "", unit)
		gsub(/[ \t,]/, "", line); gsub(/[ \t,]/, "", row)
		if (qty * unit != line) { printf "ROW %s: %s x %s is not %s\n", row, qty, unit, line }
		total += line
		rows += 1
	}
	END { printf "TOTAL %d ROWS %d\n", total, rows }
' "$README")

MISMATCHED=$(printf '%s\n' "$BOM" | grep '^ROW ' || true)
if [ -z "$MISMATCHED" ]; then
	ok "every line total is its quantity times its unit price" \
		"$(printf '%s\n' "$BOM" | sed -n 's/.*\(ROWS [0-9]*\)/\1/p')"
else
	bad "every line total is its quantity times its unit price"
	printf '%s\n' "$MISMATCHED" | sed 's/^/        /'
fi

BOM_TOTAL=$(printf '%s\n' "$BOM" | sed -n 's/^TOTAL \([0-9]*\).*/\1/p')
STATED_TABLE_TOTAL=$(sed -n 's/^<!-- bom:total \([0-9]*\) -->$/\1/p' "$README")
expect "the column re-adds to the stated total" "$BOM_TOTAL" "${STATED_TABLE_TOTAL:-absent}"
expect "the spec block agrees with the table" "$BOM_TOTAL" "$STATED_BOM"
if grep -q "USD $BOM_TOTAL" "$README"; then
	ok "prose states the total" "USD $BOM_TOTAL"
else
	bad "prose states the total" "\"USD $BOM_TOTAL\" is not in README.md"
fi

# --- decay ----------------------------------------------------------------
printf '\nprice decay\n'

if read_epoch=$(date -d "$READ_ON" +%s 2>/dev/null); then
	age_days=$((($(date +%s) - read_epoch) / 86400))
	if [ "$age_days" -le "$STALE_AFTER" ]; then
		ok "prices" "read $age_days day(s) ago, stale after $STALE_AFTER"
	else
		bad "prices" "read $age_days day(s) ago, stale after $STALE_AFTER"
		note "Re-read every source in the bill of materials, edit the table and"
		note "prices_read_on together, and let this go green. A total nobody has"
		note "re-read is a budget figure that has quietly become a wrong one."
	fi
else
	owed "prices" "this date(1) cannot parse $READ_ON"
fi

# --- the review -----------------------------------------------------------
printf '\nthe software-timestamp review\n'

NEEDLES='env\.now\(|Instant::now|SystemTime::now|rdtsc|clock_gettime|gettimeofday|QueryPerformanceCounter|time\.time\(|time\.monotonic|perf_counter|datetime\.now|Date\.now'

# The control, and the reason this section is not decoration. A needle set that
# finds nothing anywhere reports a clean capture toolchain and a clean empty
# directory with the same green line.
if [ -r "$CONTROL" ]; then
	hits=$(grep -cE "$NEEDLES" "$CONTROL" || true)
	if [ "${hits:-0}" -gt 0 ]; then
		ok "the needle set fires on the control" "$hits hit(s) in input/src/stamp.rs"
	else
		bad "the needle set fires on the control" "0 hits in input/src/stamp.rs"
		note "That file reads a clock, deliberately, at interrupt time. A needle set"
		note "that cannot see it cannot see one in a capture script either, and every"
		note "green line below it is a search over nothing."
	fi
else
	owed "the needle set fires on the control" "no checkout reachable at $CONTROL"
fi

if [ -n "${F_RIG_CAPTURE-}" ] && [ -d "${F_RIG_CAPTURE}" ]; then
	found=$(grep -rnE "$NEEDLES" "$F_RIG_CAPTURE" || true)
	if [ -z "$found" ]; then
		ok "no software clock in the capture toolchain" "$F_RIG_CAPTURE"
	else
		bad "no software clock in the capture toolchain" "$F_RIG_CAPTURE"
		printf '%s\n' "$found" | sed 's/^/        /'
		note "E3-P01d fails on one. Either the reading is outside the number and the"
		note "path in timestamp-review.md must say where, or the rig is measuring the"
		note "host's clock and calling it the instrument's."
	fi
else
	owed "no software clock in the capture toolchain" "F_RIG_CAPTURE names no directory"
	note "This is the half of E3-P01d that is owed to the assembled rig. It is the"
	note "half the failure the task exists to catch actually lives in: a digitiser"
	note "whose host software stamps the trace on arrival. Printed on every run,"
	note "green or red, so that nobody reads a clean checkout as a clean rig."
fi

# --- what nothing here checks ---------------------------------------------
printf '\n  %d ok, %d bad, %d owed\n\n' "$OK" "$BAD" "$OWED"

printf '  Not checked by anything, and not checkable from a checkout:\n'
printf '    the fixture, the shroud, the gain setting, the optical step, the panel\n'
printf '    settings, and whether the rig exists at all. README.md items 1-8.\n'

if [ "$BAD" -eq 0 ]; then
	printf '\n  This directory says what it says.\n'
	exit 0
fi

printf '\n  failed:\n'
for what in "${FAILED[@]}"; do printf '    %s\n' "$what"; done
exit 1
