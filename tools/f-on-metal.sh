#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# Install F as a second boot entry on a minimal Arch Linux machine.
#
# The procedure this automates is docs/booting-on-hardware.md, and the task it
# serves is E0-P18: this kernel has run outside QEMU exactly once, on a VMware
# machine on 2026-09-01 (docs/first-boot-outside-qemu.md), and never on bare
# metal. So the first boot on a real machine is still an experiment rather than
# a deployment, and that page's own opening says why a hypervisor is not the
# machine E0-P18 is about.
#
# It is written for that: `check` reports what would go wrong and changes
# nothing you did not agree to, `deploy-grub` fixes the one blocker it can,
# `install` adds a menu entry beside Arch without touching the default, and
# `uninstall` removes it. Arch stays bootable at every point — which matters
# because the machine this runs on is one somebody has to be able to use
# tomorrow.
#
#   ./tools/f-on-metal.sh check                 what this machine would do
#   ./tools/f-on-metal.sh deploy-grub           make the multiboot module exist
#   ./tools/f-on-metal.sh build                 toolchain, then build and smoke-test
#   ./tools/f-on-metal.sh install [--serial]    add the GRUB entries
#   ./tools/f-on-metal.sh uninstall             remove them
#
# `install` takes the two artefacts from ./target by default, or from
# --kernel <path> --init <path> if they were built elsewhere and copied over.
#
# Further modules are optional: *component files* (each a compiled manifest
# followed by an image, RFC 0030). The frame fills one place per component file
# it is given (RFC 0044), so what they decide is the size of the topology this
# boot has — with none, the kernel prints a line saying so and carries on.
# Neither is a failure, which is why they are not required artefacts.
#
# `cargo xtask component` builds four of them today: the store runtime and the
# three drivers. `install` carries every one it finds, sorted, because the boot
# under QEMU carries all of them and a hardware log is compared against that
# one — E0-P18's exit asks for every difference to be accounted for, and a
# topology that is smaller for no stated reason is a difference nobody meant.

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
    # The optimised image, not the debug one `cargo xtask` builds and tests.
    # The difference is boot time on a machine with real memory: the frame
    # allocator writes one word into every frame of RAM, and the debug build
    # nearly doubles that wait (medians at 4 GiB under QEMU: 3818 ms debug,
    # 2220 ms optimised). `cmd_build` builds this image and boots it under
    # QEMU before anything is installed — the optimised build is the less
    # travelled one, and the first time it was ever booted it did not work.
    KERNEL_DEFAULT="$REPO/target/x86_64-unknown-none/release/f-kernel.elf32"
    INIT_DEFAULT="$REPO/target/init/init.bin"
    # Optional, and a directory rather than a file: `cargo xtask component`
    # writes one `.fc` per component and the frame takes a place per file. A
    # checkout that has not run one has an empty directory here and the entries
    # are generated with no module lines past `init.bin`.
    COMPONENT_DIR_DEFAULT="$REPO/target/component"
else
    IN_REPO=0
    KERNEL_DEFAULT="./f-kernel.elf32"
    INIT_DEFAULT="./init.bin"
    COMPONENT_DIR_DEFAULT="."
fi

