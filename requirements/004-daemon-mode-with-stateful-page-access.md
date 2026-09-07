# Requirement 004: Daemon Mode with Stateful Page Access

**Status:** Proposed  
**Priority:** High  

---

## Description

`web2md` currently operates as a one-shot CLI tool: it fetches a URL, converts to Markdown, and exits. To support stateful, repeated access to webpages (e.g., checking for updates, interacting with session state, re-rendering after JS mutations), `web2md` needs a **daemon mode** that runs as a persistent background process.

In daemon mode, `web2md` listens on a local socket (HTTP or Unix socket) for commands. It maintains a cache of open "pages" — each with its own JS context, DOM state, and lifecycle manager. Clients can send requests to fetch new pages, trigger re-renders, inject JavaScript, and retrieve the current Markdown output — all without restarting the process or re-fetching the page.

---

## Acceptance Criteria

### AC-004-1: Daemon Startup & Shutdown
- CLI flag `--daemon` starts `web2md` in daemon mode (runs as a foreground process by default; `--daemon --background` forks to background).
- CLI flag `--daemon-port <port>` sets the HTTP listen port (default: `8765`).
- CLI flag `--daemon-socket <path>` sets a Unix socket path instead of TCP (mutually exclusive with `--daemon-port`).
- `SIGINT` / `SIGTERM` cleanly shuts down the daemon, saving any persistent state.
- A `POST /shutdown` endpoint gracefully terminates the daemon.

### AC-004-2: Page Management API
- `POST /page` — Fetch a new page by URL. Returns a `page_id` (UUID). Optional query parameters: `--with-images`, `--render-timeout`, `--quiet-period`.
- `GET /page/{id}` — Get page metadata (URL, status, timestamps, error log).
- `DELETE /page/{id}` — Close a page, freeing its JS context and DOM state.
- `GET /pages` — List all active pages with their IDs, URLs, and status.
- `DELETE /pages` — Close all pages.

### AC-004-3: Render & Retrieve API
- `GET /page/{id}/render` — Returns the final rendered HTML (triggers final render callback if page is not yet stable; waits up to the configured timeout).
- `GET /page/{id}/markdown` — Returns the Markdown output of the current page state.
- `POST /page/{id}/render-now` — Forces immediate render capture without waiting for stability.

### AC-004-4: JavaScript Injection API
- `POST /page/{id}/eval` — Injects and executes a JavaScript expression/statement in the page's JS context. Body contains `{ "script": "..." }`.
- Returns `{ "result": ..., "error": ... }` with the evaluated result or error message.
- `POST /page/{id}/eval-file` — Injects a JavaScript file from disk into the page context.

### AC-004-5: State Persistence & Session Support
- Pages maintain their DOM and JS context across requests — cookies, `localStorage`, and `sessionStorage` are preserved (in-memory).
- A `POST /page/{id}/click` endpoint simulates a click on an element (matched by CSS selector), firing associated event handlers.
- A `POST /page/{id}/fill` endpoint fills an input field (by CSS selector) and dispatches `input`/`change` events.
- After each interaction, the page enters the render lifecycle again for potential stabilization.

### AC-004-6: Health & Metrics
- `GET /health` — Returns daemon health status (uptime, page count, memory usage).
- `GET /metrics` — Returns basic Prometheus-style metrics (requests handled, pages created, render times).
- Logs are written to stdout (or a configured log file in daemon mode).
