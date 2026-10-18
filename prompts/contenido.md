## Content

When you are asked to make content for social media, it lives in `projects/contenido/`:

- `marca/` — the brand context: tone, product, audience, references. **Read it before writing copy.**
- `assets/` — logo, fonts, photos.
- `templates/` — HTML publication layouts.
- `shorts/` — one JSON per short: the ladder of scenes behind `templates/short.html`.
- `semanas/` — the calendar: one markdown per week (`2026-W38.md`).

Flow:

1. Read `marca/` and, when there are any, the previous weeks.
2. Propose 2-4 ideas with their angle, not just the headline.
3. Write the copy in the brand's tone.
4. For the image, prefer the HTML templates: copy one to a working file **inside `projects/contenido/`** (say `projects/contenido/build/`, so relative paths to `assets/` resolve), replace the text and render with
   `browse goto "file://$PWD/projects/contenido/build/<file>.html" --shot --size 1080x1080`
   (story: `1080x1920`). For photographic work, `gen-image --ref` with a brand asset.
6. For a short, the full method is in `projects/contenido/shorts/README.md`. In short: write the spec to `shorts/<slug>.json` and run `projects/contenido/bin/render-short.sh projects/contenido/shorts/<slug>.json`. That yields a 1080x1920 MP4 with no audio in `build/<slug>.mp4`.
7. Show the preview with `jimmy send` and wait for the ok before calling anything final.
8. Record what was approved in `semanas/<year>-W<week>.md`.

Each instance provides the contents of `marca/`, `assets/` and `templates/`.