# Every component file in a directory, sorted, newline separated.
#
# Sorted because the frame fills a place per module in the order the loader
# hands them over (kernel/src/component.rs walks `boot.modules()`), so the order
# decides which place each component lands in — and an order taken from the
# filesystem is one that can differ between two machines carrying the same
# files. That is the same reasoning `artefacts` in xtask/src/main.rs carries
# about link order, and it was learned there the expensive way.
#
# It is deliberately *not* the order the QEMU boot uses. That one is `COMPONENTS`
# in xtask/src/main.rs, a hand-written list, and this script cannot read it from
# a machine with no checkout. So place indices here will differ from the ones in
# the QEMU log, and that is one of the differences E0-P18's exit asks to be
# accounted for rather than a fault: a component is found by the magic in its
# record and not by its position, so what moves is which place it occupies and
# never which bytes it is built from.
component_files() {
    [ -d "$1" ] || return 0
    find "$1" -maxdepth 1 -name '*.fc' -type f 2>/dev/null | LC_ALL=C sort
}

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
# Is there a real UART at 0x3f8?
#
# 0 yes, 1 no, 2 could not tell — and the third is the reason this is a function
# rather than a grep. The first version asked dmesg, with stderr discarded, and
# `dmesg` is root-only wherever `kernel.dmesg_restrict` is 1, which is the
# default on Arch. So every non-root run reported a missing serial port on a
# machine that had one: a check answering a question it had not been able to
# ask. "I could not tell" is a different answer from "there is none" and must
# never be rendered as one.
#
# sysfs first, because it needs no privilege and describes the current state
# rather than a boot-time log that can wrap out of the ring buffer.
SERIAL_WHY=""
serial_at_3f8() {
    local port type
    if [ -r /sys/class/tty/ttyS0/type ] && [ -r /sys/class/tty/ttyS0/port ]; then
        type=$(cat /sys/class/tty/ttyS0/type 2>/dev/null || echo 0)
        port=$(tr 'a-f' 'A-F' < /sys/class/tty/ttyS0/port 2>/dev/null || echo "")
        # The 8250 driver registers the legacy addresses whether or not anything
        # answers at them, so `port` alone proves nothing — a machine with no
        # serial hardware still shows 0x3F8 here. `type` is the probe result:
        # 0 is PORT_UNKNOWN, and anything else is a UART it actually identified
        # (4 is a 16550A).
        if [ "$type" != "0" ] && [ "${port#0x}" = "3F8" ]; then
            SERIAL_WHY="/sys/class/tty/ttyS0: type=$type port=$port"
            return 0
        fi
        SERIAL_WHY="/sys/class/tty/ttyS0: type=$type — 0 means nothing answered at $port"
        return 1
    fi

    if dmesg >/dev/null 2>&1; then
        if dmesg 2>/dev/null | grep -qiE 'ttyS0 at I/O 0x3f8'; then
            SERIAL_WHY="dmesg: the 8250 driver found a UART at 0x3f8"
            return 0
        fi
        SERIAL_WHY="dmesg: no 8250 UART reported at 0x3f8"
        return 1
    fi

    SERIAL_WHY="no sysfs entry, and dmesg is refused — kernel.dmesg_restrict is 1. Re-run with sudo."
    return 2
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
        red "                and not in use. That is fine and does not have to be undone:"
        red "                GRUB can sit beside it on the same ESP, and you pick it from"
        red "                the firmware boot menu when you want F. systemd-boot cannot"
        red "                load a multiboot 1 kernel at all, so something has to."
        red ""
        red "                  $PROG deploy-grub     does the above, after showing you"
        red "                                        exactly what it will run"
        fail=$((fail + 1))
        OFFER_DEPLOY=1
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
    local src=0
    serial_at_3f8 || src=$?
    if [ "$src" -eq 0 ]; then
        grn "serial          a real UART at 0x3f8 (COM1) — the kernel probed it"
        echo "                $SERIAL_WHY"
        echo "                connect at ${BAUD} 8N1"
    elif [ "$src" -eq 2 ]; then
        ylw "serial          could not tell"
        ylw "                $SERIAL_WHY"
        warn=$((warn + 1))
    else
        red "serial          no UART detected at 0x3f8"
        red "                $SERIAL_WHY"
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
        # One of them this script can fix, so it offers rather than only naming
        # the command — but only when it could actually carry it out, and never
        # without asking. `check` changes nothing you did not say yes to.
        if [ "${OFFER_DEPLOY:-0}" -eq 1 ] && [ "$(id -u)" -eq 0 ] && [ -t 0 ]; then
            echo
            printf 'Deploy GRUB now, so the multiboot module exists? [y/N] '
            local reply=""
            read -r reply || true
            case "$reply" in
                [yY]|[yY][eE][sS]) echo; cmd_deploy_grub; return $? ;;
                *) ylw "not run — '$PROG deploy-grub' when you are ready." ;;
            esac
        elif [ "${OFFER_DEPLOY:-0}" -eq 1 ]; then
            echo
            echo "Run '$PROG deploy-grub' as root to have this script do the GRUB half."
        fi
        return 1
    fi
    grn "no blocking problems, $warn warning(s)."
    echo "Next: $PROG install"
}

# ----------------------------------------------------------------------------

