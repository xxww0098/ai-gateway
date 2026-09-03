/** Read CLI OAuth files already on this host. No new browser login. */
export interface LocalOauthFile {
    provider: 'claude' | 'codex' | 'xai' | 'kiro';
    source: string;
    accessToken: string;
    refreshToken: string;
    idToken: string;
    expiresAt: string;
    email: string;
}
export interface ImportReport {
    found: Array<Omit<LocalOauthFile, 'accessToken' | 'refreshToken' | 'idToken'> & {
        hasAccessToken: boolean;
        hasRefreshToken: boolean;
    }>;
    uploaded?: {
        count: number;
        status: number;
        body: unknown;
    };
    error?: string;
}
export declare function wellKnownPaths(home?: string): string[];
export declare function parseCliJson(raw: unknown, source: string): LocalOauthFile[];
export declare function discoverLocalOauth(home?: string): Promise<LocalOauthFile[]>;
export declare function redact(files: LocalOauthFile[]): ImportReport['found'];
export declare function toUploadJson(file: LocalOauthFile): Record<string, unknown>;
/** POST each file to the gateway auth-files inventory (admin JWT or admin agw- key). */
export declare function uploadToGateway(origin: string, apiKey: string, files: LocalOauthFile[], fetchFn?: typeof fetch): Promise<{
    count: number;
    status: number;
    body: unknown;
}>;
export declare function importLocalAndMaybeUpload(input: {
    origin?: string;
    apiKey?: string;
    home?: string;
    fetchFn?: typeof fetch;
}): Promise<ImportReport>;
