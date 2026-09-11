# Ollama Cloud

API key to `https://ollama.com/v1/chat/completions`. Not localhost:11434. No PKCE. Reject `BEGIN PUBLIC KEY`. `ollama signin` is local-daemon SSH signing — not a Bearer we can hop.

## Cache

None documented. Strip Codex/Grok fields only. Do not invent a sticky conversation id.

## 稳定性

`GET /api/quota` 404s — quota is `GET /api/usage` plus `POST /api/me` (GET `/api/me` is 405; fields are PascalCase `Email`/`Name`/`Plan`). `limits.*.usage` is a 0..1 fraction, not a percent (`0.095` = 9.5% used = 90.5% remaining). Missing `resets_at` uses global unix buckets, never `now+5h` / `now+7d` and never scraped settings HTML: session `18000 - (epoch % 18000)`; weekly `604800 - ((epoch - 4d) % 604800)` (Monday 00:00 UTC). Do not treat `id_ed25519.pub` / `ollama signin` as a Cloud key, do not invent `cached_tokens`, and do not guess `contextWindow` / vision from a family-size or name regex — live `POST /api/show` wins.
