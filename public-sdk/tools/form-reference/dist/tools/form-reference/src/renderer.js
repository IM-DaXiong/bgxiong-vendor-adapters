import { renderInteraction } from "./media.js";
import publicInteractions from '../../../schemas/interaction-values.schema.json' with { type: 'json' };
export const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const clone = (value) => structuredClone(value);
const escape = (value) => value.replaceAll('~', '~0').replaceAll('/', '~1');
export function pointerParts(pointer) {
    if (!pointer.startsWith('/') || /~(?![01])/u.test(pointer))
        throw new Error('Unsupported field pointer.');
    const parts = pointer.slice(1).split('/').map(part => part.replaceAll('~1', '/').replaceAll('~0', '~'));
    if (parts.some(part => ['__proto__', 'prototype', 'constructor'].includes(part)))
        throw new Error('Unsafe field pointer is not renderable.');
    return parts;
}
export function pointerValue(value, pointer) { let result = value; for (const part of pointerParts(pointer))
    result = isObject(result) && Object.hasOwn(result, part) ? result[part] : undefined; return result; }
export function setPointer(value, pointer, next) {
    const parts = pointerParts(pointer);
    let parent = value;
    for (const part of parts.slice(0, -1)) {
        const child = parent[part];
        if (!isObject(child))
            parent[part] = {};
        parent = parent[part];
    }
    const last = parts.at(-1);
    if (next === undefined)
        delete parent[last];
    else
        parent[last] = clone(next);
}
/** Shape inspection is for presentation only, never a substitute for Rust schema checks. */
export function parseCapabilities(value) {
    if (isObject(value) && value.operation === 'capabilities' && typeof value.dataJson === 'string')
        value = JSON.parse(value.dataJson);
    if (!isObject(value) || value.schemaVersion !== 2 || !Array.isArray(value.capabilityContracts) || typeof value.pluginId !== 'string' || typeof value.pluginVersion !== 'string')
        throw new Error('Import the standard RuntimeCapsV2 or a capabilities Response.');
    return value;
}
export function parseSeed(value) {
    if (!isObject(value) || value.protocolVersion !== 2 || value.operation !== 'submit' || typeof value.payloadJson !== 'string')
        throw new Error('Import a complete public submit Invocation.');
    for (const key of ['profileId', 'bindingId', 'requestId', 'capabilitySlot'])
        if (typeof value[key] !== 'string' || !value[key])
            throw new Error(`The seed must explicitly supply ${key}.`);
    if (value.negotiatedVersion !== 2 || !Array.isArray(value.enabledFeatures) || typeof value.budgetMs !== 'number')
        throw new Error('The seed must explicitly supply negotiatedVersion, enabledFeatures and budgetMs.');
    const payload = JSON.parse(value.payloadJson);
    if (!isObject(payload) || payload.schemaVersion !== 2 || !isObject(payload.input) || !isObject(payload.configuration) || !Array.isArray(payload.media) || !Array.isArray(payload.uploads))
        throw new Error('payloadJson must contain the complete public OperationSubmitV2 with input, configuration, media and uploads.');
    for (const key of ['modelId', 'capabilityKey', 'operationId', 'inputSchemaDigest'])
        if (typeof payload[key] !== 'string')
            throw new Error(`The seed payload must explicitly supply ${key}.`);
    if (typeof payload.operationVersion !== 'number')
        throw new Error('The seed payload must explicitly supply operationVersion.');
    for (const media of payload.media)
        if (!isObject(media) || ['id', 'kind', 'handle', 'mime', 'fileName', 'byteLength'].some(key => typeof media[key] !== 'string'))
            throw new Error('Every seed media record must include all six public metadata fields. The browser cannot provision a missing handle.');
    return clone(value);
}
function documents(contract) {
    const result = new Map([[publicInteractions.$id, publicInteractions]]);
    for (const document of contract.schemas) {
        const schema = JSON.parse(document.schemaJson);
        if (!isObject(schema))
            throw new Error('Schema document is not a renderable object.');
        if (result.has(document.id))
            throw new Error('Duplicate schema document is not renderable.');
        result.set(document.id, schema);
        if (typeof schema.$id === 'string')
            result.set(schema.$id, schema);
    }
    return result;
}
function resolve(schema, docs, base, visited = new Set()) {
    if (typeof schema.$id === 'string') {
        if (!docs.has(schema.$id))
            throw new Error('unsupportedInteraction: nested schema resource scope is unavailable.');
        base = schema.$id;
    }
    if (typeof schema.$ref !== 'string')
        return { schema, base };
    if (Object.keys(schema).some(key => !['$ref', 'title', 'description', '$comment'].includes(key)))
        throw new Error('unsupportedInteraction: schema $ref siblings require native interpretation.');
    const [reference, fragment] = schema.$ref.split('#');
    const hash = fragment === undefined ? '' : '#' + fragment;
    let id = reference || base;
    if (reference && !docs.has(reference)) {
        try {
            id = new URL(reference, base).href;
        }
        catch {
            throw new Error('unsupportedInteraction: schema resource is not directly addressable.');
        }
    }
    const token = id + hash;
    if (visited.has(token))
        throw new Error('unsupportedInteraction: recursive schema presentation is unavailable.');
    visited.add(token);
    let document = docs.get(id);
    if (!document)
        throw new Error('unsupportedInteraction: referenced schema is not included in this capability.');
    if (hash) {
        if (!hash.startsWith('#/'))
            throw new Error('unsupportedInteraction: named schema anchors require native interpretation.');
        for (const part of decodeURIComponent(hash.slice(2)).split('/').map(p => p.replaceAll('~1', '/').replaceAll('~0', '~')))
            document = isObject(document) ? document[part] : undefined;
    }
    if (!isObject(document))
        throw new Error('unsupportedInteraction: referenced field schema is unavailable.');
    return resolve(document, docs, id, visited);
}
function simpleType(schema) { if (typeof schema.type === 'string')
    return schema.type; if (Array.isArray(schema.type)) {
    const types = schema.type.filter(type => type !== 'null');
    if (types.length === 1 && typeof types[0] === 'string')
        return types[0];
} return undefined; }
function complex(schema) { return ['oneOf', 'anyOf', 'allOf', 'if', 'then', 'else', 'dependentSchemas', 'patternProperties'].some(key => Object.hasOwn(schema, key)); }
function inferredWidget(schema) { if (Array.isArray(schema.enum))
    return 'select'; return { string: 'text', number: 'number', integer: 'integer', boolean: 'toggle', array: 'repeat', object: 'repeat' }[simpleType(schema) ?? ''] ?? 'unsupported'; }
