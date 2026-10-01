# Ferrite site

The marketing site and documentation. Plain HTML, CSS and a little JavaScript,
built by one standard-library Python script. No framework, no npm, no external
requests (the Inter font is self-hosted; see `static/fonts/OFL.txt`).

```
site/
├── build.py          generator + link checker (python3, stdlib only)
├── src/
│   ├── layout.html   page shell: header, footer, theme, <head>
│   ├── index.html    the landing page
│   └── docs/*.html   one documentation article each (front-matter comment first)
└── static/           css, js, fonts, images (copied to dist/static/)
```

## Build and preview

```sh
python3 site/build.py            # writes site/dist/ and checks every link + anchor
python3 site/build.py --serve    # same, then serves http://localhost:8000
```

`site/dist/` is generated and git-ignored. A broken internal link or `#anchor`
fails the build.

All links between pages are relative, so the site works from a project sub-path
(`https://<user>.github.io/<repo>/`) or straight from the filesystem.

## Writing docs

Add `src/docs/<slug>.html`. It must start with a comment:

```html
<!--
title: Page title
order: 5
group: Evaluation
summary: One sentence for the meta description.
-->
```

Then plain HTML. `<h2>`/`<h3>` get ids, anchors and the "On this page" list
automatically. Use `{{ROOT}}` for links to other pages (`{{ROOT}}docs/limits.html`),
`{{REPO}}` for the repository URL, and `{{icon:name}}` for an inline icon (see
`ICONS` in `build.py`). Reusable pieces live in `static/css/site.css`:
`.callout.{note,warn,good,bad}`, `.table-wrap`, `dl.defs`, `.formula`, `.diagram`.

## Keep the numbers honest

Every figure comes from `docs/EVALUATION.md` (section 8 for the current corpus).
If the evaluation changes, update the landing page tiles, the bar charts in
`src/index.html` and `src/docs/evaluation.html` together, and keep the caveats
beside the numbers. The screenshots in `static/img/` are real renders of the
app's own `view()`.

## Deploy

`.github/workflows/pages.yml` publishes `site/dist/` to GitHub Pages. It is
manual (Actions → Site → Run workflow). One-time setup in the repository
settings: **Pages → Build and deployment → Source: GitHub Actions**.

The download buttons read the rolling `latest` release through the public
GitHub API in the visitor's browser and fall back to the releases page when
there is none.
