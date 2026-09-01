#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Install F as a second boot entry on a minimal Arch Linux machine.
#
# The procedure this automates is docs/booting-on-hardware.md, and the task it
# serves is E0-P18: nothing in this repository has ever run on hardware, so the
# first boot on a real machine is an experiment rather than a deployment.
#
# It is written for that: `check` changes nothing and reports what would go
# wrong, `install` adds a menu entry beside Arch without touching the default,
# and `uninstall` removes it. Arch stays bootable at every point — which matters
# because the machine this runs on is one somebody has to be able to use
# tomorrow.
#
#   ./tools/f-on-metal.sh check                 what this machine would do
#   ./tools/f-on-metal.sh build                 toolchain, then build and smoke-test
#   ./tools/f-on-metal.sh install [--serial]    add the GRUB entries
#   ./tools/f-on-metal.sh uninstall             remove them
#
# `install` takes the two artefacts from ./target by default, or from
# --kernel <path> --init <path> if they were built elsewhere and copied over.

set -euo pipefail

PROG=$(basename "$0")
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# This is useful on its own — downloaded to the target machine, which already
# has the two artefacts and no checkout — so it must not pretend to be inside a
# repository when it is not. Without this, REPO resolves to "/" and the defaults
# become //target/..., which is a path nobody can act on and an error nobody can
# read.
if [ -f "$REPO/kernel/Cargo.toml" ] && [ -f "$REPO/rust-toolchain.toml" ]; then
    IN_REPO=1
    KERNEL_DEFAULT="$REPO/target/x86_64-unknown-none/debug/f-kernel.elf32"
    INIT_DEFAULT="$REPO/target/init/init.bin"
else
    IN_REPO=0
    KERNEL_DEFAULT="./f-kernel.elf32"
    INIT_DEFAULT="./init.bin"
fi

# Where the artefacts land, and the file that generates the menu entries.
# A dedicated 45_f rather than a block inside 40_custom, because a whole file is
# idempotent by overwriting and removable by deleting — appending to a shared
# file is how a second run leaves two entries behind.
#
# DESTDIR is the usual staged-install convention and is here so this script can
# be tested without a bootloader to break: with it set, everything is written
# under that prefix and grub-mkconfig is not run.
DESTDIR="${DESTDIR:-}"
DEST="$DESTDIR/boot/f"
GRUB_D="$DESTDIR/etc/grub.d/45_f"

BAUD=38400          # kernel/src/arch/x86_64/serial.rs, divisor 3. Not 115200.
MAX_CPUS=8          # kernel/src/percpu.rs, for the topology note below.

red()  { printf '\033[31m%s\033[0m\n' "$*"; }
grn()  { printf '\033[32m%s\033[0m\n' "$*"; }
ylw()  { printf '\033[33m%s\033[0m\n' "$*"; }
bold() { printf '\033[1m%s\033[0m\n' "$*"; }

die() { red "$PROG: $*" >&2; exit 1; }

need_root() {
    [ -n "$DESTDIR" ] && return 0
    [ "$(id -u)" -eq 0 ] || die "this needs root — re-run with sudo"
}

# ----------------------------------------------------------------------------
# Where GRUB will look for the files.
#
# The trap this exists for: if /boot is its own partition, GRUB paths are
# relative to *that* partition, so the file at /boot/f/x is /f/x to GRUB. Get
# this wrong and the entry appears in the menu and fails with "file not found"
# at the moment you are least able to debug it.
#
# `search --file --set=root` in the entry itself makes it robust either way —
# it scans for the file and sets root to whichever device holds it — but the
# path handed to `search` still has to be the partition-relative one.
grub_path() {
    if mountpoint -q /boot 2>/dev/null; then
        echo "/f"
    else
        echo "/boot/f"
    fi
}

# ----------------------------------------------------------------------------

