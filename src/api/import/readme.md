# Import APIs

The import/ APIs are special case handlers for migrating data from one instance
to another.

As an example, import_capture is nearly identical to insert_capture, however it
allows the caller to also specify the `created_at` field so that user history
from one instance may be preserved.

It's not clear if we need import/* long term, or perhaps we'll migrate to a
better architectural solution.
