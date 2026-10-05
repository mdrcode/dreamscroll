From Cloud Logs, it seems that the r_events SSE stream is not receiving or
responding to client disconnects and is instead staying alive always for its
entire 4 minute self-governed lifetime. This is a signifcant leak of server
resources. and needs investigation.