// Copy the one installer a CI job produced into ci-packages/ with a stable
// name, its SHA256 and a manifest describing what CI did (and did not) verify.
import { readdir, readFile, mkdir, copyFile, writeFile } from 'node:fs/promises'
import { join, basename } from 'node:path'
import { createHash } from 'node:crypto'

const [target, bundle] = process.argv.slice(2)
const targets = {
  'x86_64-pc-windows-msvc': 'nsis',
  'aarch64-apple-darwin': 'dmg',
  'x86_64-apple-darwin': 'dmg',
}
if (targets[target] !== bundle) throw Error(`Unsupported target/bundle: ${target} ${bundle}`)
const root = join('src-tauri', 'target', target, 'release', 'bundle', bundle)
const extension = bundle === 'nsis' ? '.exe' : '.dmg'
const files = (await readdir(root)).filter((name) => name.endsWith(extension))
if (files.length !== 1) throw Error(`Expected one installer in ${root}, found ${files.length}`)
const version = JSON.parse(await readFile('package.json', 'utf8')).version
const output = 'ci-packages'
await mkdir(output, { recursive: true })
const name = `AgentDock_${version}_${target}${extension}`
const source = join(root, files[0])
const bytes = await readFile(source)
const sha256 = createHash('sha256').update(bytes).digest('hex')
await copyFile(source, join(output, name))
await writeFile(join(output, 'SHA256SUMS.txt'), `${sha256}  ${name}\n`)
await writeFile(
  join(output, 'manifest.json'),
  JSON.stringify(
    {
      version,
      target,
      filename: name,
      bytes: bytes.length,
      sha256,
      sourceInstaller: basename(source),
      commit: process.env.GITHUB_SHA || null,
      run: process.env.GITHUB_RUN_ID || null,
      signedByDeveloper: false,
      notarized: false,
      validationScope: 'Native CI compilation, offline tests and installer creation; no real client, account or provider service was used',
    },
    null,
    2
  ) + '\n'
)
await writeFile(
  join(output, 'README.txt'),
  `AgentDock ${version} — ${target}
The installer contains no accounts, keys or official SDKs. WorkBuddy / Qoder / Trae
features use the matching desktop client installed on this machine.
Windows needs WebView2 (installed automatically when missing).
These CI packages are not code-signed or notarized; follow your OS security prompts.
`
)
console.log(JSON.stringify({ filename: name, bytes: bytes.length, sha256 }))
