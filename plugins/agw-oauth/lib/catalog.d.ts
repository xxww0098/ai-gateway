/** Parse AI-GateWay GET /v1/models into Harness resolveModel facts. */
export declare const PROVIDER = "ai-gateway";
export type Modality = 'text' | 'image';
export interface GatewayEffort {
    id: string;
    name: string;
}
export interface GatewayReasoning {
    efforts: GatewayEffort[];
    defaultEffort?: string;
}
export interface GatewayModel {
    id: string;
    name: string;
    contextLength?: number;
    maxOutputTokens?: number;
    inputModalities: Modality[];
    reasoning?: GatewayReasoning;
}
/** Map one /v1/models row. Unknown extra fields are ignored. */
export declare function parseGatewayModel(raw: unknown): GatewayModel | undefined;
/** Parse the OpenAI-shaped `{ object, data }` envelope. */
export declare function parseModelsPayload(json: unknown): GatewayModel[];
/** Harness listModels() row. */
export declare function toListModel(provider: string, model: GatewayModel): {
    inputModalities?: Modality[] | undefined;
    provider: string;
    id: string;
    name: string;
};
/** Harness resolveModel() row. Token limits come from the catalog, never guesses. */
export declare function toResolvedModel(provider: string, model: GatewayModel): {
    reasoning?: {
        defaultEffort?: string | undefined;
        efforts: GatewayEffort[];
    } | undefined;
    defaultMaxTokens?: number | undefined;
    context?: {
        contextWindow: number;
    } | undefined;
    inputModalities?: Modality[] | undefined;
    provider: string;
    id: string;
    name: string;
};
