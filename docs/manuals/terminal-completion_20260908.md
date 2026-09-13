# Command suggestions

Command suggestions use the current shell's completion definitions. Commands remain in the terminal's normal input line; choosing a suggestion inserts text without executing it. This feature does not call an AI model.

## Settings and controls

Under **Settings → Terminal → Command suggestions**, choose:

- **Automatic** (default): coalesce input for 25 ms before requesting suggestions. Continued typing does not restart that window. While a request is running, only the latest input is retained for an immediate follow-up; stale results are discarded. Exact matches remain visible. Further input requests fresh suggestions.
- **On Tab**: request suggestions only when Tab is pressed.
- **Off**: hide suggestions and stop requesting candidates. Tab retains the shell’s native behavior. Changes apply to open terminals; switching back enables suggestions again.

Use the arrow keys to select a candidate, then Tab to insert it. A mouse click also inserts the selected candidate. Enter always passes through to the shell to run the current command without applying a suggestion. Escape dismisses the list. An arrow that cannot move the selection any further is handed back to the shell, so Up on the first candidate recalls the previous command instead of being swallowed.

The backend ranks candidates by exact match, prefix match, substring match, and other native matches, preferring case-sensitive matches within exact and prefix groups. Shorter candidates come first within each group, with alphabetical ordering for ties. Native selection indices remain unchanged.

The compact suggestion list highlights the shell's current completion token and displays native descriptions when the provider supplies them. The selected candidate and popup position are retained while results refresh. Page Up and Page Down move through longer lists; Ctrl+Space requests suggestions manually. If input changes while a selection is pending, insertion waits for a current shell result and only applies the same candidate if it is still available. Accepting a suggestion closes the list, including for directories ending in `/`. Another Tab press explicitly requests suggestions again, including the next directory level. Further typing requests suggestions automatically in Automatic mode.

The popup follows the Monaco styling used by vlx-sql: compact rows, a thin border, subdued shadow and a single selection background. Width adapts to the content, up to 430 px. The persistent status bar and duplicate description panel are hidden; hover over a candidate to read its full description.

The setting is saved by the backend and synchronized between clients. Shell integration is loaded when a new terminal starts, so restart terminals that were already running before this feature was installed. Your shell startup files are not modified. A custom terminal startup command still runs after integration is loaded.

Zsh loads integration through temporary startup wrappers, after the user's login files. The wrappers preserve startup order and restore `ZDOTDIR`; no initialization command is typed into the terminal. This requires a build containing the startup-wrapper fix; existing processes retain their original integration.

## Shell support

| Shell or environment | Integration |
| --- | --- |
| Zsh | Native completion widgets and `compadd`; existing completion definitions are retained. |
| Bash 4 or later | Registered completion functions and specifications; command and file completion when no registered candidates are available. |
| Fish | Native `complete -C` candidates and descriptions. |
| PowerShell with PSReadLine | Native `CommandCompletion.CompleteInput`; PSReadLine applies the replacement range. |
| Git Bash | Bash integration, using that shell's paths and installed definitions. |
| WSL | The selected distribution's default Bash, Zsh or Fish; paths are resolved inside that distribution. |
| CMD, Bash 3.2, unintegrated shells | Native Tab behavior is retained; no suggestion list is provided. |

On Windows no integration is loaded at all for now, whatever the setting holds, and the setting itself is hidden there. Git Bash, Windows PowerShell and WSL sessions keep their native Tab behavior. Loading Bash-family integration would require typing an initialization command into the terminal, and each suggestion request creates helper processes, which is slow enough on that platform to stall typing.

PowerShell execution policy still applies to the generated integration script. This feature does not bypass that policy. Windows PowerShell scripts are written with a UTF-8 BOM so non-ASCII user paths are preserved.

Entering SSH or another nested shell does not automatically install integration in that environment. Managed agent sessions are not modified. Suggestions pause during detected command execution, alternate-screen programs, IME composition and shell history navigation, which covers Ctrl+R, Ctrl+S and the arrow keys. Arrow keys never start an automatic request: the request runs a completion widget inside the shell, and Zsh ends its continuing history search after the first recalled entry once another widget runs, so Up would stop moving through earlier commands. An unmodified Tab falls back to the shell when integration is unavailable or no candidates were extracted.

## Compatibility limits

Candidates depend on the completion definitions installed in the shell. VelaTerm does not supply a complete command catalog or a separate history-based prediction engine. Complex Bash providers may depend on Readline completion internals that a `bind -x` callback cannot reproduce exactly.

Native completion functions can run external programs and may be slow. After a request is acknowledged, the interface limits polling to two seconds; it does not kill a shell function that is still running. Ctrl+C can interrupt the shell normally. Requests are serialized per session, and both the backend and the shell reject stale selections.

The integration reserves Ctrl+F12 for candidate collection and Ctrl+F11 for applying a selection. A shell configuration that rebinds these keys after integration starts can disable the feature. Bash 4.0–4.3 additionally wraps Enter and Ctrl+J because those versions do not provide the `PS0` pre-execution hook.

## Verification status

Verified on macOS with Zsh and Bash 5.2, and in a local Linux container with Bash, Zsh, Fish and PowerShell 7.4. Tests cover native candidates, Unicode and space quoting, unchanged input after querying, stale selection rejection and insertion without execution. Browser checks use vlx-browser and cover the default automatic mode, Tab mode, setting changes and candidate insertion.

The editor-style interaction revision also verifies continuous narrowing without closing or moving the popup, native description extraction, UTF-16 highlight ranges, selection during pending updates, repeated manual requests and nested Zsh directory completion with Unicode and spaces. Descriptions come from the native provider; providers without descriptions, including many Bash definitions, keep that field empty.

Windows ConPTY, Windows PowerShell 5.1, Git Bash and WSL are deliberately excluded for now: no integration is loaded there, so only the native Tab behavior applies.

The PTY checks can be repeated with:

```sh
python3 scripts/test-terminal-completion.py --zsh /bin/zsh --bash /path/to/bash
python3 scripts/test-terminal-completion.py --zsh /bin/zsh --zsh-user-config
python3 scripts/test-terminal-completion.py --fish /usr/bin/fish --pwsh /path/to/pwsh
```