# Deploy GRUB's modules to this machine's /boot, which is what makes the
# multiboot command available at boot time.
#
# This writes a bootloader, so it does two things before it does anything: it
# works out the target itself rather than accepting a guess, and it refuses when
# it cannot. `grub-install --target=i386-pc /dev/sdX` on the wrong disk is the
# single most destructive command this script could run, so there is no default
# for it — either the disk is derived from what /boot is actually mounted from,
# or the caller passes --disk and owns the choice.
#
# It does not remove or displace systemd-boot. Both live on one ESP quite
# happily; this adds GRUB beside it and leaves the firmware's boot order alone.
cmd_deploy_grub() {
    need_root

    local disk="" esp="" assume=0 keep_order=0
    BOOT_ORDER_BEFORE=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --disk)            disk="$2"; shift 2 ;;
            --yes)             assume=1;  shift ;;
            --keep-boot-order) keep_order=1; shift ;;
            *) die "unknown option for deploy-grub: $1" ;;
        esac
    done

    command -v grub-install >/dev/null 2>&1 || die "grub-install not found — pacman -S grub"

    local cmd
    if [ -d /sys/firmware/efi ]; then
        # bootctl knows the ESP on any systemd machine, which Arch is. The
        # fallback looks for a mounted vfat in the three conventional places.
        esp=$(bootctl --print-esp-path 2>/dev/null || true)
        if [ -z "$esp" ]; then
            local c
            for c in /efi /boot/efi /boot; do
                if [ "$(findmnt -no FSTYPE "$c" 2>/dev/null)" = "vfat" ]; then esp="$c"; break; fi
            done
        fi
        [ -n "$esp" ] || die "cannot find the EFI system partition.
       Look for it with 'findmnt -t vfat', then run grub-install by hand:
         grub-install --target=x86_64-efi --efi-directory=<esp> --bootloader-id=GRUB"
        cmd="grub-install --target=x86_64-efi --efi-directory=$esp --bootloader-id=GRUB"
    else
        if [ -z "$disk" ]; then
            # The disk /boot lives on, via its parent block device. Empty when
            # /boot is on LVM, RAID or the whole device — all cases where a
            # guess would be wrong, so it stops instead.
            local src
            src=$(findmnt -no SOURCE /boot 2>/dev/null || findmnt -no SOURCE / 2>/dev/null || true)
            [ -n "$src" ] || die "cannot tell what /boot is mounted from"
            local parent
            parent=$(lsblk -no PKNAME "$src" 2>/dev/null | head -1 || true)
            [ -n "$parent" ] || die "cannot derive the disk for '$src' — LVM, RAID or a whole-device mount.
       Name it yourself, and check it twice:  $PROG deploy-grub --disk /dev/sdX"
            disk="/dev/$parent"
        fi
        [ -b "$disk" ] || die "$disk is not a block device"
        cmd="grub-install --target=i386-pc $disk"
    fi

    bold "== deploy GRUB =="
    echo
    echo "  $cmd"
    echo "  grub-mkconfig -o /boot/grub/grub.cfg"
    echo
    if [ -d /sys/firmware/efi ]; then
        echo "  ESP:            $esp"
        echo
        echo "  This adds GRUB to the ESP beside whatever is already there and does not"
        echo "  remove systemd-boot."
        echo
        ylw "  It DOES change the firmware boot order. grub-install calls efibootmgr,"
        ylw "  which adds a GRUB entry and puts it first, so GRUB becomes what boots by"
        ylw "  default. systemd-boot stays installed and selectable, and GRUB's own menu"
        ylw "  will list Arch — so the machine still boots — but the front door changes."
        if [ "$keep_order" -eq 1 ]; then
            echo
            grn "  --keep-boot-order given: the current BootOrder will be restored"
            grn "  afterwards, leaving systemd-boot as the default and GRUB reachable"
            grn "  from the firmware boot menu."
        else
            echo
            echo "  Pass --keep-boot-order to put the order back afterwards. Either way"
            echo "  the before and after are printed, with the command to restore it."
        fi
        BOOT_ORDER_BEFORE=$(efibootmgr 2>/dev/null | sed -n 's/^BootOrder: //p' || true)
        [ -n "$BOOT_ORDER_BEFORE" ] && echo "  BootOrder now:  $BOOT_ORDER_BEFORE"
    else
        echo "  Disk:           $disk   <- this gets a bootloader written to it"
        echo "  /boot is on:    $(findmnt -no SOURCE /boot 2>/dev/null || echo '(not a separate mount)')"
    fi
    echo

    if [ "$assume" -eq 0 ]; then
        if [ ! -t 0 ]; then
            die "not a terminal, so not prompting. Re-run with --yes if this is what you want."
        fi
        printf 'Run it? [y/N] '
        local reply=""
        read -r reply || true
        case "$reply" in
            [yY]|[yY][eE][sS]) ;;
            *) ylw "not run."; return 0 ;;
        esac
    fi

    $cmd
    grub-mkconfig -o /boot/grub/grub.cfg

    # The firmware boot order, honestly. grub-install has just put GRUB first;
    # say so with both values rather than leaving somebody to discover it at the
    # next reboot, and hand over the command that undoes it.
    if [ -d /sys/firmware/efi ] && [ -n "$BOOT_ORDER_BEFORE" ]; then
        local after
        after=$(efibootmgr 2>/dev/null | sed -n 's/^BootOrder: //p' || true)
        echo
        echo "BootOrder before  $BOOT_ORDER_BEFORE"
        echo "BootOrder after   ${after:-unknown}"
        if [ "$after" != "$BOOT_ORDER_BEFORE" ]; then
            if [ "$keep_order" -eq 1 ]; then
                efibootmgr -o "$BOOT_ORDER_BEFORE" >/dev/null
                grn "restored — systemd-boot is still your default; pick GRUB from the"
                grn "firmware boot menu when you want F."
            else
                ylw "GRUB is now the default boot manager. To put it back:"
                ylw "  efibootmgr -o $BOOT_ORDER_BEFORE"
            fi
        fi
    fi

    # Verify rather than assume. The whole point was to make one file exist.
    local now=""
    local d
    for d in /boot/grub/i386-pc /boot/grub/x86_64-efi /boot/grub2/i386-pc /boot/grub2/x86_64-efi; do
        [ -f "$d/multiboot.mod" ] && now="$d/multiboot.mod"
    done
    echo
    if [ -n "$now" ]; then
        grn "deployed: $now"
        echo "Re-run '$PROG check' — the multiboot finding should be gone."
    else
        red "grub-install reported success and multiboot.mod is still not in /boot."
        red "Something is deploying somewhere this script does not look. Find it with:"
        red "  find / -name multiboot.mod 2>/dev/null"
        return 1
    fi
}

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

    # The image this script installs is the optimised one, and nothing above
    # built it: `cargo xtask` builds and boots the debug image on purpose, and
    # what the release package carries is RELEASING.md's decision, not this
    # script's. It is built here, and then *booted* here, because it is the
    # less travelled build — the first time an optimised image of this kernel
    # was ever booted, 2026-09-01, a miscompiled `cpuid` wrapper cost the
    # machine every core but the first, and the debug suite had been green
    # throughout. Emulation catching that class here is the whole reason this
    # step exists.
    bold "== the optimised image this script installs =="
    sudo -u "$as_user" sh -c "cd '$REPO' \
        && cargo build -p f-kernel --target x86_64-unknown-none \
            -Zbuild-std=core,compiler_builtins --release \
        && sysroot=\$(rustc --print sysroot) \
        && host=\$(rustc -vV | sed -n 's/^host: //p') \
        && \"\$sysroot/lib/rustlib/\$host/bin/llvm-objcopy\" -O elf32-i386 \
            target/x86_64-unknown-none/release/f-kernel \
            target/x86_64-unknown-none/release/f-kernel.elf32"

    bold "== and boot that too, before the firmware is asked to =="
    qemu-system-x86_64 -kernel "$KERNEL_DEFAULT" -initrd "$INIT_DEFAULT" \
        -smp 2 -m 128M -serial null -display none \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot \
        >/dev/null 2>&1
    rc=$?
    [ "$rc" -eq 33 ] \
        || die "the optimised image did not reach M0 ok under QEMU (exit $rc).
       The debug image just booted, so this is an optimisation-dependent
       kernel bug. Do not install it; report the exit code instead."
    grn "built, and both images boot under emulation on this machine."
}

