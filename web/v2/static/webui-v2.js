document.addEventListener('DOMContentLoaded', function () {
    if (!window.htmx) {
        throw new Error('HTMX is required for webui-v2.js but was not found on window.htmx.');
    }

    setupPwaServiceWorker();
    setupTaskStatusEvents();
    setupTaskProgress();
    setupSearchShortcut();
    setupSearchClearButton();
    setupSearchEndpointRouting();
    setupCaptureExpandToggle(document);
    setupRelatedCapturesGallery(document);
    setupSearchClearOnSlashCommand();
    setupAnnotationEditorCaret(document);
    setupReilluminate();
    setupUploadInteractions();

    // htmx:load fires for content inserted by a swap (including the outerHTML
    // single-card SSE refresh) and for the initial body, so this is where
    // delegated behavior for freshly rendered cards belongs.
    window.htmx.onLoad(function (node) {
        setupCaptureExpandToggle(node);
    });

    document.body.addEventListener('htmx:afterSwap', function (e) {
        const targetId = e.target && e.target.id ? e.target.id : '';

        if (targetId === 'card-feed' || targetId.startsWith('capture-card-') || targetId.startsWith('capture-illumination-')) {
            if (window.dreamscrollTaskProgress) {
                window.dreamscrollTaskProgress.refresh();
            }
        }

        if (targetId.startsWith('capture-illumination-')) {
            const region = document.getElementById(targetId);
            const card = region && region.closest('.capture-card');
            if (card && card.classList.contains('is-expanded')) {
                const toggle = card.querySelector('.capture-card__details-toggle');
                if (toggle) {
                    toggle.textContent = 'Less';
                    toggle.setAttribute('aria-expanded', 'true');
                }
                const panel = card.querySelector('.capture-card__details-panel');
                if (panel) panel.inert = false;
            }
        }

        if (targetId.startsWith('annotation-block-')) {
            setupAnnotationEditorCaret(e.target);
        }
    });

});

const taskStatusHandlers = new Map();

function taskStatusHandlerKey(entityType, taskType) {
    return entityType + ':' + taskType;
}

// Task status is a generic signal. Register one handler per
// (entity_type, task_type); the receipt/dispatch path must not assume
// illumination or captures.
function registerTaskStatusHandler(entityType, taskType, handler) {
    taskStatusHandlers.set(taskStatusHandlerKey(entityType, taskType), handler);
}

function dispatchTaskStatus(update) {
    if (!update || typeof update.entity_type !== 'string') {
        return;
    }
    if (!update.payload || typeof update.payload.task_type !== 'string') {
        return;
    }
    const handler = taskStatusHandlers.get(
        taskStatusHandlerKey(update.entity_type, update.payload.task_type)
    );
    if (handler) {
        handler(update);
    }
}

