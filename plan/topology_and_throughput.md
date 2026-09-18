# Topology & Throughput — Cloud Run / Cloud SQL budget reference

**Date:** 2026-09-09
**Status:** Reference / overview
**Purpose:** A single place to look up the per-instance connection and concurrency budgets that shape how Dreamscroll scales on Cloud Run + Cloud SQL. Written to inform the SSE design (`sse.md`) and future scaling decisions.

> **TL;DR:** The binding constraint for long-lived SSE connections is **Cloud Run's per-instance HTTP concurrency budget**, not the Cloud SQL connection limit. The DB budget is generous; the HTTP concurrency budget is what you'll actually hit.

---

## 1. The two budgets, side by side

| Resource                                         | Limit                                                                       | Notes                                                                |
| ------------------------------------------------ | --------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| **Cloud SQL connections per Cloud Run instance** | **100** (built-in connection)                                               | Per instance, per DB. Grows as instances scale.                      |
| **Cloud Run HTTP concurrency per instance**      | **default 80** (console) or **80 × vCPUs** (gcloud/Terraform); **max 1000** | Each concurrent request = one HTTP connection.                       |
| **Cloud Run request timeout**                    | **default 5 min**, **max 60 min**                                           | SSE is a long-lived request; must raise this + keep-alives.          |
| **Cloud Run instances**                          | autoscales; idle instances scale to zero after ~15 min                      | An open SSE connection keeps an instance alive (cost consideration). |

---

## 2. Cloud SQL connection budget (per instance)

- Cloud Run caps each instance at **100 connections** to a Cloud SQL database (when using the built-in Cloud SQL connection).
- This limit is **per instance** and grows as the service scales.
- The session-store pool is `max_connections(5)` in `src/database/postgres.rs`,
	sized for the `db-f1-micro` tier. SeaORM currently has a separate pool with
	its default maximum because the service temporarily uses incompatible SQLx
	versions.
- The SSE design adds **1 dedicated `LISTEN` connection per instance** (for
	Postgres `LISTEN/NOTIFY`). The current upper-bound estimate is therefore
	**SeaORM's configured/default maximum + 5 session-store connections + 1
	listener per instance**; measure and configure this explicitly before relying
	on the old six-connection estimate.

**Conclusion:** DB connections are **not** the binding constraint. Even with several instances, the budget is comfortable.

---

## 2a. Cloud SQL PostgreSQL connection limits by tier

Cloud SQL sets the PostgreSQL `max_connections` based on the **machine type's memory** (more memory → more connections). The limit is **per database instance** (shared by all clients), so it's the *total* budget across every Cloud Run instance, not per instance.

The cheaper tiers and their approximate connection limits:

| Machine type                    | vCPU         | Memory  | Approx. `max_connections` |
| ------------------------------- | ------------ | ------- | ------------------------- |
| **`db-f1-micro`** (shared core) | 0.5 (shared) | 0.6 GB  | **~25**                   |
| **`db-g1-small`** (shared core) | 0.5 (shared) | 1.7 GB  | **~50**                   |
| **`db-custom-1-3840`**          | 1            | 3.75 GB | **~100**                  |
| **`db-custom-2-7680`**          | 2            | 7.5 GB  | **~200**                  |
| **`db-custom-4-15360`**         | 4            | 15 GB   | **~400**                  |

