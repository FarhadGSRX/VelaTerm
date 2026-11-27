// Records an existing VelaTerm Job in ~/.config/f-co/jobs.json, so the F-Co launcher can resume it in herdr and the
// waker can wake it. The entry takes the like-role's argv from launch.json, with its role page swapped for the Job
// page, and is written by spawn.mjs's own writeJob. Prints the entry; writes it only with --write.
//
//   node add-job.mjs <slug> --like <role> --cwd <dir> --space <herdr space> [--write]
import { readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

const args = process.argv.slice(2)
const opt = (name) => {
  const i = args.indexOf(`--${name}`)
  return i < 0 ? undefined : args[i + 1]
}
const slug = args[0]
const [like, cwd, space] = [opt('like'), opt('cwd'), opt('space')]
if (!slug || slug.startsWith('--') || !like || !cwd || !space) {
  console.error('usage: node add-job.mjs <slug> --like <role> --cwd <dir> --space <herdr space> [--write]')
  process.exit(2)
}

const home = homedir().replaceAll('\\', '/')
const vault = (process.env.FG_VAULT_ROOT || join(homedir(), '_fg3')).replaceAll('\\', '/')
const spec = JSON.parse(readFileSync(`${home}/.config/f-co/launch.json`, 'utf8'))
if (spec.roles?.[slug]) throw new Error(`${slug} is a role in launch.json, not a Job`)
const role = spec.roles?.[like]
if (!role) throw new Error(`no role ${like} in launch.json`)

const page = role.argv.findIndex((a) => /\/Roles\/[^/]+\.md$/.test(a))
if (page < 0) throw new Error(`${like}'s argv has no Roles page to swap`)
const argv = role.argv.map((a, i) => (i === page ? a.replace(/\/Roles\/[^/]+\.md$/, '/Roles/Job.md') : a))
const job = { cwd: cwd.replaceAll('\\', '/'), argv, like, labels: { 'herdr.space': space, 'herdr.tab': slug } }
console.log(JSON.stringify({ [slug]: job }, null, 2))

if (args.includes('--write')) {
  const { writeJob } = await import(pathToFileURL(`${vault}/Code/Fg-CC-Harness/plugins/f-co/scripts/spawn.mjs`).href)
  writeJob(`${home}/.config/f-co/jobs.json`, slug, job)
  console.log(`written to ${home}/.config/f-co/jobs.json`)
}