function setupTaskStatusEvents() {
    if (!window.EventSource) {
        console.info('Dreamscroll SSE unavailable: this browser has no EventSource support.');
        return;
    }

    const pageKind = document.body ? document.body.dataset.pageKind : '';

    function handleCaptureIlluminationTaskStatus(update) {
        if (!Number.isInteger(update.entity_id)) {
            return;
        }
        const captureId = String(update.entity_id);
        if (window.dreamscrollTaskProgress) {
            window.dreamscrollTaskProgress.accept(update);
        }
        document.dispatchEvent(new CustomEvent('dreamscroll:capture-illumination-status', {
            detail: update
        }));
        if (!isRefreshWorthyTaskStatus(update.payload.status)) {
            return;
        }

        if (pageKind === 'capture_detail') {
            if (document.body.dataset.captureId !== captureId) return;
            refreshDetailIllumination(captureId, update.timestamp);
            return;
        }

        if (pageKind === 'feed') {
            refreshFeedIllumination(captureId, update.timestamp);
        }
    }

    function refreshFeedIllumination(captureId, eventTimestamp) {
        const region = document.getElementById('capture-illumination-' + captureId);
        if (!region || !isNewerThanSnapshot(eventTimestamp, region.dataset.snapshotAt)) return;
        window.htmx.ajax('GET', '/feed/capture/' + captureId + '/illumination?animate_illumination=true', {
            source: region,
            target: region,
            swap: 'outerHTML'
        });
    }

    function refreshDetailIllumination(captureId, eventTimestamp) {
        const region = document.getElementById('capture-illumination-' + captureId);
        if (!region || !isNewerThanSnapshot(eventTimestamp, region.dataset.snapshotAt)) return;
        window.htmx.ajax('GET', '/capture/' + captureId + '/illumination?animate_illumination=true', {
            source: region,
            target: region,
            swap: 'outerHTML'
        });
    }

    registerTaskStatusHandler('capture', 'illuminate', handleCaptureIlluminationTaskStatus);

    function isNewerThanSnapshot(eventTimestamp, snapshotTimestamp) {
        if (typeof eventTimestamp !== 'string' || typeof snapshotTimestamp !== 'string') return false;
        const eventTime = parseUtcTimestampNanoseconds(eventTimestamp);
        const snapshotTime = parseUtcTimestampNanoseconds(snapshotTimestamp);
        return eventTime !== null && snapshotTime !== null && eventTime > snapshotTime;
    }

    function parseUtcTimestampNanoseconds(timestamp) {
        const match = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d+))?(?:Z|\+00:00)$/.exec(timestamp);
        if (!match) return null;

        const second = Date.parse(match[1] + 'Z');
        if (!Number.isFinite(second)) return null;
        const fractionalNanoseconds = (match[2] || '').slice(0, 9).padEnd(9, '0');
        return BigInt(Math.trunc(second / 1000)) * 1000000000n
            + BigInt(fractionalNanoseconds || '0');
    }

    function isRefreshWorthyTaskStatus(status) {
        if (!status || typeof status.name !== 'string') return false;
        return status.name === 'error_will_retry'
            || status.name === 'complete_success'
            || status.name === 'complete_failure';
    }

    // Every rendered capture, not only unilluminated or rerun-eligible cards: a
    // capture with an existing illumination can have a run in flight, and the
    // page that started it may be gone. Catch-up is how the client learns that.
    function captureIdsForCatchup() {
        const captureIds = new Set();
        document.querySelectorAll('[data-capture-id]').forEach(function (card) {
            if (card.dataset.captureId) {
                captureIds.add(card.dataset.captureId);
            }
        });
        return Array.from(captureIds);
    }

    function eventsUrlForConnection() {
        const captureIds = captureIdsForCatchup();
        if (captureIds.length === 0) return '/events';
        const params = new URLSearchParams();
        params.set('capture_ids', captureIds.join(','));
        return '/events?' + params.toString();
    }

    const activeWindowMs = 2 * 60 * 1000; // Close SSE after 2 min without input
    const heartbeatTimeoutMs = 50 * 1000; // 2.5 x 20-second heartbeat intervals
    let idleTimer = null;
    let foregroundTimer = null;
    let heartbeatTimer = null;
    let lastHeartbeatAt = null;
    let progressWatchTimer = null;
    let source = null;
    let lastActivityAt = Date.now();

    function isRecentlyActive() {
        return Date.now() - lastActivityAt < activeWindowMs;
    }

    function clearHeartbeatTimer() {
        if (heartbeatTimer !== null) {
            window.clearTimeout(heartbeatTimer);
            heartbeatTimer = null;
        }
    }

    function heartbeatIsStale() {
        return lastHeartbeatAt !== null && Date.now() - lastHeartbeatAt >= heartbeatTimeoutMs;
    }

    function closeSource() {
        if (!source) return;
        const oldSource = source;
        source = null;
        clearHeartbeatTimer();
        lastHeartbeatAt = null;
        oldSource.close();
    }

    function connectNow() {
        if (source || !isRecentlyActive()) return;

        source = new EventSource(eventsUrlForConnection(), { withCredentials: true });
        const currentSource = source;
        currentSource.addEventListener('open', function () {
            if (source !== currentSource) return;
            lastHeartbeatAt = Date.now();
            armHeartbeatTimeout(currentSource);
            armIdleClose();
        });
        currentSource.addEventListener('heartbeat', function () {
            if (source !== currentSource) return;
            lastHeartbeatAt = Date.now();
            armHeartbeatTimeout(currentSource);
        });
        currentSource.addEventListener('task-status', function (event) {
            if (source !== currentSource) return;
            let update;
            try {
                update = JSON.parse(event.data);
            } catch (_error) {
                console.error('Ignoring malformed Dreamscroll SSE payload.');
                return;
            }
            dispatchTaskStatus(update);
        });
        currentSource.onerror = function () {
            if (source !== currentSource) return;
            if (currentSource.readyState === EventSource.CLOSED) {
                closeSource();
            }
        };
    }

    function armHeartbeatTimeout(currentSource) {
        clearHeartbeatTimer();
        const remainingMs = Math.max(0, heartbeatTimeoutMs - (Date.now() - lastHeartbeatAt));
        heartbeatTimer = window.setTimeout(function () {
            heartbeatTimer = null;
            if (source !== currentSource || currentSource.readyState !== EventSource.OPEN) return;
            if (!heartbeatIsStale()) {
                armHeartbeatTimeout(currentSource);
                return;
            }
            closeSource();
            connectNow();
        }, remainingMs);
    }

    function handleForeground() {
        if (foregroundTimer !== null) return;
        foregroundTimer = window.setTimeout(function () {
            foregroundTimer = null;
            if (document.visibilityState === 'hidden') return;
            recordActivity('foregrounded');
        }, 0);
    }

    function clearIdleTimer() {
        if (idleTimer !== null) {
            window.clearTimeout(idleTimer);
            idleTimer = null;
        }
    }

    function armIdleClose() {
        clearIdleTimer();
        const idleRemainingMs = Math.max(0, activeWindowMs - (Date.now() - lastActivityAt));
        idleTimer = window.setTimeout(function () {
            idleTimer = null;
            if (isRecentlyActive()) {
                armIdleClose();
                return;
            }
            closeSource();
        }, idleRemainingMs);
    }

    function recordActivity(reason = 'user activity') {
        lastActivityAt = Date.now();
        armIdleClose();
        if (source && source.readyState === EventSource.OPEN && heartbeatIsStale()) {
            closeSource();
        }
        if (!source) {
            console.debug('Dreamscroll SSE reconnect requested by activity.', { reason });
            connectNow();
        }
    }

    // Armed only while a progress bar exists; the timer clears itself once the
    // last state is gone. A visible bar is a reason to keep status flowing, so
    // reconnect if the stream expired or a restart killed it.
    function watchProgress() {
        if (progressWatchTimer !== null) return;
        progressWatchTimer = window.setInterval(function () {
            if (!window.dreamscrollTaskProgress || !window.dreamscrollTaskProgress.hasActive()) {
                window.clearInterval(progressWatchTimer);
                progressWatchTimer = null;
                return;
            }
            recordActivity('progress watchdog');
        }, 10000);
    }

    window.dreamscrollTaskStatusEvents = {
        ensureConnected: recordActivity,
        watchProgress: watchProgress
    };

    // pointerover counts meaningful hover/entry without the high event rate of pointermove.
    ['pointerdown', 'pointerover', 'keydown', 'touchstart', 'wheel'].forEach(function (eventName) {
        window.addEventListener(eventName, function () {
            recordActivity(eventName);
        }, { passive: true });
    });
    document.addEventListener('visibilitychange', function () {
        if (document.visibilityState === 'visible') handleForeground();
    });
    window.addEventListener('focus', handleForeground);
    window.addEventListener('pageshow', function (event) {
        if (event.persisted) handleForeground();
    });
    lastActivityAt = Date.now();
    armIdleClose();
    connectNow();
}

