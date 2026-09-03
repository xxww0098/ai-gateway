/** Read CLI OAuth files already on this host. No new browser login. */
import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
function homeDir() {
    const override = process.env.AGW_LOCAL_OAUTH_HOME?.trim();
    return override || homedir();
}
export function wellKnownPaths(home = homeDir()) {
    const paths = [
        join(home, '.codex', 'auth.json'),
        join(home, '.claude', '.credentials.json'),
        join(home, '.grok', 'auth.json'),
        join(home, '.hermes', 'auth.json'),
        join(home, '.kiro', 'credentials.json'),
        join(home, '.aws', 'sso', 'cache', 'kiro-auth-token.json'),
    ];
    const claudeDir = process.env.CLAUDE_CONFIG_DIR?.trim();
    if (claudeDir)
        paths.push(join(claudeDir, '.credentials.json'));
    const grokDir = process.env.GROK_HOME?.trim();
    if (grokDir)
        paths.push(join(grokDir, 'auth.json'));
    return paths;
}
function pick(record, keys) {
    for (const key of keys) {
        const value = record[key];
        if (typeof value === 'string' && value.trim())
            return value.trim();
    }
    return '';
}
function fromFlat(record, provider, source) {
    const accessToken = pick(record, ['access_token', 'accessToken', 'key']);
    if (!accessToken)
        return undefined;
    return {
        provider,
        source,
        accessToken,
        refreshToken: pick(record, ['refresh_token', 'refreshToken']),
        idToken: pick(record, ['id_token', 'idToken']),
        expiresAt: pick(record, ['expires_at', 'expiresAt', 'expired']),
        email: pick(record, ['email', 'account']),
    };
}
export function parseCliJson(raw, source) {
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw))
        return [];
    const root = raw;
    const out = [];
    if (root.claudeAiOauth !== null && typeof root.claudeAiOauth === 'object' && !Array.isArray(root.claudeAiOauth)) {
        const cred = fromFlat(root.claudeAiOauth, 'claude', source);
        if (cred)
            out.push(cred);
    }
    const tokens = root.tokens !== null && typeof root.tokens === 'object' && !Array.isArray(root.tokens)
        ? root.tokens
        : undefined;
    if (tokens && typeof tokens.access_token === 'string') {
        const cred = fromFlat(tokens, 'codex', source);
        if (cred)
            out.push(cred);
    }
    const mode = pick(root, ['auth_mode', 'authMode']).toLowerCase();
    if ((mode === 'oidc' || mode === 'oauth' || mode === 'supergrok' || source.includes('.grok')) && out.every(row => row.provider !== 'xai')) {
        const cred = fromFlat(root, 'xai', source);
        if (cred)
            out.push(cred);
    }
    if ((source.includes('kiro') || pick(root, ['start_url', 'startUrl'])) && out.length === 0) {
        const cred = fromFlat(root, 'kiro', source);
        if (cred)
            out.push(cred);
    }
    const providers = (root.providers ?? root.auth);
    if (providers !== null && typeof providers === 'object' && !Array.isArray(providers)) {
        const map = providers;
        for (const [key, value] of Object.entries(map)) {
            if (value === null || typeof value !== 'object' || Array.isArray(value))
                continue;
            const entry = value;
            const nested = entry.tokens !== null && typeof entry.tokens === 'object' && !Array.isArray(entry.tokens)
                ? entry.tokens
                : entry;
            if (/codex|chatgpt/i.test(key) && out.every(row => row.provider !== 'codex')) {
                const cred = fromFlat(nested, 'codex', source);
                if (cred)
                    out.push(cred);
            }
            if (/grok|xai|x-ai/i.test(key) && out.every(row => row.provider !== 'xai')) {
                const cred = fromFlat(nested, 'xai', source);
                if (cred)
                    out.push(cred);
            }
        }
    }
    return out;
}
export async function discoverLocalOauth(home = homeDir()) {
    const found = [];
    const seen = new Set();
    for (const path of wellKnownPaths(home)) {
        if (seen.has(path))
            continue;
        seen.add(path);
        let text;
        try {
            text = await readFile(path, 'utf8');
        }
        catch (error) {
            if (error !== null && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') {
                continue;
            }
            throw error;
        }
        let parsed;
        try {
            parsed = JSON.parse(text);
        }
        catch {
            continue;
        }
        found.push(...parseCliJson(parsed, path));
    }
    return found;
}
export function redact(files) {
    return files.map(file => ({
        provider: file.provider,
        source: file.source,
        expiresAt: file.expiresAt,
        email: file.email,
        hasAccessToken: file.accessToken.length > 0,
        hasRefreshToken: file.refreshToken.length > 0,
    }));
}
export function toUploadJson(file) {
    return {
        provider: file.provider,
        access_token: file.accessToken,
        refresh_token: file.refreshToken,
        id_token: file.idToken,
        expires_at: file.expiresAt,
        email: file.email,
        token_data: {
            access_token: file.accessToken,
            refresh_token: file.refreshToken,
            id_token: file.idToken,
        },
    };
}
/** POST each file to the gateway auth-files inventory (admin JWT or admin agw- key). */
export async function uploadToGateway(origin, apiKey, files, fetchFn = fetch) {
    const boundary = `agw${Date.now()}`;
    const chunks = [];
    for (const file of files) {
        const name = `${file.provider}-local.json`;
        chunks.push(`--${boundary}\r\n`);
        chunks.push(`Content-Disposition: form-data; name="file"; filename="${name}"\r\n`);
        chunks.push('Content-Type: application/json\r\n\r\n');
        chunks.push(`${JSON.stringify(toUploadJson(file), null, 2)}\r\n`);
    }
    chunks.push(`--${boundary}--\r\n`);
    const response = await fetchFn(`${origin.replace(/\/+$/, '')}/api/panel/admin/sdk-management/auth-files`, {
        method: 'POST',
        headers: {
            authorization: `Bearer ${apiKey}`,
            'content-type': `multipart/form-data; boundary=${boundary}`,
        },
        body: chunks.join(''),
    });
    let body;
    try {
        body = await response.json();
    }
    catch {
        body = undefined;
    }
    return { count: files.length, status: response.status, body };
}
export async function importLocalAndMaybeUpload(input) {
    const files = await discoverLocalOauth(input.home);
    const report = { found: redact(files) };
    if (files.length === 0)
        return report;
    if (!input.origin || !input.apiKey)
        return report;
    try {
        report.uploaded = await uploadToGateway(input.origin, input.apiKey, files, input.fetchFn);
    }
    catch (error) {
        report.error = error instanceof Error ? error.message : String(error);
    }
    return report;
}
