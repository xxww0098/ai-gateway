/**
 * AGW-Oauth: OAuth into AI-GateWay from DeepSeek Harness.
 * Function plugin — export name, inject, Config, apply. No export default.
 */
import type { Context } from '@deepseek-ai/cordis';
import { Config } from './config.js';
export declare const name = "agw-oauth";
export declare const inject: string[];
export { Config };
export type { Config as ConfigType } from './config.js';
export { PROVIDER, parseModelsPayload, parseGatewayModel, toResolvedModel } from './catalog.js';
export { startDevice, pollDevice } from './oauth.js';
export { AgwAdapter } from './adapter.js';
export declare function apply(ctx: Context, config: {
    origin: string;
}): void;
