# Shift shell integration for bash — OSC 133 block markers + OSC 7 cwd.
# bash has no ZDOTDIR equivalent: Shift spawns bash with
# `--rcfile ~/.local/share/shift/bashrc`, a shim that replays your own
# startup files and then sources this one. Sourcing it manually from
# ~/.bashrc is harmless (guarded below).

[[ -n "$SHIFT_SHELL_INTEGRATION" ]] && return
export SHIFT_SHELL_INTEGRATION=1

__shift_report_cwd() {
    printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$PWD"
}

__shift_preexec() {
    # DEBUG trap fires for every simple command; only mark the first one
    # after a prompt.
    if [[ -n "$__shift_at_prompt" ]]; then
        unset __shift_at_prompt
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
    __shift_report_cwd
    printf '\e]133;A\a'
    __shift_at_prompt=1
}

PROMPT_COMMAND="__shift_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
trap '__shift_preexec' DEBUG

# B marker: end of prompt / start of user input.
PS1="${PS1}\[$(printf '\e]133;B\a')\]"
