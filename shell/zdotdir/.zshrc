# Runway ZDOTDIR shim — user config first, then Runway integration on top.
_runway_orig="${RUNWAY_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_runway_orig/.zshrc" ]] && ZDOTDIR="$_runway_orig" source "$_runway_orig/.zshrc"
ZDOTDIR="$_runway_orig"
[[ -f "$RUNWAY_INTEGRATION_ZSH" ]] && source "$RUNWAY_INTEGRATION_ZSH"
