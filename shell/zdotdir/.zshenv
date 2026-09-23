# Shift ZDOTDIR shim — forwards to the user's real zsh config.
_shift_orig="${SHIFT_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_shift_orig/.zshenv" ]] && source "$_shift_orig/.zshenv"
