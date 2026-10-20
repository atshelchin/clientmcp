# ClientMCP website

The project site: a landing page and the documentation. SvelteKit, prerendered in full,
deployed as a Cloudflare Worker that serves static assets.

```sh
bun install
bun run dev        # http://localhost:5173
bun run build      # prerenders every page into .svelte-kit/cloudflare
bun run preview    # the build, in workerd
bunx wrangler deploy
```

Live at <https://clientmcp.dev>: the Worker `clientmcp-website`, with `clientmcp.dev` as
its custom domain (`wrangler.jsonc`). The name `clientmcp` belongs to the bridge in
`crates/worker`; keep the two apart. The domain is also set as `paths.origin` in
`vite.config.ts` and as `site.url` in `src/lib/site.ts`, which the canonical links, Open
Graph tags and `sitemap.xml` use. `static/og.png` is the share image.

## Where the content comes from

| Page           | Source                                                                   |
| -------------- | ------------------------------------------------------------------------ |
| `/`            | `src/routes/+page.svelte`; code samples in `src/lib/content/snippets.ts` |
| `/docs`        | `../README.md`                                                           |
| `/docs/sdk`    | `../sdk/js/README.md`                                                    |
| `/docs/spec`   | `../SPEC.md`                                                             |
| `/docs/design` | `../docs/design.md`                                                      |

The docs are the repository's own Markdown, rendered at build time by
`src/lib/server/markdown.ts`: headings get GitHub's anchors, links to the files above
go to their pages, and links to any other repository path go to GitHub. Edit the
Markdown, not the site. Adding a document is one entry in `src/lib/server/docs.ts`.

The landing page restates facts from those files (performance figures, limits,
quick start). When they change, update the landing page too.

## The hero demo

`src/lib/components/HeroDemo.svelte` is an illustration, not a connection to a bridge.
It is built the way the protocol asks apps to be built: one `actions` object, called
by the UI (the visitor and a scripted cursor) and by the scripted agent. Visitors can
tick todos, add one, and allow or deny the agent's request. It pauses off-screen and
in background tabs, has Pause/Replay controls, and shows a still, finished state under
`prefers-reduced-motion`.

## Design

Two signal colours carry the idea: `--human` (orange, the UI side) and `--agent`
(blue, the MCP side). Tokens, light and dark, are in `src/app.css`. Type is Geist,
Geist Mono and Instrument Serif (italic accents), self-hosted via Fontsource.

## Checks

```sh
bun run check            # svelte-check
bun run lint             # prettier + eslint
bunx vitest run          # unit tests (Markdown rendering, link rewriting)
bunx playwright test     # e2e against the built site in workerd
```
