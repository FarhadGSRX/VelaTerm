// Steps 4, 5 and 10a of moving one VelaTerm role to herdr, on Windows, macOS or Linux: launch it through the F-Co
// launcher in this machine's herdr core, wait, report its state, labels and registry entry, then name its tab (every
// role) and its space (only when this launch created it). Run it only after the role is archived in VelaTerm and
// nothing holds its conversation (precheck.py, wait-archived.py). The same steps as move-role.ps1.
//
//   node move-role.mjs --slug <s> --conversation <id> --cwd <launch_cwd> --zulip <machine/s> [--space-name <n>]
//                      [--wait 25] [--private]
//
// --private: never read its screen, and print nothing of `hc get` beyond state, status and labels (its `title` is
// Claude's conversation title). HC_BIN and HERDR_APP_BIN override where hc and the app's CLI are.
import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

const args = process.argv.slice(2)
const opt = (name) => {
  const i = args.indexOf(`--${name}`)
  return i < 0 ? undefined : args[i + 1]
}
const [slug, conversation, cwd, zulip] = ['slug', 'conversation', 'cwd', 'zulip'].map(opt)
if (!slug || !conversation || !cwd || !zulip) {
  console.error('usage: node move-role.mjs --slug <s> --conversation <id> --cwd <dir> --zulip <machine/s> [--space-name <n>] [--wait 25] [--private]')
  process.exit(2)
}
const spaceName = opt('space-name')
const wait = Number(opt('wait') ?? 25)
const isPrivate = args.includes('--private')

const win = process.platform === 'win32'
const local = process.env.LOCALAPPDATA ?? ''
// herdr-core's install.sh puts `hc` in ~/.local/bin on macOS and Linux.
const unixHc = join(homedir(), '.local', 'bin', 'hc')
const hc = process.env.HC_BIN || (win ? join(local, 'Programs', 'herdr-core', 'hc.exe') : existsSync(unixHc) ? unixHc : 'hc')
const app = process.env.HERDR_APP_BIN || (win ? join(local, 'Programs', 'herdr-app', 'herdr-app.exe') : 'herdr-app')
const vault = process.env.FG_VAULT_ROOT || join(homedir(), '_fg3')
const launch = join(vault, 'Code', 'Fg-CC-Harness', 'plugins', 'f-co', 'scripts', 'launch.mjs')

const run = (bin, argv) => {
  const r = spawnSync(bin, argv, { encoding: 'utf8', windowsHide: true })
  if (r.error) throw new Error(`${bin}: ${r.error.message}`)
  return { code: r.status, out: (r.stdout ?? '').trim(), err: (r.stderr ?? '').trim() }
}
const json = (text) => {
  try { return JSON.parse(text) } catch { return undefined }
}

// A headless machine (the Mac: herdr-core only) has no app CLI; 10a is then done from the app that shows it.
const listed = spawnSync(app, ['ls'], { encoding: 'utf8', windowsHide: true })
const hasApp = !listed.error
const before = new Set(hasApp ? (json(listed.stdout)?.spaces ?? []).map((s) => s.id) : [])
const started = run(hc, ['run', '--bg', '--no-parent', '--name', slug, '--cwd', cwd, '--', 'node', launch, slug, '--resume', conversation])
console.log(`launch: ${started.out || started.err}`)
await new Promise((done) => setTimeout(done, wait * 1000))

// Keep only the fields used; a parse failure prints nothing of the output (it could hold the title).
const got = json(run(hc, ['get', slug, '--json']).out) ?? {}
console.log(`state: ${got.state} status=${JSON.stringify(got.status)} labels=${JSON.stringify(got.labels)}`)

const space = got.labels?.['herdr.space']
if (!hasApp) {
  console.log(`no herdr app here (${app}): 10a is done from the app that shows this machine`)
} else {
  run(app, ['do', 'tab.rename', '--id', slug, '--name', slug])
  console.log(`tab renamed: ${slug}`)
  if (spaceName && space && !before.has(space)) {
    run(app, ['do', 'space.rename', '--id', space, '--name', spaceName])
    console.log(`space ${space} was new: renamed to ${spaceName}`)
  } else if (space) console.log(`space ${space} already existed: left as is`)
}

const registry = json(readFileSync(join(homedir(), '.zulip-mesh', 'sessions.json'), 'utf8'))
const r = registry?.by_name?.[zulip] ?? {}
console.log(`registry ${zulip}: host=${r.host} hc_terminal=${r.hc_terminal} velaterm_id=${r.velaterm_id ?? ''} conv=${r.conversation_id}`)
if (isPrivate) {
  console.log('screen: not read (Private role)')
} else {
  console.log('screen tail:')
  console.log(run(hc, ['read', slug]).out.split('\n').slice(-4).join('\n'))
}
