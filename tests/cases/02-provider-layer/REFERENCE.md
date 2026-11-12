# Module 02 references

Where the expected values of `tests/cases/02-provider-layer/` and
`crates/redpen-core/tests/02_provider_layer.rs` come from. None of them was taken from Redpen's own
output.

## SHA-256

FIPS 180-4, computed with `sha256sum` over the exact strings below (written with `printf '%s'`, so
there is no trailing newline; `\n` is one line feed). The method reproduces module 00's reference
hash `82cab4cdacc60d06a2ccbd424c8fb469aa308bfc7df538fd03a95348e2eeef93` for
`POST /chat/completions\n{"a":2,"b":1}`.

**Request hashes** (module 00: `METHOD path\n` + canonical body; the fake provider's base URL
`http://fake.invalid/v1` leaves the path `/chat/completions`). The body is the one the spec's
Design notes fix for `redpen chat --provider fake --message S`: model `fake-chat`, one user
message, `max_tokens` 256, `temperature` 0 written as the integer `0`.

| Message | Canonical string | SHA-256 |
|---|---|---|
| `hi` | `POST /chat/completions\n{"max_tokens":256,"messages":[{"content":"hi","role":"user"}],"model":"fake-chat","temperature":0}` | `97e299ca6283bd08e7d8b19223e7972b337d72fc0f7c36475f083f6125311a05` |
| `hello` | `POST /chat/completions\n{"max_tokens":256,"messages":[{"content":"hello","role":"user"}],"model":"fake-chat","temperature":0}` | `2fac1c064a10f694ef7caca6d959ebdba60b93e5b72afc02e8029971f1cbd321` |

**Cache keys K** (Design notes: the canonical JSON of `{"provider", "model", "prompt",
"prompt_version", "input"}`), for the input `{"text": "Their going to the libary.", "dialect":
"british"}`, model `example-chat` and prompt `fix-grammar`:

| Provider, version | Canonical string | K |
|---|---|---|
| `own_key`, 1 | `{"input":{"dialect":"british","text":"Their going to the libary."},"model":"example-chat","prompt":"fix-grammar","prompt_version":1,"provider":"own_key"}` | `f17781d369d7294097135b31fafd42208a55e1aff0edafc7c9e4eec21156741c` |
| `fake`, 1 | the same with `"provider":"fake"` | `0d969e53c78f813b9eccb1c6874c40bb9a1be3022f766f23ecd537de2f0c6719` |
| `own_key`, 2 | the same as the first with `"prompt_version":2` | `a2adb751c5e411101085b9018c2650b0317d1eea860c306edbee29d7ea425198` |

## Golden cases

- `fake-chat`: the recording holds the hand-written entry for the `hi` request, answering
  `hello`; the command prints `{"content": "hello", "from_cache": false}` and exits 0.
- `fake-chat-unrecorded`: the same command; its recording holds only an entry for the message
  `hello`, so the `hi` request is unrecorded. The command prints the `AiError` as module 01 prints
  a refusal, `{"error": {...}}` on standard output, serialised as `{"kind", "message"}` plus the
  variant's fields (`hash`, `path`), with the fake transport's message of docs/TESTING.md §1, and
  exits 3.

## Dates

`FakeClock` reads UTC unless an offset is set. `1790000000000` ms is 2026-09-21T14:13:20Z, so its
`usage_daily` day is `2026-09-21`; 8 days earlier is 2026-09-13T14:13:20Z and 6 days earlier
2026-09-15T14:13:20Z. `1790033400000` ms is 2026-09-21T23:30:00Z: at UTC+1 (offset 60) the local
day is `2026-09-22`. `1790042400000` ms is 2026-09-22T02:00:00Z: at UTC-5 (offset -300) the local
day is `2026-09-21`. Computed with GNU `date -u`.

## Local model choice

docs/PUBLIK-INTEGRATION.md §3: drop names containing `embed`, then the largest
`details.parameter_size` at or under 15B, ties by name. Over `llama3.2:3b` (3.2B), `qwen2.5:14b`
(14.8B), `example-16b` (16.0B), `nomic-embed-text` (137M) and `mixtral:8x22b` (141B), the models
at or under 15B after the embedding model is dropped are 3.2B and 14.8B, so `qwen2.5:14b`.
