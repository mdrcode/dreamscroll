 - [ X ] Enhance IlluminationTask to support persist=false (for eval) and thinking=?
   as a model parameter.
 - [ ] Add last_error_code and last_error_message to TaskRun model.
 - [ ] Add a `dreamscroll_api backfill errors` command to show which entities had
   errors (and which errors) during backfill.
 - [ ] Figure out a better solution for agents to create "smoke" environments for
   testing. Currently, sometimes they use my local dev environment and sometimes
   they hack a test database. The test code contains tons of random DB hacking.
 - [ ] Rename the internal feed fragment route from `/cards` to a feed-oriented
   path (e.g. `/feed` or `/feed/fragment`), updating templates, JS, canonical
   redirects, and tests.