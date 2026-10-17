# Rift shell integration for fish — OSC 133 block markers + OSC 7 cwd.
# Auto-loaded via vendor_conf.d (Rift appends its data dir to
# XDG_DATA_DIRS); safe to source manually too. fish 4 emits some prompt
# marking natively — duplicates are harmless, Rift's parser is idempotent.

status is-interactive; or exit 0
# Shell-local guard (not exported): a nested or exec'd fish must load its own
# hooks, which an exported variable would suppress.
if set -q __rift_loaded
    exit 0
end
set -g __rift_loaded 1

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
function __rift_report_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
end

# Prompt chips: the active Python environment, and the php and
# python3 that PATH resolves to, as iTerm2 user variables (OSC 1337
# SetUserVar, base64), sent when they change; PATH itself too (command-name
# completion follows `rift-use`).
set -g __rift_py ''
set -g __rift_path ''
function __rift_set_var
    printf '\e]1337;SetUserVar=%s=%s\a' $argv[1] (printf %s "$argv[2]" | base64 | string join '')
end
function __rift_report_env --on-event fish_prompt
    set -l py (string replace -r '.*/' '' -- "$VIRTUAL_ENV")
    test -n "$py"; or set py "$PYENV_VERSION"
    if test "$py" != "$__rift_py"
        set -g __rift_py "$py"
        __rift_set_var RiftPython "$py"
    end
    if test "$PATH" != "$__rift_path"
        set -g __rift_path "$PATH"
        __rift_set_var RiftPath (string join : $PATH)
        __rift_set_var RiftPhpBin (command -s php)
        __rift_set_var RiftPythonBin (command -s python3)
    end
end

# The PHP and Python chips' version pickers run this: `rift-use php|python
# <bin dir>` puts that directory first on PATH (dropping the one picked
# before), `rift-use pyenv <version>` sets PYENV_VERSION. This shell only.
function rift-use
    switch "$argv[1]"
        case pyenv
            set -gx PYENV_VERSION $argv[2]
            return
        case php python
        case '*'
            echo 'usage: rift-use php|python <bin dir> | rift-use pyenv <version>' >&2
            return 2
    end
    set -l dir $argv[2]
    if not test -d "$dir"
        echo "rift-use: no such directory: $dir" >&2
        return 1
    end
    set -l var __rift_use_$argv[1]
    if set -q $var
        set -gx PATH (string match -v -- $$var $PATH)
    end
    set -gx PATH $dir $PATH
    set -g $var $dir
end

# Rift runs `rift-complete` over an SSH profile's shared connection: it
# answers completion requests on stdin with this shell's completions
# (complete.fish next to this file; its header has the protocol).
set -g __rift_complete_srv (string replace -r '[^/]*$' '' -- (status filename))complete.fish
function rift-complete
    test -r $__rift_complete_srv; or return 1
    source $__rift_complete_srv
end

function __rift_preexec --on-event fish_preexec
    set -g __rift_cmd_ran 1
    printf '\e]133;C\a'
end

function __rift_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
    set -e __rift_cmd_ran
end

# Wrap fish_prompt (A before, B after) right before each prompt is drawn, so a
# fish_prompt defined later (config.fish, a prompt framework) is wrapped too.
function __rift_wrap_prompt --on-event fish_prompt
    functions fish_prompt | string match -q '*__rift_orig_prompt*'; and return
    functions -e __rift_orig_prompt
    functions -c fish_prompt __rift_orig_prompt
    function fish_prompt
        printf '\e]133;A\a'
        __rift_orig_prompt
        printf '\e]133;B\a'
    end
end

__rift_report_cwd
