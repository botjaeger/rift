# Rift completion server for zsh (crates/rift-core/src/shell_complete.rs).
#
# Run as `zsh -f -c "$(<this)" rift-complete <zsh program> [-l]`. It starts
# the user's interactive zsh inside a zpty (rc files, compinit, plugins and
# completion functions load as in a terminal), then answers requests read
# from stdin: an id, a directory, the command line, a word count n, then n+1
# words (used by the bash server; skipped here). A widget in the inner shell
# reads the directory and the line as data (`read -k`, never through the
# keymap, so the user's widgets on printable keys — autopair, magic-space —
# never see them), sets BUFFER and runs complete-word; nothing is run:
# Return and ^J are unbound there, and the `_expand` completer (which
# evaluates the word: globs with qualifiers, substitutions) is disabled.
# compadd is wrapped to print each match (the technique of
# zsh-capture-completion). The server ends when its input ends, and while a
# completion function runs, once its parent (Rift) is gone. With
# RIFT_COMPLETE_FPATH set (the remote
# `rift-complete`, whose shell already read the rc files), the inner shell
# is started with that fpath instead of reading them again. The reply on
# stdout is
#   \x1e begin <id>
#   \x1f<word>\x1f<description>      (one per match)
#   \x1e end <id>
# Requests and replies hold no control characters other than those markers;
# Rift rejects lines and directories that contain any.

zmodload zsh/zpty zsh/zselect || exit 1
zmodload zsh/system 2>/dev/null
setopt localoptions noshwordsplit
integer parent=$PPID
local prog=$1
shift
local -a args=("$@")
local line got