cmd_check() {
    bold "== what this machine would do =="
    echo

    local fail=0 warn=0

    # -- firmware ------------------------------------------------------------
    if [ -d /sys/firmware/efi ]; then
        echo "boot mode       UEFI"
        ylw "                multiboot 1 under UEFI GRUB works but is the fussier path."
        ylw "                If the entry hangs before any output, try CSM/legacy boot"
        ylw "                before concluding anything about the kernel."
        warn=$((warn + 1))
    else
        echo "boot mode       BIOS / CSM  (the reliable path for multiboot 1)"
    fi

    local sb="unknown"
    if command -v mokutil >/dev/null 2>&1; then
        sb=$(mokutil --sb-state 2>/dev/null | head -1 || echo unknown)
    elif [ -d /sys/firmware/efi/efivars ]; then
        if ls /sys/firmware/efi/efivars/SecureBoot-* >/dev/null 2>&1; then
            sb=$(od -An -t u1 /sys/firmware/efi/efivars/SecureBoot-* 2>/dev/null \
                 | awk '{print ($NF == 1) ? "SecureBoot enabled" : "SecureBoot disabled"}' | head -1)
        fi
    fi
    echo "secure boot     $sb"
    case "$sb" in
        *enabled*)
            red "                MUST BE OFF. The image is not signed and will be refused."
            fail=$((fail + 1)) ;;
    esac

    # -- grub ----------------------------------------------------------------
    if command -v grub-mkconfig >/dev/null 2>&1; then
        echo "grub            $(grub-mkconfig --version | head -1)"
    else
        red "grub            grub-mkconfig not found — pacman -S grub"
        fail=$((fail + 1))
    fi

    # Two places, and the difference is the whole diagnosis. /boot/grub/<target>/
    # is what the installed GRUB loads at boot; /usr/lib/grub/<target>/ is what
    # the distribution package ships, which grub-install copies from. Present in
    # the second and absent from the first means GRUB was never deployed to this
    # /boot — a fixable state, and a completely different problem from a GRUB
    # build that has no multiboot support at all.
    local mb="" pkg=""
    for d in /boot/grub/i386-pc /boot/grub/x86_64-efi /boot/grub2/i386-pc /boot/grub2/x86_64-efi; do
        [ -f "$d/multiboot.mod" ] && mb="$d/multiboot.mod"
    done
    for d in /usr/lib/grub/i386-pc /usr/lib/grub/x86_64-efi; do
        [ -f "$d/multiboot.mod" ] && pkg="$d/multiboot.mod"
    done

    if [ -n "$mb" ]; then
        echo "multiboot mod   $mb"
    elif [ -n "$pkg" ]; then
        red "multiboot mod   not deployed to /boot, though the package has it:"
        red "                  $pkg"
        red ""
        red "                GRUB's modules were never installed to this /boot, so the"
        red "                bootloader running on this machine cannot load a multiboot"
        red "                kernel. Deploy them:"
        if [ -d /sys/firmware/efi ]; then
            red "                  grub-install --target=x86_64-efi --efi-directory=<your ESP>"
            red "                  grub-mkconfig -o /boot/grub/grub.cfg"
            red "                (the ESP is usually /boot or /efi — check 'findmnt /boot')"
        else
            red "                  grub-install --target=i386-pc /dev/sdX"
            red "                  grub-mkconfig -o /boot/grub/grub.cfg"
        fi
        red ""
        red "                If this machine actually boots by something else — systemd-boot"
        red "                is the common one on Arch — then GRUB is installed as a package"
        red "                and not in use, and that is the thing to settle first."
        fail=$((fail + 1))
    else
        red "multiboot mod   not found in /boot/grub or /usr/lib/grub."
        red "                This GRUB cannot load a multiboot 1 kernel. On Arch:"
        red "                  pacman -S grub"
        fail=$((fail + 1))
    fi

    local timeout
    timeout=$(grep -E '^GRUB_TIMEOUT=' /etc/default/grub 2>/dev/null | tail -1 | cut -d= -f2 | tr -d '"' || echo "")
    echo "grub timeout    ${timeout:-unset}"
    if [ "${timeout:-5}" = "0" ]; then
        red "                0 means no menu is shown, so F cannot be selected."
        red "                Set GRUB_TIMEOUT=5 in /etc/default/grub."
        fail=$((fail + 1))
    fi

    # -- the console, which is the whole interface ---------------------------
    echo
    bold "-- serial, and this is the one that matters --"
    if dmesg 2>/dev/null | grep -qiE 'ttyS0 at I/O 0x3f8'; then
        grn "serial          real 16550 at 0x3f8 (COM1) — detected by the kernel"
        echo "                connect at ${BAUD} 8N1"
    else
        red "serial          no UART detected at 0x3f8"
        red ""
        red "                F has NO VIDEO OUTPUT. The multiboot header requests no"
        red "                framebuffer and the kernel writes to no display. Without a"
        red "                serial port you will see a black screen and have no way to"
        red "                tell a clean boot from a triple fault."
        red ""
        red "                A BMC with serial-over-LAN, a COM header plus bracket, or a"
        red "                PCIe serial card at the legacy 0x3f8 address. A USB serial"
        red "                adapter will NOT work — it is not COM1."
        fail=$((fail + 1))
    fi

    # -- topology ------------------------------------------------------------
    echo
    local threads
    threads=$(nproc 2>/dev/null || echo "?")
    echo "logical cpus    $threads"
    if [ "$threads" != "?" ] && [ "$threads" -gt "$MAX_CPUS" ] 2>/dev/null; then
        echo "                F shards for $MAX_CPUS and will print, correctly:"
        echo "                  note  the processor reports $threads — $((threads - MAX_CPUS)) left asleep, past MAX_CPUS"
        echo "                That is not a fault. docs/booting-on-hardware.md has the cost"
        echo "                curve and why the ceiling is $MAX_CPUS."
    fi

    # -- artefacts -----------------------------------------------------------
    echo
    for f in "$KERNEL_DEFAULT" "$INIT_DEFAULT"; do
        if [ -f "$f" ]; then
            printf 'artefact        %s (%s bytes)\n' "$f" "$(stat -c%s "$f")"
        else
            ylw "artefact        missing: $f"
            if [ "$IN_REPO" -eq 1 ]; then
                ylw "                run '$PROG build', or pass --kernel/--init"
            else
                ylw "                no checkout here, so copy the two files from the machine"
                ylw "                that built them and pass --kernel/--init, or clone:"
                ylw "                  git clone https://github.com/iritur/f"
            fi
            warn=$((warn + 1))
        fi
    done

    echo
    if [ "$fail" -gt 0 ]; then
        red "$fail blocking problem(s), $warn warning(s). Fix the blocking ones first."
        return 1
    fi
    grn "no blocking problems, $warn warning(s)."
    echo "Next: $PROG install"
}

