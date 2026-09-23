# Shift shell integration for zsh — OSC 133 block markers + OSC 7 cwd.
# Sourced automatically via the ZDOTDIR shim; safe to source manually too.

[[ -n "$SHIFT_SHELL_INTEGRATION" ]] && return
export SHIFT_SHELL_INTEGRATION=1

autoload -Uz add-zsh-hook

__shift_report_cwd() {
    printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
}

__shift_precmd() {
    local __shift_exit=$?
    if [[ -n "$__shift_cmd_ran" ]]; then
        printf '\e]133;D;%s\a' "$__shift_exit"
        unset __shift_cmd_ran
    fi
    __shift_report_cwd
    printf '\e]133;A\a'
}

__shift_preexec() {
    __shift_cmd_ran=1
    printf '\e]133;C\a'
}

add-zsh-hook precmd __shift_precmd
add-zsh-hook preexec __shift_preexec

# B marker: end of prompt / start of user input.
PS1="${PS1}%{$(printf '\e]133;B\a')%}"