export function buildFieldPlan(contract, operation, kind) {
    const plan = { fields: [], unsupported: [] };
    try {
        const docs = documents(contract), reference = kind === 'configuration' ? contract.configurationSchema : operation.inputSchema;
        const source = docs.get(reference.id);
        if (!source)
            throw new Error('unsupportedInteraction: selected schema is absent from the capability resources.');
        const controls = new Map((kind === 'input' ? operation.uiSchema.controls : []).map(control => [control.fieldPointer, control]));
        const walk = (raw, base, prefix, conditional, depth) => {
            if (depth > 64)
                throw new Error('unsupportedInteraction: schema presentation depth is unavailable.');
            const resolved = resolve(raw, docs, base), schema = resolved.schema;
            if (complex(schema) || !isObject(schema.properties))
                throw new Error(`unsupportedInteraction: ${prefix || '/'} requires a schema form this reference renderer cannot present.`);
            const required = Array.isArray(schema.required) ? schema.required : [];
            for (const key of required)
                if (typeof key !== 'string' || !Object.hasOwn(schema.properties, key))
                    plan.unsupported.push(`unsupportedInteraction: required property ${String(key)} is not present in the schema properties.`);
            for (const [key, rawField] of Object.entries(schema.properties)) {
                const pointer = `${prefix}/${escape(key)}`;
                pointerParts(pointer);
                const isRequired = required.includes(key), control = controls.get(pointer);
                let field = isObject(rawField) ? rawField : {}, fieldBase = resolved.base, issue;
                try {
                    const result = resolve(field, docs, resolved.base);
                    field = result.schema;
                    fieldBase = result.base;
                    if (typeof rawField === 'boolean' || complex(field))
                        issue = 'unsupportedInteraction: this field requires native schema interpretation.';
                }
                catch (error) {
                    issue = error instanceof Error ? error.message : 'unsupportedInteraction';
                }
                if (!control && simpleType(field) === 'object' && isObject(field.properties) && !issue) {
                    walk(field, fieldBase, pointer, conditional || !isRequired, depth + 1);
                    continue;
                }
                const widget = control?.widget ?? inferredWidget(field);
                if (kind === 'input' && isRequired && !control)
                    issue = 'unsupportedInteraction: required input has no public uiSchema control.';
                if (widget === 'unsupported')
                    issue = 'unsupportedInteraction: no supported widget for this schema field.';
                if (widget === 'select' && !Array.isArray(field.enum))
                    issue = 'unsupportedInteraction: select requires the schema enum.';
                plan.fields.push({ pointer, label: control?.label ?? (typeof field.title === 'string' ? field.title : key), description: control?.description ?? (typeof field.description === 'string' ? field.description : ''), group: control?.group ?? '', widget, required: isRequired, conditional, schema: field, mediaKind: operation.mediaSlots.find(slot => slot.inputPointer === pointer)?.mediaKind, ...(issue ? { unsupported: issue } : {}) });
            }
        };
        walk(source, reference.id, '', false, 0);
        for (const pointer of controls.keys())
            if (!plan.fields.some(field => field.pointer === pointer))
                plan.unsupported.push(`unsupportedInteraction: control ${pointer} is not addressable from the presented schema.`);
    }
    catch (error) {
        plan.unsupported.push(error instanceof Error ? error.message : 'unsupportedInteraction: schema cannot be presented.');
    }
    return plan;
}
export function assembleInvocation(seed, contract, operation, modelId, input, configuration, unsupported = []) {
    if (unsupported.length)
        throw new Error(unsupported.join('\n'));
    if (!contract.modelIds.includes(modelId) || !contract.operations.includes(operation))
        throw new Error('Choose a model and operation from the imported capability.');
    const payload = JSON.parse(seed.payloadJson);
    const changed = { ...payload, modelId, capabilityKey: contract.capabilityKey, operationId: operation.id, operationVersion: operation.version, inputSchemaDigest: operation.inputSchema.digest, input: clone(input), configuration: clone(configuration) };
    return { ...clone(seed), capabilitySlot: contract.transportSlot, payloadJson: JSON.stringify(changed) };
}
export async function sha256Bytes(bytes) { const stable = new Uint8Array(bytes); const digest = await crypto.subtle.digest('SHA-256', stable); return [...new Uint8Array(digest)].map(value => value.toString(16).padStart(2, '0')).join(''); }
export async function exportRequest(invocation) { const text = JSON.stringify(invocation, null, 2) + '\n'; return { text, requestSha256: await sha256Bytes(new TextEncoder().encode(text)), payloadSha256: await sha256Bytes(new TextEncoder().encode(invocation.payloadJson)) }; }
export function inspectResponse(value) { if (!isObject(value) || value.protocolVersion !== 2 || typeof value.operation !== 'string')
    throw new Error('Import a standard public Response.'); if (typeof value.dataJson === 'string')
    return { response: value, data: JSON.parse(value.dataJson) }; if (isObject(value.error))
    return { response: value, error: value.error }; throw new Error('Response must supply dataJson or error.'); }
