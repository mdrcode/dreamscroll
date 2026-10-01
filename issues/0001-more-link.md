When a capture partial is refreshed (e.g. after the background illuminate task
completes), the content is there but the More button (expando) does not work
correctly. I believe we rely on custom JS wiring to discover and attach event 
listeners to the More button, which may not be re-applied after the partial refresh.