# 09: Plagiarism check

## Goal

Select text anywhere, press **Plagiarism check**, and see which sentences already appear on the web,
with the sources and a citation ready to copy. Redpen searches at most eight short phrases from the
selection, through publik API web search (one cent a search) or the user's own Brave Search API key,
compares the phrases with the search result snippets on this computer, and never opens a result
page. Nothing is searched until the user presses the button.

## Depends on

03 (the `search` helper and its errors), 04 (the redaction detector), 06 (popup, Settings, consent).

## Scope

1. **Limits.** At least 40 words: "Select at least 40 words for a plagiarism check (this selection has
   23)." At most 1,500 words: "Select at most 1,500 words for a plagiarism check (this selection has
   1,812)." Neither sends anything.
2. **Sentences.** UAX #29 sentence boundaries (`unicode-segmentation`); words are UAX #29 words that
   contain a letter or digit. A sentence is **eligible** when it has at least 8 words and is not
   quoted. A sentence is **quoted** when at least 80% of its words lie between a pair of quotation
   marks (`"…"`, `“…”`, `«…»`); it is reported as "Quoted, not checked".
3. **The phrase for a sentence.** Window length L = min(12, words in the sentence). Every window of L
   consecutive words is a candidate, except a window that overlaps a span of the redaction detector:
   `redact::find_spans(text) -> Vec<Range<usize>>`, the byte ranges of everything module 04's
   `redact()` would replace (emails, phone numbers, IBANs, card-shaped and other runs of 8 or more
   digits), with the same patterns; this module adds the function to `rewrite/redact.rs` and makes
   `redact()` use it, so the two can never disagree.
   Score = the number of words of 6 or more characters. The highest score wins; ties go to the
   earliest window. A sentence with no candidate is reported as "Not checked: contains private
   details". The query is the window's words as written, joined by single spaces, inside double
   quotes (an exact-phrase search); if it is over 400 characters, words are dropped from its end
   until it fits.
4. **Which sentences.** Take the n sentences whose state is `Search` (eligible and with a phrase;
   `TooShort`, `Quoted` and `PrivateDetails` sentences are set aside first), in text order: all of
   them if n ≤ 8; otherwise the ones at indices round(i × (n − 1) / 7) for i = 0 to 7, so the eight
   are spread across the text, and the others become `NotSelected`. The report says how many were
   not searched: "15 sentences not checked (a check searches 8)". If n is 0, nothing is sent and the
   report says "No sentence could be checked." with the reasons of 2 and 3.
5. **Searching.** One search per phrase, in text order, `count` 5, through `SearchProvider` (below).
   Cached results (key in Design notes) are used without a request. **Cancel** stops sending further
   searches; finished ones are kept.
6. **Matching, on this computer.** Normalise query and snippet alike: NFKC, lower case, curly quotes
   and apostrophes made straight, HTML tags removed and `&amp; &lt; &gt; &quot; &#39;` decoded, then
   words as in 2. Strength = shared word 3-grams ÷ the query's word 3-grams. **Found on the web** at
   0.6 or more, **Similar text** at 0.3 or more, otherwise the result is not shown. A sentence takes
   the label of its strongest result; its results are listed by strength, then by search order.
7. **Report** (popup result view): the text with each searched sentence marked (Found: red underline,
   Similar: amber underline, nothing found: none), the summary line "Found on the web: 2 of 8 checked
   sentences · Similar text: 1", the not-checked lines of 2, 3 and 4, and for the selected sentence
   its sources: title, site, date when known, the label, **Open** (system browser), **Copy link**,
   **Copy citation** with a style menu (APA 7, MLA 9, Chicago 18; remembered). Below the list, always:
   "Redpen compares your text with search result snippets and does not open the pages. A match means
   the words appear on the web, not that they were copied. Check a citation against the page before
   you use it."
