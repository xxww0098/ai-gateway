export interface StoredToken {
    origin: string;
    apiKey: string;
}
export declare function defaultTokenPath(): string;
export declare class TokenStore {
    readonly path: string;
    private cache;
    constructor(path?: string);
    read(): Promise<StoredToken | undefined>;
    write(token: StoredToken): Promise<void>;
    clear(): Promise<void>;
}
