/** Device-code client for AI-GateWay /api/panel/oauth/dsh/*. */
export interface DeviceStart {
    deviceCode: string;
    userCode: string;
    verificationUri: string;
    verificationUriComplete: string;
    expiresIn: number;
    interval: number;
}
export interface DeviceApproved {
    status: 'approved';
    apiKey: string;
    origin: string;
}
export type DevicePoll = {
    status: 'pending';
} | {
    status: 'denied';
} | {
    status: 'expired';
} | DeviceApproved;
export declare class GatewayOAuthError extends Error {
    readonly code: string;
    constructor(message: string, code?: string);
}
export declare function normalizeOrigin(origin: string): string;
/** Start a device-code session. Returns as soon as the user has a URL/code. */
export declare function startDevice(origin: string, signal?: AbortSignal): Promise<DeviceStart>;
/** One poll. Does not sleep. */
export declare function pollDevice(origin: string, deviceCode: string, signal?: AbortSignal): Promise<DevicePoll>;
export declare function sleep(ms: number, signal?: AbortSignal): Promise<void>;
/** Poll until approved / denied / expired. Used by background login. */
export declare function waitForApproval(origin: string, start: DeviceStart, signal?: AbortSignal): Promise<DeviceApproved>;
