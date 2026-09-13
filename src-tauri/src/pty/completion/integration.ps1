# PSReadLine retains ownership of the input buffer and replacement offsets.
if (-not (Get-Module PSReadLine)) {
    [Console]::Write("$([char]27)]6973;@NONCE@;U$([char]7)")
    return
}
if ($script:VlxCompletionNonce -eq '@NONCE@') { return }
$script:VlxCompletionNonce = '@NONCE@'
$script:VlxCompletionSelection = '@SELECTION@'
$script:VlxCompletionRevision = 0
function Send-VlxCompletion([string]$Value) {
    [Console]::Write("$([char]27)]6973;$script:VlxCompletionNonce;$Value$([char]7)")
}
function ConvertTo-VlxHex([string]$Value) {
    [BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($Value)).Replace('-', '').ToLowerInvariant()
}
$script:VlxOriginalPrompt = $function:global:prompt
function global:prompt {
    $result = & $script:VlxOriginalPrompt
    Send-VlxCompletion 'P'
    $result
}
Set-PSReadLineKeyHandler -Chord Ctrl+F12 -ScriptBlock {
    $savedExitCode = $global:LASTEXITCODE
    $line = ''; $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
    $script:VlxCompletionBuffer = $line
    $script:VlxCompletionCursor = $cursor
    $script:VlxCompletionRevision++
    $script:VlxCompletionMatches = @()
    Send-VlxCompletion "A;$script:VlxCompletionRevision"
    try {
        if ($line.Length -gt 0) {
            $result = [System.Management.Automation.CommandCompletion]::CompleteInput($line, $cursor, $null)
            $script:VlxCompletionStart = $result.ReplacementIndex
            $script:VlxCompletionLength = $result.ReplacementLength
            $queryLength = [Math]::Max(0, $cursor - $result.ReplacementIndex)
            Send-VlxCompletion "T;$(ConvertTo-VlxHex $line.Substring($result.ReplacementIndex, $queryLength))"
            $script:VlxCompletionMatches = @($result.CompletionMatches | Select-Object -First 64)
            foreach ($item in $script:VlxCompletionMatches) {
                Send-VlxCompletion "C;$(ConvertTo-VlxHex $item.ListItemText);$(ConvertTo-VlxHex $item.ToolTip)"
            }
        }
    } finally {
        $global:LASTEXITCODE = $savedExitCode
        Send-VlxCompletion 'Z'
    }
}
Set-PSReadLineKeyHandler -Chord Ctrl+F11 -ScriptBlock {
    $parts = [IO.File]::ReadAllText($script:VlxCompletionSelection).Trim().Split(' ')
    if ($parts.Count -ne 2 -or $parts[0] -ne "$script:VlxCompletionRevision" -or $parts[1] -notmatch '^\d+$') { return }
    $index = [int]$parts[1]
    if ($index -ge $script:VlxCompletionMatches.Count) { return }
    $line = ''; $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
    if ($line -cne $script:VlxCompletionBuffer -or $cursor -ne $script:VlxCompletionCursor) { return }
    [Microsoft.PowerShell.PSConsoleReadLine]::Replace($script:VlxCompletionStart, $script:VlxCompletionLength, $script:VlxCompletionMatches[$index].CompletionText)
    $script:VlxCompletionMatches = @()
}
# Cover all submit bindings, including custom handlers, without replacing Enter's behavior.
$script:VlxOriginalReadLine = $function:global:PSConsoleHostReadLine
function global:PSConsoleHostReadLine {
    $line = & $script:VlxOriginalReadLine
    Send-VlxCompletion 'X'
    $line
}
