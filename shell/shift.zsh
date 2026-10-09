# Shift shell integration for zsh — OSC 133 block markers + OSC 7 cwd.
# Sourced automatically via the ZDOTDIR shim; safe to source manually too.

# Shell-local guard (not exported): a nested or exec'd zsh must load its own
# hooks, which an exported variable would suppress.
(( ${+__shift_loaded} )) && return
typeset -g __shift_loaded=1

autoload -Uz add-zsh-hook

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
__shift_report_cwd() {
    local LC_ALL=C dir=$PWD out= c
    local -i i
    if [[ $dir == *[^A-Za-z0-9/._~-]* ]]; then
        for (( i = 1; i <= ${#dir}; i++ )); do
            c=${dir[i]}
            case $c in
                [A-Za-z0-9/._~-]) out+=$c ;;
                *) printf -v c '%%%02X' $(( $(printf '%d' "'$c") & 255 )); out+=$c ;;
            esac
        done
        dir=$out
    fi
    printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$dir"
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

# Runs after every other precmd hook, so it sees the final PS1: prompt
# frameworks (p10k, starship, hand-built prompts) rebuild it each time.
__shift_precmd_end() {
    # B marker: end of prompt / start of user input.
    [[ $PS1 == *$'\e]133;B\a'* ]] || PS1+=$'%{\e]133;B\a%}'
    # keep this hook last even if something registered after us
    precmd_functions=(${precmd_functions:#__shift_precmd_end} __shift_precmd_end)
}

__shift_preexec() {
    __shift_cmd_ran=1
    printf '\e]133;C\a'
}

add-zsh-hook precmd __shift_precmd
add-zsh-hook precmd __shift_precmd_end
add-zsh-hook preexec __shift_preexec
