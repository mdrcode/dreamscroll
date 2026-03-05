document.addEventListener('DOMContentLoaded', function () {
    setupSearchShortcut();
    setupCaptureExpandToggle();
    setupUploadInteractions();
});

function setupSearchShortcut() {
    const searchInput = document.getElementById('header-search-input') || document.getElementById('search-input');
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

function setupCaptureExpandToggle() {
    document.querySelectorAll('.capturelist-row-seemore').forEach(function (link) {
        link.addEventListener('click', function (e) {
            e.preventDefault();
            const row = this.closest('.capturelist-row');
            if (!row) {
                return;
            }

            row.classList.toggle('expanded');
            this.textContent = row.classList.contains('expanded') ? 'Less' : 'More';
        });
    });
}

function setupUploadInteractions() {
    const filePicker = document.getElementById('file-picker');
    const uploadForm = document.getElementById('file-upload-form');
    const dropZone = document.getElementById('drop-zone-row');
    if (!filePicker || !uploadForm || !dropZone) {
        return;
    }

    filePicker.addEventListener('change', function () {
        this.form.submit();
    });

    const submitBtnContainer = document.getElementById('submit-btn-container');
    if (submitBtnContainer) {
        // Redundant with default CSS in the template; kept as a defensive fallback.
        submitBtnContainer.style.display = 'none';
    }

    let dragCounter = 0;

    document.body.addEventListener('dragenter', function (e) {
        e.preventDefault();
        e.stopPropagation();
        dragCounter++;
        if (dragCounter === 1) {
            dropZone.style.display = 'block';
            dropZone.classList.add('drag-over');
        }
    });

    document.body.addEventListener('dragover', function (e) {
        e.preventDefault();
        e.stopPropagation();
    });

    document.body.addEventListener('dragleave', function (e) {
        e.preventDefault();
        e.stopPropagation();
        dragCounter--;
        if (dragCounter === 0) {
            dropZone.style.display = 'none';
            dropZone.classList.remove('drag-over');
        }
    });

    document.body.addEventListener('drop', function (e) {
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
            uploadForm.submit();
        }
    });
}
