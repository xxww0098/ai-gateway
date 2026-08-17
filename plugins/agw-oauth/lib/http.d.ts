export interface HttpAuth {
    origin: string;
    apiKey: string;
}
export interface HttpDeps {
    config: {
        origin: string;
    };
    persist: (apiKey: string, origin: string) => Promise<void>;
    token: () => HttpAuth | undefined;
    savedOrigin: () => string | undefined;
    saveOrigin: (origin: string) => Promise<void>;
    logout: () => Promise<void>;
}
export interface HttpRequest {
    url?: string;
    method?: string;
    on?(event: string, listener: (...args: unknown[]) => void): unknown;
}
export interface HttpResponse {
    setHeader(name: string, value: string): void;
    end(body: string): void;
    statusCode: number;
}
export declare function normalizeSavedOrigin(raw: unknown): string;
export declare function readJsonBody(req: HttpRequest): Promise<unknown>;
export declare function handleHttp(req: HttpRequest, res: HttpResponse, deps: HttpDeps): Promise<void>;
