# Dreamscroll Backlog

> Format:
>
> ```text
> - [ ] Title [added: YYYY-MM-DD]
>   - Description of the task with more details
>   - [done: YYYY-MM-DD]
> ```

- [ ] Illuminator should not talk to DB directly (internally).
  - [added: 2025-12-30]
  - Ideally the illuminator should not know about DB internals or controller logic. Some
    external entity should monitor/poll the DB and enqueue into Illuminator.

- [ ] Investigate whether possible to convert from SeaORM's DbErr into AppError automatically?
  - [added: 2025-12-28]

- [ ] AppError should automatically trace internally as a convenence
  - [added: 2025-12-28]

- [ ] Extend the "body" of index.html to bottom of viewport always.
  - [added: 2025-12-19]
  - Currently when the timeline is empty (no images have been uploaded), the "body" of the HTML page is quite short vertically and so does not serve as a drag-and-drop target. You must drag the image to the very top of the page where the header is in order to trigger the drop behavior.

- [X] Move the DB migrations into connect call?
  - [added: 2025-12-20]
  - This is handled auto-magically by SeaORM v2, specifically the call
    get_schema_registry("dreamspot::model::*").sync(&conn).await?;
  - [done: 2025-12-30]

- [X] Upgrade to SeaORM v2
  - [added: 2025-12-30]
  - [done: 2025-12-30]

- [X] Prevent or mitigate multiple processes from opening local SQLite file
  - [added: 2025-12-19]
  - Add file locking or a mutex to ensure only one process can access the local SQLite database at a time. Or just fail informatively to avoid corruption.
  - Actually just enabled WAL mode here and that seems durable enough for local dev purposes.
  - [done: 2025-12-19]
