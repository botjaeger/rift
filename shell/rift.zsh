# Rift shell integration for zsh — OSC 133 block markers + OSC 7 cwd.
# Sourced automatically via the ZDOTDIR shim; safe to source manually too.

# Shell-local guard (not exported): a nested or exec'd zsh must load its own
# hooks, which an exported variable would suppress.
(( ${+__rift_loaded} )) && return
typeset -g __rift_loaded=1

autoload -Uz add-zsh-hook

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
__rift_report_cwd() {
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
typeset -g __rift_py= __rift_path=
__rift_set_var() {
    printf '\e]1337;SetUserVar=%s=%s\a' "$1" "$(printf %s "$2" | base64 | tr -d '\n')"
}
__rift_report_env() {
    local py=${VIRTUAL_ENV-}
    py=${py:t}
    [[ -n $py ]] || py=${PYENV_VERSION-}
    if [[ $py != "$__rift_py" ]]; then
        __rift_py=$py
        __rift_set_var RiftPython "$py"
    fi
    if [[ $PATH != "$__rift_path" ]]; then
        __rift_path=$PATH
        __rift_set_var RiftPhpBin "${commands[php]-}"
        __rift_set_var RiftPythonBin "${commands[python3]-}"
    fi
}

# The PHP and Python chips' version pickers run this: `rift-use php|python
# <bin dir>` puts that directory first on PATH (dropping the one picked
# before), `rift-use pyenv <version>` sets PYENV_VERSION. This shell only.
typeset -g __rift_use_php= __rift_use_python=
rift-use() {
    local kind=${1-} dir=${2-} prev
    case $kind in
        pyenv) export PYENV_VERSION=$dir; return ;;
        php) prev=$__rift_use_php ;;
        python) prev=$__rift_use_python ;;
        *) print -u2 'usage: rift-use php|python <bin dir> | rift-use pyenv <version>'; return 2 ;;
    esac
    if [[ ! -d $dir ]]; then
        print -u2 "rift-use: no such directory: $dir"
        return 1
    fi
    [[ -n $prev ]] && path=(${path:#$prev})
    path=($dir $path)
    typeset -g __rift_use_$kind=$dir
}

__rift_precmd() {
    local __rift_exit=$?
    # a child zsh has returned: this shell's own ZDOTDIR again
    if (( ${+__rift_user_zdotdir} )); then
        ZDOTDIR=$__rift_user_zdotdir
        unset __rift_user_zdotdir
    fi
    if [[ -n "${__rift_cmd_ran-}" ]]; then
        printf '\e]133;D;%s\a' "$__rift_exit"
        unset __rift_cmd_ran
    fi
    __rift_report_cwd
    __rift_report_env
    printf '\e]133;A\a'
}

# Runs after every other precmd hook, so it sees the final PS1: prompt
# frameworks (p10k, starship, hand-built prompts) rebuild it each time.
__rift_precmd_end() {
    # B marker: end of prompt / start of user input.
    [[ $PS1 == *$'\e]133;B\a'* ]] || PS1+=$'%{\e]133;B\a%}'
    # keep this hook last even if something registered after us
    precmd_functions=(${precmd_functions:#__rift_precmd_end} __rift_precmd_end)
}

__rift_preexec() {
    __rift_cmd_ran=1
    printf '\e]133;C\a'
}

# Children: an interactive `zsh` or `exec zsh` typed here starts with ZDOTDIR
# on the Rift shim, which reads your real config (RIFT_ORIG_ZDOTDIR) and
# then loads this file. Only that command sees the shim; this shell and every
# other command keep your ZDOTDIR. `zsh -c`, scripts and `command zsh` are
# left alone.
typeset -g __rift_shim=${${RIFT_INTEGRATION_ZSH:-${(%):-%x}}:h}/zdotdir
typeset -g __rift_child_re='^[[:space:]]*(exec[[:space:]]+)?([^[:space:]]*/)?zsh([[:space:]]+-[a-bd-zA-Z]+)*[[:space:]]*$'
__rift_child_zdotdir() {
    [[ -f $__rift_shim/.zshrc && $1 =~ $__rift_child_re ]] || return 0
    typeset -g __rift_user_zdotdir=${ZDOTDIR:-$HOME}
    export RIFT_ORIG_ZDOTDIR=$__rift_user_zdotdir
    export ZDOTDIR=$__rift_shim
}

add-zsh-hook precmd __rift_precmd
add-zsh-hook precmd __rift_precmd_end
add-zsh-hook preexec __rift_preexec
add-zsh-hook preexec __rift_child_zdotdir