# ----------------------------------------------------------------------------

cmd_install() {
    need_root

    local kernel="$KERNEL_DEFAULT" init="$INIT_DEFAULT" serial=0
    local components="$COMPONENT_DIR_DEFAULT" want_components=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --kernel)     kernel="$2"; shift 2 ;;
            --init)       init="$2";   shift 2 ;;
            --components) components="$2"; want_components=1; shift 2 ;;
            --serial)     serial=1;    shift ;;
            *) die "unknown option for install: $1" ;;
        esac
    done

    [ -f "$kernel" ] || die "kernel not found: $kernel"
    [ -f "$init" ]   || die "init module not found: $init"
    # Named explicitly and holding nothing is an error; defaulted and empty is
    # not. The kernel boots either way and says what it got, so a machine with
    # no component files is a smaller topology rather than a failure.
    if [ "$want_components" -eq 1 ]; then
        [ -d "$components" ] || die "component directory not found: $components"
        [ -n "$(component_files "$components")" ] \
            || die "no *.fc component files in: $components"
    fi

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
    # One module line per component file, or nothing at all. Built as a single
    # string so the two menu entries below cannot disagree about what is there.
    local module_component="" count=0 f base
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        base=$(basename "$f")
        install -m 0644 "$f" "$DEST/$base"
        module_component="${module_component}
    module ${gp}/${base}"
        count=$((count + 1))
        echo "component       $DEST/$base"
    done <<EOC
