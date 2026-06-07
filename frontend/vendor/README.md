# Vendored frontend dependencies

Pinned versions used by `comemory graph serve`. Bumping any of these
requires re-running the smoke checklist in `docs/cli-reference.md`.

| File | Upstream | Version |
|------|----------|---------|
| cytoscape.min.js | https://unpkg.com/cytoscape@3.30.2/dist/cytoscape.min.js | 3.30.2 |
| cose-base.min.js | https://unpkg.com/cose-base@2.2.0/cose-base.js | 2.2.0 |
| layout-base.min.js | https://unpkg.com/layout-base@2.0.1/layout-base.js | 2.0.1 |
| cytoscape-cose-bilkent.min.js | https://unpkg.com/cytoscape-cose-bilkent@4.1.0/cytoscape-cose-bilkent.js | 4.1.0 |
| marked.min.js | https://unpkg.com/marked@14.1.3/marked.min.js | 14.1.3 |
| purify.min.js | https://unpkg.com/dompurify@3.1.7/dist/purify.min.js | 3.1.7 |

Verify with `shasum -a 256 *.js` against `checksums.txt`.
