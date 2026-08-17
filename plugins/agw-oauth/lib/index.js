/**
 * AGW-Oauth: OAuth into AI-GateWay from DeepSeek Harness.
 * Function plugin — export name, inject, Config, apply. No export default.
 */
import { AgwAdapter, PROVIDER } from './adapter.js';
import { parseModelsPayload } from './catalog.js';
import { Config, resolveOrigin } from './config.js';
import { currentWatch, resetLoginWatch, startLogin, usageText } from './command.js';
import { handleHttp } from './http.js';
import { TokenStore } from './store.js';
export const name = 'agw-oauth';
export const inject = ['llm'];
export { Config };
export { PROVIDER, parseModelsPayload, parseGatewayModel, toResolvedModel } from './catalog.js';
export { startDevice, pollDevice } from './oauth.js';
export { AgwAdapter } from './adapter.js';
export { handleHttp } from './http.js';
export function apply(ctx, config) {
    ctx.logger.info('[agw-oauth] plugin loaded!');
    const store = new TokenStore();
    let models = [];
    let registration;
    let savedOrigin;
    const auth = () => {
        // filled after first read; sync snapshot for the adapter
        return snapshot;
    };
    let snapshot;
    const adapter = new AgwAdapter(auth, () => models);
    const ensureAdapter = () => {
        if (snapshot === undefined) {
            registration?.replace([]);
            return;
        }
        if (registration === undefined) {
            registration = ctx.llm.registerAdapter([PROVIDER], adapter);
        }
        else {
            registration.replace([PROVIDER]);
        }
    };
    const refreshModels = async () => {
        if (snapshot === undefined) {
            models = [];
            return;
        }
        const response = await fetch(`${snapshot.origin}/v1/models`, {
            headers: { authorization: `Bearer ${snapshot.apiKey}` },
        });
        if (!response.ok) {
            throw new Error(`GET /v1/models failed: HTTP ${response.status}`);
        }
        models = parseModelsPayload(await response.json());
        ensureAdapter();
    };
    const persist = async (apiKey, origin) => {
        snapshot = { apiKey, origin };
        savedOrigin = origin;
        await store.write(snapshot);
        await refreshModels();
    };
    const logout = async () => {
        resetLoginWatch();
        snapshot = undefined;
        savedOrigin = undefined;
        models = [];
        await store.clear();
        ensureAdapter();
    };
    const saveOrigin = async (origin) => {
        await store.writeOrigin(origin);
        savedOrigin = origin;
        if (snapshot !== undefined)
            snapshot = { ...snapshot, origin };
    };
    const loginOrigin = () => resolveOrigin(config, snapshot?.origin ?? savedOrigin);
    void (async () => {
        savedOrigin = await store.peekOrigin();
        const token = await store.read();
        if (token === undefined) {
            ctx.logger.info('[agw-oauth] not logged in; run /agw login or Settings → AGW Oauth');
            return;
        }
        snapshot = token;
        savedOrigin = token.origin;
        try {
            await refreshModels();
            ctx.logger.info(`[agw-oauth] ready origin=${token.origin} models=${models.length}`);
        }
        catch (error) {
            ctx.logger.warn(`[agw-oauth] listed no models: ${error instanceof Error ? error.message : String(error)}`);
            ensureAdapter();
        }
    })();
    const commands = ctx.get('commands');
    if (commands !== undefined) {
        commands.register({
            name: 'agw',
            description: 'AI-GateWay OAuth: status, login, logout',
            input: { hint: '[status|login|logout]' },
            handler: async (invocation) => {
                const action = (invocation.rawInput.trim().split(/\s+/)[0] ?? 'status').toLowerCase();
                if (action === 'help' || action === '-h' || action === '--help') {
                    return { kind: 'success', text: usageText() };
                }
                if (action === 'logout') {
                    await logout();
                    return { kind: 'success', text: 'Logged out of AI-GateWay.' };
                }
                if (action === 'login') {
                    return startLogin(loginOrigin(), persist, invocation.signal);
                }
                const watch = currentWatch();
                const login = snapshot === undefined ? 'not logged in' : `ok (${snapshot.origin})`;
                const extra = watch === undefined
                    ? ''
                    : watch.status === 'waiting'
                        ? ' — browser login in progress'
                        : watch.status === 'error'
                            ? ` — last login error: ${watch.detail ?? 'failed'}`
                            : ' — last login finished';
                return {
                    kind: 'success',
                    text: [
                        `AI-GateWay: ${login}${extra}`,
                        snapshot === undefined ? '' : `Models: ${models.map(m => m.id).join(', ') || '(none listed)'}`,
                        '',
                        usageText(),
                    ].filter(Boolean).join('\n'),
                };
            },
        });
    }
    ctx.inject(['webServer'], (httpCtx) => {
        const server = httpCtx.webServer;
        httpCtx.effect(() => server.register({
            kind: 'prefix',
            path: '/agw-oauth',
            handler: (req, res) => {
                void handleHttp(req, res, {
                    config,
                    persist,
                    token: () => snapshot,
                    savedOrigin: () => savedOrigin,
                    saveOrigin,
                    logout,
                });
            },
        }), 'agw-oauth: http api');
    });
}
