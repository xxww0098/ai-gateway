import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.strictTypeChecked,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
      parserOptions: {
        project: ['./tsconfig.node.json', './tsconfig.app.json'],
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // The project does not enable React Compiler. These compiler-focused rules
      // flag common dashboard data-loading effects as hard errors, while the
      // regular hooks dependency rules below still protect correctness.
      'react-hooks/immutability': 'off',
      'react-hooks/incompatible-library': 'off',
      'react-hooks/preserve-manual-memoization': 'off',
      'react-hooks/purity': 'off',
      'react-hooks/set-state-in-effect': 'off',

      // --- strictTypeChecked systemic overrides ---
      // The API client layer returns `any` by design; typing it requires a
      // dedicated API-layer refactor tracked separately.
      '@typescript-eslint/no-unsafe-assignment': 'warn',
      '@typescript-eslint/no-unsafe-argument': 'warn',
      '@typescript-eslint/no-unsafe-member-access': 'warn',
      '@typescript-eslint/no-unsafe-return': 'warn',
      '@typescript-eslint/no-unsafe-call': 'warn',

      // shadcn/ui components use deprecated `ElementRef`; upstream issue.
      '@typescript-eslint/no-deprecated': 'warn',

      // Common React patterns: void expressions in callbacks, floating promises
      // in useEffect, and promise-returning event handlers are idiomatic.
      '@typescript-eslint/no-confusing-void-expression': 'warn',
      '@typescript-eslint/no-floating-promises': 'warn',
      '@typescript-eslint/no-misused-promises': 'warn',

      // Numbers in template literals are safe and readable.
      '@typescript-eslint/restrict-template-expressions': [
        'error',
        { allowNumber: true },
      ],

      // Defensive nullish checks are intentional in dynamic UI code.
      '@typescript-eslint/no-unnecessary-condition': 'warn',

      // Additional strictTypeChecked rules set to warn for existing codebase:
      // These catch real issues but are pervasive in the current code.
      '@typescript-eslint/no-unnecessary-type-conversion': 'warn',
      '@typescript-eslint/require-await': 'warn',
      '@typescript-eslint/no-base-to-string': 'warn',
      '@typescript-eslint/no-non-null-assertion': 'warn',
      '@typescript-eslint/no-unnecessary-type-assertion': 'warn',
      '@typescript-eslint/no-unnecessary-template-expression': 'warn',
      '@typescript-eslint/no-dynamic-delete': 'warn',
      '@typescript-eslint/use-unknown-in-catch-callback-variable': 'warn',
    },
  },
  {
    files: ['src/shared/components/ui/**/*.{ts,tsx}'],
    rules: {
      'react-refresh/only-export-components': 'off',
    },
  },
  {
    files: [
      'src/features/admin-users/**/*.{ts,tsx}',
      'src/pages/admin/users/**/*.{ts,tsx}',
    ],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: '@/features/admin-proxy/api',
              message: 'Admin user management is CPA account code and must not call SDK management APIs.',
            },
          ],
          patterns: [
            {
              group: ['**/features/admin-proxy/api'],
              message: 'Admin user management is CPA account code and must not call SDK management APIs.',
            },
          ],
        },
      ],
    },
  },
])
