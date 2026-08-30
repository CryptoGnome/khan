Load when the company needs ANY image (logo, coin-ticker art, share card, placeholder): generate_image is production; pillow is fallback only.
# image_generation — how the company makes images

## The production path
Use the built-in `generate_image` tool (OpenRouter image models; default ~$0.01, stronger overrides exist). Local pillow rendering is the fallback ONLY — for purely geometric assets (solid, gradient, grid, a typographic text-logo) or when the real path is down. Never ship pillow output as token art, a share card, or anything a human will judge as "the art".

## Prompting
ONE flowing sentence, not a keyword pile, front-loading the subject: [subject with key details] → [style/medium] → [composition/shot] → [lighting/color]. Words that must appear IN the image go in "double quotes". Phrase avoids as positives ("clean empty background", never "no clutter" — negatives are ignored). Iterate by changing ONE thing, never re-rolling the same prompt.

## ALWAYS convert the bytes to a real PNG before hosting
The tool may write **WEBP** (RIFF magic) even when the path ends in `.png`, and metadata hosts want a real PNG (`\x89PNG`). After every call:
```python
from PIL import Image
im = Image.open(src).convert('RGB')
im.resize((1024,1024), Image.Resampling.LANCZOS).save(out_png, 'PNG', optimize=True)
```
Verify before shipping: size > ~30KB (smaller usually means a failed/blank render), magic is `\x89PNG` not RIFF, unique colors in the thousands (a few dozen = a flat render, not real art), and LOOK at the saved file.

## Do not probe chat-completion "image" models
Chat-hybrid models with no real images endpoint burn tokens on reasoning and return empty/corrupt bytes — a `status: ok` with billed cost there is a seller flicker, not a working image. Budget was burned proving this once; the lane is closed.

## Gotchas
- Never print or log the provider API key.
- Evergreen branded assets (share cards) regenerate through their dedicated script, not a one-off generate_image call — unless replacing the asset on purpose.

## OUR INSTANCE
Record here: default + override model IDs with prices, the share-card script path, and one known-good example render.
