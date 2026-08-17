import Schema from '@deepseek-ai/schemastery';
export interface Config {
    origin: string;
}
export declare const Config: Schema<Config>;
export declare function resolveOrigin(config: Config, stored?: string): string;
