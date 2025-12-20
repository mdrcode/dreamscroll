# Dreamscroll Backlog

> Format:
>
> ```text
> - [ ] Title [added: YYYY-MM-DD]
>   - Description of the task with more details
>   - [done: YYYY-MM-DD]
> ```

- [ ] Extend the "body" of index.html to bottom of viewport always.
  - [added: 2025-12-19]
  - Currently when the timeline is empty (no images have been uploaded), the "body" of the HTML page is quite short vertically and so does not serve as a drag-and-drop target. You must drag the image to the very top of the page where the header is in order to trigger the drop behavior.

- [X] Prevent or mitigate multiple processes from opening local SQLite file
  - [added: 2025-12-19]
  - Add file locking or a mutex to ensure only one process can access the local SQLite database at a time. Or just fail informatively to avoid corruption.
  - Actually just enabled WAL mode here and that seems durable enough for local dev purposes.
  - [done: 2025-12-19]
