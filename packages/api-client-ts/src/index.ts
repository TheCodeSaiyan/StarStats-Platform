/**
 * Public surface of the api-client-ts package.
 *
 * The bulk of this file is type-only re-exports of the generated
 * OpenAPI schema. The generated module is committed (see
 * scripts/generate.ts) so consumers don't need a Cargo toolchain just
 * to `pnpm install`.
 *
 * To regenerate after server changes:
 *   pnpm --filter api-client-ts run generate
 */

export type { components, operations, paths } from './generated/schema.js';

import type { components } from './generated/schema.js';

// Convenience aliases for the most-used response shapes. Add more
// here when callers start to need them — the goal is to keep the
// import surface short on the consumer side.
export type AuthResponse = components['schemas']['AuthResponse'];
export type SignupRequest = components['schemas']['SignupRequest'];
export type LoginRequest = components['schemas']['LoginRequest'];
export type VerifyEmailRequest = components['schemas']['VerifyEmailRequest'];
export type VerifyEmailResponse = components['schemas']['VerifyEmailResponse'];

export type StartRequest = components['schemas']['StartRequest'];
export type StartResponse = components['schemas']['StartResponse'];
export type RedeemRequest = components['schemas']['RedeemRequest'];
export type RedeemResponse = components['schemas']['RedeemResponse'];
export type DeviceListResponse = components['schemas']['DeviceListResponse'];
export type DeviceDto = components['schemas']['DeviceDto'];

export type IngestResponse = components['schemas']['IngestResponse'];
export type IngestBatch = components['schemas']['IngestBatchSchema'];
export type EventEnvelope = components['schemas']['EventEnvelopeSchema'];
export type EventMetadata = components['schemas']['EventMetadataSchema'];
export type EntityRef = components['schemas']['EntityRefSchema'];
export type EntityKind = components['schemas']['EntityKindSchema'];
export type EventSource = components['schemas']['EventSourceSchema'];
export type FieldProvenance = components['schemas']['FieldProvenanceSchema'];
export type GameEvent = components['schemas']['GameEventSchema'];

export type SummaryResponse = components['schemas']['SummaryResponse'];
export type ListResponse = components['schemas']['ListResponse'];
export type EventDto = components['schemas']['EventDto'];
export type TypeCount = components['schemas']['TypeCount'];
