# Creating a Vertex Collection

Create:

```bash
gcloud beta vector-search collections create dreamscroll-dev-collection5 \
  --vector-schema=_project/gcloud/schema_vertex_vector.json \
  --data-schema=_project/gcloud/schema_vertex_capture_data.json \
  --location=us-central1 \
  --project=mdrcode
```

Verify:

```bash
gcloud beta vector-search collections describe dreamscroll-dev-collection5 \
  --location=us-central1 \
  --project=mdrcode
```
