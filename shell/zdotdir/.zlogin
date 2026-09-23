# Runway ZDOTDIR shim — forwards to the user's real zsh config.
_runway_orig="${RUNWAY_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_runway_orig/.zlogin" ]] && source "$_runway_orig/.zlogin"
