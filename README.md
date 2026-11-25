# Synector site

Static multi-page site served by GitHub Pages from this branch.

The application source lives on `main` in the same repository. This branch contains only the site:

```
index.html                 landing page
404.html                   branded not-found page
styles.css                 landing design system
main.js                    language, menu and copy controls
docs/
  index.html               documentation hub
  getting-started.html     install and first rule
  rules.html               rule authoring reference
  scripting.html           Lua reference
  examples.html            rule recipes
  faq.html                 troubleshooting
  architecture.html        engine and runtime internals
  docs.css                 documentation shell and long-form typography
  docs.js                  code copy controls
assets/                    logo, icon, product tour
fonts/                     self-hosted Archivo, Inter and Geist Mono
robots.txt
sitemap.xml
.nojekyll
```

The page uses no build step at request time, no external requests, no analytics and no tracking.
The hero product tour is a local silent MP4 that autoplays muted and loops, with no overlay controls.

## Documentation pages

The reference pages under `docs/` (`rules.html`, `scripting.html`, `examples.html`, `faq.html` and
`architecture.html`) are generated from the Markdown documents on `main`. Edit the Markdown source on
`main` and regenerate the HTML; do not hand-edit the generated pages or their anchors. The docs hub
and `getting-started.html` are hand-written.

## Publishing

Repository settings, Pages, Deploy from a branch, branch `gh-pages`, folder `/ (root)`.
