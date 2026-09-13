# VelaTerm keeps Readline authoritative and calls the session's registered completion functions.
[[ $- == *i* ]] || return
_vlxc_nonce='@NONCE@'
_vlxc_emit() { printf '\e]6973;%s;%s\a' "$_vlxc_nonce" "$1"; }
# Bash 3 has no READLINE_LINE/READLINE_POINT interface for bind -x callbacks.
if (( BASH_VERSINFO[0] < 4 )); then _vlxc_emit U; return; fi
_vlxc_selection='@SELECTION@'
_vlxc_revision=0
_vlxc_values=() _vlxc_cursors=()
_vlxc_hex() {
  local LC_ALL=C value=$1 byte i
  REPLY=''
  for ((i=0; i<${#value}; i++)); do
    printf -v byte '%02x' "'${value:i:1}"
    REPLY+=$byte
  done
}
_vlxc_ready() { _vlxc_emit P; }
_vlxc_busy() { _vlxc_emit X; }

_vlxc_query() {
  local saved_status=$? line=$READLINE_LINE point=$READLINE_POINT
  local quoting_locale=${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}
  local LC_ALL=C
  local i ch quote='' escape=0 start=0 end word='' spec func='' prev='' current='' candidate quoted REPLY
  local -a words=() COMPREPLY=()
  _vlxc_buffer=$line _vlxc_cursor=$point
  _vlxc_values=() _vlxc_cursors=()
  _vlxc_revision=$((_vlxc_revision + 1))
  _vlxc_emit "A;$_vlxc_revision"
  if [[ -z $line ]]; then _vlxc_emit Z; return "$saved_status"; fi
  # Tokenize without evaluating command substitutions or expanding user input.
  for ((i=0; i<point; i++)); do
    ch=${line:i:1}
    if ((escape)); then word+=$ch; escape=0
    elif [[ $ch == \\ && $quote != "'" ]]; then escape=1
    elif [[ -n $quote ]]; then
      if [[ $ch == "$quote" ]]; then quote=''; else word+=$ch; fi
    elif [[ $ch == '"' || $ch == "'" ]]; then quote=$ch
    elif [[ $ch == ' ' || $ch == $'\t' ]]; then
      if ((i > start)); then words+=("$word"); fi
      word='' start=$((i+1))
    elif [[ $ch == ';' || $ch == '|' || $ch == '&' || $ch == $'\n' ]]; then
      words=(); word='' start=$((i+1))
    else word+=$ch
    fi
  done
  current=$word
  _vlxc_hex "$current"
  _vlxc_emit "T;$REPLY"
  words+=("$current")
  end=$point
  while ((end < ${#line})); do
    ch=${line:end:1}
    [[ -z $quote && ( $ch == ' ' || $ch == $'\t' || $ch == ';' || $ch == '|' || $ch == '&' ) ]] && break
    if [[ -n $quote && $ch == "$quote" ]]; then quote=''; fi
    end=$((end+1))
  done
  local COMP_LINE=$line COMP_POINT=$point COMP_CWORD=$((${#words[@]} - 1)) COMP_TYPE=9 COMP_KEY=9
  local -a COMP_WORDS=("${words[@]}")
  if (( COMP_CWORD > 0 )); then
    prev=${words[COMP_CWORD-1]}
    spec=$(builtin complete -p -- "${words[0]}" 2>/dev/null)
    if [[ -z $spec ]]; then
      if declare -F _completion_loader >/dev/null; then _completion_loader "${words[0]}" >/dev/null 2>&1; fi
      spec=$(builtin complete -p -- "${words[0]}" 2>/dev/null)
    fi
    local -a parts=()
    # Only parse Bash's own shell-quoted serialization, never the user's command line.
    if [[ $spec == 'complete '* ]]; then eval "parts=(${spec#complete })"; fi
    for ((i=0; i<${#parts[@]}-1; i++)); do
      if [[ ${parts[i]} == -F ]]; then func=${parts[i+1]}; break; fi
    done
    if [[ $func =~ ^[a-zA-Z0-9_.:-]+$ ]] && declare -F "$func" >/dev/null; then
      "$func" "${words[0]}" "$current" "$prev" >/dev/null 2>&1
    elif ((${#parts[@]} > 1)); then
      while IFS= read -r candidate; do COMPREPLY+=("$candidate"); done < <(
        builtin compgen "${parts[@]:0:${#parts[@]}-1}" -- "$current" 2>/dev/null
      )
    fi
  fi
  if ((${#COMPREPLY[@]} == 0)); then
    while IFS= read -r candidate; do COMPREPLY+=("$candidate"); done < <(
      if ((COMP_CWORD == 0)); then compgen -c -- "$current"; else compgen -f -- "$current"; fi
    )
  fi
  for candidate in "${COMPREPLY[@]}"; do
    ((${#_vlxc_values[@]} >= 64)) && break
    [[ -n $candidate ]] || continue
    [[ -d $candidate && $candidate != */ ]] && candidate+=/
    LC_ALL=$quoting_locale printf -v quoted %q "$candidate"
    _vlxc_values+=("${line:0:start}${quoted}${line:end}")
    _vlxc_cursors+=($((start + ${#quoted})))
    _vlxc_hex "$candidate"
    _vlxc_emit "C;$REPLY;"
  done
  _vlxc_emit Z
  return "$saved_status"
}
_vlxc_accept() {
  local revision index
  read -r revision index < "$_vlxc_selection" || return
  [[ $revision == "$_vlxc_revision" && $index =~ ^[0-9]+$ ]] || return
  [[ $READLINE_LINE == "$_vlxc_buffer" && $READLINE_POINT == "$_vlxc_cursor" ]] || return
  ((index < ${#_vlxc_values[@]})) || return
  READLINE_LINE=${_vlxc_values[index]} READLINE_POINT=${_vlxc_cursors[index]}
  _vlxc_values=()
}
bind -x '"\e[24;5~":_vlxc_query'
bind -x '"\e[23;5~":_vlxc_accept'
# Keep the existing prompt command and its exit status intact.
if declare -p PROMPT_COMMAND 2>/dev/null | command grep -q 'declare -a'; then
  PROMPT_COMMAND+=('_vlxc_ready')
else
  PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }_vlxc_ready"
fi
if (( BASH_VERSINFO[0] > 4 || BASH_VERSINFO[1] >= 4 )); then
  PS0="${PS0-}"'$(_vlxc_busy)'
else
  # Older Bash has no pre-execution prompt hook.
  bind -x '"\e[99;1~":_vlxc_busy'
  bind '"\e[99;2~":accept-line'
  bind '"\C-m":"\e[99;1~\e[99;2~"'
  bind '"\C-j":"\e[99;1~\e[99;2~"'
fi
