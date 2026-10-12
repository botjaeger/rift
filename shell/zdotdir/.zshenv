# Rift ZDOTDIR shim — forwards to the user's real zsh config.
_rift_shim="$ZDOTDIR"
_rift_orig="${RIFT_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_rift_orig/.zshenv" ]] && source "$_rift_orig/.zshenv"
# A user .zshenv that sets ZDOTDIR (XDG setups) would make zsh skip this shim
# for .zprofile/.zshrc: remember the user's directory, point back at the shim.
if [[ "$ZDOTDIR" != "$_rift_shim" ]]; then
    RIFT_ORIG_ZDOTDIR="${ZDOTDIR:-$HOME}"
    ZDOTDIR="$_rift_shim"
fi
unset _rift_shim
