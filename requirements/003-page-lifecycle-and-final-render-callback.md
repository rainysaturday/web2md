# Requirement 003: Page Lifecycle & Final Render Callback

**Status:** Proposed  
**Priority:** High  

---

## Description

Webpages that rely on JavaScript often go through a lifecycle: initial HTML parse, script execution, asynchronous content loading, and eventual stabilization. `web2md` must manage this lifecycle to capture the "final" rendered HTML once the page is stable, rather than snapshotting a partially-loaded DOM.

This requirement defines a **final render callback** mechanism: after all synchronous scripts have run, `setTimeout`/`setInterval` timers have been processed, and the DOM has stabilized, a callback is invoked to return the finalized HTML. A **maximum render timeout** prevents waiting indefinitely on pages that continuously update (e.g., live tickers, chat widgets).

---

## Acceptance Criteria

### AC-003-1: Render Lifecycle States
- The page lifecycle has four distinct states:
  1. **`Fetching`** — HTML is being downloaded.
  2. **`Parsing`** — HTML is being parsed and initial synchronous scripts execute.
  3. **`Rendering`** — Async scripts, timers, and fetch responses are being processed.
  4. **`Stable`** — No pending timers, network requests, or microtasks remain; the DOM is finalized.

### AC-003-2: Stabilization Detection
- The system detects "stability" by monitoring:
  - Pending `setTimeout` / `setInterval` callbacks.
  - Pending `Promise` microtasks.
  - In-flight `fetch` / `XMLHttpRequest` requests.
  - Pending `requestAnimationFrame` callbacks.
  - Active `MutationObserver` callbacks.
- The page is considered stable when all of the above queues are empty and no new tasks have been scheduled for a configurable **quiet period** (default: 100ms).

### AC-003-3: Final Render Callback
- When stability is detected, the **final render callback** fires with the fully-updated HTML string.
- The callback is exposed in the Rust API as a function that returns `Result<String, Error>`.
- The callback can also be invoked early via a CLI flag (`--render-now`) to force capture of the current DOM state.

### AC-003-4: Maximum Render Timeout
- A **maximum render timeout** is configurable (default: 30 seconds).
- If the timeout elapses before stability is detected, the current DOM is captured and returned with a warning.
- The timeout applies to the entire lifecycle from `Fetching` to `Stable`.
- If the timeout is hit, subsequent timer/network activity is suppressed to prevent further mutations.

### AC-003-5: Idempotent Final HTML
- The HTML returned by the final render callback is idempotent: if it were re-parsed and re-rendered under the same conditions, the resulting HTML would be structurally identical.
- The returned HTML is valid (well-formed) — all opened tags are closed.

### AC-003-6: Timeout Configuration
- CLI flag `--render-timeout <seconds>` to set the maximum render timeout.
- CLI flag `--quiet-period <milliseconds>` to configure the stability quiet period.
- CLI flag `--render-now` to skip waiting for stability and capture the DOM immediately.
