# Creating a Cloud Tasks queue

```bash
gcloud tasks queues create ingest \
  --location=us-central1 \
  --max-dispatches-per-second=2 \
  --max-concurrent-dispatches=5 \
  --max-attempts=3 \
  --min-backoff=10s \
  --max-backoff=600s \
  --max-doublings=16 \
  --http-uri-override=host:dreamscroll-hook-xdrchnynaq-uc.a.run.app,path:/_wh/cloudtask/ingest \
  --http-oidc-service-account-email-override=cloud-tasks-invoker@mdrcode.iam.gserviceaccount.com \
  --http-oidc-token-audience-override=https://dreamscroll-hook-xdrchnynaq-uc.a.run.app
```

Verify:

```bash
gcloud tasks queues describe ingest \
  --location=us-central1 \
  --project=mdrcode
```
