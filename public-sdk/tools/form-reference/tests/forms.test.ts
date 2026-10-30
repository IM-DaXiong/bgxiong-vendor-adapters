import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import type { CapabilityContractV2, Invocation, OperationSubmitV2 } from '../../../sdk/typescript/src/generated.ts';
import { assembleInvocation, buildFieldPlan, exportRequest, inspectResponse, parseCapabilities, parseSeed, pointerValue, setPointer } from '../src/renderer.ts';
import { maskReference, ObjectUrlPool, pixelBox, pixelPoint, timeSpan } from '../src/media.ts';

const fixture = JSON.parse(readFileSync(new URL('../../../examples/coverage-capabilities.json', import.meta.url), 'utf8')) as { capabilities: CapabilityContractV2[] };
const interactions = JSON.parse(readFileSync(new URL('../../../schemas/interaction-values.schema.json', import.meta.url), 'utf8')) as { $defs: Record<string, { required: string[] }> };
function seedFor(contract = fixture.capabilities[0]): Invocation {
  const operation = contract.operations[0];
  const payload: OperationSubmitV2 = { schemaVersion: 2, modelId: contract.modelIds[0], capabilityKey: contract.capabilityKey, operationId: operation.id, operationVersion: operation.version, inputSchemaDigest: operation.inputSchema.digest, input: { prompt: 'A local fixture' }, configuration: { endpointBaseUrl: 'https://example.invalid' }, media: [{ id: 'source', kind: 'video', handle: 'explicit-development-source', mime: 'video/mp4', fileName: 'fixture.mp4', byteLength: '9007199254740993' }], uploads: [{ mediaId: 'source', uploadRef: 'explicit-upload-reference' }] };
  return { protocolVersion: 2, operation: 'submit', profileId: 'explicit-profile', capabilitySlot: contract.transportSlot, requestId: 'explicit-request', budgetMs: 30000, payloadJson: JSON.stringify(payload), bindingId: 'explicit-binding', negotiatedVersion: 2, enabledFeatures: [] };
}

test('published required configuration and input controls remain visible without filling defaults', () => {
  for (const contract of fixture.capabilities) {
    const config = buildFieldPlan(contract, contract.operations[0], 'configuration');
    assert.deepEqual(config.unsupported, []);
    assert.ok(config.fields.some(field => field.pointer === '/endpointBaseUrl' && field.required));
    const input = buildFieldPlan(contract, contract.operations[0], 'input');
    assert.deepEqual(input.unsupported, []);
    assert.ok(input.fields.length > 0);
    assert.ok(input.fields.every(field => !field.unsupported));
    assert.ok(input.fields.filter(field => field.required).every(field => field.label.length > 0));
  }
});

test('same model capabilities stay distinct and export changes only explicit selection/draft fields', () => {
  const contracts = fixture.capabilities.filter(contract => contract.modelIds.includes('multi-purpose-example'));
  assert.ok(contracts.length >= 3);
  assert.ok(new Set(contracts.map(contract => contract.capabilityKey)).size >= 3);
  const seed = seedFor(), before = structuredClone(seed), original = JSON.parse(seed.payloadJson) as OperationSubmitV2;
  for (const contract of contracts) {
    const selected = contract.operations[0];
    const invocation = assembleInvocation(seed, contract, selected, contract.modelIds[0], { intentionallyUnvalidated: 3 }, { endpointBaseUrl: 'https://example.invalid' });
    const payload = JSON.parse(invocation.payloadJson) as OperationSubmitV2;
    assert.equal(payload.capabilityKey, contract.capabilityKey);
    assert.equal(payload.inputSchemaDigest, selected.inputSchema.digest);
    assert.equal(invocation.capabilitySlot, contract.transportSlot);
    assert.deepEqual(payload.media, original.media);
    assert.deepEqual(payload.uploads, original.uploads);
    assert.equal(payload.media[0].byteLength, '9007199254740993');
    assert.equal(invocation.requestId, seed.requestId);
    assert.equal(invocation.profileId, seed.profileId);
    assert.equal(invocation.bindingId, seed.bindingId);
  }
  assert.deepEqual(seed, before);
});

test('unrenderable required interaction explicitly blocks producing an executable-looking request', () => {
  const contract = structuredClone(fixture.capabilities[0]), operation = contract.operations[0];
  const resource = contract.schemas.find(schema => schema.id === operation.inputSchema.id)!;
  resource.schemaJson = JSON.stringify({ type: 'object', properties: { selection: { $ref: 'https://unregistered.invalid/required-shape' } }, required: ['selection'], additionalProperties: false });
  operation.uiSchema.controls = [{ fieldPointer: '/selection', widget: 'point', label: 'Selection', description: '', group: '' }];
  const plan = buildFieldPlan(contract, operation, 'input');
  const issue = plan.fields.find(field => field.pointer === '/selection')?.unsupported;
  assert.ok(issue?.startsWith('unsupportedInteraction:'));
  assert.throws(() => assembleInvocation(seedFor(contract), contract, operation, contract.modelIds[0], {}, {}, [issue!]));
});