# sourced inside the user's shell after its rc files
umask 077
local setup_dir=$(mktemp -d "${TMPDIR:-/tmp}/rift-complete.XXXXXXXX") || exit 1
local setup=$setup_dir/setup.zsh
trap 'rm -rf -- "$setup_dir"' EXIT
cat >| $setup <<'EOF'
PROMPT= RPROMPT= PS2= PS4=
# this hidden shell keeps no history of its own: nothing it does reaches
# the user's $HISTFILE
unset HISTFILE
unset zle_bracketed_paste
[[ -n $RIFT_COMPLETE_FPATH ]] && fpath=(${(s.:.)RIFT_COMPLETE_FPATH})
unsetopt beep
# compsys, if the rc files did not load it
(( $+functions[compdef] )) || {
  # This warm server needs no shared dump: it is executable shell code.
  autoload -Uz compinit && compinit -u -D -d /dev/null
}
bindkey -e
bindkey '^M' undefined-key
bindkey '^J' undefined-key
zstyle ':completion:*' list-grouped false
zstyle ':completion:*' insert-tab false
zstyle ':completion:*' list-separator ''
# keep matches for this request only: never insert, list or menu-select
__rift_quiet() { compstate[insert]=''; compstate[list]=''; compstate[old_list]=''; }
# _expand evaluates the word being completed (`*(e:…:)`, `$(…)`): the line
# is data, so it never runs here (it otherwise answers nothing; the next
# completer goes on)
_expand() { return 1 }
# one request field after the key, up to ^_: read as data, not as keys
__rift_field() {
  local c
  REPLY=
  while read -k1 c; do
    [[ $c == $'\x1f' ]] && return 0
    REPLY+=$c
  done
  return 1
}
# ^X^G <dir> ^_ <line> ^_: complete <line> in <dir> (keys the terminal itself
# never takes, should they arrive before the editor starts: ^R would reprint)
__rift_request() {
  local dir
  __rift_field || return
  dir=$REPLY
  __rift_field || return
  builtin cd -q -- "$dir" 2>/dev/null
  BUFFER=$REPLY
  CURSOR=$#BUFFER
  compprefuncs=()
  comppostfuncs=(__rift_quiet)
  print -rn -- $'\n\x1e'beg$'\n'
  zle complete-word
  print -rn -- $'\n\x1e'fin$'\n'
  BUFFER=
}
zle -N __rift_request
bindkey '^X^G' __rift_request
zmodload zsh/zutil
# every match a completion function adds is printed, with its description
compadd() {
  # -O/-A/-D only collect matches into an array: not ours
  if [[ ${@[1,(i)(-|--)]} == *-(O|A|D)\ * ]]; then
    builtin compadd "$@"
    return $?
  fi
  typeset -a __hits __dscr __tmp
  if (( $@[(I)-d] )); then
    __tmp=${@[$[${@[(i)-d]}+1]]}
    if [[ $__tmp == \(* ]]; then
      eval "__dscr=$__tmp"
    else
      __dscr=( "${(@P)__tmp}" )
    fi
  fi
  builtin compadd -A __hits -D __dscr "$@"
  setopt localoptions norcexpandparam extendedglob
  typeset -A apre hpre hsuf asuf
  zparseopts -E P:=apre p:=hpre S:=asuf s:=hsuf
  integer dirsuf=0
  if [[ -z $hsuf && "${${@//-default-/}% -# *}" == *-[[:alnum:]]#f* ]]; then
    dirsuf=1
  fi
  [[ -n $__hits ]] || return
  # -Q: the matches are quoted already (file names); print them literal
  integer quoted=0
  [[ -n ${(M)@[1,(i)(-|--)]:#-[[:alpha:]]#Q[[:alpha:]]#} ]] && quoted=1
  local dsuf dscr i word
  for i in {1..$#__hits}; do
    word=$__hits[$i]
    (( quoted )) && word=${(Q)word}
    word=$IPREFIX$apre$hpre$word
    (( dirsuf )) && [[ -z $hsuf$asuf && -d $word ]] && dsuf=/ || dsuf=
    (( $#__dscr >= $i )) && dscr=${${__dscr[$i]}##$__hits[$i] #} || dscr=
    dscr=${dscr##-- }
    print -r -- $'\x1f'"$word$dsuf$hsuf$asuf"$'\x1f'"$dscr"
  done
}
print -r -- $'\x1e'rdy
EOF

# The inner shell starts through a ZDOTDIR shim: each file sources the user's
# own, then the last one unsets HISTFILE, so no line typed into this shell can
# reach the user's history, whatever INC_APPEND_HISTORY or SHARE_HISTORY say
# (HIST_IGNORE_SPACE alone is not enough: a compinit in the rc files asking
# about insecure directories eats the line's leading space). The sourcing
# stays inline: inside a function the rc's `typeset`s would be local.
local load='if [[ -n $__rift_zd ]]; then ZDOTDIR=$__rift_zd; else unset ZDOTDIR; fi
[[ -r ${ZDOTDIR:-$HOME}/NAME ]] && source ${ZDOTDIR:-$HOME}/NAME
__rift_zd=${ZDOTDIR-}
'
local finish='if [[ -n $__rift_zd ]]; then ZDOTDIR=$__rift_zd; else unset ZDOTDIR; fi
unset __rift_zd __rift_shim
'
print -r -- '__rift_shim=$ZDOTDIR __rift_zd=$RIFT_COMPLETE_ZDOTDIR
unset RIFT_COMPLETE_ZDOTDIR
'"${load//NAME/.zshenv}"'ZDOTDIR=$__rift_shim' >| $setup_dir/.zshenv
print -r -- "${load//NAME/.zprofile}"'ZDOTDIR=$__rift_shim' >| $setup_dir/.zprofile
print -r -- "${load//NAME/.zshrc}"'unset HISTFILE
if [[ -o login ]]; then ZDOTDIR=$__rift_shim; else
'"$finish"'fi' >| $setup_dir/.zshrc
print -r -- "${load//NAME/.zlogin}"'unset HISTFILE
'"$finish" >| $setup_dir/.zlogin
export RIFT_COMPLETE_ZDOTDIR=${ZDOTDIR-} ZDOTDIR=$setup_dir

# HIST_IGNORE_SPACE too, for a shell that skips the rc files (-f, the remote
# `rift-complete`): the line that loads the setup has a leading space
RIFT_COMPLETE_SHELL=1 zpty rift ${(q)prog} -o histignorespace ${(q)args[@]} -i || exit 1
integer pty=$REPLY
# Wait until the inner shell wrote something; false once the parent that
# asks is gone (Rift quit or crashed while a completion function hangs).
__rift_wait() {
  local -a reply
  while :; do
    zselect -t 100 -r $pty && return 0
    (( ${sysparams[ppid]:-$parent} == parent )) && kill -0 $parent 2>/dev/null || return 1
  done
}
zpty -w rift " source ${(q)setup}"
# the rc files print what they like; the setup's marker ends it
while __rift_wait && zpty -r rift got; do
  [[ $got == *$'\x1e'rdy* ]] && break
done
rm -rf -- $setup_dir
print -r -- $'\x1e'ready

while IFS= read -r id && IFS= read -r cwd && IFS= read -r line && IFS= read -r n; do
  [[ $n == <-> ]] || break
  # the words are for bash; zsh splits the line itself
  repeat $(( n + 1 )); do IFS= read -r got || break 2; done
  print -r -- $'\x1e'"begin $id"
  zpty -w -n rift $'\C-x\C-g'"$cwd"$'\x1f'"$line"$'\x1f'
  local inside=0
  while :; do
    __rift_wait && zpty -r rift got || break 2
    got=${got//[$'\r\n']/}
    if [[ $got == *$'\x1e'fin* ]]; then
      (( inside )) && break
    elif [[ $got == *$'\x1e'beg* ]]; then
      inside=1
    elif (( inside )) && [[ $got == *$'\x1f'* ]]; then
      print -r -- $'\x1f'"${got#*$'\x1f'}"
    fi
  done
  print -r -- $'\x1e'"end $id"
done
zpty -d rift
