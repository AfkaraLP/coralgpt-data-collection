# Configuration

Copy `.env.example` to `.env` and fill in `USERNAME`, `PASSWORD`, and
`DISCORD_WEBHOOK_URL`. The bot loads these values from the environment or the
local `.env` file. `PORT` and `DB_PATH` are also required; the example provides
local defaults.

`DISCORD_WEBHOOK_URL` is the complete Discord webhook URL used for player-join
notifications. It is a credential: keep it in the ignored `.env` file, not in
source code. Request errors omit the URL to avoid leaking its token.

Clan and OpenAI-compatible agent settings are optional; uncomment the relevant
entries in `.env.example` after copying it. `AUTH` uses `Header-Name|APIKey`.
Never commit real credentials. Revoke or rotate any credential exposed in Git
history; removing it from history does not invalidate it.

# Available routes

- `POST /chat`: sends the request body as chat message
- `POST /tab`: displays the current tablist
- `POST /jump`: makes the bot jump once
- `POST /walk/{direction}`: moves the bot (forward, backward, left, right, forward-left, forward-right, backward-left, backward-right, stop)
- `POST /sneak/{state}`: toggles sneaking (on, off)
- `POST /slot/{slot}`: selects hotbar slot (0-8)
- `GET /slot`: returns the current hotbar slot
- `GET /messages`: returns the last 1000 chat messages
- `GET /stats/{player}`: returns the player's stats as json or null

Use WASD in the web UI to move the bot. Toggle sneak and select hotbar slots with the buttons in the sidebar.
