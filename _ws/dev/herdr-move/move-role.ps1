# Steps 4, 5 and 10a of moving one VelaTerm role to herdr: launch it through the F-Co launcher in this machine's
# herdr core, wait, report its state, labels and registry entry, then name its tab (every role) and its space (only
# when this launch created it). Run it only after the role is archived in VelaTerm and nothing holds its
# conversation (precheck.py, wait-archived.py).
#
#   .\move-role.ps1 -Slug um-mgr -Conversation <id> -Cwd <launch --print cwd> -Zulip cx1/um-mgr -SpaceName 3-Areas
#
# -Private: never read its screen, and print nothing of `hc get` beyond state, status and labels (its `title` is
# Claude's conversation title).
param(
    [Parameter(Mandatory)] [string]$Slug,
    [Parameter(Mandatory)] [string]$Conversation,
    [Parameter(Mandatory)] [string]$Cwd,
    [Parameter(Mandatory)] [string]$Zulip,
    [string]$SpaceName,
    [int]$Wait = 25,
    [switch]$Private
)
$hc = Join-Path $env:LOCALAPPDATA 'Programs\herdr-core\hc.exe'
$app = Join-Path $env:LOCALAPPDATA 'Programs\herdr-app\herdr-app.exe'
$vault = if ($env:FG_VAULT_ROOT) { $env:FG_VAULT_ROOT } else { Join-Path $HOME '_fg3' }
$launch = Join-Path $vault 'Code\Fg-CC-Harness\plugins\f-co\scripts\launch.mjs'

$before = (& $app ls | ConvertFrom-Json).spaces.id
$hcArgs = @('run', '--bg', '--no-parent', '--name', $Slug, '--cwd', $Cwd, '--', 'node', $launch, $Slug, '--resume', $Conversation)
"launch: " + (& $hc @hcArgs 2>&1 | Out-String).Trim()
Start-Sleep -Seconds $Wait

# Keep only the fields used, in the same pipeline, and drop errors (a parse error could echo the title).
$g = & $hc get $Slug --json 2>$null | Out-String | ConvertFrom-Json -ErrorAction SilentlyContinue |
    Select-Object state, status, labels
"state: $($g.state) status=$($g.status | ConvertTo-Json -Compress) labels=$($g.labels | ConvertTo-Json -Compress)"

$space = $g.labels.'herdr.space'
& $app do tab.rename --id $Slug --name $Slug | Out-Null
"tab renamed: $Slug"
if ($SpaceName -and $space -and ($before -notcontains $space)) {
    & $app do space.rename --id $space --name $SpaceName | Out-Null
    "space $space was new: renamed to $SpaceName"
} elseif ($space) { "space $space already existed: left as is" }

$r = (Get-Content (Join-Path $HOME '.zulip-mesh\sessions.json') -Raw | ConvertFrom-Json).by_name.$Zulip
"registry $Zulip`: host=$($r.host) hc_terminal=$($r.hc_terminal) velaterm_id=$($r.velaterm_id) conv=$($r.conversation_id)"
if ($Private) { "screen: not read (Private role)"; return }
"screen tail:"
& $hc read $Slug 2>&1 | Select-Object -Last 4