8. **Citations**, plain text, built from the result's `title`, its `site` with a leading `www.`
   removed, its `url`, the **published date** (only when `age` is an ISO date `YYYY-MM-DD`; otherwise
   there is none) and the **access date** (today, from `Clock`). Date forms: long `September 30,
   2026`; APA `2026, September 30`; MLA `30 Sept. 2026` (MLA abbreviates every month except May,
   June and July: Jan., Feb., Mar., Apr., Aug., Sept., Oct., Nov., Dec.). MLA drops the URL's
   `https://` or `http://`. A title ending in `?` or `!` gets no added full stop. For the result
   `{title: "Coral reefs of the world", site: "www.example.org", url:
   "https://www.example.org/reefs/coral", age: "2026-09-30"}` accessed on 2026-10-26, the six
   strings are exactly:

   | Style | With a published date | Without one |
   |---|---|---|
   | APA 7 | `Coral reefs of the world. (2026, September 30). example.org. https://www.example.org/reefs/coral` | `Coral reefs of the world. (n.d.). example.org. Retrieved October 26, 2026, from https://www.example.org/reefs/coral` |
   | MLA 9 | `"Coral reefs of the world." example.org, 30 Sept. 2026, www.example.org/reefs/coral. Accessed 26 Oct. 2026.` | `"Coral reefs of the world." example.org, www.example.org/reefs/coral. Accessed 26 Oct. 2026.` |
   | Chicago 18 | `example.org. "Coral reefs of the world." September 30, 2026. https://www.example.org/reefs/coral.` | `example.org. "Coral reefs of the world." Accessed October 26, 2026. https://www.example.org/reefs/coral.` |

   This module checks each pattern against the style's own guidance (the APA Style website, the MLA
   Handbook's public pages, the Chicago Manual of Style Online quick guide) and records the pages and
   dates in Design notes before the golden files are written; a correction there changes this table
   in the same commit.
9. **Search providers** (`providers.search` in `settings.json`: `null` | `publik` | `own_key`):
   - **publik API**: `publik::search` from module 03, `POST {base_url}/search`.
   - **Your own key**: a Brave Search API key, saved in `secrets.json` as `search_key`. Settings label:
     "Brave Search API key (for plagiarism checking)". Request: `GET
     https://api.search.brave.com/res/v1/web/search?q=<q>&count=5` with `X-Subscription-Token` and
     `Accept: application/json` (this is the vendor's documented form; check it against Brave's API
     documentation before implementing and record any change in Design notes). Each result in
     `web.results` maps to `{url, title, snippet: description, site: meta_url.hostname, age: the date
     part of page_age}`. Retries: twice on timeout, 429 and 5xx, after 0.5 s and 1 s. **Test** in
     Settings sends one search for `redpen` with `count` 1 and shows "Works." or the error.
   - Resolution (contract §6a): a saved own search key always comes first (saving one sets
     `providers.search` to `own_key`; clearing it sets it back to `null`); otherwise publik API when
     `providers.search` is `publik` and a credential exists; otherwise the search consent of 10. A
     local model cannot search: if the consent is cancelled, the check shows "Plagiarism checks need
     publik API or a Brave Search API key (Settings → Providers)." and nothing is sent.
