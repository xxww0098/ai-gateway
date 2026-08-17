/** Device-code client for AI-GateWay /api/panel/oauth/dsh/*. */
export class GatewayOAuthError extends Error {
    code;
    constructor(message, code = 'OAUTH') {
        super(message);
        this.code = code;
        this.name = 'GatewayOAuthError';
    }
}
function envelopeData(json) {
    if (json === null || typeof json !== 'object' || Array.isArray(json)) {
        throw new GatewayOAuthError('AI-GateWay returned a non-object body');
    }
    const body = json;
    if (body.code !== 0 && body.code !== undefined) {
        throw new GatewayOAuthError(typeof body.message === 'string' ? body.message : 'AI-GateWay error');
    }
    if (body.data === null || typeof body.data !== 'object' || Array.isArray(body.data)) {
        throw new GatewayOAuthError('AI-GateWay response missing data');
    }
    return body.data;
}
async function postJson(url, body, signal) {
    const response = await fetch(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
        ...signal === undefined ? {} : { signal },
    });
    let json;
    try {
        json = await response.json();
    }
    catch {
        throw new GatewayOAuthError(`AI-GateWay HTTP ${response.status}`);
    }
    if (!response.ok) {
        const message = json !== null && typeof json === 'object' && 'message' in json
            && typeof json.message === 'string'
            ? json.message
            : `AI-GateWay HTTP ${response.status}`;
        throw new GatewayOAuthError(message);
    }
    return json;
}
export function normalizeOrigin(origin) {
    return origin.trim().replace(/\/+$/, '');
}
/** Start a device-code session. Returns as soon as the user has a URL/code. */
export async function startDevice(origin, signal) {
    const base = normalizeOrigin(origin);
    const json = await postJson(`${base}/api/panel/oauth/dsh/device/code`, { origin: base }, signal);
    const data = envelopeData(json);
    const deviceCode = String(data.device_code ?? '');
    const userCode = String(data.user_code ?? '');
    const verificationUri = String(data.verification_uri ?? '');
    if (deviceCode.length === 0 || userCode.length === 0 || verificationUri.length === 0) {
        throw new GatewayOAuthError('AI-GateWay device start missing user_code or URL');
    }
    const complete = typeof data.verification_uri_complete === 'string' && data.verification_uri_complete.length > 0
        ? data.verification_uri_complete
        : `${verificationUri}?user_code=${userCode}`;
    const expiresIn = typeof data.expires_in === 'number' && data.expires_in > 0 ? data.expires_in : 600;
    const interval = typeof data.interval === 'number' && data.interval > 0 ? data.interval : 2;
    return { deviceCode, userCode, verificationUri, verificationUriComplete: complete, expiresIn, interval };
}
/** One poll. Does not sleep. */
export async function pollDevice(origin, deviceCode, signal) {
    const base = normalizeOrigin(origin);
    const json = await postJson(`${base}/api/panel/oauth/dsh/device/token`, { device_code: deviceCode }, signal);
    const data = envelopeData(json);
    const status = String(data.status ?? '');
    if (status === 'approved') {
        const apiKey = String(data.api_key ?? '');
        const resolvedOrigin = String(data.origin ?? base);
        if (apiKey.length === 0)
            throw new GatewayOAuthError('approved poll missing api_key');
        return { status: 'approved', apiKey, origin: normalizeOrigin(resolvedOrigin) };
    }
    if (status === 'denied' || status === 'expired' || status === 'pending') {
        return { status };
    }
    throw new GatewayOAuthError(`unexpected poll status: ${status}`);
}
export async function sleep(ms, signal) {
    if (signal?.aborted)
        throw new GatewayOAuthError('aborted', 'ABORTED');
    await new Promise((resolve, reject) => {
        const timer = setTimeout(resolve, ms);
        const onAbort = () => {
            clearTimeout(timer);
            reject(new GatewayOAuthError('aborted', 'ABORTED'));
        };
        signal?.addEventListener('abort', onAbort, { once: true });
    });
}
/** Poll until approved / denied / expired. Used by background login. */
export async function waitForApproval(origin, start, signal) {
    const deadline = Date.now() + start.expiresIn * 1000;
    while (Date.now() < deadline) {
        const result = await pollDevice(origin, start.deviceCode, signal);
        if (result.status === 'approved')
            return result;
        if (result.status === 'denied')
            throw new GatewayOAuthError('authorization denied');
        if (result.status === 'expired')
            throw new GatewayOAuthError('authorization expired');
        await sleep(start.interval * 1000, signal);
    }
    throw new GatewayOAuthError('authorization expired');
}
