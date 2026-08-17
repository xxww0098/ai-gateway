import { existsSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = dirname(dirname(fileURLToPath(import.meta.url)))
const require = createRequire(import.meta.url)

function resolveEsbuildBin() {
  const candidates = [
    process.env.ESBUILD_BINARY_PATH,
    join(root, 'node_modules/@esbuild/linux-x64/bin/esbuild'),
    join(root, 'node_modules/esbuild/bin/esbuild'),
  ]
  try {
    candidates.push(join(dirname(require.resolve('esbuild/package.json')), 'bin/esbuild'))
  } catch {
    // esbuild may be provided as a platform binary only
  }
  for (const candidate of candidates) {
    if (typeof candidate === 'string' && existsSync(candidate)) return candidate
  }
  console.error('prepare: esbuild is not installed (needed to bundle lib/client.js)')
  process.exit(1)
}

const PACKAGE_NAME = 'dsh-agw-oauth'
const banner = `window.__ModuleLoader__.load({ id: ${JSON.stringify(PACKAGE_NAME)}, factory: (require) => {\nvar module = { exports: {} }; var exports = module.exports;`
const footer = 'return module.exports; } });'
const bin = resolveEsbuildBin()
const result = spawnSync(bin, [
  'src/client/index.ts',
  '--bundle',
  '--format=cjs',
  '--platform=browser',
  '--target=es2022',
  '--outfile=lib/client.js',
  '--jsx=automatic',
  '--external:react',
  '--external:react/jsx-runtime',
  '--external:@deepseek-ai/cordis',
  '--external:@deepseek-ai/dsh-client-ui-slots',
  '--external:@deepseek-ai/dsh-client-runtime/client',
  `--banner:js=${banner}`,
  `--footer:js=${footer}`,
], { cwd: root, stdio: 'inherit' })
if (result.error) {
  console.error(result.error.message)
  process.exit(1)
}
process.exit(result.status ?? 1)
