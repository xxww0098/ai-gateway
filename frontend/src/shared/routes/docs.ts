/**
 * Public access-guide paths. These live outside UserLayout (no login required).
 */
export const docsRoutes = {
  root: '/docs',
  overview: '/docs',
  quickstart: '/docs/quickstart',
  openai: '/docs/openai',
  claude: '/docs/claude',
  codex: '/docs/codex',
  cursor: '/docs/cursor',
} as const

export type DocsRoutePath = (typeof docsRoutes)[keyof typeof docsRoutes]

export const docsSlugs = ['quickstart', 'openai', 'claude', 'codex', 'cursor'] as const

export type DocsSlug = (typeof docsSlugs)[number]

export function isDocsSlug(value: string | undefined): value is DocsSlug {
  return value !== undefined && (docsSlugs as readonly string[]).includes(value)
}

export function docsPath(slug?: DocsSlug): string {
  return slug ? `${docsRoutes.root}/${slug}` : docsRoutes.root
}
