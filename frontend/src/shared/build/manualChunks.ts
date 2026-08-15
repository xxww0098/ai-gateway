/**
 * Manual chunk splitting logic for Vite's rollup configuration.
 *
 * Maps known library paths to named chunks for optimal caching.
 * Extracted from vite.config.ts for testability.
 */

/**
 * Determines the chunk name for a given module ID.
 *
 * @param id - The full file path of the module being bundled
 * @returns The chunk name if the module matches a known library, undefined otherwise
 */
export function manualChunks(id: string): string | undefined {
  if (id.includes('/node_modules/react') || id.includes('/node_modules/react-dom') || id.includes('/node_modules/react-router-dom')) {
    return 'react'
  }
  if (id.includes('/node_modules/recharts')) {
    return 'charts'
  }
  if (id.includes('/node_modules/@radix-ui')) {
    return 'radix'
  }
  if (id.includes('/node_modules/@stripe')) {
    return 'payments'
  }
  if (id.includes('/node_modules/zod')) {
    return 'validation'
  }
  if (id.includes('/node_modules/react-hook-form')) {
    return 'forms'
  }
  if (id.includes('/node_modules/lucide-react')) {
    return 'icons'
  }
  if (id.includes('/node_modules/@tanstack')) {
    return 'query'
  }
  return undefined
}
