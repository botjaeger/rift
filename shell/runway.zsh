# Runway shell integration for zsh — OSC 133 block markers + OSC 7 cwd.
# Sourced automatically via the ZDOTDIR shim; safe to source manually too.

[[ -n "$RUNWAY_SHELL_INTEGRATION" ]] && return
export RUNWAY_SHELL_INTEGRATION=1

autoload -Uz add-zsh-hook

__runway_report_cwd() {
    printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
}

__runway_precmd() {
    local __runway_exit=$?
    if [[ -n "$__runway_cmd_ran" ]]; then
        printf '\e]133;D;%s\a' "$__runway_exit"
        unset __runway_cmd_ran
    fi
    __runway_report_cwd
    printf '\e]133;A\a'
}

__runway_preexec() {
    __runway_cmd_ran=1
    printf '\e]133;C\a'
}

add-zsh-hook precmd __runway_precmd
add-zsh-hook preexec __runway_preexec

# B marker: end of prompt / start of user input.
PS1="${PS1}%{$(printf '\e]133;B\a')%}"
