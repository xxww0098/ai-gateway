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
export class TokenStore {
    path;
    cache;
    constructor(path = defaultTokenPath()) {
        this.path = path;
    }
    async read() {
        if (this.cache !== undefined)
            return this.cache;
        try {
            const raw = await readFile(this.path, 'utf8');
            const parsed = JSON.parse(raw);
            if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed))
                return undefined;
            const origin = typeof parsed.origin === 'string'
                ? parsed.origin.trim()
                : '';
            const apiKey = typeof parsed.api_key === 'string'
                ? parsed.api_key.trim()
                : '';
            if (origin.length === 0 || apiKey.length === 0)
                return undefined;
            this.cache = { origin, apiKey };
            return this.cache;
        }
        catch (error) {
            if (errCode(error) === 'ENOENT')
                return undefined;
            throw error;
        }
    }
    async write(token) {
        this.cache = token;
        await mkdir(dirname(this.path), { recursive: true });
        await writeFile(this.path, `${JSON.stringify({ origin: token.origin, api_key: token.apiKey }, null, 2)}\n`);
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
