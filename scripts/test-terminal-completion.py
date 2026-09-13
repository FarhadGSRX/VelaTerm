"""Exercise native completion through real PTYs without changing shell profiles."""

import argparse
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parent.parent


def verify(shell, family, user_config=False):
    with tempfile.TemporaryDirectory(prefix="vlx-completion-pty-") as directory:
        root = Path(directory)
        selection = root / "selection"
        selection.write_text("0 0\n")
        script = root / ("integration.ps1" if family == "pwsh" else "integration")
        extension = "ps1" if family == "pwsh" else family
        script.write_text((ROOT / f"src-tauri/src/pty/completion/integration.{extension}").read_text()
                          .replace("@NONCE@", "test").replace("@SELECTION@", str(selection)))
        pid, fd = pty.fork()
        if pid == 0:
            os.environ["TERM"] = "xterm-256color"
            os.environ["ZDOTDIR"] = directory
            arguments = {"zsh": ["-f", "-i"], "bash": ["--noprofile", "--norc", "-i"],
                         "fish": ["--no-config", "-i"],
                         "pwsh": ["-NoLogo", "-NoProfile", "-NoExit", "-Command",
                                  f"Import-Module PSReadLine; . '{script}'"]}[family]
            if family == "zsh" and user_config:
                os.environ.pop("ZDOTDIR", None)
                arguments = ["-l", "-i"]
            os.execv(shell, [shell, *arguments])
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))

        def read(seconds=0.2):
            output = b""
            if family == "pwsh":
                seconds = max(seconds * 3, 1)
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([fd], [], [], 0.03)[0]:
                    try:
                        chunk = os.read(fd, 65536)
                        output += chunk
                        if b"\x1b[6n" in chunk:
                            os.write(fd, b"\x1b[1;1R")
                    except OSError:
                        break
            return output

        def until(marker, timeout=20):
            output = b""
            deadline = time.monotonic() + timeout
            while marker not in output and time.monotonic() < deadline:
                output += read(0.1)
            return output

        def send(value):
            os.write(fd, value.encode() if isinstance(value, str) else value)

        def query():
            read()
            send(b"\x1b[24;5~")
            output = until(b"6973;test;Z\x07")
            revision = re.search(rb"6973;test;A;(\d+)\x07", output)
            assert revision, (family, "no completion response", output)
            assert b"6973;test;Z\x07" in output, (family, "incomplete response", output)
            labels = [bytes.fromhex(s.decode()).decode() for s in
                      re.findall(rb"6973;test;C;([0-9a-f]*);", output)]
            assert b"6973;test;T;" in output, (family, "missing native completion token")
            if family == "zsh" and "branch-two" in labels:
                descriptions = [bytes.fromhex(s.decode()).decode() for s in
                                re.findall(rb"6973;test;C;[0-9a-f]*;([0-9a-f]*)", output)]
                assert "Second branch" in descriptions[labels.index("branch-two")], (family, descriptions)
            return revision.group(1).decode(), labels

        def accept(revision, index):
            selection.write_text(f"{revision} {index}\n")
            send(b"\x1b[23;5~")
            read()

        def buffer():
            send(b"\x1b[21;5~" if family == "pwsh" else b"\x1b[42~")
            read()
            return (root / "buffer").read_text().removesuffix("\n")

        try:
            if family != "pwsh":
                read(1)
                send(f"source '{script}'\r")
            output = until(b"6973;test;P\x07")
            assert b"6973;test;P\x07" in output, (family, "integration unavailable", output)
            if family == "zsh":
                setup = ("_vlxtest() { local -a native_items=('中文 path:Unicode argument' 'branch-two:Second branch'); _describe arguments native_items; }; compdef _vlxtest vlxtest; "
                         f"_vlxdump() {{ printf %s \"$BUFFER\" > '{root}/buffer'; }}; "
                         "zle -N _vlxdump; bindkey '^[[42~' _vlxdump")
            elif family == "bash":
                setup = ("_vlxtest() { COMPREPLY=('中文 path' branch-two); }; complete -F _vlxtest vlxtest; "
                         f"_vlxdump() {{ printf %s \"$READLINE_LINE\" > '{root}/buffer'; }}; "
                         "bind -x '\"\\e[42~\":_vlxdump'")
            elif family == "fish":
                setup = ("complete -c vlxtest -f -a \"'中文 path' branch-two\"; "
                         f"function _vlxdump; commandline --current-buffer > '{root}/buffer'; end; "
                         "bind \\e\\[42~ _vlxdump")
            else:
                setup = ("function global:vlxtest { param([string]$Value) }; "
                         "Register-ArgumentCompleter -CommandName vlxtest -ParameterName Value -ScriptBlock { "
                         "[System.Management.Automation.CompletionResult]::new(\"'中文 path'\", '中文 path', 'ParameterValue', 'unicode'); "
                         "[System.Management.Automation.CompletionResult]::new('branch-two', 'branch-two', 'ParameterValue', 'branch') }; "
                         "Set-PSReadLineKeyHandler -Chord Ctrl+F10 -ScriptBlock { $line=''; $cursor=0; "
                         "[Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line,[ref]$cursor); "
                         f"[IO.File]::WriteAllText('{root}/buffer',$line) }}")
            if family == "pwsh":
                fixture = root / "fixture.ps1"
                fixture.write_text(setup)
                send(f". '{fixture}'\r")
            else:
                send(setup + "\r")
            output = until(b"6973;test;P\x07")
            assert b"6973;test;P\x07" in output, (family, "fixture setup did not finish", output)
            send("vlxtest ")
            revision, labels = query()
            assert "中文 path" in labels, (family, labels, buffer())
            assert buffer() == "vlxtest ", (family, "query changed input")
            accept(revision, labels.index("中文 path"))
            value = buffer()
            # The raw buffer must encode exactly one argument, even with spaces and non-ASCII text.
            assert value in ("vlxtest 中文\\ path", "vlxtest '中文 path'",
                             "vlxtest $'\\344\\270\\255\\346\\226\\207 path'"), (family, value)
            send(b"\x03")
            read()
            send("vlxtest ")
            revision, labels = query()
            send("changed")
            read()
            accept(revision, labels.index("branch-two"))
            assert buffer() == "vlxtest changed", (family, "stale candidate replaced input")
            send(b"\x03")
            read()
            send("vlxtest br suffix")
            send(b"\x1b[D" * len(" suffix"))
            revision, labels = query()
            accept(revision, labels.index("branch-two"))
            assert buffer() == "vlxtest branch-two suffix", (family, buffer())
            send(b"\x03")
            read()
            if family == "zsh":
                (root / "中文 folder" / "child").mkdir(parents=True)
                send(f"cd {root}/中")
                revision, labels = query()
                accept(revision, labels.index("中文 folder/"))
                assert buffer().endswith("中文\\ folder/"), (family, "directory suffix or quoting", buffer())
                send("ch")
                revision, labels = query()
                accept(revision, labels.index("child/"))
                assert buffer().endswith("中文\\ folder/child/"), (family, "nested directory completion", buffer())
                send(b"\x03")
                read()
            marker = root / "executed"
            if family == "pwsh":
                fixture.write_text(f"function global:vlxtest {{ param([string]$Value); [IO.File]::WriteAllText('{marker}',$Value) }}")
                send(f". '{fixture}'\r")
            else:
                send((f"function vlxtest; printf '%s' \"$argv[1]\" > '{marker}'; end\r" if family == "fish"
                      else f"vlxtest() {{ printf '%s' \"$1\" > '{marker}'; }}\r"))
            until(b"6973;test;P\x07")
            send("vlxtest ")
            revision, labels = query()
            accept(revision, labels.index("中文 path"))
            assert not marker.exists(), (family, "selection executed a command")
            send("\r")
            output = read()
            assert marker.read_text() == "中文 path", (family, "incorrect shell quoting")
            assert b"6973;test;X\x07" in output, (family, "missing execution boundary")
            print(f"PASS {family}: native candidates, Unicode/space quoting, unchanged query input, stale rejection, insert without execution")
        finally:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.close(fd)
            os.waitpid(pid, 0)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--zsh")
    parser.add_argument("--bash", help="Bash 4 or later")
    parser.add_argument("--fish")
    parser.add_argument("--pwsh")
    parser.add_argument("--zsh-user-config", action="store_true", help="Also verify Zsh with the current user's startup files")
    args = parser.parse_args()
    if not any((args.zsh, args.bash, args.fish, args.pwsh)):
        parser.error("Supply at least one shell executable")
    for name in ("zsh", "bash", "fish", "pwsh"):
        if getattr(args, name):
            verify(getattr(args, name), name)
    if args.zsh and args.zsh_user_config:
        verify(args.zsh, "zsh", user_config=True)
