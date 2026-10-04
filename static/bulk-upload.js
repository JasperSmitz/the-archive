// One native File per request; no batch byte buffers, background jobs, or automatic retries.
const MAX_FILES = 100;
const TIMEOUT_MS = 120000;
const successful = state => state === "uploaded" || state === "duplicate";
const uncertain = message => ({ state: "unconfirmed", message, halt: true });
const safeId = id => typeof id === "string" && /^[1-9][0-9]{0,18}$/.test(id)
    && (id.length < 19 || id <= "9223372036854775807");

export class UploadQueue {
    constructor(send, changed = () => {}) {
        this.send = send;
        this.changed = changed;
        this.files = [];
        this.running = false;
        this.started = false;
        this.stopping = false;
        this.notice = "Choose files and shared metadata, then start.";
    }
    addFiles(files) {
        if (this.running || this.started) return false;
        if (files.length + this.files.length > MAX_FILES) {
            this.notice = "Choose at most 100 files in this batch. No files from that selection were added.";
            this.changed();
            return false;
        }
        this.files.push(...Array.from(files, file => ({ file, state: "pending", message: "" })));
        this.changed();
        return true;
    }
    remove(entry) {
        if (this.running || this.started) return;
        this.files = this.files.filter(item => item !== entry);
        this.changed();
    }
    reset() {
        if (this.running) return;
        this.files = [];
        this.started = false;
        this.notice = "Choose files for a new batch. Previously archived images remain saved.";
        this.changed();
    }
    stop() {
        if (!this.running) return;
        this.stopping = true;
        this.notice = "Stopping after the current upload settles. Remaining files stay here.";
        this.changed();
    }
    async run(mode, metadata) {
        if (this.running) return;
        const candidates = this.files.filter(item => mode === "retry"
            ? item.state === "failed" || item.state === "unconfirmed" : item.state === "pending");
        if (!candidates.length) return;
        const snapshot = Object.freeze({ ...metadata, characters: Object.freeze([...metadata.characters]) });
        // Set synchronously, before any await: repeated clicks cannot start another run.
        this.running = true;
        this.started = true;
        this.stopping = false;
        this.notice = "Uploading sequentially. Shared metadata is fixed for this run.";
        this.changed();
        try {
            for (const item of candidates) {
                if (this.stopping) break;
                item.state = "uploading";
                item.message = "Waiting for confirmation…";
                this.changed();
                let outcome;
                try {
                    outcome = await this.send(item.file, snapshot);
                } catch {
                    outcome = uncertain("The response was lost. This image may already be archived. Retry explicitly when the service is available.");
                }
                item.state = outcome.state;
                item.id = outcome.id;
                item.message = outcome.message;
                if (outcome.halt) {
                    this.stopping = true;
                    this.notice = outcome.message + " No further files were started. Keep this page open; explicitly resume pending files or retry failed/unconfirmed files.";
                }
                this.changed();
            }
            if (!this.stopping) this.notice = "Run finished. Check each outcome below; retry failures explicitly or start pending files.";
            else if (!this.notice.includes("No further files")) this.notice = "Stopped. The in-flight upload has settled. Explicitly resume pending files or retry failed/unconfirmed files.";
        } finally {
            this.running = false;
            this.changed();
        }
    }
}

export async function sendFile(file, metadata, fetcher = globalThis.fetch) {
    const form = new FormData();
    form.append("uploader", metadata.uploader);
    form.append("artist", metadata.artist);
    form.append("source_url", metadata.source_url);
    for (const id of metadata.characters) form.append("characters", id);
    form.append("file", file, file.name);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
    try {
        const response = await fetcher("/images", {
            method: "POST", body: form, credentials: "same-origin",
            headers: { Accept: "application/json" }, signal: controller.signal,
        });
        // A redirect/login page is not a confirmation, even when it ends in 200.
        if (response.redirected || response.type === "opaqueredirect") {
            return uncertain("The upload redirected instead of confirming. Sign in again in a new tab; this image may already be archived.");
        }
        // These may be plain-text responses from middleware, before the handler runs.
        if (response.status === 401 || response.status === 403) {
            return { state: "failed", halt: true, message: response.status === 401
                ? "Sign in again in a new tab, then retry. Your session is missing or expired."
                : "Access was rejected. Check your session and the configured application origin before retrying." };
        }
        if (response.status >= 500) return uncertain("The archive is unavailable or in maintenance. This upload is unconfirmed; retry when the service returns.");
        if (response.status === 413) return { state: "failed", message: "File/request exceeds the upload limit (20 MiB per file). Choose a smaller file." };
        const contentType = (response.headers.get("content-type") || "").split(";", 1)[0].trim().toLowerCase();
        if (contentType !== "application/json") return uncertain("The server returned HTML or another unexpected response instead of confirming. Check your login and service availability before retrying.");
        // The endpoint's bounded JSON is small; limit even an unexpected proxy response.
        const reader = response.body.getReader();
        const chunks = [];
        let size = 0;
        try {
            while (true) {
                const { done, value } = await reader.read();
                if (done) break;
                size += value.length;
                if (size > 8192) {
                    await reader.cancel();
                    return uncertain("The server returned an oversized confirmation. Retry explicitly; this image may already be archived.");
                }
                chunks.push(value);
            }
        } finally { reader.releaseLock(); }
        const bytes = new Uint8Array(size);
        let offset = 0;
        for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
        const body = JSON.parse(new TextDecoder().decode(bytes));
        if (response.ok && successful(body.status) && safeId(body.id)) {
            return { state: body.status, id: body.id, message: body.status === "duplicate"
                ? "Already archived. Existing attribution, metadata, and characters were kept; use Edit to change them."
                : "Archived with this run’s shared metadata and characters." };
        }
        if ((response.status === 422 || response.status === 409) && body.status === "failed" && typeof body.message === "string") {
            return { state: "failed", message: body.message.slice(0, 1000) };
        }
        return uncertain("No valid upload confirmation was received. Check service availability before retrying; the image may already be archived.");
    } catch {
        return uncertain("The connection failed, timed out, or the response could not be read. This image may already be archived; retry explicitly when the service is available.");
    } finally { clearTimeout(timer); }
}