# ----------------------------------------------------------------------------

cmd_build() {
    if [ "$IN_REPO" -eq 0 ]; then
        die "build needs the repository, and this copy is not inside one.

       git clone https://github.com/iritur/f
       cd f && sudo ./tools/f-on-metal.sh build

       Or build elsewhere and use: $PROG install --kernel <path> --init <path>"
    fi
    need_root
    bold "== toolchain =="
    pacman -S --needed --noconfirm rustup git qemu-system-x86

    # rustup reads rust-toolchain.toml and fetches the pin, including the
    # rust-src and llvm-tools components the build-std and elf32 steps need.
    # Doing it as the invoking user, not root, so the toolchain is not installed
    # into root's home and then unusable.
    local as_user="${SUDO_USER:-$(id -un)}"
    bold "== fetching the pinned toolchain as $as_user =="
    sudo -u "$as_user" sh -c "cd '$REPO' && rustup show"

    # `run` rather than `build`, deliberately: it builds the kernel *and* the
    # init module, and then boots the result under QEMU on this machine. Proving
    # it boots emulated here before asking the firmware to do it means a failure
    # on metal has one fewer explanation.
    bold "== build, and boot it under QEMU first =="
    sudo -u "$as_user" sh -c "cd '$REPO' && cargo xtask run"
    grn "built, and it boots under emulation on this machine."
}

