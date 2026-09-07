# Requirement 005: CLI Integration for New Modes

**Status:** Proposed  
**Priority:** Medium  

---

## Description

The new JavaScript, rendering, and daemon capabilities must be accessible through `web2md`'s CLI. The existing CLI should continue to work as before (backward compatible), while new flags and subcommands expose the extended functionality.

This requirement covers the CLI surface changes needed to support the architecture defined in Requirements 001–004. Users should be able to choose between the current static mode and the new dynamic JS-enabled mode, invoke the daemon, configure rendering timeouts, and optionally inject scripts.

---

## Acceptance Criteria

### AC-005-1: Backward Compatible Default
- Running `web2md <url>` without any new flags continues to work exactly as before: fetches HTML, cleans it, converts to Markdown, and outputs. No JavaScript is executed, no daemon is started.

### AC-005-2: JavaScript Mode Flag
- `--enable-js` flag enables JavaScript execution for the fetched page.
- When `--enable-js` is passed, the page goes through the full render lifecycle (Requirement 003) before Markdown conversion.
- Without `--enable-js`, the tool behaves as it does today.

### AC-005-3: Render Lifecycle Flags
- `--render-timeout <seconds>` — Maximum time to wait for page stabilization (default: 30, only applies with `--enable-js`).
- `--quiet-period <milliseconds>` — Stability quiet period (default: 100, only applies with `--enable-js`).
- `--render-now` — Capture the current DOM immediately without waiting for stability (only applies with `--enable-js`).
- `--no-external-scripts` — Do not fetch or execute external `<script src="...">` references (default: external scripts are fetched).

### AC-005-4: Script Injection
- `--inject-script <path>` — Inject a JavaScript file into the page before execution (can be specified multiple times).
- `--inject-code <code>` — Inject an inline JavaScript snippet before execution (can be specified multiple times).

### AC-005-5: Daemon Subcommand
- `web2md daemon` — Subcommand to enter daemon mode (alternative to `--daemon` flag).
- `web2md daemon --port <port>` — Set the HTTP listen port.
- `web2md daemon --socket <path>` — Use Unix socket.
- `web2md daemon --background` — Fork to background.
- `web2md daemon --log-file <path>` — Write logs to file instead of stdout.

### AC-005-6: Client Subcommands (for daemon interaction)
- `web2md client fetch <url>` — Send a `POST /page` request to a running daemon (default: `http://localhost:8765`).
- `web2md client render <page-id>` — Send a `GET /page/{id}/render` request.
- `web2md client markdown <page-id>` — Send a `GET /page/{id}/markdown` request.
- `web2md client eval <page-id> <script>` — Send a `POST /page/{id}/eval` request.
- `web2md client click <page-id> <selector>` — Send a `POST /page/{id}/click` request.
- `web2md client fill <page-id> <selector> <value>` — Send a `POST /page/{id}/fill` request.
- `web2md client close <page-id>` — Send a `DELETE /page/{id}` request.
- `web2md client health` — Send a `GET /health` request.
- `web2md daemon-url <url>` — Override the daemon URL for client commands (default: `http://localhost:8765`).

### AC-005-7: Help & Documentation
- `web2md help` and `web2md --help` clearly document all new flags and subcommands.
- `web2md help daemon` and `web2md help client` show subcommand-specific help.
- Man page (`web2md.1`) is updated to reflect new capabilities.
