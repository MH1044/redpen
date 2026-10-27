# ADR-0009: Plagiarism from snippets: exact-phrase search, result pages never fetched

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

A plagiarism check asks which sentences of a selection already appear on the web. Publik offers
web search at one cent a search, whatever the result count, and the user may bring a Brave Search
API key instead. Sending a whole essay to a search engine would send far more than the check
needs, fetching every result page would mean a request per result and parsing arbitrary HTML,
and an "originality percentage" would claim a precision that snippet matching cannot give.
The user's text is private: only what the check needs may leave the computer (invariant 13).

## Decision

A check sends at most eight searches, each an exact-phrase query of at most twelve words chosen
from a sentence, spread across the text, never overlapping an email, phone number, IBAN, card
number or long digit run. The answers' snippets are compared with the sentences on this computer
by word 3-gram overlap: Found on the web at 0.6 or more, Similar text at 0.3 or more. Result
pages are never fetched; Open hands a result's URL to the system browser. The report lists sources
with a citation in APA 7, MLA 9 or Chicago 18, and says, always, that a match means the words
appear on the web, not that they were copied. The total cost is shown, at most $0.08 a check on
publik API.

## Consequences

- At most eight short phrases leave the computer per check, and the fake transport can assert
  exactly that in tests.
- The check costs at most eight cents and finishes in a few seconds; a cached search costs
  nothing.
- Recall is bounded by what a search engine's snippets show: a sentence that was copied but
  paraphrased, or whose source page is not indexed, is not found, and the report says so in its
  wording.
- Costs: a second consent screen and a second provider choice (web search), and a corpus and a
  paid gate to confirm the thresholds.

## Alternatives considered

- **Fetching result pages**: a request per result (dozens per check), arbitrary HTML to parse, and
  the user's reading of result pages visible to every site.
- **Scraping a search engine without a key**: fragile, against the engine's terms, and no
  developer agreement to stand on.
- **An overall originality percentage**: implies a precision the method lacks; per-sentence labels
  with sources say what is actually known.
- **Searching every sentence**: dozens of searches per check, most of them redundant, at a cost
  that would make the feature unused.
