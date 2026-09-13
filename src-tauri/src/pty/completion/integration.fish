status is-interactive; or return
set -g _vlxc_nonce '@NONCE@'
set -g _vlxc_selection '@SELECTION@'
set -g _vlxc_revision 0
function _vlxc_emit
    printf '\e]6973;%s;%s\a' $_vlxc_nonce $argv[1]
end
function _vlxc_hex
    env LC_ALL=C od -An -v -tx1 | string replace -ar '[ \n]' '' | string join ''
end
function _vlxc_ready --on-event fish_prompt
    _vlxc_emit P
end
function _vlxc_busy --on-event fish_preexec
    _vlxc_emit X
end
function _vlxc_query
    set -g _vlxc_buffer (commandline --current-buffer | string collect)
    set -g _vlxc_cursor (commandline --cursor)
    set -g _vlxc_revision (math $_vlxc_revision + 1)
    set -g _vlxc_values
    _vlxc_emit "A;$_vlxc_revision"
    set -l query (commandline --current-token --cut-at-cursor | string collect)
    set -l query_hex (printf %s "$query" | _vlxc_hex)
    _vlxc_emit "T;$query_hex"
    for entry in (complete -C (commandline --cut-at-cursor | string collect))
        test (count $_vlxc_values) -ge 64; and break
        set -l parts (string split -m 1 \t -- $entry)
        set -ga _vlxc_values (string escape -- $parts[1])
        set -l label (printf %s $parts[1] | _vlxc_hex)
        set -l description ''
        if test (count $parts) -gt 1
            set description (printf %s $parts[2] | _vlxc_hex)
        end
        _vlxc_emit "C;$label;$description"
    end
    _vlxc_emit Z
    commandline -f repaint
end
function _vlxc_accept
    read -l revision index < $_vlxc_selection; or return
    test "$revision" = "$_vlxc_revision"; or return
    string match -qr '^[0-9]+$' -- "$index"; or return
    test (commandline --cursor) = $_vlxc_cursor; or return
    test (commandline --current-buffer | string collect) = "$_vlxc_buffer"; or return
    set -l slot (math $index + 1)
    test $slot -le (count $_vlxc_values); or return
    commandline --current-token --replace -- $_vlxc_values[$slot]
    set -g _vlxc_values
    commandline -f repaint
end
bind \e\[24\;5~ _vlxc_query
bind \e\[23\;5~ _vlxc_accept
if bind --list-modes | string match -q insert
    bind -M insert \e\[24\;5~ _vlxc_query
    bind -M insert \e\[23\;5~ _vlxc_accept
end
