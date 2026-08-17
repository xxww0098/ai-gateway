export interface StoredToken {
    origin: string;
    apiKey: string;
}
export declare function defaultTokenPath(): string;
export declare class TokenStore {
    readonly path: string;
    private cache;
    constructor(path?: string);
    /** Logged in only when both origin and api_key exist. */
    read(): Promise<StoredToken | undefined>;
    /** Origin from disk even when no api_key (settings field / login). */
    peekOrigin(): Promise<string | undefined>;
    /** Persist credentials. apiKey may be omitted so origin-only settings work. */
    write(token: {
        origin: string;
        apiKey?: string;
    }): Promise<void>;
    /** Save origin; keep an existing api_key so a settings edit does not log out. */
    writeOrigin(origin: string): Promise<void>;
    clear(): Promise<void>;
}
