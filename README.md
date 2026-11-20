# Synector landing page

Static one-page site served by GitHub Pages from this branch.

The application source lives on `main` in the same repository. This branch contains only the site:

```
index.html
styles.css
main.js
.nojekyll
assets/
fonts/
```

The page uses no build step, no external requests, no analytics and no tracking.

## Local preview

```powershell
python -m http.server 8080 --directory .
```

Then open `http://localhost:8080/`.

## Publishing

Repository settings, Pages, Deploy from a branch, branch `gh-pages`, folder `/ (root)`.
