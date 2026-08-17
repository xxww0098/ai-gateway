export interface CommandResult {
    kind: 'success' | 'error';
    text: string;
    openUrl?: string;
    userCode?: string;
}
export interface LoginWatch {
    status: 'waiting' | 'ok' | 'error';
    detail?: string;
    openUrl?: string;
    userCode?: string;
}
export declare function resetLoginWatch(): void;
export declare function currentWatch(): LoginWatch | undefined;
export declare function usageText(): string;
export declare function startLogin(origin: string, persist: (apiKey: string, origin: string) => Promise<void>, signal?: AbortSignal): Promise<CommandResult>;
