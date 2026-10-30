/** Maps an actual displayed media rectangle to source pixels, without an FPS assumption. */
export function pixelPoint(mediaId, clientX, clientY, rect, width, height) {
    if (![clientX, clientY, rect.left, rect.top, rect.width, rect.height, width, height].every(Number.isFinite) || rect.width <= 0 || rect.height <= 0 || width <= 0 || height <= 0)
        throw new Error('unsupportedInteraction: decoded source dimensions are unavailable');
    return { mediaId, x: Math.min(width - 1, Math.max(0, Math.floor((clientX - rect.left) * width / rect.width))), y: Math.min(height - 1, Math.max(0, Math.floor((clientY - rect.top) * height / rect.height))) };
}
export function pixelBox(mediaId, from, to, rect, width, height) {
    const first = pixelPoint(mediaId, from[0], from[1], rect, width, height);
    const last = pixelPoint(mediaId, to[0], to[1], rect, width, height);
    if (first.x === last.x || first.y === last.y)
        throw new Error('Drag a box with positive width and height.');
    const x = Math.min(first.x, last.x), y = Math.min(first.y, last.y);
    return { mediaId, x, y, width: Math.abs(last.x - first.x) + 1, height: Math.abs(last.y - first.y) + 1 };
}
export function timeSpan(mediaId, startMs, endMs) { return { mediaId, startMs, endMs }; }
export function maskReference(mediaId, maskMediaId) { return { mediaId, maskMediaId }; }
/** Retains only the user's active fixtures, with an explicit byte budget. */
export class ObjectUrlPool {
    entries = new Map();
    bytes = 0;
    closed = false;
    limit;
    create;
    revoke;
    constructor(limit = 64 * 1024 * 1024, create = (blob) => URL.createObjectURL(blob), revoke = (url) => URL.revokeObjectURL(url)) { this.limit = limit; this.create = create; this.revoke = revoke; }
    replace(id, blob) {
        if (this.closed)
            throw new Error('Preview pool has been released.');
        const previous = this.entries.get(id);
        const cost = blob.size + id.length * 2 + 256; // Includes URL/key bookkeeping, even for empty files.
        if (this.bytes - (previous?.bytes ?? 0) + cost > this.limit)
            throw new Error('Preview fixtures exceed the retained-resource byte budget. Release a preview first.');
        const url = this.create(blob);
        this.release(id);
        this.entries.set(id, { url, bytes: cost });
        this.bytes += cost;
        return url;
    }
    get(id) { return this.entries.get(id)?.url; }
    release(id) { const entry = this.entries.get(id); if (entry) {
        this.entries.delete(id);
        this.bytes -= entry.bytes;
        this.revoke(entry.url);
    } }
    clear() { for (const id of [...this.entries.keys()])
        this.release(id); }
    dispose() { this.clear(); this.closed = true; }
}
function detach(element) {
    if (element instanceof HTMLMediaElement) {
        element.pause();
        element.removeAttribute('src');
        element.load();
    }
    else
        element.removeAttribute('src');
}
export async function probeFixture(media, url, signal) {
    if (media.kind === 'file')
        return { mediaId: media.id, kind: media.kind };
    const element = document.createElement(media.kind === 'image' ? 'img' : media.kind === 'audio' ? 'audio' : 'video');
    return new Promise((resolve, reject) => {
        const event = media.kind === 'image' ? 'load' : 'loadedmetadata';
        const cleanup = () => { element.removeEventListener(event, loaded); element.removeEventListener('error', failed); signal.removeEventListener('abort', aborted); detach(element); };
        const failed = () => { cleanup(); reject(new Error('unsupportedInteraction: the browser could not decode this fixture. Rust probing remains authoritative.')); };
        const aborted = () => { cleanup(); reject(new DOMException('Fixture preview cancelled', 'AbortError')); };
        const loaded = () => {
            const info = { mediaId: media.id, kind: media.kind };
            if (element instanceof HTMLImageElement) {
                info.width = element.naturalWidth;
                info.height = element.naturalHeight;
            }
            else {
                if (Number.isFinite(element.duration))
                    info.durationMs = Math.round(element.duration * 1000);
                if (element instanceof HTMLVideoElement) {
                    info.width = element.videoWidth;
                    info.height = element.videoHeight;
                }
            }
            cleanup();
            resolve(info);
        };
        if (signal.aborted) {
            aborted();
            return;
        }
        element.addEventListener(event, loaded, { once: true });
        element.addEventListener('error', failed, { once: true });
        signal.addEventListener('abort', aborted, { once: true });
        if (element instanceof HTMLMediaElement)
            element.preload = 'metadata';
        element.src = url;
    });
}
export function renderInteraction(widget, initial, context, change, required = false) {
    const element = document.createElement('div');
    element.className = 'interaction';
    const sourceSelect = document.createElement('select');
    sourceSelect.setAttribute('aria-label', 'Source media ID');
    sourceSelect.required = required;
    sourceSelect.append(new Option('Choose an existing media ID', ''));
    const candidates = context.media.filter(media => widget === 'time_span' ? media.kind === 'audio' || media.kind === 'video' : media.kind === 'image' || media.kind === 'video');
    for (const media of candidates)
        sourceSelect.append(new Option(`${media.id} · ${media.fileName}`, media.id));
    let object = initial && typeof initial === 'object' && !Array.isArray(initial) ? initial : {};
    const publish = (value) => { object = value; change(value); };
    sourceSelect.value = typeof object.mediaId === 'string' ? object.mediaId : '';
    const panel = document.createElement('div'), notice = document.createElement('p');
    notice.className = 'warning';
    element.append(sourceSelect, panel, notice);
    let unsupported, cleanups = [];
    const redraw = () => {
        for (const cleanup of cleanups)
            cleanup();
        cleanups = [];
        panel.replaceChildren();
        unsupported = undefined;
        notice.textContent = '';
        const id = sourceSelect.value, info = context.fixtures.get(id), url = context.urls.get(id);
        if (!id || !info || !url) {
            unsupported = 'unsupportedInteraction: attach a decoded local fixture for the selected existing media ID.';
            notice.textContent = unsupported;
            return;
        }
        const numeric = (key, step = 'any') => {
            const label = document.createElement('label');
            label.textContent = key;
            const input = document.createElement('input');
            input.type = 'number';
            input.min = '0';
            input.step = step;
            input.setAttribute('aria-label', key);
            input.required = required;
            if (typeof object[key] === 'number')
                input.value = String(object[key]);
            label.append(input);
            panel.append(label);
            return input;
        };
        if (widget === 'mask') {
            if (!info.width || !info.height) {
                unsupported = 'unsupportedInteraction: source dimensions are unavailable.';
                notice.textContent = unsupported;
                return;
            }
            const mask = document.createElement('select');
            mask.required = required;
            mask.setAttribute('aria-label', 'Mask media ID');
            mask.append(new Option('Choose an existing image mask ID', ''));
            for (const media of context.media.filter(m => m.kind === 'image'))
                mask.append(new Option(`${media.id} · ${media.fileName}`, media.id));
            mask.value = typeof object.maskMediaId === 'string' ? object.maskMediaId : '';
            const update = () => {
                const maskInfo = context.fixtures.get(mask.value);
                unsupported = maskInfo?.width && maskInfo?.height ? undefined : 'unsupportedInteraction: attach a decoded mask image fixture.';
                notice.textContent = unsupported ?? (maskInfo?.width !== info.width || maskInfo?.height !== info.height ? 'Mask dimensions differ from source; Rust will reject this selection.' : 'Mask uses source pixel dimensions. Rust validates the media references.');
                if (mask.value)
                    publish({ ...maskReference(id, mask.value) });
            };
            mask.addEventListener('change', update);
            panel.append(mask);
            const selectedMask = context.fixtures.get(mask.value);
            unsupported = selectedMask?.width && selectedMask?.height ? undefined : 'unsupportedInteraction: attach a decoded mask image fixture.';
            notice.textContent = unsupported ?? '';
            return;
        }
        if (widget === 'time_span') {
            if (info.durationMs === undefined) {
                unsupported = 'unsupportedInteraction: source duration is unavailable.';
                notice.textContent = unsupported;
                return;
            }
            const media = document.createElement(info.kind === 'audio' ? 'audio' : 'video');
            media.controls = true;
            media.preload = 'metadata';
            media.src = url;
            panel.append(media);
            cleanups.push(() => detach(media));
            const start = numeric('startMs', '1'), end = numeric('endMs', '1');
            start.max = end.max = String(info.durationMs);
            const update = () => { if (start.value !== '' && end.value !== '')
                publish({ ...timeSpan(id, start.valueAsNumber, end.valueAsNumber) }); };
            start.addEventListener('input', update);
            end.addEventListener('input', update);
            for (const [name, input] of [['Set start from playhead', start], ['Set end from playhead', end]]) {
                const button = document.createElement('button');
                button.type = 'button';
                button.textContent = name;
                button.addEventListener('click', () => { input.value = String(Math.round(media.currentTime * 1000)); update(); });
                panel.append(button);
            }
            const note = document.createElement('p');
            note.textContent = `Source duration: ${info.durationMs} ms. Selections use actual playback milliseconds; no FPS or frame-count conversion.`;
            panel.append(note);
            return;
        }
        if (!info.width || !info.height || info.kind !== 'image') {
            unsupported = 'unsupportedInteraction: point/box drawing requires a decoded source image fixture; extract a real frame explicitly for video.';
            notice.textContent = unsupported;
            return;
        }
        const image = document.createElement('img');
        image.src = url;
        image.alt = 'Source pixel selection';
        image.draggable = false;
        panel.append(image);
        cleanups.push(() => detach(image));
        const x = numeric('x'), y = numeric('y'), width = widget === 'box' ? numeric('width') : undefined, height = widget === 'box' ? numeric('height') : undefined;
        const update = () => { const controls = [x, y, ...(width && height ? [width, height] : [])]; if (controls.every(input => input.value !== ''))
            publish(width && height ? { mediaId: id, x: x.valueAsNumber, y: y.valueAsNumber, width: width.valueAsNumber, height: height.valueAsNumber } : { mediaId: id, x: x.valueAsNumber, y: y.valueAsNumber }); };
        for (const control of [x, y, width, height])
            control?.addEventListener('input', update);
        let origin;
        image.addEventListener('pointerdown', event => { origin = [event.clientX, event.clientY]; image.setPointerCapture(event.pointerId); });
        image.addEventListener('pointercancel', () => { origin = undefined; });
        image.addEventListener('pointerup', event => {
            if (!origin)
                return;
            try {
                const value = widget === 'box' ? pixelBox(id, origin, [event.clientX, event.clientY], image.getBoundingClientRect(), info.width, info.height) : pixelPoint(id, event.clientX, event.clientY, image.getBoundingClientRect(), info.width, info.height);
                x.value = String(value.x);
                y.value = String(value.y);
                if ('width' in value && 'height' in value && width && height) {
                    width.value = String(value.width);
                    height.value = String(value.height);
                }
                publish({ ...value });
                notice.textContent = `${info.width} × ${info.height} source pixels.`;
            }
            catch (error) {
                notice.textContent = error instanceof Error ? error.message : 'Selection could not be drawn.';
            }
            finally {
                origin = undefined;
            }
        });
    };
    sourceSelect.addEventListener('change', () => { if (sourceSelect.value && typeof object.mediaId === 'string')
        publish({ ...object, mediaId: sourceSelect.value }); redraw(); });
    redraw();
    return { element, unsupported: () => unsupported, dispose: () => { for (const cleanup of cleanups)
            cleanup(); cleanups = []; element.remove(); } };
}
