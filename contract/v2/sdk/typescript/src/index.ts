export type * from './generated.js';
import type { Invocation } from './generated.js';

/** Encoding preserves all explicitly frozen fields. Native validation is required. */
export function encodeInvocation(value: Invocation): string {
    return JSON.stringify(value);
}

/** Parsing alone does not claim that a value passed the native schema validator. */
export function parseWireJson(source: string): unknown {
    return JSON.parse(source) as unknown;
}
