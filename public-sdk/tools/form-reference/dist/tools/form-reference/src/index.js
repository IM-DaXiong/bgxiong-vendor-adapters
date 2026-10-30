import { ObjectUrlPool, probeFixture } from "./media.js";
import { assembleInvocation, buildFieldPlan, exportRequest, inspectResponse, isObject, parseCapabilities, parseSeed, renderSchemaForm, sha256Bytes } from "./renderer.js";
import schemaPolicy from '../../../protocol/schema-policy.json' with { type: 'json' };
const MAX_PUBLIC_FRAME_BYTES = schemaPolicy.limits.maxFrameBytes;
const DOWNLOAD_RETAINED_RESOURCE_BUDGET = 16 * 1024 * 1024; // Local UI resource retention, not a wire limit.
const byId = (id) => { const element = document.getElementById(id); if (!element)
    throw new Error(`Missing reference page element ${id}`); return element; };
const status = byId('status'), capability = byId('capability'), model = byId('model'), operation = byId('operation');
const fixtureId = byId('fixture-id'), fixtureFile = byId('fixture-file'), exportButton = byId('export');
const fixtures = new Map(), urls = new ObjectUrlPool(), downloads = new ObjectUrlPool(DOWNLOAD_RETAINED_RESOURCE_BUDGET), pending = new Map();
let caps, seed, configuration;
let inputDraft = {}, configDraft = {}, inputForm, configForm;
const drafts = new Map();
let activeKey = '';
function message(text, error = false) { status.textContent = text; status.className = error ? 'status warning' : 'status'; }
function failure(error) { message(error instanceof Error ? error.message : 'The local file could not be presented.', true); }
async function readJson(file) {
    if (file.size > MAX_PUBLIC_FRAME_BYTES)
        throw new Error(`JSON file exceeds the public ${MAX_PUBLIC_FRAME_BYTES}-byte frame budget.`);
    const bytes = new Uint8Array(await file.arrayBuffer());
    const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    return { value: JSON.parse(text), hash: await sha256Bytes(bytes) };
}
function selected() {
    const contract = caps?.capabilityContracts[Number(capability.value)], chosen = contract?.operations[Number(operation.value)];
    return contract && chosen ? { contract, operation: chosen } : undefined;
}
function saveDraft() { if (activeKey && inputForm && configForm)
    drafts.set(activeKey, { input: inputForm.value(), configuration: configForm.value() }); }