// Client-side estimated progress, scoped to each capture card. The percentage
// represents elapsed time against the server's empirical p50 estimate; it is
// not a measurement of work completed inside the AI task.
function setupTaskProgress() {
    const states = new Map();

    function elementsFor(captureId) {
        const card = document.getElementById('capture-card-' + captureId);
        if (!card) return null;
        const wrap = card.querySelector('[data-capture-task-progress]');
        if (!wrap) return null;
        return {
            wrap: wrap,
            bar: wrap.querySelector('.capture-card__task-progress-bar'),
            text: wrap.querySelector('.capture-card__task-progress-text')
        };
    }

    function show(captureId, label, indeterminate, showBar, percent) {
        const elements = elementsFor(captureId);
        if (!elements || !elements.bar || !elements.text) return;
        if (window.dreamscrollTaskStatusEvents) {
            window.dreamscrollTaskStatusEvents.watchProgress();
        }
        elements.wrap.hidden = false;
        elements.wrap.classList.toggle('is-indeterminate', indeterminate);
        elements.wrap.classList.toggle('is-message-only', !showBar);
        elements.bar.style.width = indeterminate ? '' : percent + '%';
        elements.text.textContent = label;
    }

    function stopTimer(state) {
        if (state.progressTimer) {
            window.clearInterval(state.progressTimer);
            state.progressTimer = null;
        }
    }

    function hideSoon(captureId, state) {
        stopTimer(state);
        if (state.hideTimer) window.clearTimeout(state.hideTimer);
        state.hideTimer = window.setTimeout(function () {
            const elements = elementsFor(captureId);
            if (elements) {
                elements.wrap.hidden = true;
                elements.wrap.classList.remove('is-indeterminate');
            }
            states.delete(captureId);
        }, 1800);
    }

    function animateEstimatedProgress(captureId, state) {
        if (!state.processingStartedAt || !state.estimatedDurationMs) return;
        stopTimer(state);
        const update = function () {
            const elapsed = Math.max(0, Date.now() - state.processingStartedAt);
            state.percent = Math.max(state.percent,
                Math.min(90, (elapsed / state.estimatedDurationMs) * 100));
            show(captureId, estimatedProgressLabel(state, elapsed),
                false, true, state.percent);
        };
        update();
        state.progressTimer = window.setInterval(update, 150);
    }

    function estimatedProgressLabel(state, elapsed) {
        const ratio = elapsed / state.estimatedDurationMs;
        if (ratio <= 0.9) {
            const remaining = state.estimatedDurationMs - elapsed;
            return 'Illuminating (est. ' + formatDuration(remaining) + ')';
        }
        if (ratio <= 1.2) {
            return 'Illuminating...';
        }
        return 'Illuminating (longer than est.)...';
    }

    function formatDuration(durationMs) {
        return Math.max(1, Math.ceil(durationMs / 1000)) + 's';
    }

    function renderState(captureId, state) {
        if (state.status === 'queued') {
            show(captureId, 'Queued for illumination...', true, false);
        } else if (state.status === 'in_progress') {
            if (state.estimatedDurationMs > 0) {
                if (state.progressTimer === null) animateEstimatedProgress(captureId, state);
                else show(captureId, estimatedProgressLabel(state,
                    Math.max(0, Date.now() - state.processingStartedAt)),
                    false, true, state.percent);
            } else {
                show(captureId, 'Illuminating...', true, true);
            }
        } else if (state.status === 'error_will_retry') {
            show(captureId, 'Retrying illumination...', true, false);
        } else if (state.status === 'complete_failure') {
            show(captureId, 'Illumination failed.', false, false);
        }
    }

    window.dreamscrollTaskProgress = {
        start: function (newCaptureId) {
            const captureId = String(newCaptureId);
            const existingState = states.get(captureId);
            if (existingState) {
                renderState(captureId, existingState);
                return;
            }
            const state = {
                percent: 0,
                status: 'queued',
                processingStartedAt: null,
                estimatedDurationMs: null,
                progressTimer: null,
                hideTimer: null
            };
            states.set(captureId, state);
            show(captureId, 'Queued for illumination...', true, false);
        },
        accept: function (update) {
            if (!update || String(update.entity_id) === 'undefined') return;
            const captureId = String(update.entity_id);
            const name = update.payload && update.payload.status && update.payload.status.name;
            let state = states.get(captureId);

            // Catch-up includes the latest status for every capture. Terminal
            // statuses should not create transient UI during a page load.
            if (!state && (name === 'complete_success' || name === 'complete_failure')) {
                return;
            }

            if (!state && name !== 'queued' && name !== 'in_progress' && name !== 'error_will_retry') return;

            if (!state) {
                state = {
                    percent: 0,
                    status: null,
                    processingStartedAt: null,
                    estimatedDurationMs: null,
                    progressTimer: null,
                    hideTimer: null
                };
                states.set(captureId, state);
            }
            state.status = name;
            if (name === 'queued') {
                show(captureId, 'Queued for illumination...', true, false);
            } else if (name === 'in_progress') {
                state.processingStartedAt = Date.parse(update.payload.processing_started_at || '') || Date.now();
                state.estimatedDurationMs = Number(update.payload.estimated_duration_ms_p50);
                if (Number.isFinite(state.estimatedDurationMs) && state.estimatedDurationMs > 0) {
                    animateEstimatedProgress(captureId, state);
                } else {
                    show(captureId, 'Illuminating...', true, true);
                }
            } else if (name === 'error_will_retry') {
                stopTimer(state);
                show(captureId, 'Retrying illumination...', true, false);
            } else if (name === 'complete_success') {
                stopTimer(state);
                const elements = elementsFor(captureId);
                if (elements) {
                    elements.wrap.hidden = true;
                    elements.wrap.classList.remove('is-indeterminate', 'is-message-only');
                }
                states.delete(captureId);
            } else if (name === 'complete_failure') {
                stopTimer(state);
                show(captureId, 'Illumination failed.', false, false);
                hideSoon(captureId, state);
            }
        },
        refresh: function () {
            states.forEach(function (state, captureId) {
                renderState(captureId, state);
            });
        },
        hasActive: function () {
            return states.size > 0;
        }
    };
}

