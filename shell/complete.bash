# shellcheck shell=bash
# Rift completion server for bash (crates/rift-core/src/shell_complete.rs).
#
# Run as `bash [-l] -i -c "$(<this)"`, so the user's rc files (and
# bash-completion, when they load it) are read first. Answers requests read
# from stdin: an id, a directory, the command line (for zsh and fish; the
# line is not used here), the index n of the word being completed, then
# n+1 words, one per line (the current command's words, the last one the
# word being completed). The words are data: the
# completion function registered for the command (`complete -F`, loaded on
# demand through bash-completion's loader) is called with COMP_WORDS set to
# them, as bash does on Tab; nothing is evaluated. The reply on stdout is
#   \x1e begin <id>
#   \x1f<word>\x1f                   (one per match)
#   \x1e end <id>
# Requests hold no control characters; Rift rejects words that contain any.

__rift_reply() {
  local w
  for w in "$@"; do
    [[ -n $w ]] && printf '\037%s\037\n' "$w"
  done
}

# Matches for COMP_WORDS / COMP_CWORD under the completion spec of `$1`.
__rift_complete() {
  local cmd=$1 spec func cur prev w
  spec=$(complete -p -- "$cmd" 2>/dev/null)
  if [[ -z $spec ]]; then
    # bash-completion loads completion files on first use
    if declare -F __load_completion >/dev/null; then
      __load_completion "$cmd" >/dev/null 2>&1
    elif declare -F _comp_load >/dev/null; then
      _comp_load -- "$cmd" >/dev/null 2>&1
    elif declare -F _completion_loader >/dev/null; then
      _completion_loader "$cmd" >/dev/null 2>&1
    fi
    spec=$(complete -p -- "$cmd" 2>/dev/null)
  fi
  cur=${COMP_WORDS[COMP_CWORD]}
  prev=${COMP_WORDS[COMP_CWORD-1]}
  if [[ -z $spec ]]; then
    # no spec (bash-completion not installed): the names bash itself knows
    # for its builtins, as zsh and fish complete them out of the box
    case $cmd in
      kill)
        if [[ $cur == -* ]]; then
          while IFS= read -r w; do
            __rift_reply "-$w"
          done < <(compgen -A signal -- "${cur#-}" 2>/dev/null)
          return
        fi
        spec='complete -j'
        ;;
      trap) spec='complete -A signal' ;;
      unset | export | readonly | declare | typeset | local) spec='complete -v' ;;
      alias | unalias) spec='complete -a' ;;
      fg | bg | jobs | disown | wait) spec='complete -j' ;;
      help) spec='complete -A helptopic' ;;
      shopt) spec='complete -A shopt' ;;
      set) [[ $prev == [-+]o ]] && spec='complete -A setopt' ;;
      type | which | hash | command) spec='complete -c' ;;
      builtin | enable) spec='complete -b' ;;
    esac
  fi
  [[ -n $spec ]] || return
  COMPREPLY=()
  if [[ $spec =~ \ -F\ ([^ ]+) ]]; then
    func=${BASH_REMATCH[1]}
    declare -F -- "$func" >/dev/null || return
    "$func" "$cmd" "$cur" "$prev" </dev/null >/dev/null 2>&1
  fi
  # word lists and actions (`complete -W …`, `-A …`, `-d`) through compgen;
  # only option letters from the spec are passed, never its text
  local -a acts=()
  local re=' -A ([a-z]+)'
  local rest=$spec
  while [[ $rest =~ $re ]]; do
    acts+=(-A "${BASH_REMATCH[1]}")
    rest=${rest#*"${BASH_REMATCH[0]}"}
  done
  re=' -([abcdefgjksuv])( |$)'
  rest=$spec
  while [[ $rest =~ $re ]]; do
    acts+=("-${BASH_REMATCH[1]}")
    rest=${rest#*" -${BASH_REMATCH[1]}"}
  done
  # `-W 'a b c'` as `complete -p` prints it (single-quoted)
  re=" -W '([^']*)'"
  if [[ $spec =~ $re ]]; then
    acts+=(-W "${BASH_REMATCH[1]}")
  fi
  if ((${#acts[@]})); then
    while IFS= read -r w; do
      COMPREPLY+=("$w")
    done < <(compgen "${acts[@]}" -- "$cur" 2>/dev/null)
  fi
  # file-name specs (`-d`, `-A directory`, `-o filenames`, `-o dirnames`):
  # a directory ends in `/`, as readline shows it on Tab
  if [[ $spec =~ \ -(d|A\ directory|o\ filenames|o\ dirnames)(\ |$) ]]; then
    local i
    for i in "${!COMPREPLY[@]}"; do
      w=${COMPREPLY[i]}
      [[ -n $w && $w != */ && -d $w ]] && COMPREPLY[i]=$w/
    done
  fi
  __rift_reply "${COMPREPLY[@]}"
}

__rift_serve() {
  local id cwd line n i w
  printf '\036ready\n'
  while IFS= read -r id && IFS= read -r cwd && IFS= read -r line && IFS= read -r n; do
    [[ $n =~ ^[0-9]+$ ]] || break
    COMP_WORDS=()
    for ((i = 0; i <= n; i++)); do
      IFS= read -r w || return
      COMP_WORDS+=("$w")
    done
    COMP_CWORD=$n
    : "$line"
    COMP_LINE="${COMP_WORDS[*]}"
    COMP_POINT=${#COMP_LINE}
    COMP_TYPE=9
    COMP_KEY=9
    printf '\036begin %s\n' "$id"
    if builtin cd -- "$cwd" 2>/dev/null; then
      __rift_complete "${COMP_WORDS[0]}"
    fi
    printf '\036end %s\n' "$id"
  done
}

__rift_serve
