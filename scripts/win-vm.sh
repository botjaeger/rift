#!/usr/bin/env bash
# Run a Rift script on a Windows machine over SSH and fetch what it wrote.
#
#   scripts/win-vm.sh [--build] [--desk SECONDS] [--env K=V]... SCRIPT.rift OUT_DIR
#
# RIFT_WIN_HOST (default claude@192.168.64.3) needs OpenSSH with key login.
# The run happens in the logged-in user's desktop session (a scheduled task
# with /it): a program started from SSH itself has no desktop. Rift runs
# isolated (XDG dirs under C:\rift, RIFT_NO_NOTIFY=1), not --headless, so
# the real window, title bar and GPU path are what is tested.
#
# --build      cross-build the release binary for x86_64-pc-windows-gnu first
# --desk N     also capture the whole desktop N seconds after the start
# --env K=V    set an environment variable for the run (repeatable)
#
# The script's own `shot` / `dump` paths should point under C:\rift\out\;
# that folder is emptied before the run and copied to OUT_DIR after it,
# with out.log (stdout and stderr, then the exit code) and desk.png.
set -euo pipefail

host=${RIFT_WIN_HOST:-claude@192.168.64.3}
target=x86_64-pc-windows-gnu
build=0
desk=0
env=''
while [[ $# -gt 0 ]]; do
    case $1 in
    --build) build=1; shift ;;
    --desk) desk=$2; shift 2 ;;
    --env) env+="set $2"$'\r\n'; shift 2 ;;
    *) break ;;
    esac
done
[[ $# -eq 2 ]] || { echo "usage: $0 [--build] [--desk SECONDS] SCRIPT.rift OUT_DIR" >&2; exit 2; }
script=$1
out=$2
root=$(cd "$(dirname "$0")/.." && pwd)
exe=$root/target/$target/release/rift-terminal.exe

if ((build)); then
    (cd "$root" && cargo build --release --locked --target "$target" -p rift-terminal)
fi
[[ -f $exe ]] || { echo "no $exe (use --build)" >&2; exit 1; }
mkdir -p "$out"

ssh=(ssh -o BatchMode=yes "$host")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

printf '@echo off\r\nset XDG_CONFIG_HOME=C:\\rift\\cfg\r\nset XDG_DATA_HOME=C:\\rift\\data\r\nset RIFT_NO_NOTIFY=1\r\nset RUST_LOG=info\r\n%sC:\\rift\\test\\rift-terminal.exe --script C:\\rift\\t.rift > C:\\rift\\out\\out.log 2>&1\r\necho exit %%ERRORLEVEL%% >> C:\\rift\\out\\out.log\r\n' "$env" >"$work/run.cmd"
cat >"$work/desk.ps1" <<EOF
Start-Sleep $desk
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
\$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
\$bmp = New-Object System.Drawing.Bitmap \$b.Width, \$b.Height
\$g = [System.Drawing.Graphics]::FromImage(\$bmp)
\$g.CopyFromScreen(\$b.Location, [System.Drawing.Point]::Empty, \$b.Size)
\$bmp.Save('C:\rift\out\desk.png')
EOF

# the test copy lives in C:\rift\test: a Rift the user opened from
# C:\rift keeps its own .exe locked
"${ssh[@]}" '(if exist C:\rift\out rmdir /s /q C:\rift\out) & for %d in (C:\rift C:\rift\out C:\rift\cfg C:\rift\data C:\rift\test) do @if not exist %d mkdir %d'
scp -q -o BatchMode=yes "$exe" "$host:C:/rift/test/rift-terminal.exe"
scp -q -o BatchMode=yes "$work/run.cmd" "$work/desk.ps1" "$host:C:/rift/"
scp -q -o BatchMode=yes "$script" "$host:C:/rift/t.rift"
"${ssh[@]}" 'schtasks /create /tn riftvm /tr C:\rift\run.cmd /sc once /st 23:59 /it /f >nul && schtasks /run /tn riftvm >nul'
if ((desk > 0)); then
    "${ssh[@]}" 'schtasks /create /tn riftvmdesk /tr "powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\rift\desk.ps1" /sc once /st 23:59 /it /f >nul && schtasks /run /tn riftvmdesk >nul'
fi

# wait for the exit line (at most 3 minutes)
for _ in $(seq 1 60); do
    if "${ssh[@]}" 'findstr /b exit C:\rift\out\out.log >nul 2>nul'; then
        break
    fi
    "${ssh[@]}" 'powershell -NoProfile -Command "Start-Sleep 3"'
done
if ((desk > 0)); then
    # shellcheck disable=SC2016 # PowerShell's own variables, run remotely
    "${ssh[@]}" 'powershell -NoProfile -Command "$i = 0; while (!(Test-Path C:\rift\out\desk.png) -and $i -lt 60) { Start-Sleep 1; $i++ }"'
fi
scp -q -r -o BatchMode=yes "$host:C:/rift/out/*" "$out/"
tail -1 "$out/out.log"
