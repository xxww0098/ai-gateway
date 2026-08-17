import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { homedir } from 'node:os';
export function defaultTokenPath() {
    const home = process.env.DSH_HOME?.trim() || join(homedir(), '.dsh');
    return join(home, 'agw-oauth.json');
}
function errCode(error) {
    if (error !== null && typeof error === 'object' && 'code' in error) {
        const code = error.code;
        return typeof code === 'string' ? code : undefined;
    }
    return undefined;
}
function parseRecord(raw) {
    const parsed = JSON.parse(raw);
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed))
        return undefined;
    const origin = typeof parsed.origin === 'string'
        ? parsed.origin.trim()
        : '';
    const apiKey = typeof parsed.api_key === 'string'
        ? parsed.api_key.trim()
        : '';
    if (origin.length === 0)
        return undefined;
    return { origin, apiKey };
}
export class TokenStore {
    path;
    cache;
    constructor(path = defaultTokenPath()) {
        this.path = path;
    }
    /** Logged in only when both origin and api_key exist. */
    async read() {
        if (this.cache !== undefined)
            return this.cache;
        try {
            const record = parseRecord(await readFile(this.path, 'utf8'));
            if (record === undefined || record.apiKey.length === 0)
                return undefined;
            this.cache = record;
            return this.cache;
        }
        catch (error) {
            if (errCode(error) === 'ENOENT')
                return undefined;
            throw error;
        }
    }
    /** Origin from disk even when no api_key (settings field / login). */
    async peekOrigin() {
        if (this.cache !== undefined)
            return this.cache.origin;
        try {
            return parseRecord(await readFile(this.path, 'utf8'))?.origin;
        }
        catch (error) {
            if (errCode(error) === 'ENOENT')
                return undefined;
            throw error;
        }
    }
    /** Persist credentials. apiKey may be omitted so origin-only settings work. */
    async write(token) {
        const origin = token.origin.trim().replace(/\/+$/, '');
        const apiKey = (token.apiKey ?? '').trim();
        if (origin.length === 0)
            throw new Error('origin is required');
        this.cache = apiKey.length > 0 ? { origin, apiKey } : undefined;
        const body = apiKey.length > 0 ? { origin, api_key: apiKey } : { origin };
        await mkdir(dirname(this.path), { recursive: true });
        await writeFile(this.path, `${JSON.stringify(body, null, 2)}\n`);
    }
    /** Save origin; keep an existing api_key so a settings edit does not log out. */
    async writeOrigin(origin) {
        const current = await this.read();
        await this.write({ origin, apiKey: current?.apiKey });
    }
    async clear() {
        this.cache = undefined;
        try {
            await writeFile(this.path, '{}\n');
        }
        catch (error) {
            if (errCode(error) !== 'ENOENT')
                throw error;
        }
    }
}
