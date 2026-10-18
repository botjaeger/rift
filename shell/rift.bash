# shellcheck shell=bash disable=SC2128,SC2178,SC2179  # PROMPT_COMMAND is a string or an array (bash 5.1+); both handled
# Rift shell integration for bash — OSC 133 block markers + OSC 7 cwd.
# bash has no ZDOTDIR equivalent: Rift spawns bash with
# `--rcfile ~/.local/share/rift/bashrc`, a shim that replays your own
# startup files and then sources this one. Sourcing it manually from
# ~/.bashrc is harmless (guarded below).

# Shell-local guard (not exported): a nested or exec'd bash must load its own
# hooks, which an exported variable would suppress. A second source (the shim
# runs this file after your rc, which may have sourced it and then appended
# to PROMPT_COMMAND) only puts our last entry back last before the first
# prompt.
if [[ -n "${__rift_loaded-}" ]]; then
    __rift_keep_last
    return
fi
__rift_loaded=1

__rift_b=$'\e]133;B\a'

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
__rift_report_cwd() {
    local LC_ALL=C dir=$PWD out='' c i
    if [[ $dir =~ [^A-Za-z0-9/._~-] ]]; then
        for ((i = 0; i < ${#dir}; i++)); do
            c=${dir:i:1}
            case $c in
                [A-Za-z0-9/._~-]) out+=$c ;;
                *)
                    printf -v c '%%%02X' $(($(printf '%d' "'$c") & 255))
                    out+=$c
                    ;;
            esac
        done
        dir=$out
    fi
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$dir"
}

# Prompt chips: the active Python environment, and the php and
# python3 that PATH resolves to, as iTerm2 user variables (OSC 1337
# SetUserVar, base64), sent when they change; PATH itself too (command-name
# completion follows `rift-use`).
__rift_py=
__rift_path=
__rift_set_var() {
    printf '\e]1337;SetUserVar=%s=%s\a' "$1" "$(printf %s "$2" | base64 | tr -d '\n')"
}
__rift_report_env() {
    local py=${VIRTUAL_ENV-}
    py=${py##*/}
    [[ -n $py ]] || py=${PYENV_VERSION-}
    if [[ $py != "$__rift_py" ]]; then
        __rift_py=$py
        __rift_set_var RiftPython "$py"
    fi
    if [[ $PATH != "$__rift_path" ]]; then
        __rift_path=$PATH
        __rift_set_var RiftPath "$PATH"
        __rift_set_var RiftPhpBin "$(type -P php)"
        __rift_set_var RiftPythonBin "$(type -P python3)"
    fi
}

# The PHP and Python chips' version pickers run this: `rift-use php|python
# <bin dir>` puts that directory first on PATH (dropping the one picked
# before), `rift-use pyenv <version>` sets PYENV_VERSION. This shell only.
__rift_use_php=
__rift_use_python=
rift-use() {
    local kind=${1-} dir=${2-} prev
    case $kind in
        pyenv) export PYENV_VERSION=$dir; return ;;
        php) prev=$__rift_use_php ;;
        python) prev=$__rift_use_python ;;
        *) echo 'usage: rift-use php|python <bin dir> | rift-use pyenv <version>' >&2; return 2 ;;
    esac
    if [[ ! -d $dir ]]; then
        echo "rift-use: no such directory: $dir" >&2
        return 1
    fi
    if [[ -n $prev ]]; then
        local p=":$PATH:"
        p=${p//":$prev:"/:}
        p=${p#:}
        PATH=${p%:}
    fi
    export PATH="$dir:$PATH"
    hash -r
    printf -v "__rift_use_$kind" '%s' "$dir"
}

# Fallback for bash < 4.4 (no PS0): the DEBUG trap fires for every simple
# command, including the ones inside PROMPT_COMMAND, so it only counts once
# the last PROMPT_COMMAND entry (__rift_prompt_end) has armed it.
__rift_preexec() {
    # an empty Enter armed the trap too: the next prompt cycle starts with
    # our own entry, which is not a user command
    if [[ -n "${__rift_at_prompt-}" && "$BASH_COMMAND" != __rift_prompt_command ]]; then
        # PROMPT_COMMAND entries appended after ours still run after
        # __rift_prompt_end in this cycle (the order is restored from the
        # next one): not user commands
        [[ -n "${__rift_late-}" && "$__rift_late" == *"$BASH_COMMAND"* ]] && return
        __rift_at_prompt=
        __rift_cmd_ran=1
        # one fork: the newest history entry, then plain expansions
        __rift_cmdline_of "$(HISTTIMEFORMAT='' builtin history 1 2>/dev/null)" "${__rift_hist_n-}"
        printf '\e]133;C%s\a' "$__rift_cl"
    fi
}

# From `history 1`'s line ($1), sets __rift_hist_no (the entry's number,
# empty with no history) and __rift_cl: `;cmdline_url=<the command line>`
# for the C marker (kitty's form; the block header keeps it exact even where
# the screen's echo is not, e.g. a reattached shell replayed at another
# width), or nothing. The line is history's copy, so only when this command
# entered the history: the entry is not number $2 (HISTCONTROL, `set +o
# history` or a repeated line leave it out, and the newest entry is then an
# earlier command's). Percent-encoded as in rift.zsh. No subshell: it sets
# variables of the shell that calls it.
__rift_cmdline_of() {
    local c=$1
    __rift_cl=
    c=${c#"${c%%[![:space:]]*}"}
    __rift_hist_no=${c%%[!0-9]*}
    [[ -n $__rift_hist_no && $__rift_hist_no != "${2-}" ]] || return 0
    c=${c#"$__rift_hist_no"}
    c=${c#\*}
    c=${c#  }
    c=${c//\%/%25}
    c=${c//$'\a'/%07}
    c=${c//$'\e'/%1B}
    c=${c//;/%3B}
    c=${c//$'\n'/%0A}
    c=${c//$'\r'/%0D}
    c=${c//$'\030'/%18}
    c=${c//$'\032'/%1A}
    __rift_cl=";cmdline_url=$c"
}

# PS0's half (it runs in a subshell, once the command entered the history).
__rift_cmdline() {
    __rift_cmdline_of "$(HISTTIMEFORMAT='' builtin history 1 2>/dev/null)"
    printf '%s' "$__rift_cl"
}

__rift_prompt_command() {
    local exit=$?
    if [[ -n "${__rift_cmd_ran-}" ]]; then
        printf '\e]133;D;%s\a' "$exit"
        unset __rift_cmd_ran
    fi
    __rift_at_prompt=
    __rift_report_cwd
    __rift_report_env
    printf '\e]133;A\a'
    __rift_late_entries
    __rift_keep_last
}

# The text of the PROMPT_COMMAND entries after __rift_prompt_end: this
# cycle runs them after it, whatever __rift_keep_last changes now.
__rift_late_entries() {
    __rift_late=
    if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
        local e seen=
        for e in "${PROMPT_COMMAND[@]}"; do
            [[ -n $seen ]] && __rift_late+=$e$'\n'
            [[ $e == __rift_prompt_end ]] && seen=1
        done
    elif [[ ${PROMPT_COMMAND-} == *__rift_prompt_end* ]]; then
        __rift_late=${PROMPT_COMMAND##*__rift_prompt_end}
    fi
}

# Runs after every other PROMPT_COMMAND entry, so it sees the final PS1.
__rift_prompt_end() {
    # B marker: end of prompt / start of user input. Re-added whenever a
    # prompt framework (or a PROMPT_COMMAND entry) rebuilt PS1 without it.
    [[ $PS1 == *"$__rift_b"* ]] || PS1+="\\[$__rift_b\\]"
    if [[ -n "${__rift_use_ps0-}" ]]; then
        [[ ${PS0-} == *"$__rift_c"* ]] || PS0+=$__rift_c
        # PS0 runs __rift_cmdline in a subshell: only when HISTCMD moved
        # past this, the command entered the history
        __rift_hist_at=$HISTCMD
    else
        __rift_at_prompt=1
        # the newest entry as the prompt shows (PROMPT_COMMAND's `history
        # -a; history -n` may have added other sessions' lines): a command
        # that adds none is not given one of them
        __rift_cmdline_of "$(HISTTIMEFORMAT='' builtin history 1 2>/dev/null)"
        __rift_hist_n=$__rift_hist_no
    fi
}

# Keep __rift_prompt_end the last PROMPT_COMMAND entry even when the user's
# own config appended to it after this file was sourced.
__rift_keep_last() {
    if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
        if [[ "${PROMPT_COMMAND[${#PROMPT_COMMAND[@]} - 1]}" != __rift_prompt_end ]]; then
            local -a keep=()
            local e
            for e in "${PROMPT_COMMAND[@]}"; do
                [[ $e == __rift_prompt_end ]] || keep+=("$e")
            done
            PROMPT_COMMAND=("${keep[@]}" __rift_prompt_end)
        fi
    elif [[ "${PROMPT_COMMAND-}" != *$'\n'__rift_prompt_end ]]; then
        PROMPT_COMMAND=${PROMPT_COMMAND-}
        PROMPT_COMMAND=${PROMPT_COMMAND//$'\n'__rift_prompt_end/}
        PROMPT_COMMAND+=$'\n'__rift_prompt_end
    fi
}

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(__rift_prompt_command "${PROMPT_COMMAND[@]}" __rift_prompt_end)
else
    PROMPT_COMMAND="__rift_prompt_command${PROMPT_COMMAND:+$'\n'$PROMPT_COMMAND}"$'\n'__rift_prompt_end
fi

# Children: bash has no ZDOTDIR, only `--rcfile` reaches a child, so `bash`
# and `exec bash` typed at this prompt get it through an alias (the trailing
# space of `exec ` lets the alias expand after it). Non-interactive uses
# (`bash -c`, scripts) ignore --rcfile. Your own aliases of these names win.
__rift_child_rc=${RIFT_INTEGRATION_BASH-}

# Rift runs `rift-complete` over an SSH profile's shared connection: it
# answers completion requests on stdin with this shell's completion specs
# (complete.bash next to this file; its header has the protocol).
__rift_complete_srv="${BASH_SOURCE[0]%/*}/complete.bash"
rift-complete() {
    [[ -r $__rift_complete_srv ]] || return 1
    # shellcheck source=complete.bash disable=SC1091
    source "$__rift_complete_srv"
}

if [[ -n $__rift_child_rc && -f "${__rift_child_rc%/*}/bashrc-child" ]]; then
    printf -v __rift_child_rc '%q' "${__rift_child_rc%/*}/bashrc-child"
    # shellcheck disable=SC2139  # the path is meant to expand now
    alias bash >/dev/null 2>&1 || alias bash="bash --rcfile $__rift_child_rc"
    alias exec >/dev/null 2>&1 || alias exec='exec '
fi
unset __rift_child_rc

# C marker. bash >= 4.4 expands PS0 right after a command is read and before
# it runs (blank lines do not expand it), so it fires exactly once per
# command whatever PROMPT_COMMAND holds. The array subscript assignment
# records that a command ran (a command substitution would run in a subshell).
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))) && shopt -q promptvars; then
    __rift_use_ps0=1
    __rift_c=$'\e]133;C$( ((HISTCMD > __rift_hist_at)) && __rift_cmdline)\a${__rift_nil[__rift_cmd_ran=1]-}'
else
    __rift_cmdline_of "$(HISTTIMEFORMAT='' builtin history 1 2>/dev/null)"
    __rift_hist_n=$__rift_hist_no
    trap '__rift_preexec' DEBUG
fi
