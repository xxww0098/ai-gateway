declare module '@deepseek-ai/dsh-llm' {
  export class LlmAdapter {
    providerInfo?(provider: string): unknown
    listModels?(provider: string): Promise<unknown>
    resolveModel?(provider: string, model: string, signal?: AbortSignal): Promise<unknown>
    stream(options: GenerateOptions): AsyncIterable<StreamChunk>
  }
  export class LlmError extends Error {
    constructor(message: string, code: string, extra?: Record<string, unknown>)
  }
  export function attributionHeaders(): Record<string, string>
  export type GenerateOptions = {
    provider: string
    model: string
    reasoningEffort?: string
    messages: Array<{ role: string, content: Array<{ type: string, [k: string]: unknown }> }>
    system?: string
    tools?: unknown[]
    temperature?: number
    maxTokens?: number
    stop?: string[]
    signal?: AbortSignal
  }
  export type StreamChunk = { type: string, [k: string]: unknown }
}

declare module '@deepseek-ai/cordis' {
  export interface Context {
    llm: {
      registerAdapter(providers: string[], adapter: unknown): { replace(providers: string[]): void }
    }
    logger: { info(msg: string): void, warn(msg: string): void, error(msg: string | unknown): void }
    get(name: string): unknown
    inject(deps: string[], fn: (ctx: Context) => void): void
    effect(fn: () => (() => void) | void, name?: string): void
  }
}

declare module '@deepseek-ai/schemastery' {
  type Schema<T> = { [k: string]: unknown }
  interface SchemaFactory {
    object<T>(shape: T): Schema<T>
    string(): { default(value: string): unknown }
  }
  const Schema: SchemaFactory
  export default Schema
}
declare const process: { env: Record<string, string | undefined>, exit(code: number): never }
declare module "node:fs/promises" { export function mkdir(path: string, opts?: { recursive?: boolean }): Promise<void>; export function readFile(path: string, enc: string): Promise<string>; export function writeFile(path: string, data: string): Promise<void> }
declare module "node:path" { export function dirname(p: string): string; export function join(...p: string[]): string }
declare module "node:os" { export function homedir(): string }
declare class URL { constructor(input: string); protocol: string }