test('public interaction schema references render locally without network retrieval', () => {
  const contract = structuredClone(fixture.capabilities[0]), operation = contract.operations[0];
  const resource = contract.schemas.find(schema => schema.id === operation.inputSchema.id)!;
  resource.schemaJson = JSON.stringify({ type: 'object', properties: { selection: { $ref: 'https://bgxiong.local/vendor-adapter/v2/interaction-values.schema.json#/$defs/point' } }, required: ['selection'] });
  operation.uiSchema.controls = [{ fieldPointer: '/selection', widget: 'point', label: 'Selection', description: '', group: '' }];
  const plan = buildFieldPlan(contract, operation, 'input');
  assert.deepEqual(plan.unsupported, []); assert.equal(plan.fields[0].widget, 'point'); assert.equal(plan.fields[0].unsupported, undefined);
});

test('pixel point/box and VFR time/mask preserve the exact public shapes', () => {
  const rect = { left: 10, top: 20, width: 200, height: 100 };
  const point = pixelPoint('source', 209.9, 119.9, rect, 1000, 500);
  assert.deepEqual(point, { mediaId: 'source', x: 999, y: 499 });
  const box = pixelBox('source', [10, 20], [209.9, 119.9], rect, 1000, 500);
  assert.deepEqual(box, { mediaId: 'source', x: 0, y: 0, width: 1000, height: 500 });
  const span = timeSpan('source', 33, 119); // real VFR timestamps, no FPS arithmetic
  assert.deepEqual(span, { mediaId: 'source', startMs: 33, endMs: 119 });
  const mask = maskReference('source', 'mask');
  for (const [widget, value] of [['point', point], ['box', box], ['time_span', span], ['mask', mask]] as const) assert.deepEqual(Object.keys(value).sort(), [...interactions.$defs[widget].required].sort());
  // The presentation layer does not repair invalid intervals into success.
  assert.deepEqual(timeSpan('source', 119, 33), { mediaId: 'source', startMs: 119, endMs: 33 });
});

test('request-file and payload hashes cover their exact exported UTF-8 bytes', async () => {
  const seed = parseSeed(seedFor()), exported = await exportRequest(seed);
  assert.equal(exported.requestSha256, createHash('sha256').update(exported.text, 'utf8').digest('hex'));
  assert.equal(exported.payloadSha256, createHash('sha256').update(seed.payloadJson, 'utf8').digest('hex'));
  assert.deepEqual(JSON.parse(exported.text), seed);
  assert.equal(JSON.parse(exported.text).payloadJson, seed.payloadJson);
});

test('object URLs are revoked on replacement/cancellation/release/unload; budget failure preserves current resource', () => {
  let next = 0; const revoked: string[] = [];
  const pool = new ObjectUrlPool(1024, () => `blob:test-${++next}`, url => revoked.push(url));
  const first = pool.replace('source', new Blob(['12']));
  const second = pool.replace('source', new Blob(['123'])); assert.deepEqual(revoked, [first]);
  assert.throws(() => pool.replace('source', new Blob(['x'.repeat(1025)])), /budget/);
  assert.equal(pool.get('source'), second); assert.deepEqual(revoked, [first]);
  pool.release('source'); assert.deepEqual(revoked, [first, second]);
  const third = pool.replace('pending', new Blob(['x'])); pool.release('pending'); assert.ok(revoked.includes(third));
  const fourth = pool.replace('active', new Blob(['x'])); pool.dispose(); assert.ok(revoked.includes(fourth)); assert.equal(pool.get('active'), undefined);
  assert.throws(() => pool.replace('late', new Blob(['x'])), /released/);
});

test('imports use standard wire objects, preserve receipt errors, and reject pointer prototype writes', () => {
  const runtime = { schemaVersion: 2, pluginId: 'local.reference', pluginVersion: '1.0.0', capabilityContracts: fixture.capabilities };
  const parsed = parseCapabilities({ protocolVersion: 2, operation: 'capabilities', dataJson: JSON.stringify(runtime) });
  assert.equal(parsed.capabilityContracts.length, fixture.capabilities.length);
  assert.throws(() => parseSeed({ protocolVersion: 2, operation: 'submit', payloadJson: '{}' }));
  const receipt = { protocolVersion: 2, operation: 'submit', error: { code: 'INVALID_INPUT', message: 'Native validation rejected input', retryable: false, vendorCode: null, vendorRequestId: null } };
  assert.deepEqual(inspectResponse(receipt).error, receipt.error);
  const value = {}; setPointer(value, '/a~1b/value~0', 'visible'); assert.equal(pointerValue(value, '/a~1b/value~0'), 'visible');
  assert.throws(() => setPointer(value, '/__proto__/polluted', true));
  assert.equal(({} as Record<string, unknown>).polluted, undefined);
});

test('page entry is local-only and clearly separates export from native validation', () => {
  const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
  // JSON module imports use connect-src; same-origin public assets must load.
  assert.ok(html.includes("connect-src 'self'"));
  assert.ok(html.includes("default-src 'none'"));
  assert.ok(html.includes('Rust schema/reference/media checks remain authoritative'));
  assert.ok(!html.includes('ui/src') && !html.includes('localhost:1420'));
});
