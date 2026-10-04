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
automatically (the id is the slugified heading, so link to `#the-renderer-gpu-or-cpu`
for "The renderer: GPU or CPU"; the build fails on a link to an id that does not
exist). Use `{{ROOT}}` for links to other pages (`{{ROOT}}docs/limits.html`),
`{{REPO}}` for the repository URL, `{{icon:name}}` for an inline icon (see
`ICONS` in `build.py`), and `{{VERSION}}` / `{{ASOF}}` for the app version (read
from `crates/ferrite-shell/Cargo.toml`) and the "pages last checked" date. Reusable
pieces live in `static/css/site.css`: `.callout.{note,warn,good,bad}`,
`.table-wrap`, `dl.defs`, `.formula`, `.diagram`, `<kbd>`, and for screenshots
`figure.shot` with the variants `.wide` (full column), `.narrow` (at most 420 px)
and `.half` (side by side in a `.shots.stack` row).

Sidebar order is the `order:` number; a group's pages must be consecutive. At the
last audit it is Start (Overview, Getting started, Run it from source), Use
(Using the browser, Models and settings, Debugging and logs), Defense (Threat
model, How the defense works), Evaluation (Evaluation, Metrics, Reproduce it,
Live evaluation), Honesty (Limits).

## Keep the numbers honest

Every figure comes from `docs/EVALUATION.md` (section 8 for the current corpus;
sections 9 and 10 for the live runner and the AgentDojo import). The landing page
tiles and bar charts in `src/index.html`, the tables in `src/docs/evaluation.html`
and the corpus counts in `src/docs/reproduce.html` and `src/docs/live-evaluation.html`
must change together, with the caveats kept beside the numbers.

Counts are checked against the repository, not copied: at the last audit
`crates/ferrite-eval/tests/` held 16 + 10 + 3 hand-written cases and 909 generated
ones (`python3 scripts/gen_redteam_corpus.py --check` says all 909 match), which is
the 938-case corpus `just eval` runs, and 1,046 files in `agentdojo_full/`
(949 attacks and 97 benign twins; `agentdojo_full_manifest.json`). The AgentDojo
import is **not** in the headline numbers and has no results; a live-runner figure
(`2,092 runs, about 7,300 calls`) comes from `live_eval --plan --provider gemini
--model x --corpus agentdojo`, which calls nothing. If a number is not in
`docs/EVALUATION.md` or a command output you ran, it does not go on the site.

Commands, flags and shortcuts are read from the `justfile`, `scripts/`,
`crates/ferrite-ui/src/lib.rs` (`handle_key_press`) and the example programs' own
`--help`, and each page says what has not been run (macOS, Windows, a real GPU, a
real model). When you re-audit, bump `DOCS_AS_OF` in `build.py`.

Two statements go stale fastest and are dated on purpose: which commit the
published `latest` release was built from (check it with
`gh api repos/rayanjainn/Ferrite-Browser/releases/tags/latest`), and which
features that build lacks. Update them whenever CI publishes a new build.

## Screenshots

The images in `static/img/` are renders of the app's own `view()` function, not
mock-ups and not photographs of a window. They are made headlessly:

1. Build the shell without Servo (`cargo build -p ferrite-shell`). The stub engine
   cannot render pages, so the UI would be empty.
2. Add a **temporary seed**: a throwaway module in `crates/ferrite-ui` called from
   `launch()` that fills `FerriteBrowser` with sample state (tabs, a chat, a pending
   review, DevTools rows, a page control, a crash notice), keyed by an environment
   variable. It must never be committed: finish with `git status --short crates`
   empty and `grep -rn "FERRITE_DEMO\|TEMP-DEMO\|demo_seed" crates` empty.
3. For the page inside the browser, serve a few local demo pages
   (`python3 -m http.server`) and render them with
   `target/debug/examples/page_shot <url> 3000 out.png <w> <h> 1` at exactly the
   size of the page area. The seed puts that PNG where the engine's frame goes.
   Real engine output (the console messages, the request list) is read from the same
   `page_shot` run, so those rows are not invented.
4. Run `ferrite-shell ui` under `xvfb-run` or `Xvfb` with `ICED_BACKEND=tiny-skia`
   (a software renderer), a throwaway `HOME` and `FERRITE_HOME`, wait a few
   seconds, and capture with `import -window root`. Do it once per theme.
5. Crop to the part that matters, quantize to 256 colours with Pillow, and keep each
   file under about 150 KB. Look at every image before using it.

Captions must say what is real and what is sample state ("rendered from sample
state"; "the page is a real engine render of a local demo page"). Do not caption a
seeded state as the engine behaving that way. Everything was rendered on Linux at
scale 1, so shortcuts read `Ctrl`; nothing was rendered on macOS or at Retina
density. Light and dark variants are paired (`*-light.png`, `*-dark.png`) with the
`theme-shot-light` / `theme-shot-dark` classes.

## Deploy

`.github/workflows/pages.yml` publishes `site/dist/` to GitHub Pages. It is
manual (Actions → Site → Run workflow). One-time setup in the repository
settings: **Pages → Build and deployment → Source: GitHub Actions**.

The download buttons read the rolling `latest` release through the public
GitHub API in the visitor's browser and fall back to the releases page when
there is none.
