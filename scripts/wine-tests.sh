#!/bin/sh
# Run every Windows test binary under Wine, each from its crate's folder:
# the `windows` gate's second half (scripts/gates.sh), inside
# packaging/windows-wine.Dockerfile. Usage: wine-tests.sh <repo>
# The binaries are listed one per line in
# target/x86_64-pc-windows-gnu/win-tests.txt. One test thread: the x86
# emulation under Docker on Apple silicon crashes Wine with several
# ("rosetta error: invalid gdt selector").
set -u
repo=$1
bins="$repo/target/x86_64-pc-windows-gnu/debug/deps"
failed=0
while read -r t; do
    case $t in
        rift_core-*|shell_complete-*|completion_bench-*|remote_complete-*) crate=rift-core ;;
        rift_pty-*) crate=rift-pty ;;
        rift_renderer-*) crate=rift-renderer ;;
        rift_terminal-*) crate=rift-terminal ;;
        *) crate=rift-remote ;;
    esac
    echo "== $t"
    run() { (cd "$repo/crates/$crate" && WINEDEBUG=err+all timeout 900 wine "$bins/$t" --test-threads=1 2>&1); }
    out=$(run)
    rc=$?
    # the emulator itself crashed (never a test's failure): up to twice more
    for again in 1 2; do
        if [ "$rc" -eq 0 ] || ! printf '%s\n' "$out" | grep -q 'rosetta error'; then break; fi
        echo "   (x86 emulation crashed; running $t again, $again of 2)"
        out=$(run)
        rc=$?
    done
    printf '%s\n' "$out" | grep -E '^test .*FAILED|^test result|panicked at|^---- ' || true
    if [ "$rc" -ne 0 ]; then
        printf '%s\n' "$out" | tail -40
        failed=1
    fi
done < "$repo/target/x86_64-pc-windows-gnu/win-tests.txt"
exit "$failed"
