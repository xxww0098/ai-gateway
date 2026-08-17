import { AgwSection } from './AgwSection.tsx'
import { en, zh, NS } from './locales.ts'

export const inject = ['slots', 'locale']

export function apply(ctx: {
  effect(fn: () => (() => void) | void, name?: string): void
  locale: {
    register(ns: string, dicts: { zh: unknown, en: unknown }): () => void
    bind(ns: string): (key: string) => string
  }
  slots: {
    inject(name: string, fn: () => unknown): void
    register(entry: {
      name: string
      id: string
      order: number
      label: () => string
      inject: () => { t: (key: string) => string }
    }, component: unknown): unknown
  }
}): void {
  ctx.effect(() => ctx.locale.register(NS, { zh, en }), 'agw-oauth: locales')
  const t = ctx.locale.bind(NS)
  const inject = () => ({ t })
  ctx.slots.inject('settings.section', () => ctx.slots.register({
    name: 'settings.section',
    id: 'agw-oauth',
    order: 50,
    label: () => t('nav'),
    inject,
  }, AgwSection))
}
