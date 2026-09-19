# Creating Cloud Tasks queues

The production application creates tasks with the target URL and OIDC token on
each task. The queue commands below are for creating the queues and their retry
policies; they do not need HTTP target or OIDC overrides.

The queue names intentionally match the webhook route suffixes configured by the
application:

| Queue | Webhook route |
| --- | --- |
| `illuminate` | `/_wh/cloudtask/illuminate` |
| `search_index` | `/_wh/cloudtask/search_index` |
| `spark` | `/_wh/cloudtask/spark` |

Use the same dedicated service account configured as
`TASK_OIDC_SERVICE_ACCOUNT_EMAIL` when granting Cloud Tasks permission to mint
tokens for task delivery.

```bash
gcloud tasks queues create illuminate \
  --location=us-central1 \
  --max-dispatches-per-second=2 \
  --max-concurrent-dispatches=5 \
  --max-attempts=3 \
  --min-backoff=10s \
  --max-backoff=600s \
  --max-doublings=16
```

Verify:

```bash
gcloud tasks queues describe illuminate \
  --location=us-central1 \
  --project=mdrcode
```
