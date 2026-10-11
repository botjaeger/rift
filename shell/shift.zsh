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

# Prompt chips: the active Python environment, and the php and
# python3 that PATH resolves to, as iTerm2 user variables (OSC 1337
# SetUserVar, base64), sent when they change.
typeset -g __shift_py= __shift_path=
__shift_set_var() {
    printf '\e]1337;SetUserVar=%s=%s\a' "$1" "$(printf %s "$2" | base64 | tr -d '\n')"
}
__shift_report_env() {
    local py=${VIRTUAL_ENV:t}
    [[ -n $py ]] || py=${PYENV_VERSION-}
    if [[ $py != "$__shift_py" ]]; then
        __shift_py=$py
        __shift_set_var ShiftPython "$py"
    fi
    if [[ $PATH != "$__shift_path" ]]; then
        __shift_path=$PATH
        __shift_set_var ShiftPhpBin "${commands[php]-}"
        __shift_set_var ShiftPythonBin "${commands[python3]-}"
    fi
}

# The PHP and Python chips' version pickers run this: `shift-use php|python
# <bin dir>` puts that directory first on PATH (dropping the one picked
# before), `shift-use pyenv <version>` sets PYENV_VERSION. This shell only.
typeset -g __shift_use_php= __shift_use_python=
shift-use() {
    local kind=$1 dir=$2 prev
    case $kind in
        pyenv) export PYENV_VERSION=$dir; return ;;
        php) prev=$__shift_use_php ;;
        python) prev=$__shift_use_python ;;
        *) print -u2 'usage: shift-use php|python <bin dir> | shift-use pyenv <version>'; return 2 ;;
    esac
    if [[ ! -d $dir ]]; then
        print -u2 "shift-use: no such directory: $dir"
        return 1
    fi
    [[ -n $prev ]] && path=(${path:#$prev})
    path=($dir $path)
    typeset -g __shift_use_$kind=$dir
}

__shift_precmd() {
    local __shift_exit=$?
    # a child zsh has returned: this shell's own ZDOTDIR again
    if (( ${+__shift_user_zdotdir} )); then
        ZDOTDIR=$__shift_user_zdotdir
        unset __shift_user_zdotdir
    fi
    if [[ -n "$__shift_cmd_ran" ]]; then
        printf '\e]133;D;%s\a' "$__shift_exit"
        unset __shift_cmd_ran
    fi
    __shift_report_cwd
    __shift_report_env
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

# Children: an interactive `zsh` or `exec zsh` typed here starts with ZDOTDIR
# on the Shift shim, which reads your real config (SHIFT_ORIG_ZDOTDIR) and
# then loads this file. Only that command sees the shim; this shell and every
# other command keep your ZDOTDIR. `zsh -c`, scripts and `command zsh` are
# left alone.
typeset -g __shift_shim=${SHIFT_INTEGRATION_ZSH:h}/zdotdir
typeset -g __shift_child_re='^[[:space:]]*(exec[[:space:]]+)?([^[:space:]]*/)?zsh([[:space:]]+-[a-bd-zA-Z]+)*[[:space:]]*$'
__shift_child_zdotdir() {
    [[ -f $__shift_shim/.zshrc && $1 =~ $__shift_child_re ]] || return 0
    typeset -g __shift_user_zdotdir=${ZDOTDIR:-$HOME}
    export SHIFT_ORIG_ZDOTDIR=$__shift_user_zdotdir
    export ZDOTDIR=$__shift_shim
}

add-zsh-hook precmd __shift_precmd
add-zsh-hook precmd __shift_precmd_end
add-zsh-hook preexec __shift_preexec
add-zsh-hook preexec __shift_child_zdotdir
