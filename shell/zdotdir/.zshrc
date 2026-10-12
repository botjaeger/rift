# Rift ZDOTDIR shim — user config first, then Rift integration on top.
_rift_orig="${RIFT_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_rift_orig/.zshrc" ]] && ZDOTDIR="$_rift_orig" source "$_rift_orig/.zshrc"
ZDOTDIR="$_rift_orig"
[[ -f "$RIFT_INTEGRATION_ZSH" ]] && source "$RIFT_INTEGRATION_ZSH"
