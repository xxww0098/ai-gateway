import Schema from '@deepseek-ai/schemastery';
export const Config = Schema.object({
    origin: Schema.string().default(process.env.AGW_ORIGIN ?? ''),
});
export function resolveOrigin(config, stored) {
    const value = (stored ?? config.origin ?? process.env.AGW_ORIGIN ?? '').trim().replace(/\/+$/, '');
    return value;
}