function setupReilluminate() {
    document.addEventListener('click', function (event) {
        const button = event.target.closest('[data-reilluminate]');
        if (!button || button.disabled) return;
        event.preventDefault();
        reilluminate(button);
    });

    // The footer note is not part of the SSE illumination swap, so terminal task
    // statuses re-enable (or drop) the button here.
    document.addEventListener('dreamscroll:capture-illumination-status', function (event) {
        const update = event.detail;
        if (!update || !update.payload || !update.payload.status) return;
        const status = update.payload.status.name;
        const selector = '[data-reilluminate][data-capture-id="' + update.entity_id + '"]';

        if (status === 'queued' || status === 'in_progress' || status === 'error_will_retry') {
            document.querySelectorAll(selector).forEach(function (button) {
                button.disabled = true;
                button.setAttribute('aria-busy', 'true');
            });
            return;
        }

        document.querySelectorAll(selector).forEach(function (button) {
            // A retry button exists only because there was no illumination. Once one
            // lands, the action no longer makes sense; a note-backed button stays.
            if (status === 'complete_success' && button.hasAttribute('data-reilluminate-retry')) {
                button.remove();
                return;
            }
            button.disabled = false;
            button.removeAttribute('aria-busy');
        });
    });
}

async function reilluminate(button) {
    const captureId = button.dataset.captureId;
    button.disabled = true;
    button.setAttribute('aria-busy', 'true');

    try {
        const response = await fetch('/capture/' + encodeURIComponent(captureId) + '/illuminate', {
            method: 'POST',
            credentials: 'same-origin',
            headers: { Accept: 'application/json' }
        });
        // 409 means a run is already in flight; stay attached to it.
        if (response.ok || response.status === 409) {
            if (window.dreamscrollTaskProgress) {
                window.dreamscrollTaskProgress.start(captureId);
            }
            return;
        }
        throw new Error('re-illuminate request failed: ' + response.status);
    } catch (_error) {
        button.disabled = false;
        button.removeAttribute('aria-busy');
        button.title = 'Could not start illumination. Try again.';
    }
}

function setupAnnotationEditorCaret(rootNode) {
    const editors = [];
    if (rootNode && rootNode.matches && rootNode.matches('.capture-card__annotation-input')) {
        editors.push(rootNode);
    }
    if (rootNode && rootNode.querySelectorAll) {
        rootNode.querySelectorAll('.capture-card__annotation-input').forEach(function (node) {
            editors.push(node);
        });
    }

    editors.forEach(function (editor) {
        if (editor.dataset.caretBound === 'true') {
            return;
        }
        editor.dataset.caretBound = 'true';

        window.requestAnimationFrame(function () {
            const len = editor.value.length;
            editor.focus({ preventScroll: true });
            editor.setSelectionRange(len, len);
        });
    });
}

