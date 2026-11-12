# Rift shell integration for PowerShell (7 and Windows PowerShell 5.1):
# OSC 133 block markers + OSC 7 cwd. Rift starts PowerShell with
# `-NoExit -Command ". '<this file>'"`, so the user's profile runs first and
# its prompt is the one wrapped; safe to dot-source manually too.

if ($global:__RiftLoaded) { return }
$global:__RiftLoaded = $true

# UTF-8 both ways: ConPTY passes bytes, and box drawing, emoji and non-Latin
# file names must not turn into "?"
try {
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
    [Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
    $global:OutputEncoding = [System.Text.UTF8Encoding]::new($false)
} catch {}

$global:__RiftEsc = [char]27
$global:__RiftBel = [char]7
$global:__RiftPrompts = 0
$global:__RiftOrigPrompt = $function:prompt

# OSC 7 wants a percent-encoded path: file://host/C:/Users/me/my%20dir
#   (Windows: file://host/C:/Users/me; macOS and Linux: file://host/home/me)
function global:__RiftCwdUrl {
    $loc = $ExecutionContext.SessionState.Path.CurrentLocation
    if ($loc.Provider.Name -ne 'FileSystem') { return $null }
    $parts = $loc.ProviderPath -split '[\\/]' | Where-Object { $_ } |
        ForEach-Object { [uri]::EscapeDataString($_) }
    "file://$([Environment]::MachineName)/" + ($parts -join '/')
}

function global:prompt {
    # first: any command below resets $?
    $ok = $?
    $code = if ($ok) { 0 } elseif ($global:LASTEXITCODE) { $global:LASTEXITCODE } else { 1 }
    $e = $global:__RiftEsc
    $b = $global:__RiftBel
    $out = ''
    # D closes the block of the command that just ran (none before the first
    # prompt)
    if ($global:__RiftPrompts -gt 0) { $out += "$e]133;D;$code$b" }
    $global:__RiftPrompts++
    $url = __RiftCwdUrl
    if ($url) { $out += "$e]7;$url$b" }
    $out += "$e]133;A$b"
    $out += & $global:__RiftOrigPrompt
    $out += "$e]133;B$b"
    $out
}

# C marker with the command line as read (kitty's cmdline_url,
# percent-encoded): the block header keeps it exact. Only with PSReadLine
# (on by default in every interactive PowerShell 5.1+).
if (Get-Command PSConsoleHostReadLine -ErrorAction SilentlyContinue) {
    # Rift ends a command with LF on macOS and Linux (CR on Windows): .NET
    # reads LF as Ctrl+Enter, which PSReadLine would take as a new line
    try { Set-PSReadLineKeyHandler -Chord 'Ctrl+Enter' -Function AcceptLine } catch {}
    $global:__RiftOrigReadLine = $function:PSConsoleHostReadLine
    function global:PSConsoleHostReadLine {
        $line = & $global:__RiftOrigReadLine
        [Console]::Write("$($global:__RiftEsc)]133;C;cmdline_url=$([uri]::EscapeDataString($line))$($global:__RiftBel)")
        $line
    }
}
