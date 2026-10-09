# shellcheck shell=bash disable=SC2128,SC2178,SC2179  # PROMPT_COMMAND is a string or an array (bash 5.1+); both handled
# Shift shell integration for bash — OSC 133 block markers + OSC 7 cwd.
# bash has no ZDOTDIR equivalent: Shift spawns bash with
# `--rcfile ~/.local/share/shift/bashrc`, a shim that replays your own
# startup files and then sources this one. Sourcing it manually from
# ~/.bashrc is harmless (guarded below).

# Shell-local guard (not exported): a nested or exec'd bash must load its own
# hooks, which an exported variable would suppress. A second source (the shim
# runs this file after your rc, which may have sourced it and then appended
# to PROMPT_COMMAND) only puts our last entry back last before the first
# prompt.
if [[ -n "${__shift_loaded-}" ]]; then
    __shift_keep_last
    return
fi
__shift_loaded=1

__shift_b=$'\e]133;B\a'

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
__shift_report_cwd() {
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

# Fallback for bash < 4.4 (no PS0): the DEBUG trap fires for every simple
# command, including the ones inside PROMPT_COMMAND, so it only counts once
# the last PROMPT_COMMAND entry (__shift_prompt_end) has armed it.
__shift_preexec() {
    # an empty Enter armed the trap too: the next prompt cycle starts with
    # our own entry, which is not a user command
    if [[ -n "$__shift_at_prompt" && "$BASH_COMMAND" != __shift_prompt_command ]]; then
        # PROMPT_COMMAND entries appended after ours still run after
        # __shift_prompt_end in this cycle (the order is restored from the
        # next one): not user commands
        [[ -n "$__shift_late" && "$__shift_late" == *"$BASH_COMMAND"* ]] && return
        __shift_at_prompt=
        __shift_cmd_ran=1
        printf '\e]133;C\a'
    fi
}

__shift_prompt_command() {
    local exit=$?
    if [[ -n "$__shift_cmd_ran" ]]; then
        printf '\e]133;D;%s\a' "$exit"
        unset __shift_cmd_ran
    fi
    __shift_at_prompt=
    __shift_report_cwd
    printf '\e]133;A\a'
    __shift_late_entries
    __shift_keep_last
}

# The text of the PROMPT_COMMAND entries after __shift_prompt_end: this
# cycle runs them after it, whatever __shift_keep_last changes now.
__shift_late_entries() {
    __shift_late=
    if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
        local e seen=
        for e in "${PROMPT_COMMAND[@]}"; do
            [[ -n $seen ]] && __shift_late+=$e$'\n'
            [[ $e == __shift_prompt_end ]] && seen=1
        done
    elif [[ $PROMPT_COMMAND == *__shift_prompt_end* ]]; then
        __shift_late=${PROMPT_COMMAND##*__shift_prompt_end}
    fi
}

# Runs after every other PROMPT_COMMAND entry, so it sees the final PS1.
__shift_prompt_end() {
    # B marker: end of prompt / start of user input. Re-added whenever a
    # prompt framework (or a PROMPT_COMMAND entry) rebuilt PS1 without it.
    [[ $PS1 == *"$__shift_b"* ]] || PS1+="\\[$__shift_b\\]"
    if [[ -n "$__shift_use_ps0" ]]; then
        [[ $PS0 == *"$__shift_c"* ]] || PS0+=$__shift_c
    else
        __shift_at_prompt=1
    fi
}

# Keep __shift_prompt_end the last PROMPT_COMMAND entry even when the user's
# own config appended to it after this file was sourced.
__shift_keep_last() {
    if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
        if [[ "${PROMPT_COMMAND[${#PROMPT_COMMAND[@]} - 1]}" != __shift_prompt_end ]]; then
            local -a keep=()
            local e
            for e in "${PROMPT_COMMAND[@]}"; do
                [[ $e == __shift_prompt_end ]] || keep+=("$e")
            done
            PROMPT_COMMAND=("${keep[@]}" __shift_prompt_end)
        fi
    elif [[ "$PROMPT_COMMAND" != *$'\n'__shift_prompt_end ]]; then
        PROMPT_COMMAND=${PROMPT_COMMAND//$'\n'__shift_prompt_end/}
        PROMPT_COMMAND+=$'\n'__shift_prompt_end
    fi
}

if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(__shift_prompt_command "${PROMPT_COMMAND[@]}" __shift_prompt_end)
else
    PROMPT_COMMAND="__shift_prompt_command${PROMPT_COMMAND:+$'\n'$PROMPT_COMMAND}"$'\n'__shift_prompt_end
fi

# Children: bash has no ZDOTDIR, only `--rcfile` reaches a child, so `bash`
# and `exec bash` typed at this prompt get it through an alias (the trailing
# space of `exec ` lets the alias expand after it). Non-interactive uses
# (`bash -c`, scripts) ignore --rcfile. Your own aliases of these names win.
if [[ -f "${SHIFT_INTEGRATION_BASH%/*}/bashrc-child" ]]; then
    printf -v __shift_child_rc '%q' "${SHIFT_INTEGRATION_BASH%/*}/bashrc-child"
    # shellcheck disable=SC2139  # the path is meant to expand now
    alias bash >/dev/null 2>&1 || alias bash="bash --rcfile $__shift_child_rc"
    alias exec >/dev/null 2>&1 || alias exec='exec '
    unset __shift_child_rc
fi

# C marker. bash >= 4.4 expands PS0 right after a command is read and before
# it runs (blank lines do not expand it), so it fires exactly once per
# command whatever PROMPT_COMMAND holds. The array subscript assignment
# records that a command ran (a command substitution would run in a subshell).
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))) && shopt -q promptvars; then
    __shift_use_ps0=1
    __shift_c=$'\e]133;C\a${__shift_nil[__shift_cmd_ran=1]-}'
else
    trap '__shift_preexec' DEBUG
fi