function setupRelatedCapturesGallery(rootNode) {
    const galleries = [];
    if (rootNode.matches && rootNode.matches('[data-related-captures]')) {
        galleries.push(rootNode);
    }
    if (rootNode.querySelectorAll) {
        rootNode.querySelectorAll('[data-related-captures]').forEach(function (node) {
            galleries.push(node);
        });
    }

    galleries.forEach(function (gallery) {
        if (gallery.dataset.relatedCapturesBound === 'true') {
            return;
        }

        const grid = gallery.querySelector('.related-captures__grid');
        const emptyState = gallery.querySelector('.related-captures__empty');
        const previewsUrl = gallery.dataset.previewsUrl;
        if (!grid || !emptyState || !previewsUrl) {
            return;
        }

        gallery.dataset.relatedCapturesBound = 'true';
        loadRelatedCaptures(gallery, grid, emptyState, previewsUrl);
    });
}

async function loadRelatedCaptures(gallery, grid, emptyState, previewsUrl) {
    gallery.setAttribute('aria-busy', 'true');

    let captures;
    try {
        const response = await fetch(previewsUrl, {
            credentials: 'same-origin',
            headers: { Accept: 'application/json' }
        });
        if (!response.ok) {
            throw new Error('Related previews request failed: ' + response.status);
        }
        captures = await response.json();
    } catch (_error) {
        emptyState.textContent = 'Could not load related captures.';
        emptyState.hidden = false;
        grid.replaceChildren();
        gallery.removeAttribute('aria-busy');
        return;
    }

    gallery.removeAttribute('aria-busy');
    if (!Array.isArray(captures)) {
        captures = [];
    }

    const images = captures
        .map(function (capture) {
            if (!capture || !capture.id || typeof capture.thumbnail_url !== 'string' || !capture.thumbnail_url) {
                return null;
            }
            return {
                id: capture.id,
                thumbnail_url: capture.thumbnail_url,
                summary: typeof capture.summary === 'string' ? capture.summary : ''
            };
        })
        .filter(function (image) {
            return image && image.id && image.thumbnail_url;
        });

    function getColumnCount() {
        const raw = getComputedStyle(grid).getPropertyValue('--related-captures-cols').trim();
        const parsed = Number.parseInt(raw, 10);
        if (!Number.isFinite(parsed) || parsed < 1) {
            return 1;
        }
        return parsed;
    }

    function renderColumns() {
        if (images.length === 0) {
            emptyState.textContent = 'No related captures found.';
            emptyState.hidden = false;
            grid.replaceChildren();
            return;
        }

        emptyState.hidden = true;

        const columnCount = getColumnCount();
        const buckets = Array.from({ length: columnCount }, function () {
            return [];
        });

        images.forEach(function (image, idx) {
            buckets[idx % columnCount].push(image);
        });

        const fragment = document.createDocumentFragment();
        buckets.forEach(function (bucket) {
            const column = document.createElement('div');
            column.className = 'related-captures__column';

            bucket.forEach(function (image) {
                const link = document.createElement('a');
                link.className = 'related-captures__item';
                link.href = '/capture/' + String(image.id);
                link.setAttribute('aria-label', image.summary || 'Related capture');
                if (image.summary) {
                    link.title = image.summary;
                }

                const img = document.createElement('img');
                img.src = image.thumbnail_url;
                img.alt = 'similar capture image';
                img.loading = 'lazy';

                link.appendChild(img);
                column.appendChild(link);
            });

            fragment.appendChild(column);
        });

        grid.replaceChildren(fragment);
    }

    let lastCount = -1;
    function rerenderIfNeeded() {
        const count = getColumnCount();
        if (count !== lastCount) {
            lastCount = count;
            renderColumns();
        }
    }

    let resizeTimer = null;
    window.addEventListener('resize', function () {
        if (resizeTimer !== null) {
            window.clearTimeout(resizeTimer);
        }
        resizeTimer = window.setTimeout(rerenderIfNeeded, 120);
    });

    rerenderIfNeeded();
}

function setupPwaServiceWorker() {
    if (!('serviceWorker' in navigator)) {
        return;
    }

    navigator.serviceWorker.register('/sw.js').catch(function (err) {
        console.warn('Service worker registration failed:', err);
    });
}

function isEditableTarget(node) {
    if (!node || !node.tagName) {
        return false;
    }

    const tagName = node.tagName.toUpperCase();
    if (tagName === 'TEXTAREA') {
        return true;
    }

    if (tagName === 'INPUT') {
        const inputType = (node.getAttribute('type') || 'text').toLowerCase();
        return inputType !== 'checkbox' && inputType !== 'radio' && inputType !== 'button' && inputType !== 'submit';
    }

    return !!node.isContentEditable;
}

