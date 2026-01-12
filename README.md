# Available routes

- `POST /chat`: sends the request body as chat message
- `POST /tab`: displays the current tablist
- `POST /jump`: makes the bot jump once
- `POST /walk/{direction}`: moves the bot (forward, backward, left, right, forward-left, forward-right, backward-left, backward-right, stop)
- `POST /sneak/{state}`: toggles sneaking (on, off)
- `POST /slot/{slot}`: selects hotbar slot (0-8)
- `GET /slot`: returns the current hotbar slot
- `GET /messages`: returns the last 1000 chat messages

Use WASD in the web UI to move the bot. Toggle sneak and select hotbar slots with the buttons in the sidebar.
