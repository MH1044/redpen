# Module 03 references

Where the expected values of `tests/cases/03-publik/`, `crates/redpen-core/tests/03_publik.rs` and
`app/src-tauri/tests/03_publik.rs` come from. None of them was taken from Redpen's own output.

## SHA-256

FIPS 180-4, computed with `sha256sum` over the exact strings below (written with `printf '%s'`, so
there is no trailing newline).

**Idempotency keys** (docs/PUBLIK-INTEGRATION.md §5): the first 32 hex characters of the cache key
K for a first call, or of SHA-256(`<K>:<n>`) for the n-th Try again after an answer failed
validation.

The spec's K (module 00, test 2), `22f53c07ac6b84e42b6fea37d9f1ce05a6c42780307b5932ccf2031f7ef66dd1`:

| n | String hashed | SHA-256 | Key |
|---|---|---|---|
| 0 | none: K itself | | `22f53c07ac6b84e42b6fea37d9f1ce05` |
| 1 | `<K>:1` | `b6058582879f1920e30934ad9402c4db9e12da66f924d6ffd0f8a0dd6a514b9f` | `b6058582879f1920e30934ad9402c4db` |
| 2 | `<K>:2` | `84491dd9d4f881c8ec68ba4c829a264a950259a273f524ab864456639c33e87d` | `84491dd9d4f881c8ec68ba4c829a264a` |

**Cache keys K** (docs/ARCHITECTURE.md §4: the canonical JSON of `{"provider", "model", "prompt",
"prompt_version", "input"}`) for the call the core tests make: Fix grammar, prompt `fix-grammar`,
input `{"text": "Their going to the libary.", "dialect": "british"}`.

| Provider, model, version | Canonical string | K |
|---|---|---|
| `publik`, `publik-fast`, 1 | `{"input":{"dialect":"british","text":"Their going to the libary."},"model":"publik-fast","prompt":"fix-grammar","prompt_version":1,"provider":"publik"}` | `748f0ab593f7379e156586aeba37f558d87039ad6da30400ee0c179dba3135b4` |
| `publik`, `publik-fast`, 2 | the same with `"prompt_version":2` | `58caca32c98b3257e71ff220c47c6df33293b53f682c2b5db297a652e522d820` |
| `own_key`, `example-chat`, 1 | module 02's reference (tests/cases/02-provider-layer/REFERENCE.md) | `f17781d369d7294097135b31fafd42208a55e1aff0edafc7c9e4eec21156741c` |

The idempotency keys of the first row:

| n | SHA-256 of `<K>:<n>` | Key |
|---|---|---|
| 0 | none: K itself | `748f0ab593f7379e156586aeba37f558` |
| 1 | `fc0efd139ba7f503da3f33503006000307bddbf4bf93f72084587f4161e78e7b` | `fc0efd139ba7f503da3f335030060003` |
| 2 | `7ba59aa7d4199e866e157ae016c9b78c69769d47243fe194085eed311d9d2ac0` | `7ba59aa7d4199e866e157ae016c9b78c` |

and of the second row, n = 0: `58caca32c98b3257e71ff220c47c6df3`.

## Dates

The fake clock starts at `1790000000000` ms, 2026-09-21T14:13:20Z, so its `usage_daily` day is
`2026-09-21` (tests/cases/02-provider-layer/REFERENCE.md).

## Money

Balances and charges are integer micro-dollars (docs/ARCHITECTURE.md §4), so every figure is exact:
3120000 is $3.12, 1000000 is $1.00, 50000 is $0.05, 10000 is one cent (one search,
docs/PUBLIK-CONTRACT.md §6a), 2100 is $0.0021 and 100 is $0.0001, both under one cent and so written
with four decimals (the spec's Design notes, `balance_line`). Two answers charged 210 and 180 add up
to 390 (bullet 1, R11).

## Strings

Every expected string is copied from its source: the data-path sentence from
docs/PUBLIK-INTEGRATION.md §4 (two lines there, joined by one space), `DISCLOSURE` and the
`balance_line` forms from the spec's Design notes, the error messages from the spec's `AiError`
table and docs/PUBLIK-CONTRACT.md §5 and §8, the self-test lines from the spec's acceptance test 11,
and the version from the workspace `Cargo.toml` (0.1.0 when these tests were written).

## Fake responses

Each `*.fake.json` is one hand-written response, `{"about", "status", "headers", "body"}`, with
invented values; recordings (`recording.json`) never carry status headers, so these live here
(docs/TESTING.md §1). The key in them is the contract tests' fake `pk_live_a`; links are on
`publikhq.com` with invented paths, and search results on reserved example domains.

| File | Stands for |
|---|---|
| `installs-201`, `installs-201-funded` | a new install at $0.00 and at $0.05 |
| `installs-200-replay` | a replay of a known install_id, `key: null` |
| `installs-403-install-revoked`, `-again` | a revoked install, twice, with different messages |
| `installs-429`, `installs-503-with-message`, `installs-500` | busy; another code with a message; a 500 without one |
| `revoke-200`, `wallet-200`, `models-200` | Disconnect, the wallet ($3.12, linked), the model list |
| `chat-200-status` | an answer carrying all eight `x-publik-*` headers of contract §7 |
| `chat-200-no-choices` | an answer with nothing usable |
| `chat-402-anonymous`, `chat-402-claimed` | 402 before and after linking, each with links besides `top_up_url` |
| `chat-401-invalid-api-key`, `chat-401-key-revoked`, `chat-403-install-revoked` | disconnected |
| `chat-429`, `chat-503`, `chat-500`, `chat-400` | rate limited, busy, a server error, another 4xx |
| `search-200`, `search-200-header-charge` | two results charged in the body; one result charged in the header only |
| `search-402`, `search-502`, `search-504` | a search 402; the provider failed or timed out, nothing charged |

The fake app token the tests use is built at run time from the slug, so the committed
`app/src-tauri/publik-app-token.txt` stays the only place a `pat_` token can appear. The test that
checks provisioning never sends the hostname reads this computer's name at run time; no name is
written down anywhere.
