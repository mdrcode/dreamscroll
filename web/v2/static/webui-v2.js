document.addEventListener('DOMContentLoaded', function () {
    if (!window.htmx) {
        throw new Error('HTMX is required for webui-v2.js but was not found on window.htmx.');
    }

    setupSearchShortcut();
    setupSearchClearButton();
    setupSearchEndpointRouting();
    setupCaptureExpandToggle(document);
    setupSearchClearOnSlashCommand();
    setupUploadInteractions();

    document.body.addEventListener('htmx:afterSwap', function (e) {
        if (e.target && e.target.id === 'card-feed') {
            setupCaptureExpandToggle(e.target);
        }
    });
});

function setupSearchShortcut() {
    const searchInput = document.getElementById('header-search-input');
    if (!searchInput) {
        return;
    }

    document.addEventListener('keydown', function (e) {
        if (e.key === '/' && document.activeElement !== searchInput) {
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

        // Clear should always reload timeline results with an empty query.
        reloadFeedFrame();
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

        e.detail.path = '/cards';
        const parameters = e.detail.parameters || (e.detail.parameters = {});
        applyFeedParameters(parameters, state, false);
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
        content: 'blend',
        query: searchInput ? searchInput.value.trim() : ''
    };
}

function applyFeedParameters(parameters, state, includeContent) {
    setFeedParameter(parameters, 'limit', state.limit);
    setFeedParameter(parameters, 'query', state.query);

    if (includeContent && state.content !== 'blend') {
        setFeedParameter(parameters, 'content', state.content);
        return;
    }

    setFeedParameter(parameters, 'content', null);
}

function setupCaptureExpandToggle(rootNode) {
    rootNode.querySelectorAll('.capture-card__details-toggle').forEach(function (link) {
        if (link.dataset.bound === 'true') {
            return;
        }

        link.dataset.bound = 'true';
        link.addEventListener('click', function (e) {
            e.preventDefault();
            const row = this.closest('.capture-card');
            if (!row) {
                return;
            }

            row.classList.toggle('is-expanded');
            this.textContent = row.classList.contains('is-expanded') ? 'Less' : 'More';
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
    applyFeedParameters(params, state, true);

    const query = params.toString();
    if (query.length === 0) {
        return '/cards';
    }
    return '/cards?' + query;
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
    if (!filePicker || !uploadForm || !dropZone || !progressWrap || !progressBar || !progressText) {
        return;
    }

    let isUploading = false;
    let hideProgressTimer = null;

    function setUploadProgress(percent, label) {
        const safePercent = Math.max(0, Math.min(100, Math.round(percent)));
        progressBar.style.width = safePercent + '%';
        progressText.textContent = label || (safePercent + '%');
    }

    function setUploadInFlight(inFlight) {
        isUploading = inFlight;
        if (hideProgressTimer) {
            window.clearTimeout(hideProgressTimer);
            hideProgressTimer = null;
        }

        if (inFlight) {
            uploadForm.classList.add('is-uploading');
            progressWrap.classList.add('is-visible');
            return;
        }

        uploadForm.classList.remove('is-uploading');
        filePicker.value = '';
        hideProgressTimer = window.setTimeout(function () {
            progressWrap.classList.remove('is-visible');
            setUploadProgress(0, '0%');
            hideProgressTimer = null;
        }, 1400);
    }

    function submitManagedUpload(file) {
        if (!file || isUploading) {
            return;
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
                setUploadProgress(100, 'Processing...');
                reloadFeedFrame();
                setUploadInFlight(false);
                return;
            }

            setUploadProgress(0, 'Upload failed');
            setUploadInFlight(false);
        });

        xhr.addEventListener('error', function () {
            setUploadProgress(0, 'Upload failed');
            setUploadInFlight(false);
        });

        xhr.send(formData);
    }

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
            dropZone.classList.add('drag-over');
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
            dropZone.classList.remove('drag-over');
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
        dropZone.classList.remove('drag-over');

        const files = e.dataTransfer.files;
        if (files.length > 0 && files[0].type.startsWith('image/')) {
            const dataTransfer = new DataTransfer();
            dataTransfer.items.add(files[0]);
            filePicker.files = dataTransfer.files;
            submitManagedUpload(files[0]);
        }
    });
}