# ----------------------------------------------------------------------------

cmd_install() {
    need_root

    local kernel="$KERNEL_DEFAULT" init="$INIT_DEFAULT" serial=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --kernel) kernel="$2"; shift 2 ;;
            --init)   init="$2";   shift 2 ;;
            --serial) serial=1;    shift ;;
            *) die "unknown option for install: $1" ;;
        esac
    done

    [ -f "$kernel" ] || die "kernel not found: $kernel"
    [ -f "$init" ]   || die "init module not found: $init"

    # Validate before touching the bootloader. An entry that points at the wrong
    # kind of file fails at boot, which is the worst place to find out.
    [ "$(head -c 4 "$kernel" | od -An -t x1 | tr -d ' \n')" = "7f454c46" ] \
        || die "$kernel is not an ELF file"
    [ "$(od -An -t u1 -j4 -N1 "$kernel" | tr -d ' ')" = "1" ] \
        || die "$kernel is not ELF32 — GRUB's multiboot command needs the .elf32 image,
       not the ELF64 one cargo leaves beside it"
    # 0x1BADB002, little-endian, within the first 8 KiB as multiboot 1 requires.
    od -An -t x1 -N 8192 "$kernel" | tr -d ' \n' | grep -q '02b0ad1b' \
        || ylw "warning: no multiboot 1 header found in the first 8 KiB of $kernel"

    local gp; gp=$(grub_path)

    bold "== installing =="
    install -d -m 0755 "$DEST"
    install -m 0644 "$kernel" "$DEST/f-kernel.elf32"
    install -m 0644 "$init"   "$DEST/init.bin"
    echo "artefacts       $DEST/  (GRUB sees them at $gp/)"

    # /etc/grub.d exists on any machine with GRUB, so this is belt and braces —
    # but a script that writes a bootloader config should not assume a directory
    # it did not check for.
    install -d -m 0755 "$(dirname "$GRUB_D")"

    cat > "$GRUB_D" <<EOF
#!/bin/sh
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Generated by tools/f-on-metal.sh. Remove with: f-on-metal.sh uninstall
#
# \`search --file --set=root\` rather than a hardcoded device: it finds whichever
# partition holds the image and sets root to it, so this entry survives /boot
# being its own partition, moving disk, or changing device names.
cat <<'MENU'
menuentry "F — milestone M0 (serial ${BAUD} 8N1)" --class f {
    echo "F: loading. All output is on COM1 at ${BAUD} 8N1 — there is no video."
    search --no-floppy --file --set=root ${gp}/f-kernel.elf32
    multiboot ${gp}/f-kernel.elf32
    module ${gp}/init.bin
}

