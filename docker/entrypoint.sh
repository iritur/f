#!/bin/sh
# Make the mounted volumes writable by the development user, then stop being
# root for everything after that.
#
# This exists because of one Docker behaviour that is easy to be surprised by:
# a named volume mounted over a path arrives owned by root, so a container
# running as a non-root user cannot write to it. `target/`, the cargo registry
# and the cargo git cache are all named volumes here — for good reasons, set
# out in docker/README.md — so all three need this.
#
# The fix has to happen at run time, as root, because ownership of a volume is
# not a property of the image.
set -e

F_UID="${F_UID:-1000}"
F_GID="${F_GID:-1000}"

if [ "$(id -u)" = "0" ]; then
    # /opt/cargo itself, not recursively: cargo writes its global package lock
    # (.package-cache) directly there, and the toolchain below it should stay
    # root-owned and read-only.
    chown "$F_UID:$F_GID" /opt/cargo

    for dir in /work/target /opt/cargo/registry /opt/cargo/git /opt/cargo/advisory-dbs; do
        mkdir -p "$dir"
        # Recursive only when the top-level owner is wrong, which is the
        # first-run case when there is almost nothing to recurse over. Once the
        # registry is populated this test is false and costs nothing.
        if [ "$(stat -c %u "$dir")" != "$F_UID" ]; then
            chown -R "$F_UID:$F_GID" "$dir"
        fi
    done

    # setpriv changes the ids and nothing else, so HOME still says /root and
    # the first thing any shell does is fail to read a file it cannot see.
    export HOME=/home/f

    # Git refuses to operate in a repository owned by somebody else — the
    # `dubious ownership` guard — and /work is a bind mount whose ownership
    # Docker Desktop fakes on Windows and does not fake on Linux. The result is
    # that `git` did not work inside this image at all, in either direction:
    # every command failed with the same fatal, including the ones the build
    # tooling runs. Marking the one directory this image exists to build is the
    # narrow fix; `--global` here is the container's own config and not the
    # user's, because $HOME is inside the image.
    #
    # Found by `cargo xtask history append`, which asks git which commit it is
    # recording. Nothing before it had needed git, so the gap was invisible.
    su_git() { setpriv --reuid="$F_UID" --regid="$F_GID" --init-groups -- "$@"; }
    su_git git config --global --add safe.directory /work 2>/dev/null || true

    # setpriv rather than su or sudo: no PAM, no login shell, no extra process
    # sitting between the caller and the command, so an exit code and a signal
    # both reach the right place. `cargo xtask run` asserts on QEMU's exit
    # code, so this matters here more than it usually would.
    exec setpriv --reuid="$F_UID" --regid="$F_GID" --init-groups -- "$@"
fi

exec "$@"
