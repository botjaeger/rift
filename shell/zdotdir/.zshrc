# Rift ZDOTDIR shim — user config first, then Rift integration on top.
_rift_orig="${RIFT_ORIG_ZDOTDIR:-$HOME}"
# The system rc (macOS /etc/zshrc) sets HISTFILE from ZDOTDIR, which is this
# shim: the history goes to the user's file, as in any other terminal (an rc
# that sets its own HISTFILE still wins). Lines earlier versions left in the
# shim's file go back to the user's, once.
if [[ $HISTFILE == "$ZDOTDIR/.zsh_history" ]]; then
    HISTFILE="$_rift_orig/.zsh_history"
    if [[ -s "$ZDOTDIR/.zsh_history" ]] &&
        mv -f "$ZDOTDIR/.zsh_history" "$ZDOTDIR/.zsh_history.$$" 2>/dev/null; then
        cat "$ZDOTDIR/.zsh_history.$$" >> "$HISTFILE" && rm -f "$ZDOTDIR/.zsh_history.$$"
    fi
fi
[[ -f "$_rift_orig/.zshrc" ]] && ZDOTDIR="$_rift_orig" source "$_rift_orig/.zshrc"
ZDOTDIR="$_rift_orig"
[[ -f "$RIFT_INTEGRATION_ZSH" ]] && source "$RIFT_INTEGRATION_ZSH"
