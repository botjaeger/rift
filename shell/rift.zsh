# Rift shell integration for zsh — OSC 133 block markers + OSC 7 cwd.
# Sourced automatically via the ZDOTDIR shim; safe to source manually too.

# Shell-local guard (not exported): a nested or exec'd zsh must load its own
# hooks, which an exported variable would suppress.
(( ${+__rift_loaded} )) && return
typeset -g __rift_loaded=1

autoload -Uz add-zsh-hook

# Percent-encode $1 into REPLY: `%`, `;`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
__rift_pct() {
    local LC_ALL=C s=$1 out= c
    local -i i
    if [[ $s == *[^A-Za-z0-9/._~-]* ]]; then
        for (( i = 1; i <= ${#s}; i++ )); do
            c=${s[i]}
            case $c in
                [A-Za-z0-9/._~-]) out+=$c ;;
                *) printf -v c '%%%02X' $(( (#c) & 255 )); out+=$c ;;
            esac
        done
        s=$out
    fi
    REPLY=$s
}

# OSC 7 wants a percent-encoded path.
__rift_report_cwd() {
    local REPLY
    __rift_pct "$PWD"
    printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$REPLY"
}

# Prompt chips: the active Python environment, and the php and
# python3 that PATH resolves to, as iTerm2 user variables (OSC 1337
# SetUserVar, base64), sent when they change; PATH itself too (command-name
# completion follows `rift-use`).
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
        __rift_set_var RiftPath "$PATH"
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

# Rift runs `rift-complete` over an SSH profile's shared connection: it
# answers completion requests on stdin with this shell's completion
# functions (complete.zsh next to this file; its header has the protocol).
typeset -g __rift_complete_srv=${${RIFT_INTEGRATION_ZSH:-${(%):-%x}}:h}/complete.zsh
rift-complete() {
    [[ -r $__rift_complete_srv ]] || return 1
    # the inner shell gets this one's fpath instead of reading the rc files again
    TERM=${TERM:-xterm} RIFT_COMPLETE_FPATH=${(j.:.)fpath} \
        command zsh -f -c "$(<$__rift_complete_srv)" rift-complete zsh -f
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
    __rift_report_widgets
    # keep this hook last even if something registered after us
    precmd_functions=(${precmd_functions:#__rift_precmd_end} __rift_precmd_end)
}

# C marker with the command line as read (kitty's cmdline_url): the block
# header keeps it exact even where the screen's echo is not (a reattached
# shell replayed at another width). `%' first, then what would end or split
# the sequence; the rest goes as is.
__rift_preexec() {
    __rift_cmd_ran=1
    local c=${1//\%/%25}
    c=${c//$'\a'/%07}
    c=${c//$'\e'/%1B}
    c=${c//;/%3B}
    c=${c//$'\n'/%0A}
    c=${c//$'\r'/%0D}
    c=${c//$'\030'/%18}
    c=${c//$'\032'/%1A}
    printf '\e]133;C;cmdline_url=%s\a' "$c"
}

# Shell widgets: ⌃R, ⌃T and ⌥C run your own widgets (fzf, atuin) on Rift's
# command line. Each prompt reports which of them your config binds to a
# widget of its own (OSC 6973;K;<letters r t c>). For such a key Rift types
# `%Z<before>%Z<after>` (its command line split at the cursor, every byte
# but letters and digits as %XX), then ^X^Y and the key's letter: the
# wrapper below acknowledges (OSC 6973;L), loads that line, runs your widget, and reports the line it
# left (OSC 6973;W;<before>;<after>, percent-encoded) for Rift to edit, then
# empties its own. A widget that runs a command itself (fzf's ⌥C cd) gets
# no report: the command runs, the next prompt follows, and Rift's command
# line keeps what it held (as push-line would bring it back).
zmodload -i zsh/zleparameter 2>/dev/null
typeset -gA __rift_wkeys=(r '^R' t '^T' c '^[c')

# The user widget `key` runs (none when it is a builtin one) into REPLY.
__rift_widget_of() {
    REPLY=${${(z)"$(bindkey -M main -- "$1" 2>/dev/null)"}[2]-}
    [[ ${widgets[$REPLY]-} == user:* ]] || REPLY=
}

__rift_report_widgets() {
    local k out= REPLY
    for k in r t c; do
        __rift_widget_of "${__rift_wkeys[$k]}"
        [[ -n $REPLY ]] && out+=$k
    done
    printf '\e]6973;K;%s\a' "$out"
}

__rift_widget_accept() {
    typeset -g __rift_accepted=1
    zle __rift_orig_accept-line -- "$@"
}

# Rift's command line keeps the line: a push-line would bring it back into
# the shell's own at the next prompt, under what Rift types there.
__rift_widget_push() {
    BUFFER=
}

__rift_widget() {
    local p=$BUFFER left right REPLY w n
    [[ $p == *%Z*%Z* ]] || return 0
    printf '\e]6973;L\a'
    right=${p##*%Z}
    left=${p%\%Z*}
    left=${left##*%Z}
    # only letters, digits and %XX arrive: nothing else for %b to expand
    printf -v left '%b' "${left//\%/\\x}"
    printf -v right '%b' "${right//\%/\\x}"
    BUFFER=$left$right
    CURSOR=${#left}
    __rift_widget_of "${__rift_wkeys[${KEYS[-1]}]-}"
    w=$REPLY
    typeset -g __rift_accepted=
    if [[ -n $w ]]; then
        # notice a widget that accepts the line (it runs a command itself)
        zle -A accept-line __rift_orig_accept-line
        zle -N accept-line __rift_widget_accept
        for n in push-line push-input push-line-or-edit; do
            zle -A $n __rift_orig_$n
            zle -N $n __rift_widget_push
        done
        zle "$w"
        for n in accept-line push-line push-input push-line-or-edit; do
            zle -A __rift_orig_$n $n
        done
    fi
    [[ -n $__rift_accepted ]] && return 0
    __rift_pct "$LBUFFER"
    left=$REPLY
    __rift_pct "$RBUFFER"
    printf '\e]6973;W;%s;%s\a' "$left" "$REPLY"
    BUFFER=
}
zle -N __rift_widget
() {
    local m k
    for m in emacs viins; do
        for k in r t c; do
            bindkey -M $m "^X^Y$k" __rift_widget
        done
    done
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
