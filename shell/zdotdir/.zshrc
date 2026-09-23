# Shift ZDOTDIR shim — user config first, then Shift integration on top.
_shift_orig="${SHIFT_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_shift_orig/.zshrc" ]] && ZDOTDIR="$_shift_orig" source "$_shift_orig/.zshrc"
ZDOTDIR="$_shift_orig"
[[ -f "$SHIFT_INTEGRATION_ZSH" ]] && source "$SHIFT_INTEGRATION_ZSH"