10. **Consent**, on a press of Plagiarism check with no own search key saved and `providers.search`
    unset (`plagiarism_check` answers `NeedsSearchConsent`): the popup state `search-consent`
    (module 06's reducer gains it) renders `search_consent_info()`: title "Choose how Redpen searches
    the web"; data-path sentence "A plagiarism check sends up to eight short phrases from the text you
    selected, each at most twelve words, to a web search, and nothing else. It runs only when you
    press Plagiarism check."; with no Publik credential yet, `publik::DISCLOSURE` verbatim and the
    buttons **Continue with publik API**, **Use my own search key**, **Cancel**; with a credential,
    the cost line "publik API charges one cent a search, at most $0.08 a check." and **Use publik
    API**, **Use my own search key**, **Cancel**. `search_choose("publik")` provisions if needed
    (module 03's `publik_connect`) and sets `providers.search`; "Use my own search key" opens
    Settings → Providers → Web search. At $0.00 the pending check is not sent and module 06's
    first-run card shows with Try again, its alternatives reading **Use my own search key** instead of
    the own key and local model buttons.
11. **Money.** The button's tooltip says "Up to 8 searches · up to $0.08 with publik API". The footer
    after a check: "Searched with publik API · 8 searches · $0.08 · $3.04 of publik balance" (the
    charge summed from `charge_micros`, $0.00 when every result came from the cache) or "Searched with
    your Brave Search API key · 8 searches". `usage_daily` counts each search under feature
    `plagiarism` (this module adds `Feature::Plagiarism` to module 02's enum; it has no tier). Eight
    searches stay under the contract's ten-call batch confirmation.
12. **Errors**, per search, never retried automatically on publik API. "Stopping" keeps the finished
    results, sends nothing more, fills `PlagiarismReport.stop` with the message (and the one link,
    where there is one) and labels the report "Stopped after N of 8 searches", N being the searches
    that finished:
    - publik 402: stop with `error.message`, exactly one link (`error.top_up_url`) and "or use your
      own search key in Settings".
    - publik 502 or 504 (`ProviderFailed`): mark that sentence "The search provider failed for this
      sentence; nothing was charged." and go on with the rest; the report offers **Try again** for the
      failed sentences only.
    - publik 401 and 403 `install_revoked`: stop with "publik API disconnected this computer." and
      **Reconnect**; 429: stop with `error.message`; 503: stop with "publik API is busy; try again in
      a moment"; other codes: stop with "publik API <code>: <error.message>"; timeout (30 s): stop
      with "publik API did not answer within 30 seconds. Try again."
    - Brave 401 or 403: stop with "Brave Search did not accept the key. Check it in Settings →
      Providers."; 429 and 5xx after the retries, and other codes: stop with "Brave Search answered
      <code>. Try again later."; timeout: stop with "Brave Search did not answer within 30 seconds."
    - Offline: stop with "No connection. A plagiarism check needs a connection; checking still
      works."
13. **Privacy.** Only the query phrases leave the computer. No result page is fetched; **Open** hands
    the URL to the system browser. The sent log (when on) records each query with endpoint `/search`.
    Logs never contain a query, a result URL or the key.
14. The paid gate `scripts/gates eval-plagiarism` (docs/TESTING.md §4) over the corpus in
    `tests/cases/09-plagiarism-check/corpus/`; the maintainer runs it once (at most $0.80) and the
    thresholds are confirmed or set from that run.
15. The CLI gains `redpen plagiarism (--plan-only | --provider fake|publik-env|own-key-env
    [--recording R]) [--style S] FILE`: `--plan-only` prints the plan (sentences, phrases, skips) and
    sends nothing; with a provider it prints the plan and the report as JSON. `publik-env` reads
    `PUBLIK_API_KEY` and `PUBLIK_API_BASE_URL`; `own-key-env` reads `REDPEN_SEARCH_KEY`.

## Files

- `crates/redpen-core/src/plagiarism/{mod.rs, sentences.rs, phrase.rs, select.rs, matching.rs,
  cite.rs, report.rs}`, `crates/redpen-core/src/provider/search.rs` (`SearchProvider`, the Brave
  client, cache), `crates/redpen-core/src/flow/plagiarism.rs`
- `app/src/windows/Plagiarism.tsx`, `app/src/components/{Sources.tsx, CitationMenu.tsx}`,
  `app/src/state/plagiarism.ts`; Settings → Providers gains the Web search section
- `app/src-tauri/src/commands/plagiarism.rs`
- `crates/redpen-cli/src/plagiarism.rs`
- `SECURITY.md` (the search requests), `docs/HANDBOOK.md` (the plagiarism chapter)
- Tests: `crates/redpen-core/tests/09_plagiarism.rs`, `app/src/state/09-plagiarism.test.ts`, golden
  cases in `tests/cases/09-plagiarism-check/`

## Interfaces

```rust
pub struct SearchResult { pub url: String, pub title: String, pub snippet: String,
                          pub site: String, pub age: Option<String> }
pub struct SearchOutcome { pub results: Vec<SearchResult>, pub charge_micros: Option<u64>,
                           pub from_cache: bool }
pub trait SearchProvider { async fn search(&self, q: &str, count: u8) -> Result<SearchOutcome, SearchError>; }
pub enum SearchError { NeedsCredit { message: String, top_up_url: Option<String> },
                       ProviderFailed, Disconnected, RateLimited(String), Busy, Timeout, Offline,
                       Other { code: u16, message: String } }

pub struct PlagiarismPlan { pub sentences: Vec<PlannedSentence>, pub not_searched: usize }
pub enum SentenceState { Search { query: String }, TooShort, Quoted, PrivateDetails, NotSelected }
pub fn find_spans(text: &str) -> Vec<std::ops::Range<usize>>;                 // in rewrite/redact.rs
pub fn plan(text: &str) -> Result<PlagiarismPlan, PlagiarismError>;           // TooFew, TooMany
pub fn strength(query: &str, snippet: &str) -> f32;
pub enum CitationStyle { Apa7, Mla9, Chicago18 }
pub fn cite(r: &SearchResult, style: CitationStyle, accessed: Date) -> String;
pub struct SearchStop { pub message: String, pub link: Option<String>,
                        pub action: Option<StopAction> }                     // Reconnect | TryAgain
```

Tauri: `plagiarism_check {session_id}` → `PlagiarismReport {sentences[], summary, provider_line,
charge_micros?, stopped_after?, stop?: SearchStop}`, or before any search the errors
`PlagiarismError` (`TooFew`, `TooMany`), `NeedsSearchConsent` and `NeedsSearchProvider`;
`search_consent_info` → `SearchConsentInfo {title, data_path, disclosure?, cost_line?}`;
`search_choose("publik" | "own_key")`; `plagiarism_retry_failed {session_id}`; `copy_citation
{session_id, result_id, style}`; `search_key_save` / `search_key_clear`; `set_providers` gains
`search`; `provider_test("search_key")`. Popup key 7 runs the check.

## Out of scope

- Fetching or reading result pages; scraping a search engine's result pages as a keyless fallback;
  checking against private databases, journals or a document store; an overall "percent original"
  score; AI-writing detection.

## Design notes

- Decided: snippets only. Exact-phrase search already finds the page; reading it would mean a second
  network request per result and parsing arbitrary HTML.
- Decided: the cache key is SHA-256 of the canonical `{"provider", "model": "search", "prompt":
  "search", "prompt_version": 1, "input": {"q", "count"}}`, stored in `ai_cache` with the same
  expiry as rewrites (at most 7 days; 0 keeps it in memory only).
- Before implementing: fetch `https://publikhq.com/skills/search.md` raw with `curl` again, record
  changes here, and ask the maintainer to run `scripts/publik-proof --search` once (one cent).
- Further decisions are filled in here during implementation.

## Acceptance tests

Golden cases in `tests/cases/09-plagiarism-check/` (input text, recorded searches, expected plan and
report); flow tests with `FakeCapture` and `FakeTransport`; Vitest for the view.

1. 23 words → `TooFew` with the message; 1,812 words → `TooMany`; neither makes a request.
2. Sentences of 5, 8 and 20 words → `TooShort`, `Search`, `Search`; a sentence 90% inside “…” →
   `Quoted`.
3. Phrase choice golden: a 20-word sentence gives the expected 12-word window; a window containing
   `ana@example.com` is never chosen; a sentence where every window holds a 9-digit number →
   `PrivateDetails`.
4. 23 eligible sentences → searched indices [0, 3, 6, 9, 13, 16, 19, 22] and "15 sentences not
   checked (a check searches 8)".
5. Request on publik API: exactly `POST /search`, body `{"q":"\"…\"","count":5}`, the key only in
   `Authorization`; on the own key: the Brave URL with `q` and `count`, the key only in
   `X-Subscription-Token`.
6. `strength`: identical text → 1.0; a snippet sharing 6 consecutive words of a 10-word query →
   0.5 (Similar text); `<strong>`-wrapped words and `&amp;` match as plain text.
7. A recorded run with two Found and one Similar → summary "Found on the web: 2 of 8 checked
   sentences · Similar text: 1" and the expected marks.
8. Citations: the six strings of scope 8 exactly; with the title `Are reefs dying?` no full stop is
   added after it in any style; an access date in March gives `Mar.` in MLA and `March` elsewhere.
9. 402 on the third search: two searches made before it, none after; "Stopped after 2 of 8 searches";
   exactly one link; nothing cached for the third.
10. 502 on the fourth search: the other seven run; the fourth shows "nothing was charged"; Try again
    sends only the fourth; no automatic retry.
11. A second identical check sends no request and its footer shows $0.00.
12. A saved own search key → the Brave request, never `/search`, even with a Publik credential and
    `providers.search` = `publik`; a Brave 429 is retried twice, a publik 429 never.
13. Only Local chosen, no search key and `providers.search` unset → `NeedsSearchConsent`; Cancel →
    the "need publik API or a Brave Search API key" message; zero requests throughout.
14. Across a whole scripted check the fake transport sees only search requests: no result URL is
    ever requested.
15. With the sent log on, each query is logged with endpoint `/search`; the `tracing` output holds no
    query text, result URL or key.
16. Consent: first press with `providers.search` unset and no search key shows the search consent
    strings exactly, with `DISCLOSURE` when there is no credential and the cost line when there is;
    with no credential and `balance_micros: 0` after Continue, the check is not sent and the
    first-run card shows with **Use my own search key** and no local-model button.
17. `usage_daily` gains one `plagiarism` call per search and the summed `charge_micros`.
18. Reducer: idle → "Searching 3 of 8… (publik API)" → report; Cancel after three keeps three
    results and sends no fourth.
19. `find_spans` covers exactly what `redact()` replaces on `tests/cases/04-rewrite-engine/redaction.jsonl`.
20. A selection whose every sentence is short, quoted or private → "No sentence could be checked."
    and zero requests; a Brave 401 stops with its message after one request.

## Status

Planned
