## 2026-09-27 - Unsanitized SVG markup in CommandPalette {@html}
**Vulnerability:** CommandPalette rendered command icon SVG strings using `{@html}` without DOMPurify sanitization, allowing potential script injection if dynamic commands or icons contain untrusted markup.
**Learning:** Svelte's `{@html}` directive renders raw HTML strings directly into the DOM without escaping or sanitization.
**Prevention:** Always pass any markup rendered via `{@html}` through `DOMPurify.sanitize(markup, { USE_PROFILES: { svg: true } })` prior to insertion.
