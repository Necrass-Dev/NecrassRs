# NecrassRs website

Astro builds the landing page and Starlight documentation as a static site for
`https://necrass.rs`. The schema evolution scene is a hydrated Solid component;
the surrounding page and documentation are rendered at build time.

## Develop

Install [mise](https://mise.jdx.dev/), then run from this directory:

```sh
mise trust
mise install
mise exec -- pnpm install --frozen-lockfile
mise exec -- pnpm dev
```

`mise.toml` pins Node.js and pnpm. Project packages, including oxlint and oxfmt,
use exact versions in `package.json` and the committed `pnpm-lock.yaml`.

```sh
mise exec -- pnpm fmt
mise exec -- pnpm lint
mise exec -- pnpm fmt:check
mise exec -- pnpm check
mise exec -- pnpm build
mise exec -- pnpm exec playwright install chromium
mise exec -- pnpm test
```

The browser check starts its own preview server on port 4322. It checks the
scroll sequence in both directions, manual controls, mobile layout, reduced
motion, no-JavaScript content, documentation navigation, and local links. Browser
screenshots are written to the ignored `test-results/` directory.

Oxlint checks JavaScript, TypeScript, and Solid TSX. Oxfmt formats its supported
languages, including TSX, CSS, JSON, YAML, TOML, and Markdown. Oxfmt 0.70 does not
support `.astro` templates; those are maintained manually and checked by
`astro check` and the production build.

## Content and motion

- `src/pages/index.astro`: landing page, metadata, and build-time highlighting.
- `src/components/Workflow.tsx`: Solid schema evolution scene.
- `src/styles/landing.css`: responsive layout, color inversion, and motion.
- `src/content/docs/docs/`: Markdown source for `/docs/` pages. The former
  repository documents live here, with their implementation-status notes retained.
- `public/docs/experiments/`: downloadable experiment data.

The initial demo imports the real CLI starter's SDL and resolver source. Adding
`version: String!` demonstrates a new resolver stub after `cargo build`, without
pretending to generate business logic. This is a visual demonstration, not a Rust
compiler running in the browser.

Wide, tall screens get the pinned scroll sequence. Smaller viewports and readers
who request reduced motion use the same explicit step controls without pinning.
Without JavaScript, both before/after examples remain readable in document order.

Use `/docs/page-name/` links between published pages and GitHub source links for
repository files. Keep design targets distinguished from implemented behavior.
The site currently follows its source checkout; it does not publish release
snapshots or claim that the complete scope of issue #21 is finished.

## GitHub Pages

The documentation workflow builds and checks pull requests. On `main` or a manual
run from `main`, it uploads `docs/dist` and deploys through GitHub Pages.

Before the first deployment:

1. Select **GitHub Actions** as the repository's Pages source.
2. Set the Pages custom domain to **necrass.rs** and configure its DNS according
   to [GitHub's custom domain instructions](https://docs.github.com/en/pages/configuring-a-custom-domain-for-your-github-pages-site/managing-a-custom-domain-for-your-github-pages-site).
3. Enable HTTPS when GitHub finishes certificate provisioning.

The Astro site URL is `https://necrass.rs`, with no repository-name base path.
DNS and repository settings are managed separately from this checkout.