$(component_files "$components")
EOC
    if [ "$count" -eq 0 ]; then
        echo "component       none  (the kernel will say so and carry on)"
    else
        echo "component       $count file(s) — the frame fills one place per file, RFC 0044"
    fi
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
    insmod part_gpt
    insmod part_msdos
    insmod fat
    insmod ext2
    insmod multiboot
    search --no-floppy --file --set=root ${gp}/f-kernel.elf32
    multiboot ${gp}/f-kernel.elf32
    module ${gp}/init.bin${module_component}
}

menuentry "F — milestone M0, 60s timer jitter run" --class f {
    echo "F: loading with timer=60. Output on COM1 at ${BAUD} 8N1."
    insmod part_gpt
    insmod part_msdos
    insmod fat
    insmod ext2
    insmod multiboot
    search --no-floppy --file --set=root ${gp}/f-kernel.elf32
    multiboot ${gp}/f-kernel.elf32 timer=60
    module ${gp}/init.bin${module_component}
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

    if ! serial_at_3f8; then
        echo
        red "WARNING: no UART confirmed at 0x3f8 on this machine."
        red "         $SERIAL_WHY"
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
    echo "Keep that log. E0-P18's exit asks for it in docs/ beside the QEMU one."
    echo "docs/first-boot-outside-qemu.md is the shape, and it is the record of a"
    echo "run rather than a page edited afterwards. Every difference has to be"
    echo "accounted for: memory map, core count, cores versus present, and the"
    echo "trace hash, which MUST differ and is not a determinism failure here."
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

  $PROG check
      Report what this machine would do. Changes nothing unless you answer yes
      to something it offers.
  $PROG deploy-grub [opts]
      Deploy GRUB's modules to /boot, so the multiboot command exists at boot.
      Shows the exact command first and asks. Does not remove systemd-boot —
      both live on one ESP, and you pick GRUB from the firmware boot menu.
  $PROG build
      Install the toolchain, build, and boot the result under QEMU here first.
  $PROG install [options]
      Add the GRUB entries beside Arch, without touching the default.
  $PROG uninstall
      Remove them and regenerate grub.cfg.

deploy-grub options:
  --disk <dev>      BIOS only: the disk to write to. Derived from /boot when it
                    can be, and required when it cannot — LVM, RAID, whole-device.
  --yes             do not prompt
  --keep-boot-order UEFI only: restore the firmware BootOrder afterwards, so
                    grub-install does not leave GRUB as the default boot manager

install options:
  --kernel <path>   default $KERNEL_DEFAULT
  --init   <path>   default $INIT_DEFAULT
  --components <d>  directory of component files, default $COMPONENT_DIR_DEFAULT.
                    Every *.fc in it is carried, sorted, and the frame fills one
                    place per file (RFC 0044); with none the kernel says so and
                    carries on
  --serial          also send GRUB's own menu to serial at $BAUD

The procedure is docs/booting-on-hardware.md. Read the two facts that cost an
afternoon before starting: F has no video output at all, and its console is
$BAUD baud rather than the 115200 everybody reaches for.
EOF
}

case "${1:-}" in
    check)       shift; cmd_check "$@" ;;
    deploy-grub) shift; cmd_deploy_grub "$@" ;;
    build)     shift; cmd_build "$@" ;;
    install)   shift; cmd_install "$@" ;;
    uninstall) shift; cmd_uninstall "$@" ;;
    -h|--help) usage ;;
    *)         usage; exit 1 ;;
esac
