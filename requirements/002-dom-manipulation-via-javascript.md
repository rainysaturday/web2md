# Requirement 002: DOM Manipulation via JavaScript

**Status:** Proposed  
**Priority:** High  

---

## Description

When JavaScript executes on a fetched webpage, it must be able to manipulate the page's HTML structure — modifying elements, adding/removing nodes, changing attributes and text content. These DOM changes must be reflected in the HTML that `webmd` captures for Markdown conversion.

This requirement covers the bridge between the JS runtime and the HTML parser so that `document.createElement`, `innerHTML`, `appendChild`, `removeChild`, `setAttribute`, and similar standard DOM APIs update the underlying HTML document that `webmd` will ultimately render to Markdown.

CSS rendering, computed styles, and visual layout are **not** required — only the structural HTML tree needs to be updated.

---

## Acceptance Criteria

### AC-002-1: Core DOM API Surface
- The JS environment exposes at minimum the following standard DOM APIs:
  - `document.createElement(tagName)` / `document.createTextNode(text)`
  - `element.innerHTML` (getter and setter)
  - `element.textContent` (getter and setter)
  - `element.setAttribute(name, value)` / `element.getAttribute(name)` / `element.removeAttribute(name)`
  - `element.appendChild(child)` / `element.removeChild(child)` / `element.replaceChild(new, old)`
  - `element.insertBefore(new, reference)`
  - `element.querySelector(selector)` / `element.querySelectorAll(selector)`
  - `element.closest(selector)`
  - `element.classList.add/remove/toggle`
  - `element.style` (basic inline style property access, no computed styles)
  - `parentElement`, `children`, `nextSibling`, `previousSibling`, `firstChild`, `lastChild` traversal properties

### AC-002-2: DOM Changes Persist in HTML
- After JS execution completes, the serialized HTML (obtained via `document.documentElement.outerHTML` or equivalent) accurately reflects all DOM mutations made by scripts.
- The updated HTML is what gets passed to the Markdown conversion pipeline.

### AC-002-3: Event Handlers
- `document.addEventListener` and `element.addEventListener` are supported for standard events (e.g., `DOMContentLoaded`, `load`, `click`).
- `window.onload` and `document.onreadystatechange` are supported.
- Events dispatched by scripts are processed and can trigger further DOM changes.

### AC-002-4: Timing Functions
- `setTimeout` and `setInterval` are supported with microsecond precision.
- `requestAnimationFrame` is supported (aliased to a timer, not vsync).
- A mechanism exists to flush pending timers and microtasks to reach a stable DOM state.

### AC-002-5: Mutation Observation
- `MutationObserver` API is supported so that scripts can observe and react to DOM changes.
- Observed mutations are processed and can trigger callbacks within the JS environment.

### AC-002-6: Limitation & Safety
- DOM API calls that would require layout information (e.g., `offsetWidth`, `getBoundingClientRect`) return sensible defaults (e.g., `0` or `{top:0, left:0, width:0, height:0}`) rather than throwing.
- `window.getComputedStyle` returns an empty style declaration.
- `document.write` is supported only during page load and is blocked afterwards.