function setupSearchShortcut() {
    const searchInput = document.getElementById('header-search-input');
    if (!searchInput) {
        return;
    }

    document.addEventListener('keydown', function (e) {
        if (e.key !== '/') {
            return;
        }

        if (isEditableTarget(e.target)) {
            return;
        }

        if (document.activeElement !== searchInput) {
            e.preventDefault();
            searchInput.focus();
        }
    });
}

function setupSearchClearOnSlashCommand() {
    document.body.addEventListener('ds-clear-search-input', function () {
        const searchInput = document.getElementById('header-search-input');
        const clearButton = document.getElementById('header-search-clear');
        if (!searchInput) {
            return;
        }
        searchInput.value = '';
        if (clearButton) {
            clearButton.classList.add('is-hidden');
        }
    });
}

function setupSearchClearButton() {
    const searchForm = document.getElementById('header-search-form');
    const searchInput = document.getElementById('header-search-input');
    const clearButton = document.getElementById('header-search-clear');
    if (!searchForm || !searchInput || !clearButton) {
        return;
    }

    function syncClearButtonVisibility() {
        if (searchInput.value.trim().length > 0) {
            clearButton.classList.remove('is-hidden');
            return;
        }
        clearButton.classList.add('is-hidden');
    }

    clearButton.addEventListener('click', function () {
        searchInput.value = '';
        syncClearButtonVisibility();

        // Only the feed reloads in place; detail/entity search navigates.
        if (document.body && document.body.dataset.pageKind === 'feed') {
            document.body.dataset.feedContentMode = 'timeline';
            reloadFeedFrame();
        }
    });

    searchInput.addEventListener('input', syncClearButtonVisibility);
    syncClearButtonVisibility();
}

function setupSearchEndpointRouting() {
    const searchForm = document.getElementById('header-search-form');
    const searchInput = document.getElementById('header-search-input');
    if (!searchForm || !searchInput) {
        return;
    }

    searchForm.addEventListener('submit', function (e) {
        const query = searchInput.value.trim();
        if (!query.startsWith('/')) {
            return;
        }

        e.preventDefault();
        window.htmx.ajax('POST', '/command', {
            values: { raw_command: query },
            target: '#card-feed',
            swap: 'none'
        });
    }, true);

    searchForm.addEventListener('submit', function () {
        const query = searchInput.value.trim();
        if (query.startsWith('/')) {
            return;
        }

        // Dismiss mobile keyboards on successful search submit.
        searchInput.blur();
    });

    searchForm.addEventListener('htmx:configRequest', function (e) {
        const state = getCurrentFeedState();

        if (state.query.startsWith('/')) {
            e.preventDefault();
            return;
        }

        if (document.body && document.body.dataset.pageKind === 'feed') {
            document.body.dataset.feedContentMode = state.query ? 'search' : 'timeline';
        }

        e.detail.path = '/feed';
        const parameters = e.detail.parameters || (e.detail.parameters = {});
        applyFeedParameters(parameters, state);
    });
}

function setFeedParameter(parameters, key, value) {
    if (parameters instanceof URLSearchParams) {
        if (value === null || value === '') {
            parameters.delete(key);
            return;
        }

        parameters.set(key, value);
        return;
    }

    if (value === null || value === '') {
        delete parameters[key];
        return;
    }

    parameters[key] = value;
}

function getCurrentFeedState() {
    const searchInput = document.getElementById('header-search-input');

    return {
        limit: currentLimitParam(),
        query: searchInput ? searchInput.value.trim() : ''
    };
}

function applyFeedParameters(parameters, state) {
    setFeedParameter(parameters, 'limit', state.limit);
    setFeedParameter(parameters, 'query', state.query);
}

function setupCaptureExpandToggle(rootNode) {
    rootNode.querySelectorAll('.capture-card__details-toggle').forEach(function (button) {
        if (button.dataset.bound === 'true') {
            return;
        }

        button.dataset.bound = 'true';
        const row = button.closest('.capture-card');
        const panel = row && row.querySelector('.capture-card__details-panel');
        const expanded = !!(row && row.classList.contains('is-expanded'));
        button.setAttribute('aria-expanded', expanded ? 'true' : 'false');
        if (panel) {
            panel.inert = !expanded;
        }

        button.addEventListener('click', function (e) {
            e.preventDefault();
            if (!row) {
                return;
            }

            row.classList.toggle('is-expanded');
            const isExpanded = row.classList.contains('is-expanded');
            this.textContent = isExpanded ? 'Less' : 'More';
            this.setAttribute('aria-expanded', isExpanded ? 'true' : 'false');
            if (panel) {
                panel.inert = !isExpanded;
            }
        });
    });
}

function currentLimitParam() {
    const params = new URLSearchParams(window.location.search || '');
    const limit = params.get('limit');
    if (!limit) {
        return null;
    }
    return limit;
}

function buildFeedUrlFromCurrentState() {
    const state = getCurrentFeedState();
    const params = new URLSearchParams();
    applyFeedParameters(params, state);

    const query = params.toString();
    if (query.length === 0) {
        return '/feed';
    }
    return '/feed?' + query;
}

function reloadFeedFrame() {
    const cardFeed = document.getElementById('card-feed');
    if (!cardFeed) {
        return;
    }

    const url = buildFeedUrlFromCurrentState();
    window.htmx.ajax('GET', url, {
        target: '#card-feed',
        swap: 'innerHTML'
    });
}

