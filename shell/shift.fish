# Shift shell integration for fish — OSC 133 block markers + OSC 7 cwd.
# Auto-loaded via vendor_conf.d (Shift appends its data dir to
# XDG_DATA_DIRS); safe to source manually too. fish 4 emits some prompt
# marking natively — duplicates are harmless, Shift's parser is idempotent.

status is-interactive; or exit 0
# Shell-local guard (not exported): a nested or exec'd fish must load its own
# hooks, which an exported variable would suppress.
if set -q __shift_loaded
    exit 0
end
set -g __shift_loaded 1

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
function __shift_report_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' (hostname) (string escape --style=url -- $PWD)
end

# Prompt chips: the active Python environment, and the php and
# python3 that PATH resolves to, as iTerm2 user variables (OSC 1337
# SetUserVar, base64), sent when they change.
set -g __shift_py ''
set -g __shift_path ''
function __shift_set_var
    printf '\e]1337;SetUserVar=%s=%s\a' $argv[1] (printf %s "$argv[2]" | base64 | string join '')
end
function __shift_report_env --on-event fish_prompt
    set -l py (string replace -r '.*/' '' -- "$VIRTUAL_ENV")
    test -n "$py"; or set py "$PYENV_VERSION"
    if test "$py" != "$__shift_py"
        set -g __shift_py "$py"
        __shift_set_var ShiftPython "$py"
    end
    if test "$PATH" != "$__shift_path"
        set -g __shift_path "$PATH"
        __shift_set_var ShiftPhpBin (command -s php)
        __shift_set_var ShiftPythonBin (command -s python3)
    end
end

# The PHP and Python chips' version pickers run this: `shift-use php|python
# <bin dir>` puts that directory first on PATH (dropping the one picked
# before), `shift-use pyenv <version>` sets PYENV_VERSION. This shell only.
function shift-use
    switch "$argv[1]"
        case pyenv
            set -gx PYENV_VERSION $argv[2]
            return
        case php python
        case '*'
            echo 'usage: shift-use php|python <bin dir> | shift-use pyenv <version>' >&2
            return 2
    end
    set -l dir $argv[2]
    if not test -d "$dir"
        echo "shift-use: no such directory: $dir" >&2
        return 1
    end
    set -l var __shift_use_$argv[1]
    if set -q $var
        set -gx PATH (string match -v -- $$var $PATH)
    end
    set -gx PATH $dir $PATH
    set -g $var $dir
end

function __shift_preexec --on-event fish_preexec
    set -g __shift_cmd_ran 1
    printf '\e]133;C\a'
end

function __shift_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
    set -e __shift_cmd_ran
end

# Wrap fish_prompt (A before, B after) right before each prompt is drawn, so a
# fish_prompt defined later (config.fish, a prompt framework) is wrapped too.
function __shift_wrap_prompt --on-event fish_prompt
    functions fish_prompt | string match -q '*__shift_orig_prompt*'; and return
    functions -e __shift_orig_prompt
    functions -c fish_prompt __shift_orig_prompt
    function fish_prompt
        printf '\e]133;A\a'
        __shift_orig_prompt
        printf '\e]133;B\a'
    end
end

__shift_report_cwd
