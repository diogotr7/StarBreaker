# starbreaker.app

Source for the [starbreaker.app](https://starbreaker.app) website. Built
with [Astro](https://astro.build) + [Starlight](https://starlight.astro.build),
deployed to GitHub Pages.

## Develop

```bash
cd website
npm install
npm run dev
```

The dev server runs on <http://localhost:4321>.

## Build

```bash
npm run build      # outputs to website/dist/
npm run preview    # serves the built site locally
```

## Deploy

Pushes to `main` that touch `website/**` trigger
`.github/workflows/deploy-website.yml`, which builds the site and publishes
it to GitHub Pages.

The `public/CNAME` file pins the custom domain to `starbreaker.app`.

## Adding a wiki page

Drop a markdown file into `src/content/docs/wiki/`. The sidebar
(`autogenerate`) picks it up automatically. Use the frontmatter:

```yaml
---
title: Page title
description: One-line description used for SEO and search.
---
```
