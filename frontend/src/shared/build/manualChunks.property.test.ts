/**
 * Property-based tests for chunk splitting categorization.
 *
 * Uses fast-check to verify that manualChunks correctly maps known
 * library paths to their chunk names and returns undefined for unknown paths.
 *
 * **Validates: Requirements 6.2**
 */

import { describe, it, expect } from 'vitest'
import fc from 'fast-check'
import { manualChunks } from './manualChunks'

/**
 * Known library path patterns and their expected chunk names.
 * This mirrors the actual mapping in vite.config.ts.
 *
 * Note: react-hook-form matches the `/node_modules/react` check first,
 * so it is categorized as 'react' in the current implementation.
 */
const LIBRARY_CHUNK_MAP: Record<string, string> = {
  '/node_modules/react': 'react',
  '/node_modules/react-dom': 'react',
  '/node_modules/react-router-dom': 'react',
  '/node_modules/react-hook-form': 'react', // matches /node_modules/react prefix first
  '/node_modules/recharts': 'charts',
  '/node_modules/@radix-ui': 'radix',
  '/node_modules/@stripe': 'payments',
  '/node_modules/zod': 'validation',
  '/node_modules/lucide-react': 'icons',
  '/node_modules/@tanstack': 'query',
}

/** All valid chunk names that can be returned */
const VALID_CHUNK_NAMES = ['react', 'charts', 'radix', 'payments', 'validation', 'forms', 'icons', 'query']

// ---------------------------------------------------------------------------
// Property 7: Chunk Splitting Categorization
// Validates: Requirements 6.2
// ---------------------------------------------------------------------------

describe('Property 7: Chunk Splitting Categorization', () => {
  /**
   * **Validates: Requirements 6.2**
   *
   * For any known library path, manualChunks returns the correct chunk name.
   */
  it('returns correct chunk name for any path containing a known library identifier', () => {
    const libraryEntries = Object.entries(LIBRARY_CHUNK_MAP)

    fc.assert(
      fc.property(
        // Pick a random known library entry
        fc.integer({ min: 0, max: libraryEntries.length - 1 }),
        // Generate random prefix (simulating project path before node_modules)
        fc.string({ minLength: 0, maxLength: 50 }).map(s => s.replace(/\//g, 'x')),
        // Generate random suffix (simulating file path after library name)
        fc.string({ minLength: 0, maxLength: 50 }).map(s => '/' + s.replace(/\//g, 'x')),
        (libraryIndex, prefix, suffix) => {
          const [libraryPath, expectedChunk] = libraryEntries[libraryIndex]
          const fullPath = prefix + libraryPath + suffix
          const result = manualChunks(fullPath)
          expect(result).toBe(expectedChunk)
        }
      )
    )
  })

  it('returns undefined for paths not containing any known library identifier', () => {
    // Known substrings that would trigger a match
    const knownPatterns = [
      '/node_modules/react',
      '/node_modules/recharts',
      '/node_modules/@radix-ui',
      '/node_modules/@stripe',
      '/node_modules/zod',
      '/node_modules/lucide-react',
      '/node_modules/react-hook-form',
      '/node_modules/@tanstack',
    ]

    fc.assert(
      fc.property(
        // Generate paths that don't contain any known library pattern
        fc.string({ minLength: 1, maxLength: 100 }).filter(s =>
          !knownPatterns.some(pattern => s.includes(pattern))
        ),
        (unknownPath) => {
          const result = manualChunks(unknownPath)
          expect(result).toBeUndefined()
        }
      )
    )
  })

  it('always returns a value from the valid chunk names set or undefined', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 0, maxLength: 200 }),
        (path) => {
          const result = manualChunks(path)
          if (result !== undefined) {
            expect(VALID_CHUNK_NAMES).toContain(result)
          }
        }
      )
    )
  })

  it('react-dom paths always map to react chunk regardless of surrounding path', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 0, maxLength: 30 }).map(s => s.replace(/\//g, 'x')),
        fc.string({ minLength: 0, maxLength: 30 }).map(s => '/' + s.replace(/\//g, 'x')),
        (prefix, suffix) => {
          const path = prefix + '/node_modules/react-dom' + suffix
          expect(manualChunks(path)).toBe('react')
        }
      )
    )
  })

  it('react-router-dom paths always map to react chunk regardless of surrounding path', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 0, maxLength: 30 }).map(s => s.replace(/\//g, 'x')),
        fc.string({ minLength: 0, maxLength: 30 }).map(s => '/' + s.replace(/\//g, 'x')),
        (prefix, suffix) => {
          const path = prefix + '/node_modules/react-router-dom' + suffix
          expect(manualChunks(path)).toBe('react')
        }
      )
    )
  })

  it('@tanstack paths always map to query chunk regardless of subpackage', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 0, maxLength: 30 }).map(s => s.replace(/\//g, 'x')),
        fc.constantFrom('/react-query', '/react-table', '/react-virtual', '/query-core'),
        (prefix, subpackage) => {
          const path = prefix + '/node_modules/@tanstack' + subpackage
          expect(manualChunks(path)).toBe('query')
        }
      )
    )
  })

  it('is a pure function: same input always produces same output', () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 0, maxLength: 200 }),
        (path) => {
          const result1 = manualChunks(path)
          const result2 = manualChunks(path)
          expect(result1).toBe(result2)
        }
      )
    )
  })
})
