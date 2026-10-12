# Rift ZDOTDIR shim — forwards to the user's real zsh config.
_rift_orig="${RIFT_ORIG_ZDOTDIR:-$HOME}"
[[ -f "$_rift_orig/.zlogin" ]] && source "$_rift_orig/.zlogin"
