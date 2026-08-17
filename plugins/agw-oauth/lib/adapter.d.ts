import { LlmAdapter } from '@deepseek-ai/dsh-llm';
import type { GenerateOptions, StreamChunk } from '@deepseek-ai/dsh-llm';
import { PROVIDER, type GatewayModel } from './catalog.js';
import { contentToOpenAi } from './stream.js';
export interface AdapterAuth {
    origin: string;
    apiKey: string;
}
export declare class AgwAdapter extends LlmAdapter {
    private readonly auth;
    private readonly models;
    constructor(auth: () => AdapterAuth | undefined, models: () => GatewayModel[]);
    providerInfo(provider: string): {
        id: string;
        name: string;
    };
    listModels(provider: string): Promise<{
        inputModalities?: import("./catalog.js").Modality[] | undefined;
        provider: string;
        id: string;
        name: string;
    }[]>;
    resolveModel(provider: string, model: string, _signal?: AbortSignal): Promise<{
        reasoning?: {
            defaultEffort?: string | undefined;
            efforts: import("./catalog.js").GatewayEffort[];
        } | undefined;
        defaultMaxTokens?: number | undefined;
        context?: {
            contextWindow: number;
        } | undefined;
        inputModalities?: import("./catalog.js").Modality[] | undefined;
        provider: string;
        id: string;
        name: string;
    }>;
    stream(options: GenerateOptions): AsyncIterable<StreamChunk>;
}
export { PROVIDER, contentToOpenAi };
