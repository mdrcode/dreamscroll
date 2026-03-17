document.addEventListener('DOMContentLoaded', function () {
    setupSearchShortcut();
    setupSearchEndpointRouting();
    setupCaptureExpandToggle(document);
    setupFeedModeControls();
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
        if (!searchInput) {
            return;
        }
        searchInput.value = '';
    });
}

function setupSearchEndpointRouting() {
    const searchForm = document.getElementById('header-search-form');
    const searchInput = document.getElementById('header-search-input');
    if (!searchForm || !searchInput) {
        return;
    }

    searchForm.addEventListener('submit', function (e) {
        const q = searchInput.value.trim();
        if (!q.startsWith('/')) {
            return;
        }

        e.preventDefault();
        if (window.htmx) {
            window.htmx.ajax('POST', '/v2/command', {
                values: { q: q },
                target: '#card-feed',
                swap: 'none'
            });
            return;
        }

        fetch('/v2/command', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/x-www-form-urlencoded; charset=UTF-8'
            },
            body: new URLSearchParams({ q: q })
        });
    }, true);

    searchForm.addEventListener('htmx:configRequest', function (e) {
        const q = searchInput.value.trim();

        if (q.startsWith('/')) {
            e.preventDefault();
            return;
        }

        e.detail.path = '/v2/cards';
    });
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

function setupFeedModeControls() {
    const modeInput = document.getElementById('feed-mode-input');
    const searchInput = document.getElementById('header-search-input');
    const feedControls = document.getElementById('feed-controls');
    const modeButtons = Array.from(document.querySelectorAll('#feed-controls [data-feed-mode]'));
    if (!modeInput || modeButtons.length === 0) {
        return;
    }

    function updateFeedControlsVisibility() {
        if (!feedControls || !searchInput) {
            return;
        }
        const q = searchInput.value.trim();
        feedControls.style.display = q.length > 0 ? 'none' : '';
    }

    function applyMode(mode) {
        modeInput.value = mode;
        modeButtons.forEach(function (btn) {
            const isActive = btn.getAttribute('data-feed-mode') === mode;
            btn.classList.toggle('feed-action--active', isActive);
        });
    }

    function requestFeed(mode) {
        const params = new URLSearchParams();
        params.set('content', mode);
        params.set('n', '30');

        const q = (searchInput && searchInput.value) ? searchInput.value.trim() : '';
        if (q.length > 0) {
            params.set('q', q);
        }

        const url = '/v2/cards?' + params.toString();
        if (window.htmx) {
            window.htmx.ajax('GET', url, {
                target: '#card-feed',
                swap: 'innerHTML'
            });
            return;
        }

        window.location.href = '/v2?' + params.toString();
    }

    modeButtons.forEach(function (btn) {
        btn.addEventListener('click', function (e) {
            e.preventDefault();

            const clickedMode = btn.getAttribute('data-feed-mode');
            if (!clickedMode) {
                return;
            }

            const nextMode = modeInput.value === clickedMode ? 'blend' : clickedMode;
            applyMode(nextMode);
            requestFeed(nextMode);
        });
    });

    if (searchInput) {
        searchInput.addEventListener('input', updateFeedControlsVisibility);
    }

    applyMode(modeInput.value || 'blend');
    updateFeedControlsVisibility();
}

function setupUploadInteractions() {
    const filePicker = document.getElementById('file-picker');
    const uploadForm = document.getElementById('file-upload-form');
    const dropZone = document.getElementById('drop-zone-row');
    if (!filePicker || !uploadForm || !dropZone) {
        return;
    }

    filePicker.addEventListener('change', function () {
        if (window.htmx) {
            window.htmx.trigger(uploadForm, 'submit');
            return;
        }
        this.form.submit();
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
            if (window.htmx) {
                window.htmx.trigger(uploadForm, 'submit');
            } else {
                uploadForm.submit();
            }
        }
    });
}