menuentry "F — milestone M0, 60s timer jitter run" --class f {
    echo "F: loading with timer=60. Output on COM1 at ${BAUD} 8N1."
    search --no-floppy --file --set=root ${gp}/f-kernel.elf32
    multiboot ${gp}/f-kernel.elf32 timer=60
    module ${gp}/init.bin
}
MENU
EOF
    chmod 0755 "$GRUB_D"
    echo "menu entries    $GRUB_D"

    if [ "$serial" -eq 1 ]; then
        # Opt-in, because it changes where *Arch's* menu goes too. Worth it: a
        # boot that fails before the kernel starts is otherwise indistinguishable
        # from one that fails after it.
        cp -n "$DESTDIR/etc/default/grub" "$DESTDIR/etc/default/grub.f-backup" 2>/dev/null || true
        sed -i '/^GRUB_TERMINAL/d;/^GRUB_SERIAL_COMMAND/d' "$DESTDIR/etc/default/grub"
        {
            echo "GRUB_TERMINAL=\"serial console\""
            echo "GRUB_SERIAL_COMMAND=\"serial --unit=0 --speed=${BAUD} --word=8 --parity=no --stop=1\""
        } >> "$DESTDIR/etc/default/grub"
        echo "grub console    also on serial (backup: /etc/default/grub.f-backup)"
    fi

    # Back up before regenerating. This is the one step that can cost somebody
    # their machine, so the previous config is kept whatever happens.
    if [ -n "$DESTDIR" ]; then
        ylw "DESTDIR set — staged into $DESTDIR, grub.cfg not regenerated."
    else
        local backup="/boot/grub/grub.cfg.f-backup.$(date +%Y%m%d%H%M%S)"
        if [ -f /boot/grub/grub.cfg ]; then
            cp /boot/grub/grub.cfg "$backup"
            echo "grub.cfg backup $backup"
        fi
        grub-mkconfig -o /boot/grub/grub.cfg
    fi

    echo
    grn "done. Arch is still the default entry — nothing about its boot changed."

    if ! dmesg 2>/dev/null | grep -qiE 'ttyS0 at I/O 0x3f8'; then
        echo
        red "WARNING: no UART detected at 0x3f8 on this machine."
        red "F writes to nothing else. If that is still true at boot you will see a"
        red "black screen and be unable to tell success from a triple fault."
        red "Run '$PROG check' for what to do about it."
    fi
    echo
    bold "To run it:"
    echo "  1. connect a serial console at ${BAUD} 8N1 (screen /dev/ttyS0 ${BAUD}, or the BMC)"
    echo "  2. reboot and pick \"F — milestone M0\" from the GRUB menu"
    echo "  3. success is the log ending in 'M0 ok' and the machine then sitting still —"
    echo "     there is no reboot and no exit code on hardware, only the log"
    echo
    echo "Keep that log. E0-P18's exit asks for it in docs/postmortem/, beside the"
    echo "QEMU one, with the differences accounted for. The trace hash WILL differ:"
    echo "the memory map and core count are in the boot log by design."
}

# ----------------------------------------------------------------------------

cmd_uninstall() {
    need_root
    rm -f "$GRUB_D"
    rm -rf "$DEST"
    if [ -f "$DESTDIR/etc/default/grub.f-backup" ]; then
        mv "$DESTDIR/etc/default/grub.f-backup" "$DESTDIR/etc/default/grub"
        echo "restored /etc/default/grub"
    fi
    if [ -n "$DESTDIR" ]; then
        ylw "DESTDIR set — removed from $DESTDIR, grub.cfg not regenerated."
        return 0
    fi
    grub-mkconfig -o /boot/grub/grub.cfg
    grn "removed. The grub.cfg.f-backup.* files are left in /boot/grub for you to delete."
}

# ----------------------------------------------------------------------------

usage() {
    cat <<EOF
$PROG — install F as a second boot entry on a minimal Arch Linux machine.

  $PROG check                 report what this machine would do. Changes nothing.
  $PROG build                 install the toolchain, build, and boot it under QEMU first
  $PROG install [options]     add the GRUB entries beside Arch
  $PROG uninstall             remove them and regenerate grub.cfg

install options:
  --kernel <path>   default $KERNEL_DEFAULT
  --init   <path>   default $INIT_DEFAULT
  --serial          also send GRUB's own menu to serial at $BAUD

The procedure is docs/booting-on-hardware.md. Read the two facts that cost an
afternoon before starting: F has no video output at all, and its console is
$BAUD baud rather than the 115200 everybody reaches for.
EOF
}

case "${1:-}" in
    check)     shift; cmd_check "$@" ;;
    build)     shift; cmd_build "$@" ;;
    install)   shift; cmd_install "$@" ;;
    uninstall) shift; cmd_uninstall "$@" ;;
    -h|--help) usage ;;
    *)         usage; exit 1 ;;
esac
