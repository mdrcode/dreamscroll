# Creating a Vertex Collection

Create:

```bash
gcloud beta vector-search collections create dreamscroll-dev-collection5 \
  --data-schema=src/search/gcloud/schema_vertex_data.json \
  --vector-schema=src/search/gcloud/schema_vertex_vector.json \
  --location=us-central1 \
  --project=mdrcode
```

Verify:

```bash
gcloud beta vector-search collections describe dreamscroll-dev-collection5 \
  --location=us-central1 \
  --project=mdrcode
```
