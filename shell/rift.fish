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

# Shell widgets: ⌃R, ⌃T and ⌥C run your own bindings (fzf, fzf.fish, atuin)
# on Rift's command line. Each prompt reports which of them your config binds
# in the current mode (OSC 6973;K;<letters r t c>) and binds ^X^Y and the
# key's letter to: load the line (acknowledged as OSC 6973;L), your binding's
# commands, report. For such a key Rift types `%Z<before>%Z<after>` (its
# command line split at the cursor, every byte but letters and digits as
# %XX), then that sequence; the report is the line your binding left (OSC
# 6973;W;<before>;<after>, percent-encoded), and the shell's own line is
# emptied. A binding that runs the line itself (`commandline -f execute`,
# atuin's enter_accept) gets no report: fish runs that queued execute only
# after the whole binding, so the report would empty the line first. The
# command runs, the next prompt follows, and Rift's command line keeps what
# it held (as in zsh).
function __rift_widget_load
    # notice a queued execute: `commandline` is wrapped while the binding
    # runs (unless something else already defines a function of that name)
    set -g __rift_widget_exec
    set -g __rift_widget_wrap
    if not functions -q commandline
        set -g __rift_widget_wrap 1
        function commandline
            if contains -- execute $argv
                and begin
                    contains -- -f $argv; or contains -- --function $argv
                end
                set -g __rift_widget_exec 1
            end
            builtin commandline $argv
        end
    end
    set -l p (commandline | string collect)
    string match -q -- '*%Z*%Z*' "$p"; or return 0
    printf '\e]6973;L\a'
    set -l parts (string split -- %Z "$p")
    # printf decodes exactly (string's own output ends in a newline); only
    # letters, digits and %XX arrive, so \xXX is all it expands
    set -l left (printf '%b' (string replace -a -- % '\x' $parts[-2]) | string split0)
    set -l right (printf '%b' (string replace -a -- % '\x' $parts[-1]) | string split0)
    commandline -r -- "$left$right"
    commandline -C (string length -- "$left")
end

function __rift_widget_post
    if test -n "$__rift_widget_wrap"
        functions -e commandline
        set -g __rift_widget_wrap
    end
    # the binding queued an execute: leave the line for it (a blank line
    # runs nothing, so it still reports)
    if test -n "$__rift_widget_exec"
        set -g __rift_widget_exec
        commandline | string trim | string length -q; and return 0
    end
    # `commandline` and `string sub` each end their output in a newline:
    # those are dropped once encoded, so the line's own newlines survive
    set -l at (commandline -C)
    set -l line (commandline | string split0)
    set -l l (string sub -l $at -- "$line" | string split0)
    set -l r (string sub -s (math $at + 1) -- "$line" | string split0)
    set l (string escape --style=url -- "$l" | string replace -r -- '%0A$' '')
    set r (string escape --style=url -- "$r" | string replace -r -- '%0A%0A$' '')
    printf '\e]6973;W;%s;%s\a' "$l" "$r"
    commandline -r ''
end

function __rift_widget_bind
    set -g __rift_bound $argv
end

function __rift_report_widgets --on-event fish_prompt
    set -l out
    set -l key
    for k in r t c
        switch $k
            case r
                set key \cr
            case t
                set key \ct
            case c
                set key \ec
        end
        for line in (bind --user $key 2>/dev/null)
            # the listing is fish code: run it with a stand-in for `bind`
            eval (string replace -r '^bind ' '__rift_widget_bind ' -- $line)
            set -l mode default
            set -l cmds
            set -l skip
            set -l seen_key
            for a in $__rift_bound
                if test -n "$skip"
                    test "$skip" = mode; and set mode $a
                    set skip
                else if test -n "$seen_key"
                    set -a cmds $a
                else if contains -- $a -M --mode
                    set skip mode
                else if contains -- $a -m --sets-mode
                    set skip other
                else if string match -qr -- '^--mode=' $a
                    set mode (string replace -- --mode= '' $a)
                else if not string match -q -- '-*' $a
                    set seen_key 1
                end
            end
            test (count $cmds) -gt 0; or continue
            bind -M $mode \cx\cy$k __rift_widget_load $cmds __rift_widget_post
            test "$mode" = "$fish_bind_mode"; and set -a out $k
        end
    end
    printf '\e]6973;K;%s\a' (string join '' $out)
end

# C marker with the command line as read (kitty's cmdline_url,
# percent-encoded): the block header keeps it exact even where the screen's
# echo is not (a reattached shell replayed at another width).
function __rift_preexec --on-event fish_preexec
    set -g __rift_cmd_ran 1
    printf '\e]133;C;cmdline_url=%s\a' (string escape --style=url -- $argv[1])
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
