// Only stage verified, complete native installer sets. Never publish partial releases.
import { readdir, readFile, mkdir, copyFile, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { createHash } from 'node:crypto'
import { pathToFileURL } from 'node:url'
export async function prepareRelease(input, output, tag, version) {
  if (tag !== `v${version}` || !/^\d+\.\d+\.\d+$/.test(version))
    throw Error('Tag must match stable package version')
  const targets = new Map([
    ['aarch64-apple-darwin', '.dmg'],
    ['x86_64-apple-darwin', '.dmg'],
    ['x86_64-pc-windows-msvc', '.exe'],
  ])
  const checked = []
  const entries = await readdir(input, { withFileTypes: true })
  if (entries.length !== 3 || entries.some((e) => !e.isDirectory()))
    throw Error('Expected exactly three artifact directories')
  for (const entry of entries) {
    const dir = join(input, entry.name)
    const m = JSON.parse(await readFile(join(dir, 'manifest.json'), 'utf8'))
    const extension = targets.get(m.target)
    if (!extension || m.version !== version) throw Error('Unexpected or duplicate target/version')
    const name = `AgentDock_${version}_${m.target}${extension}`
    if (m.filename !== name) throw Error('Invalid installer filename')
    const bytes = await readFile(join(dir, name))
    const sha = createHash('sha256').update(bytes).digest('hex')
    if (
      !bytes.length ||
      bytes.length > 512 * 1024 * 1024 ||
      bytes.length !== m.bytes ||
      sha !== m.sha256
    )
      throw Error('Installer integrity failure')
    targets.delete(m.target)
    checked.push({ dir, name, sha })
  }
  if (targets.size) throw Error('Missing native target')
  await mkdir(output, { recursive: true })
  if ((await readdir(output)).length) throw Error('Release staging directory must be empty')
  for (const { dir, name, sha } of checked) {
    await copyFile(join(dir, name), join(output, name))
    await writeFile(join(output, `${name}.sha256`), `${sha}  ${name}\n`)
  }
  await writeFile(
    join(output, 'SHA256SUMS.txt'),
    checked.map((x) => `${x.sha}  ${x.name}\n`).join('')
  )
  return checked
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const version = JSON.parse(await readFile('package.json', 'utf8')).version
  await prepareRelease(
    process.argv[2] || 'release-artifacts',
    process.argv[3] || 'release-files',
    process.env.RELEASE_TAG,
    version
  )
}
