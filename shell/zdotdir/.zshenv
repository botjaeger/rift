# Shift ZDOTDIR shim — forwards to the user's real zsh config.
_shift_shim="$ZDOTDIR"
_shift_orig="${SHIFT_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_shift_orig/.zshenv" ]] && source "$_shift_orig/.zshenv"
# A user .zshenv that sets ZDOTDIR (XDG setups) would make zsh skip this shim
# for .zprofile/.zshrc: remember the user's directory, point back at the shim.
if [[ "$ZDOTDIR" != "$_shift_shim" ]]; then
    SHIFT_ORIG_ZDOTDIR="${ZDOTDIR:-$HOME}"
    ZDOTDIR="$_shift_shim"
fi
unset _shift_shim
