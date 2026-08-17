import Schema from '@deepseek-ai/schemastery'

export interface Config {
  origin: string
}

export const Config: Schema<Config> = Schema.object({
  origin: Schema.string().default(process.env.AGW_ORIGIN ?? ''),
})

export function resolveOrigin(config: Config, stored?: string): string {
  const value = (stored ?? config.origin ?? process.env.AGW_ORIGIN ?? '').trim().replace(/\/+$/, '')
  return value
}
