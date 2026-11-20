# illumination: one image, one structured reading

Status: v2, prompt `capture_illumination_v2.4`. Implemented in `v2/`; called only through
`logic::illuminate::exec` on the illumination worker.

Provider behavior — endpoints, `store`, retention, the output-token ceiling, and thought-summary
retrieval — lives in [`plan/prod-vertex-gemini.md`](../../plan/prod-vertex-gemini.md).

Illumination turns a capture's image into a summary, a description, suggested searches, and the
entities a knowledge graph can link. It is one inference call per attempt. The module owns the
prompt, the response schema, the typed result, and the parsing rules; it does not know about tasks,
retries, queues, or persistence. Those live in `logic::illuminate` and `api::service`.

## Shape

```
capture ──▶ v2::illuminate ──▶ InferenceOutcome<Illumination>
                                      │
                     illumination_raw ◀┤ every attempt
                                      │
                 illumination + observations + search_index ◀┘
                     projection only, one transaction
```

Every attempt keeps its raw JSON. The relational projection is optional (`persist_projection`) and
is the work the knowledge graph can link. Reruns append readings; the UI shows the latest.

## Input

`v2::illuminate` builds three ordered parts:

1. `prompt::PROMPT` — the stable task instructions and entity taxonomy.
2. the capture's image bytes and MIME type.
3. an optional owner note, wrapped by `annotation_hint`.

The order is deliberate. Parts 1 and 2 are byte-identical across runs, so provider prefix caching
can reuse them; the note is last, closest to generation, and changing it only invalidates itself.
The note is a separate text part rather than a suffix inside `PROMPT`, for the same reason.

## Owner annotation

`CaptureInfo.annotation` is the capture's one active note, written by the user after a wrong
reading. When present, the note is evidence about the image. The wrapper says so, and it says just
as clearly that the note is user data, not instructions — the field is user-supplied and could
carry a prompt injection. The wrapped note is delimited by `<owner_note>` tags.

`model::annotation::Model::MAX_CONTENT_CHARS` (1,000 characters) bounds it. `set_annotation`
rejects longer notes with a 400, and the annotation editor caps input with `maxlength`.
`annotation_hint` clips anything over the limit anyway (for rows written before the limit),
preferring a word boundary, and logs a warning.

An annotation changes no schema and no output field. It only changes what the model sees.

## Output contract

| field | shape | use |
| --- | --- | --- |
| `summary` | string | list rows, cards |
| `details` | string | capture page narrative |
| `suggested_searches` | string array | follow-up searches |
| `entities` | array of `Entity` | observations for the graph |

`Entity` carries `name`, `type` (`KnowledgeEntityType`), optional `description`, optional
`search_query`, and optional `platform_link` (`platform`, `handle`, optional `display_name`, and a
`url` the model never supplies). `Platform` is intentionally open: known names are canonicalized,
unknown names survive as `Platform::Other`, so a new platform does not collapse to `other`.

`Illumination::from_raw_json` parses the fields but retains the original JSON. Unknown or future
fields therefore survive into `illumination_raw` even when the typed projection ignores them.

## Persistence and downstream

- `logic::illuminate::exec` records inference metadata, then the raw result. The prompt version is
  stored with that metadata as `params.prompt_version`.
- When `persist_projection` is set, `insert_illumination_v2` upserts the illumination row and
  replaces its observations and search-index row in one transaction.
- `Beacon::new_illumination` enqueues the search-index task and, when a projection exists, the
  knowledge-link task. The graph sees one illumination as one unit of work.

## Prompt versions

`PROMPT_VERSION` names the prompt that produced an illumination. Bump the minor (`v2.3` → `v2.4`)
when the wording, examples, or input framing change; bump the major only if the response contract
changes. The version travels on every inference row, so old and new readings of the same capture
remain distinguishable after a rerun.

## Evaluating a change

Use captures that already have an owner annotation and a wrong reading. Rerun through the API —
the old reading remains in history, the latest is what the capture page shows:

```
DREAMSCROLL_API_TOKEN=... cargo run --bin dreamscroll_api -- \
  --host localhost:8080 illuminate <capture_id>
```

`--raw-json` prints the provider JSON instead of Markdown; `--thinking-level` and `--web-search`
pin the other variables. Compare the summary, the entity names, and their descriptions against the
previous reading, not just whether the wrong entity disappeared.

## Files

| file | role |
| --- | --- |
| `v2/prompt.rs` | prompt text and prompt version |
| `v2/annotation_hint.rs` | owner-note wrapper, framing, and clipping |
| `v2/illuminate.rs` | input assembly, inference call, response schema |
| `v2/types.rs` | typed result, entity/platform rules, Markdown rendering |
| `knowledge_entity_type.rs` | the closed entity taxonomy |
