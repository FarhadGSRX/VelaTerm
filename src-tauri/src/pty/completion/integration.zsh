# VelaTerm native completion. This file is sourced after the user's startup files.
[[ -o interactive ]] || return
typeset -g _vlxc_nonce='@NONCE@' _vlxc_selection='@SELECTION@'
typeset -gi _vlxc_revision=0 _vlxc_cursor=0 _vlxc_capturing=0
typeset -ga _vlxc_values _vlxc_cursors _vlxc_labels _vlxc_descriptions
typeset -g _vlxc_buffer
_vlxc_emit() { printf '\e]6973;%s;%s\a' "$_vlxc_nonce" "$1" }
_vlxc_hex() {
  local LC_ALL=C value=$1 byte
  local -i i
  REPLY=''
  for ((i=1; i<=${#value}; i++)); do
    printf -v byte '%02x' "'${value[i]}"
    REPLY+=$byte
  done
}
_vlxc_ready() { _vlxc_emit P }
_vlxc_busy() { _vlxc_emit X }
autoload -Uz add-zsh-hook add-zle-hook-widget
add-zsh-hook preexec _vlxc_busy
add-zle-hook-widget line-init _vlxc_ready
add-zle-hook-widget line-finish _vlxc_busy
_vlxc_keymap() { if [[ $KEYMAP == vicmd ]]; then _vlxc_busy; else _vlxc_ready; fi }
add-zle-hook-widget keymap-select _vlxc_keymap
add-zle-hook-widget isearch-update _vlxc_busy
add-zle-hook-widget isearch-exit _vlxc_keymap
(( $+functions[compdef] )) || { autoload -Uz compinit; compinit -i; }

_vlxc_compadd() {
  local -a _vlxc_hits _vlxc_display
  local -i hit_index=0 file_matches=0
  local arg hit left right replacement suffix='' prefix='' pending='' display_name='' file_prefix='' display_label extra_suffix
  # Capture-only requests belong to completers themselves, not the displayed candidate list.
  for arg in "$@"; do
    if [[ $arg == -[OAD]* ]]; then builtin compadd "$@"; return; fi
    if [[ -n $pending ]]; then
      case $pending in -S|-s) suffix+=$arg;; -P|-p) prefix+=$arg;; -d) display_name=$arg;; -W) file_prefix=$arg;; esac
      pending=''
    elif [[ $arg == -- ]]; then break
    elif [[ $arg == -[SPsp] ]]; then pending=$arg
    elif [[ $arg == -d || $arg == -ld ]]; then pending=-d
    elif [[ $arg == -W ]]; then pending=-W
    elif [[ $arg =~ '^-[a-zA-Z]*f[a-zA-Z]*$' ]]; then file_matches=1
    fi
  done
  if [[ $display_name == \(*\) ]]; then
    local display_words=${display_name[2,-2]}
    _vlxc_display=("${(@Q)${(z)display_words}}")
  elif [[ -n $display_name ]]; then
    _vlxc_display=("${(@P)display_name}")
  fi
  builtin compadd -A _vlxc_hits -D _vlxc_display "$@"
  # Preserve native match accounting so fallback matchers do not add unrelated candidates.
  builtin compadd "$@"
  left=${BUFFER[1,$(( CURSOR - ${#PREFIX} ))]}
  right=${BUFFER[$(( CURSOR + ${#SUFFIX} + 1 )),-1]}
  for hit in "${_vlxc_hits[@]}"; do
    (( ++hit_index ))
    (( ${#_vlxc_values} >= 64 )) && break
    display_label=${(Q)hit}
    extra_suffix=$suffix
    if (( file_matches )) && [[ -d "${(Q)file_prefix}${(Q)hit}" && -z $extra_suffix ]]; then
      [[ $hit == */ ]] || extra_suffix=/
      [[ $display_label == */ ]] || display_label+=/
    fi
    [[ ${_vlxc_labels[(Ie)$display_label]} != 0 ]] && continue
    replacement="${prefix}${hit}${extra_suffix}"
    _vlxc_values+=("${left}${replacement}${right}")
    _vlxc_cursors+=($(( ${#left} + ${#replacement} )))
    _vlxc_labels+=("$display_label")
    _vlxc_descriptions+=("${_vlxc_display[hit_index]-}")
  done
}
_vlxc_complete_internal() {
  local REPLY
  _vlxc_hex "${(Q)PREFIX}"
  _vlxc_emit "T;$REPLY"
  local previous=${functions[compadd]-}
  functions[compadd]=$functions[_vlxc_compadd]
  { _main_complete 2>/dev/null } always {
    if [[ -n $previous ]]; then functions[compadd]=$previous; else unfunction compadd; fi
    compstate[insert]=''
    compstate[list]=''
  }
}
zle -C _vlxc_collect complete-word _vlxc_complete_internal
_vlxc_query() {
  local saved_status=$? label label_hex REPLY
  local -i item=0
  _vlxc_buffer=$BUFFER _vlxc_cursor=$CURSOR
  _vlxc_values=() _vlxc_cursors=() _vlxc_labels=() _vlxc_descriptions=()
  (( ++_vlxc_revision ))
  _vlxc_emit "A;$_vlxc_revision"
  if [[ -n $BUFFER ]]; then
    zle _vlxc_collect
  fi
  BUFFER=$_vlxc_buffer CURSOR=$_vlxc_cursor
  for label in "${_vlxc_labels[@]}"; do
    (( ++item ))
    _vlxc_hex "$label"; label_hex=$REPLY
    _vlxc_hex "${_vlxc_descriptions[item]}"
    _vlxc_emit "C;$label_hex;$REPLY"
  done
  _vlxc_emit Z
  zle -R
  return $saved_status
}
_vlxc_accept() {
  local revision index
  read -r revision index < "$_vlxc_selection" || return
  [[ $revision == $_vlxc_revision && $index == <-> ]] || return
  [[ $BUFFER == $_vlxc_buffer && $CURSOR == $_vlxc_cursor ]] || return
  (( index >= 0 && index < ${#_vlxc_values} )) || return
  BUFFER=${_vlxc_values[$((index + 1))]}
  CURSOR=${_vlxc_cursors[$((index + 1))]}
  _vlxc_values=()
  zle -R
}
zle -N _vlxc_query
zle -N _vlxc_accept
for _vlxc_map in emacs viins; do
  bindkey -M "$_vlxc_map" '^[[24;5~' _vlxc_query
  bindkey -M "$_vlxc_map" '^[[23;5~' _vlxc_accept
done
unset _vlxc_map