export function renderSchemaForm(container, plan, initial, context, changed) {
    const current = clone(initial), interactions = [], cleanups = [];
    container.replaceChildren();
    for (const issue of plan.unsupported) {
        const note = document.createElement('p');
        note.className = 'warning';
        note.textContent = issue;
        container.append(note);
    }
    for (const field of plan.fields) {
        const group = document.createElement('div');
        group.className = 'field';
        const label = document.createElement('label');
        label.textContent = `${field.group ? field.group + ' · ' : ''}${field.label}${field.required ? field.conditional ? ' (required when parent supplied)' : ' *' : ''}`;
        const identifier = `field-${crypto.randomUUID()}`;
        label.htmlFor = identifier;
        group.append(label);
        const description = document.createElement('small');
        description.textContent = `${field.pointer}${field.description ? ' · ' + field.description : ''}`;
        group.append(description);
        const update = (value) => { setPointer(current, field.pointer, value); changed(); };
        const initialValue = pointerValue(current, field.pointer);
        if (field.unsupported) {
            const issue = document.createElement('p');
            issue.className = 'warning';
            issue.textContent = field.unsupported;
            group.append(issue);
        }
        else if (['point', 'box', 'mask', 'time_span'].includes(field.widget)) {
            const editor = renderInteraction(field.widget, initialValue, context, update, field.required && !field.conditional);
            interactions.push({ field, editor });
            group.append(editor.element);
            cleanups.push(() => editor.dispose());
        }
        else {
            let input;
            if (field.widget === 'select' || field.widget === 'toggle' || field.widget === 'media') {
                const select = document.createElement('select');
                input = select;
                select.append(new Option('Not supplied', ''));
                let choices = field.widget === 'toggle' ? [true, false] : field.widget === 'media' ? context.media.filter(media => !field.mediaKind || media.kind === field.mediaKind).map(media => ({ mediaId: media.id })) : field.schema.enum;
                const many = field.widget === 'media' && simpleType(field.schema) === 'array';
                select.multiple = many;
                for (const [index, choice] of choices.entries()) {
                    const option = new Option(typeof choice === 'string' ? choice : JSON.stringify(choice), String(index));
                    option.selected = many ? Array.isArray(initialValue) && initialValue.some(item => JSON.stringify(item) === JSON.stringify(choice)) : JSON.stringify(initialValue) === JSON.stringify(choice);
                    select.append(option);
                }
                select.addEventListener('change', () => update(many ? [...select.selectedOptions].filter(option => option.value !== '').map(option => choices[Number(option.value)]) : select.value === '' ? undefined : choices[Number(select.value)]));
            }
            else if (field.widget === 'multiline' || field.widget === 'repeat') {
                const textarea = document.createElement('textarea');
                input = textarea;
                if (initialValue !== undefined)
                    textarea.value = field.widget === 'repeat' ? JSON.stringify(initialValue, null, 2) : String(initialValue);
                textarea.addEventListener('input', () => { try {
                    const next = field.widget === 'repeat' ? textarea.value === '' ? undefined : JSON.parse(textarea.value) : textarea.value;
                    textarea.setCustomValidity('');
                    update(next);
                }
                catch {
                    textarea.setCustomValidity('Enter a JSON array/object as described by the schema.');
                    changed();
                } });
            }
            else {
                const element = document.createElement('input');
                input = element;
                element.type = field.widget === 'number' || field.widget === 'integer' ? 'number' : 'text';
                if (element.type === 'number') {
                    element.step = field.widget === 'integer' ? '1' : 'any';
                    if (typeof field.schema.minimum === 'number')
                        element.min = String(field.schema.minimum);
                    if (typeof field.schema.maximum === 'number')
                        element.max = String(field.schema.maximum);
                }
                if (typeof field.schema.minLength === 'number')
                    element.minLength = field.schema.minLength;
                if (typeof field.schema.maxLength === 'number')
                    element.maxLength = field.schema.maxLength;
                if (initialValue !== undefined)
                    element.value = String(initialValue);
                element.addEventListener('input', () => update(element.type === 'number' ? element.value === '' ? undefined : element.valueAsNumber : element.value));
            }
            input.id = identifier;
            input.required = field.required && !field.conditional;
            group.append(input);
            const clear = document.createElement('button');
            clear.type = 'button';
            clear.textContent = 'Omit field';
            clear.addEventListener('click', () => { input.value = ''; update(undefined); });
            group.append(clear);
        }
        const details = document.createElement('details'), summary = document.createElement('summary'), schemaText = document.createElement('pre');
        summary.textContent = 'Source field schema';
        schemaText.textContent = JSON.stringify(field.schema, null, 2);
        details.append(summary, schemaText);
        group.append(details);
        container.append(group);
    }
    return { value: () => clone(current), unsupported: () => [...plan.unsupported, ...plan.fields.filter(field => field.unsupported && ((field.required && !field.conditional) || pointerValue(current, field.pointer) !== undefined)).map(field => `${field.pointer}: ${field.unsupported}`), ...interactions.filter(({ field, editor }) => editor.unsupported() && ((field.required && !field.conditional) || pointerValue(current, field.pointer) !== undefined)).map(({ field, editor }) => `${field.pointer}: ${editor.unsupported()}`)], dispose: () => { for (const cleanup of cleanups)
            cleanup(); container.replaceChildren(); } };
}