function unsupported() { return [...(inputForm?.unsupported() ?? []), ...(configForm?.unsupported() ?? [])]; }
function refresh() {
    const selection = selected();
    const issues = unsupported();
    byId('unsupported').textContent = issues.join('\n');
    exportButton.disabled = !seed || !selection || !inputForm || !configForm || issues.length > 0;
    if (!seed || !selection || !inputForm || !configForm)
        return;
    try {
        const invocation = assembleInvocation(seed, selection.contract, selection.operation, model.value, inputForm.value(), configForm.value());
        byId('envelope').textContent = JSON.stringify({ ...invocation, payloadJson: JSON.parse(invocation.payloadJson) }, null, 2);
    }
    catch (error) {
        exportButton.disabled = true;
        failure(error);
    }
}
function paintForms() {
    saveDraft();
    inputForm?.dispose();
    configForm?.dispose();
    inputForm = configForm = undefined;
    const selection = selected();
    if (!seed || !selection) {
        refresh();
        return;
    }
    const payload = JSON.parse(seed.payloadJson);
    activeKey = JSON.stringify([selection.contract.localId, model.value, selection.operation.id, selection.operation.version]);
    const existing = drafts.get(activeKey);
    inputDraft = existing?.input ?? structuredClone(payload.input);
    configDraft = existing?.configuration ?? structuredClone(configuration ?? payload.configuration);
    const context = { media: payload.media, fixtures, urls };
    configForm = renderSchemaForm(byId('configuration-form'), buildFieldPlan(selection.contract, selection.operation, 'configuration'), configDraft, context, refresh);
    inputForm = renderSchemaForm(byId('input-form'), buildFieldPlan(selection.contract, selection.operation, 'input'), inputDraft, context, refresh);
    refresh();
}
function populateOperations() {
    const contract = caps?.capabilityContracts[Number(capability.value)];
    model.replaceChildren();
    operation.replaceChildren();
    if (contract) {
        for (const id of contract.modelIds)
            model.append(new Option(id, id));
        for (const [index, item] of contract.operations.entries())
            operation.append(new Option(`${item.label} · ${item.id} v${item.version}`, String(index)));
        if (seed) {
            const payload = JSON.parse(seed.payloadJson);
            if (contract.modelIds.includes(payload.modelId))
                model.value = payload.modelId;
            const index = contract.operations.findIndex(item => item.id === payload.operationId && item.version === payload.operationVersion);
            if (index >= 0)
                operation.value = String(index);
        }
    }
    model.disabled = operation.disabled = !contract;
    paintForms();
}
function populateCapabilities() {
    saveDraft();
    capability.replaceChildren();
    for (const [index, item] of (caps?.capabilityContracts ?? []).entries())
        capability.append(new Option(`${item.capabilityKey} · ${item.localId}`, String(index)));
    capability.disabled = !caps?.capabilityContracts.length;
    if (seed && caps) {
        const payload = JSON.parse(seed.payloadJson);
        const index = caps.capabilityContracts.findIndex(item => item.capabilityKey === payload.capabilityKey && item.modelIds.includes(payload.modelId));
        if (index >= 0)
            capability.value = String(index);
    }
    populateOperations();
}
function releaseFixtures() {
    for (const controller of pending.values())
        controller.abort();
    pending.clear();
    inputForm?.dispose();
    configForm?.dispose();
    inputForm = configForm = undefined;
    urls.clear();
    fixtures.clear();
    byId('fixture-summary').replaceChildren();
}
function populateMedia() {
    fixtureId.replaceChildren();
    if (seed)
        for (const media of JSON.parse(seed.payloadJson).media)
            fixtureId.append(new Option(`${media.id} · ${media.fileName}`, media.id));
    fixtureId.disabled = fixtureFile.disabled = fixtureId.options.length === 0;
}
for (const [id, action] of [
    ['capabilities-file', async (value) => { caps = parseCapabilities(value); drafts.clear(); activeKey = ''; inputForm?.dispose(); configForm?.dispose(); inputForm = configForm = undefined; populateCapabilities(); }],
    ['request-file', async (value) => { const parsed = parseSeed(value); releaseFixtures(); seed = parsed; configuration = undefined; drafts.clear(); activeKey = ''; populateMedia(); populateCapabilities(); }],
    ['configuration-file', async (value) => { if (!isObject(value))
            throw new Error('Import a nonsecret configuration object.'); saveDraft(); configuration = structuredClone(value); for (const draft of drafts.values())
            draft.configuration = structuredClone(configuration); inputForm?.dispose(); configForm?.dispose(); inputForm = configForm = undefined; paintForms(); }],
]) {
    byId(id).addEventListener('change', async (event) => {
        const file = event.currentTarget.files?.[0];
        if (!file)
            return;
        try {
            const imported = await readJson(file);
            await action(imported.value);
            message(`Imported ${file.name}. Source-file SHA-256: ${imported.hash}. Rust validation has not run.`);
        }
        catch (error) {
            failure(error);
        }
    });
}
capability.addEventListener('change', () => { saveDraft(); populateOperations(); });
model.addEventListener('change', paintForms);
operation.addEventListener('change', paintForms);
byId('editor').addEventListener('change', refresh);
byId('editor').addEventListener('input', refresh);
fixtureFile.addEventListener('change', async () => {
    const file = fixtureFile.files?.[0], id = fixtureId.value;
    if (!seed || !file || !id)
        return;
    const media = JSON.parse(seed.payloadJson).media.find(item => item.id === id);
    if (!media)
        return;
    pending.get(id)?.abort();
    const controller = new AbortController();
    pending.set(id, controller);
    try {
        const url = urls.replace(id, file);
        saveDraft();
        fixtures.delete(id);
        paintForms();
        const info = await probeFixture(media, url, controller.signal);
        if (pending.get(id) !== controller || controller.signal.aborted)
            return;
        fixtures.set(id, info);
        pending.delete(id);
        saveDraft();
        paintForms();
        byId('fixture-summary').textContent = [...fixtures.values()].map(item => `${item.mediaId}: ${item.kind}${item.width && item.height ? ` · ${item.width}×${item.height} source pixels` : ''}${item.durationMs !== undefined ? ` · ${item.durationMs} ms` : ''}`).join('\n');
        message(`Decoded ${file.name} for ${id}. Existing public media metadata is preserved; configure this same source in the CLI.`);
    }
    catch (error) {
        if (pending.get(id) === controller) {
            pending.delete(id);
            urls.release(id);
            fixtures.delete(id);
            saveDraft();
            paintForms();
            if (!(error instanceof DOMException && error.name === 'AbortError'))
                failure(error);
        }
    }
    finally {
        fixtureFile.value = '';
    }
});
byId('clear-fixtures').addEventListener('click', () => { saveDraft(); releaseFixtures(); paintForms(); message('Preview resources released.'); });
byId('editor').addEventListener('submit', async (event) => {
    event.preventDefault();
    try {
        const selection = selected();
        if (!seed || !selection || !inputForm || !configForm)
            throw new Error('Import capabilities and a complete request seed first.');
        if (!byId('editor').reportValidity())
            return;
        const invocation = assembleInvocation(seed, selection.contract, selection.operation, model.value, inputForm.value(), configForm.value(), unsupported());
        const exported = await exportRequest(invocation);
        byId('request-preview').textContent = exported.text;
        byId('hashes').textContent = `request.json SHA-256: ${exported.requestSha256}\npayloadJson UTF-8 SHA-256: ${exported.payloadSha256}`;
        const link = document.createElement('a');
        const downloadUrl = downloads.replace('request', new Blob([exported.text], { type: 'application/json' }));
        link.href = downloadUrl;
        link.download = 'request.json';
        link.click();
        setTimeout(() => { if (downloads.get('request') === downloadUrl)
            downloads.release('request'); }, 1000);
        message('Exported exact public Invocation. Run the Rust CLI with explicit package/workspace/config and this request; export does not claim validation or execution.');
    }
    catch (error) {
        failure(error);
    }
});
byId('receipt-file').addEventListener('change', async (event) => {
    const file = event.currentTarget.files?.[0];
    if (!file)
        return;
    try {
        const imported = await readJson(file), receipt = inspectResponse(imported.value), container = byId('receipt');
        container.replaceChildren();
        const hash = document.createElement('p');
        hash.textContent = `Receipt-file SHA-256: ${imported.hash}`;
        container.append(hash);
        for (const [label, value] of receipt.error ? [['Standard error', receipt.error]] : [['Standard data (including outputs and usage)', receipt.data]]) {
            const heading = document.createElement('h3');
            heading.textContent = String(label);
            const pre = document.createElement('pre');
            pre.textContent = JSON.stringify(value, null, 2);
            container.append(heading, pre);
        }
        message('Imported standard Response. Correlate it with the request hashes and adjacent CLI provenance; an unrelated response is not automatically treated as this request’s result.');
    }
    catch (error) {
        failure(error);
    }
});
window.addEventListener('pagehide', () => { releaseFixtures(); urls.dispose(); downloads.dispose(); drafts.clear(); seed = undefined; caps = undefined; configuration = undefined; });