function setupUploadInteractions() {
    const filePicker = document.getElementById('file-picker');
    const uploadForm = document.getElementById('file-upload-form');
    const dropZone = document.getElementById('drop-zone-row');
    const progressWrap = document.getElementById('upload-progress');
    const progressBar = document.getElementById('upload-progress-bar');
    const progressText = document.getElementById('upload-progress-text');
    const noticeWrap = document.getElementById('upload-notice');
    const noticeMessage = document.getElementById('upload-notice-message');
    if (!filePicker || !uploadForm || !dropZone || !progressWrap || !progressBar || !progressText) {
        return;
    }

    const uploadLimitBytes = Number(uploadForm.dataset.maxUploadBytes) || 0;

    let isUploading = false;
    let hideNoticeTimer = null;
    let hideNoticeAnimationTimer = null;

    function getFeedContentMode() {
        return document.body ? document.body.dataset.feedContentMode : '';
    }

    function currentSearchQuery() {
        const searchInput = document.getElementById('header-search-input');
        return searchInput ? searchInput.value.trim() : '';
    }

    // Only the unfiltered timeline can absorb a new card without changing context. Search updates
    // the feed in place without rewriting body[data-feed-content-mode], so the query is the source
    // of truth there.
    function shouldRefreshAfterUpload() {
        const pageKind = document.body ? document.body.dataset.pageKind : '';
        return pageKind === 'feed'
            && getFeedContentMode() === 'timeline'
            && currentSearchQuery().length === 0;
    }

    function shouldShowUploadNotice() {
        return !shouldRefreshAfterUpload();
    }

    function showUploadNotice(uploadResult, errorMessage) {
        if (!noticeWrap || !noticeMessage) {
            return;
        }

        if (hideNoticeTimer) {
            window.clearTimeout(hideNoticeTimer);
            hideNoticeTimer = null;
        }

        if (hideNoticeAnimationTimer) {
            window.clearTimeout(hideNoticeAnimationTimer);
            hideNoticeAnimationTimer = null;
        }

        noticeMessage.textContent = '';
        if (uploadResult) {
            const link = document.createElement('a');
            link.href = uploadResult.capture_url;
            link.textContent = 'Capture ' + String(uploadResult.capture_id);
            noticeMessage.appendChild(link);
            noticeMessage.appendChild(document.createTextNode(' successfully uploaded.'));
        } else {
            noticeMessage.textContent = errorMessage || 'Upload failed. Please try again.';
        }

        noticeWrap.hidden = false;
        noticeWrap.classList.remove('is-hiding');
        window.requestAnimationFrame(function () {
            noticeWrap.classList.add('is-visible');
        });
        hideNoticeTimer = window.setTimeout(function () {
            noticeWrap.classList.remove('is-visible');
            noticeWrap.classList.add('is-hiding');
            hideNoticeAnimationTimer = window.setTimeout(function () {
                noticeWrap.hidden = true;
                noticeWrap.classList.remove('is-hiding');
                noticeMessage.textContent = '';
                hideNoticeAnimationTimer = null;
            }, 300);
            hideNoticeTimer = null;
        }, 8000);
    }

    function parseUploadResult(xhr) {
        if (!xhr.responseText) {
            return null;
        }

        try {
            const payload = JSON.parse(xhr.responseText);
            if (!payload || !Number.isInteger(payload.capture_id) || typeof payload.capture_url !== 'string') {
                return null;
            }
            return payload;
        } catch (_err) {
            return null;
        }
    }

    function setUploadProgress(percent, label) {
        const safePercent = Math.max(0, Math.min(100, Math.round(percent)));
        progressBar.style.width = safePercent + '%';
        progressText.textContent = label || (safePercent + '%');
    }

    function setUploadInFlight(inFlight) {
        isUploading = inFlight;

        if (inFlight) {
            uploadForm.classList.add('is-uploading');
            progressWrap.classList.add('is-visible');
            return;
        }

        uploadForm.classList.remove('is-uploading');
        filePicker.value = '';
        progressWrap.classList.remove('is-visible');
        setUploadProgress(0, '0%');
    }

    function isHeic(file) {
        const name = (file.name || '').toLowerCase();
        return file.type === 'image/heic'
            || file.type === 'image/heif'
            || name.endsWith('.heic')
            || name.endsWith('.heif');
    }

    function formatUploadSize(bytes) {
        const mb = bytes / (1024 * 1024);
        if (mb >= 1) {
            return mb.toFixed(1) + ' MB';
        }
        return Math.max(1, Math.round(bytes / 1024)) + ' KB';
    }

    // The server answers 413 without its reason (its error bodies are opaque by policy), and an
    // early 413 can even reach the browser as a Cloud Run 503 while the body is still uploading.
    // The browser knows the file and the limit, so it can say exactly what is wrong.
    function tooLargeNotice(file) {
        return 'That image is ' + formatUploadSize(file.size)
            + '; the limit is ' + formatUploadSize(uploadLimitBytes)
            + '. Save it as a smaller file (JPEG) and upload that.';
    }

    function submitManagedUpload(file) {
        if (!file || isUploading) {
            return;
        }

        if (isHeic(file)) {
            showUploadNotice(
                null,
                'HEIC/HEIF images are not supported. Export as PNG or JPEG and try again.'
            );
            return;
        }

        if (uploadLimitBytes > 0 && file.size > uploadLimitBytes) {
            showUploadNotice(null, tooLargeNotice(file));
            return;
        }

        if (window.dreamscrollTaskStatusEvents) {
            window.dreamscrollTaskStatusEvents.ensureConnected();
        }

        const formData = new FormData();
        formData.append('image', file);

        const xhr = new XMLHttpRequest();
        xhr.open('POST', '/upload');
        xhr.setRequestHeader('HX-Request', 'true');

        setUploadInFlight(true);
        setUploadProgress(0, 'Uploading 0%');

        xhr.upload.addEventListener('progress', function (event) {
            if (!event.lengthComputable) {
                setUploadProgress(85, 'Uploading...');
                return;
            }

            const percent = (event.loaded / event.total) * 100;
            setUploadProgress(percent, 'Uploading ' + Math.round(percent) + '%');
        });

        xhr.addEventListener('load', function () {
            if (xhr.status >= 200 && xhr.status < 300) {
                const uploadResult = parseUploadResult(xhr);

                if (uploadResult && window.dreamscrollTaskProgress) {
                    window.dreamscrollTaskProgress.start(uploadResult.capture_id);
                }

                if (shouldShowUploadNotice() && uploadResult) {
                    showUploadNotice(uploadResult);
                } else if (noticeWrap) {
                    noticeWrap.hidden = true;
                }

                if (shouldRefreshAfterUpload()) {
                    reloadFeedFrame();
                }

                setUploadInFlight(false);
                return;
            }

            setUploadInFlight(false);
            if (xhr.status === 413 || (uploadLimitBytes > 0 && file.size > uploadLimitBytes)) {
                showUploadNotice(null, tooLargeNotice(file));
                return;
            }
            showUploadNotice(null);
        });

        xhr.addEventListener('error', function () {
            setUploadInFlight(false);
            showUploadNotice(null);
        });

        xhr.send(formData);
    }

    uploadForm.addEventListener('submit', function (e) {
        e.preventDefault();
        if (filePicker.files && filePicker.files.length > 0) {
            submitManagedUpload(filePicker.files[0]);
        }
    });

    function clipboardEventImageFile(e) {
        if (!e.clipboardData) {
            return null;
        }

        const clipboardItems = Array.from(e.clipboardData.items || []);
        for (let i = 0; i < clipboardItems.length; i++) {
            const item = clipboardItems[i];
            if (!item.type || !item.type.startsWith('image/')) {
                continue;
            }

            const file = item.getAsFile();
            if (file) {
                return file;
            }
        }

        const clipboardFiles = Array.from(e.clipboardData.files || []);
        for (let i = 0; i < clipboardFiles.length; i++) {
            const file = clipboardFiles[i];
            if (file.type && file.type.startsWith('image/')) {
                return file;
            }
        }

        return null;
    }

    filePicker.addEventListener('change', function () {
        if (filePicker.files && filePicker.files.length > 0) {
            submitManagedUpload(filePicker.files[0]);
        }
    });

    document.addEventListener('paste', function (e) {
        if (isUploading) {
            return;
        }

        const imageFile = clipboardEventImageFile(e);
        if (!imageFile) {
            return;
        }

        if (isEditableTarget(e.target)) {
            return;
        }

        e.preventDefault();

        if (typeof DataTransfer === 'function') {
            const dataTransfer = new DataTransfer();
            dataTransfer.items.add(imageFile);
            filePicker.files = dataTransfer.files;
        }

        submitManagedUpload(imageFile);
    });

    let dragCounter = 0;

    function isFileDrag(e) {
        return !!e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files');
    }

    document.addEventListener('dragenter', function (e) {
        if (!isFileDrag(e)) {
            return;
        }
        e.preventDefault();
        e.stopPropagation();
        dragCounter++;
        if (dragCounter === 1) {
            dropZone.style.display = 'block';
        }
    });

    document.addEventListener('dragover', function (e) {
        if (!isFileDrag(e)) {
            return;
        }
        e.preventDefault();
        e.stopPropagation();
    });

    document.addEventListener('dragleave', function (e) {
        if (!isFileDrag(e)) {
            return;
        }
        e.preventDefault();
        e.stopPropagation();
        dragCounter = Math.max(0, dragCounter - 1);
        if (dragCounter === 0) {
            dropZone.style.display = 'none';
        }
    });

    document.addEventListener('drop', function (e) {
        if (!isFileDrag(e)) {
            return;
        }
        e.preventDefault();
        e.stopPropagation();
        dragCounter = 0;
        dropZone.style.display = 'none';

        const files = e.dataTransfer.files;
        if (files.length > 0 && files[0].type.startsWith('image/')) {
            const dataTransfer = new DataTransfer();
            dataTransfer.items.add(files[0]);
            filePicker.files = dataTransfer.files;
            submitManagedUpload(files[0]);
        }
    });
}