> **Note:** These are approximate defaults. Cloud SQL derives `max_connections` from memory and you can override it with the `max_connections` flag (subject to the instance's resource limits). Always confirm the live value with:
> ```sql
> SELECT * FROM pg_settings WHERE name = 'max_connections';
> ```

### Why this matters for Dreamscroll

- **`db-f1-micro` (~25 connections) is the current constraint.** The session
	pool of 5 is deliberately conservative, but the separate SeaORM pool and SSE
	listener must also be included. The safe instance count depends on the
	SeaORM pool's actual configured maximum; do not use the former six-connection
	estimate until both pools are explicitly budgeted.
- **This is the one place the DB budget *can* bite** — not per-instance, but in the *total* across instances on a tiny tier. The HTTP concurrency budget (section 3) is still the primary SSE constraint, but on `f1-micro` the DB connection total is a close second.
- **Upgrading to `db-custom-1-3840` (~100) or `db-custom-2-7680` (~200)** removes the DB connection total as a practical constraint for a personal/small app, and lets you raise the pool size if needed.

---

## 3. Cloud Run HTTP concurrency budget (per instance) — the real constraint

- **Maximum concurrent requests per instance** is configurable up to **1,000**.
- **Default:** 80 (console) or **80 × vCPUs** (gcloud/Terraform).
- **Each concurrent request = one HTTP connection.**
- **SSE connections are long-lived requests** — they occupy one concurrency slot for the *entire duration of the stream*, not just a round-trip.

### The math

```
N open SSE connections on one instance  →  consume N of its concurrency slots
```

At the default concurrency of 80, **80 concurrent SSE tabs on a single instance saturate it** — every other request (page loads, HTMX re-fetches, uploads) queues behind them.

### Why the SSE design stays within budget

- **One SSE connection per page, not per card.** A timeline with 50 cards still uses **1** connection. (Per-card connections would blow the budget instantly.)
- **Thin signals, not HTML-over-SSE.** Each connection is I/O-bound and mostly idle; the HATEOAS re-fetch pattern issues short-lived requests that occupy a slot only momentarily.

### Recommendations

1. **Set concurrency deliberately** (not the implicit default). Since SSE handlers are I/O-bound and cheap, raising concurrency (e.g. 80–200) lets one instance serve many idle SSE streams.
2. **Rely on autoscaling.** As SSE connections grow, Cloud Run adds instances — each with its own concurrency budget and its own `LISTEN` connection.
3. **Account for scale-to-zero + cost.** An open SSE connection keeps an instance alive (it's processing a request), so active users pin instances warm. Cap instances if cost is a concern.

---

## 4. Long-lived connection caveats (Cloud Run)

From the Cloud Run container contract — long-lived connections are **treated as ephemeral** and can be dropped:

- **Infrastructure restarts** can terminate/replace long-lived connections → the `LISTEN` connection **must auto-reconnect and re-issue `LISTEN`**.
- **Outbound VPC idle timeout is 10 minutes** → keep the connection active or reconnect on failure.
- **Idle instances scale to zero** after ~15 min → a `LISTEN` connection only lives while its instance is alive. Fine because the DB is the source of truth (fresh instance replays from DB).
- **Request timeout default 5 min (max 60 min)** → SSE needs a high timeout + keep-alives.

---

## 5. Current repo state

- **No concurrency / max-instances / min-instances set** in `cloudbuild.yaml`, `docker-build-push.sh`, or deploy scripts → Dreamscroll runs on Cloud Run defaults.
- Cloud SQL connection: **private IP + Direct VPC egress** (`config_prod.env` → `10.128.0.10:5432`; see `_project/gcloud/cloudsql_postgres.md`).
- SeaORM pool: currently uses SeaORM's default pool settings in
	`src/database/postgres.rs` (no configured `max_connections`).
- Session-store pool: `max_connections(5)` in `src/database/postgres.rs`.
- Temporary SQLx split: SeaORM uses SQLx 0.9 while
	`tower-sessions-sqlx-store` uses SQLx 0.8, so the service currently has two
	PostgreSQL pools. The consolidation TODO is recorded next to the pool setup
	in `src/database/postgres.rs`.

---

## 6. Related

- `_project/plans/sse.md` — the SSE design that motivated this reference.
- `_project/gcloud/cloudsql_postgres.md` — Cloud SQL connectivity setup.
- `_project/gcloud/cloud_task_queue.md` — task queueing across instances.
