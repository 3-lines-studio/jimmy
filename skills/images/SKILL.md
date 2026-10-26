---
name: images
description: Generate or edit an image with Gemini (`gen-image`).
---

# images

`gen-image "prompt" -o out.jpg` — generates or edits an image with Gemini.

- The output is always JPEG (`.jpg`/`.jpeg`): it is the only format the API accepts.
- `--size` sets the total area, not the long side: `1K` is ~1 MP, `2K` ~4 MP and `4K` ~16 MP. At `9:16`, `1K` gives 768x1376 and `2K` 1536x2752.
- `2K` is the default and enough for Instagram: it covers 1080 wide at any aspect and weighs ~3 MB. `1K` falls short (768 wide in portrait) and `4K` is overkill.
- When editing a photo, pass the input's `--aspect`: a 4:3 photo goes with `--aspect 4:3`, because the `1:1` default crops it. For new pieces, `9:16` (stories) or `1:1` (posts).
- The API normalizes the reference: a photo thousands of pixels wide gives no more detail than a ~1 MP one, it only adds seconds of upload.
- With `--ref photo.png` (repeatable) it takes that image as a reference or edits it.
- It needs `GEMINI_API_KEY`; if it is missing, say so and stop.
- For a chart, a floor plan or anything with layout: `matplotlib` or HTML+CSS with `browse --shot` (skill `python`).