function mount() {
    const section = document.getElementById("bulk-controls");
    if (!section) return null;
    const get = id => document.getElementById(id);
    const form = get("bulk-form"), picker = get("bulk-files"), metadata = get("bulk-metadata");
    const start = get("bulk-start"), retry = get("bulk-retry"), stop = get("bulk-stop"), reset = get("bulk-reset");
    const results = get("bulk-results");
    const queue = new UploadQueue(sendFile, () => {
        metadata.disabled = queue.running;
        picker.disabled = queue.running || queue.started;
        start.disabled = queue.running || !queue.files.some(item => item.state === "pending");
        retry.disabled = queue.running || !queue.files.some(item => item.state === "failed" || item.state === "unconfirmed");
        stop.disabled = !queue.running || queue.stopping;
        reset.disabled = queue.running || !queue.files.length;
        get("bulk-notice").textContent = queue.notice;
        const counts = { pending: 0, uploading: 0, uploaded: 0, duplicate: 0, failed: 0, unconfirmed: 0 };
        for (const item of queue.files) counts[item.state]++;
        const completed = counts.uploaded + counts.duplicate + counts.failed + counts.unconfirmed;
        get("bulk-summary").textContent = `Completed ${completed} / ${queue.files.length}: ${counts.uploaded} uploaded, ${counts.duplicate} duplicate, ${counts.failed} failed, ${counts.unconfirmed} unconfirmed, ${counts.pending} pending, ${counts.uploading} uploading.`;
        get("bulk-progress").max = Math.max(queue.files.length, 1);
        get("bulk-progress").value = completed;
        results.replaceChildren();
        for (const item of queue.files) {
            const row = document.createElement("tr");
            for (const value of [item.file.name, `${(item.file.size / (1024 * 1024)).toFixed(2)} MiB`, item.state]) {
                const cell = document.createElement("td");
                cell.textContent = value;
                row.append(cell);
            }
            const detail = document.createElement("td");
            const explanation = document.createElement("p");
            explanation.textContent = item.message;
            detail.append(explanation);
            if (successful(item.state) && safeId(item.id)) {
                for (const [text, path] of [[`View image #${item.id}`, `/images/${item.id}${item.state === "duplicate" ? "?duplicate=1" : ""}`], ["Edit metadata / characters", `/images/${item.id}/edit`]]) {
                    const link = document.createElement("a");
                    link.textContent = text;
                    link.href = path;
                    link.target = "_blank";
                    link.rel = "noopener";
                    detail.append(link, document.createElement("br"));
                }
            }
            row.append(detail);
            const selection = document.createElement("td");
            if (!queue.started) {
                const remove = document.createElement("button");
                remove.type = "button";
                remove.textContent = "Remove";
                remove.disabled = queue.running;
                remove.setAttribute("aria-label", `Remove ${item.file.name}`);
                remove.addEventListener("click", () => queue.remove(item));
                selection.append(remove);
            }
            row.append(selection);
            results.append(row);
        }
    });
    const run = mode => {
        if (queue.running || !form.reportValidity()) return;
        const values = new FormData(form);
        void queue.run(mode, { uploader: values.get("uploader"), artist: values.get("artist"),
            source_url: values.get("source_url"), characters: values.getAll("characters") });
    };
    form.addEventListener("submit", event => { event.preventDefault(); run("pending"); });
    retry.addEventListener("click", () => run("retry"));
    stop.addEventListener("click", () => queue.stop());
    picker.addEventListener("change", () => { queue.addFiles(picker.files); picker.value = ""; });
    reset.addEventListener("click", () => {
        if (queue.running) return;
        if (queue.files.some(item => !successful(item.state)) && !window.confirm("Discard this page’s remaining selections/results and start a new batch? Archived images stay saved.")) return;
        queue.reset();
    });
    window.addEventListener("beforeunload", event => {
        if (queue.files.some(item => !successful(item.state))) {
            event.preventDefault();
            event.returnValue = "";
        }
    });
    queue.changed();
    section.hidden = false;
    return queue;
}

export const batch = mount();
