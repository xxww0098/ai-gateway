import { existsSync, mkdirSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const root = dirname(dirname(fileURLToPath(import.meta.url)))
const require = createRequire(import.meta.url)
const tsc = join(dirname(require.resolve('typescript/package.json')), 'bin/tsc')
if (!existsSync(tsc)) { console.error('prepare: typescript is not installed'); process.exit(1) }
mkdirSync(join(root, 'lib'), { recursive: true })
const result = spawnSync(process.execPath, [tsc, '-p', 'tsconfig.prepare.json'], { cwd: root, stdio: 'inherit' })
if (result.error) { console.error(result.error.message); process.exit(1) }
if ((result.status ?? 1) !== 0) process.exit(result.status ?? 1)
const bundle = spawnSync(process.execPath, [join(root, 'scripts/bundle-client.mjs')], { cwd: root, stdio: 'inherit' })
if (bundle.error) { console.error(bundle.error.message); process.exit(1) }
process.exit(bundle.status ?? 1)
