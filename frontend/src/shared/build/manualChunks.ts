/**
 * Manual chunk splitting for Vite / Rolldown.
 *
 * Pin only libraries that already belong on the authenticated shell.
 * Do not pin async-only vendors (recharts, stripe, zod, lucide-react):
 * a named group that depends on React can capture React's CJS/jsx runtime
 * and then Vite modulepreloads that group on the landing HTML.
 */

/**
 * Determines the chunk name for a given module ID.
 *
 * @param id - The full file path of the module being bundled
 * @returns The chunk name if the module matches a known library, undefined otherwise
 */
export function manualChunks(id: string): string | undefined {
  if (
    id.includes('/node_modules/react') ||
    id.includes('/node_modules/react-dom') ||
    id.includes('/node_modules/react-router-dom')
  ) {
    return 'react'
  }
  if (id.includes('/node_modules/@radix-ui')) {
    return 'radix'
  }
  if (id.includes('/node_modules/@tanstack')) {
    return 'query'
  }
  return undefined
}
