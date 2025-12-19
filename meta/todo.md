# Dreamscroll Backlog

> Format:
>
> ```text
> - [ ] Title [added: YYYY-MM-DD] [done: YYYY-MM-DD]
>   - Description of the task with more details
> ```

- [ ] Prevent multiple processes from opening local SQLite file [added: 2024-12-19]
  - Add file locking or a mutex to ensure only one process can access the local SQLite database at a time. Or just fail informatively to avoid corruption.
