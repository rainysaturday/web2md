# web2md

`web2md` is a small Rust-based CLI tool that downloads a webpage and converts the HTML into
Markdown, making it easy to get a minimal, readable representation of a page from the command line.

Beyond simple fetching, it embeds a JavaScript engine (via [Boa](https://github.com/boa-dev/Boa))
so that pages are rendered the way a browser would render them: scripts run, DOM changes are
reflected in the output, and the tool waits for the page to stabilize before converting. It also
ships with a daemon mode for stateful, repeated access to pages.

## Features

- **HTML → Markdown conversion** — fetches a URL and outputs clean Markdown to stdout or a file
- **JavaScript execution** — runs page scripts by default (disable with `--disable-js`)
- **DOM bridge** — standard DOM APIs (`document.createElement`, `innerHTML`, `appendChild`, …)
  update the document that gets converted to Markdown
- **Render lifecycle** — waits for asynchronous content to load and the DOM to stabilize, with
  configurable timeout and quiet period
- **Script injection** — inject extra JavaScript files or inline snippets before page execution
- **User agent control** — named presets (`firefox`, `chrome`, `safari`, `edge`, `curl`) or a
  custom string
- **Daemon mode** — persistent HTTP server that keeps pages (with their JS context and DOM state)
  open for repeated interaction: re-rendering, evaluating JS, clicking, filling forms
- **Client subcommands** — interact with a running daemon from the CLI

## Building

Requires Rust (edition 2021).

```sh
cargo build --release
```

The binary is produced at `target/release/web2md`.

## Usage

### Simple mode

Convert a webpage and print the result to stdout:

```sh
web2md https://example.com
```

Write the output to a file instead:

```sh
web2md -o page.md https://example.com
```

### Options

| Option | Description |
| --- | --- |
| `-o, --output <file>` | Output file path (defaults to stdout) |
| `--with-images` | Include images in output (removed by default) |
| `--disable-js` | Disable JavaScript execution for the fetched page |
| `--render-timeout <secs>` | Maximum time to wait for page stabilization (default: 30) |
| `--quiet-period <ms>` | Stability quiet period (default: 100) |
| `--render-now` | Capture the DOM immediately without waiting for stability |
| `--no-external-scripts` | Do not fetch or execute external `<script src="...">` references |
| `--user-agent <ua>` | User agent string or preset (`firefox`, `chrome`, `safari`, `edge`, `curl`) |
| `--inject-script <file>` | Inject a JavaScript file before execution (repeatable) |
| `--inject-code <code>` | Inject an inline JavaScript snippet before execution (repeatable) |

### Daemon mode

Start the daemon (listens on `http://127.0.0.1:8765` by default):

```sh
web2md --daemon
```

The daemon keeps open pages in memory — each with its own JS context, DOM state, and lifecycle
manager — so clients can fetch once and then interact repeatedly without re-fetching.

Daemon options (both via flags and the `daemon` subcommand):

```sh
web2md --daemon --daemon-port 9000
# equivalent to:
web2md daemon --port 9000 --background --log-file daemon.log
```

> **Note:** The `--daemon-socket` flag is currently accepted but Unix socket support is not yet
> implemented; the daemon listens over HTTP on TCP only.

### Client commands

Interact with a running daemon using the `client` subcommand (default daemon URL:
`http://localhost:8765`, override with `--daemon-url`):

```sh
# Fetch a new page; prints its page ID
web2md client fetch https://example.com

# Get the rendered HTML of a page
web2md client render <page-id>

# Get the Markdown output of a page
web2md client markdown <page-id>

# Execute JavaScript in a page's context
web2md client eval <page-id> "document.title"

# Simulate a click on an element (CSS selector)
web2md client click <page-id> "button#load-more"

# Fill an input field
web2md client fill <page-id> "input[name=q]" "rust cli"

# Close a page and free its resources
web2md client close <page-id>

# Check daemon health
web2md client health
```

### REST API

The daemon exposes a small REST API on `127.0.0.1` (default port `8765`):

| Method | Endpoint | Description |
| --- | --- | --- |
| `GET` | `/health` | Health check |
| `GET` | `/metrics` | Requests handled, pages created, uptime |
| `POST` | `/shutdown` | Request daemon shutdown |
| `POST` | `/page` | Fetch a new page (JSON body: `{"url": "..."}`) |
| `GET` | `/pages` | List open pages |
| `DELETE` | `/pages` | Close all pages |
| `GET` | `/page/{id}` | Get page info |
| `DELETE` | `/page/{id}` | Close a page |
| `GET` | `/page/{id}/render` | Get rendered HTML |
| `GET` | `/page/{id}/markdown` | Get Markdown output |
| `POST` | `/page/{id}/render-now` | Capture the DOM immediately |
| `POST` | `/page/{id}/eval` | Evaluate a JavaScript snippet |
| `POST` | `/page/{id}/eval-file` | Evaluate a JavaScript file |
| `POST` | `/page/{id}/click` | Click an element (CSS selector) |
| `POST` | `/page/{id}/fill` | Fill an input field |

## Project layout

```
src/
├── main.rs        # CLI parsing, simple mode, subcommand dispatch
├── page.rs        # Page fetching and render lifecycle management
├── js_engine.rs   # Boa-based JavaScript runtime integration
├── dom_bridge.rs  # JS runtime ↔ HTML document bridge
└── daemon.rs      # Daemon HTTP server (actix-web) and client functions
requirements/      # Requirement specifications for major features
```

## License

This project is licensed under the [GNU General Public License v2.0](LICENSE)
(`GPL-2.0-only`).
