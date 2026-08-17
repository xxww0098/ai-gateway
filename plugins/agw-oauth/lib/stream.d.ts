/** OpenAI-compatible chat/completions request + SSE → Harness StreamChunks. */
export interface OpenAiMessage {
    role: 'system' | 'user' | 'assistant' | 'tool';
    content: unknown;
    tool_call_id?: string;
    tool_calls?: unknown;
}
export declare function imageDataUrl(attachment: unknown): string | undefined;
export declare function contentToOpenAi(blocks: readonly {
    type: string;
    [k: string]: unknown;
}[]): unknown;
export declare function messagesToOpenAi(options: {
    system?: string;
    messages: readonly {
        role: string;
        content: readonly {
            type: string;
            [k: string]: unknown;
        }[];
    }[];
}): OpenAiMessage[];
export declare function parseOpenAiSse(body: ReadableStream<Uint8Array>): AsyncGenerator<Record<string, unknown>>;
