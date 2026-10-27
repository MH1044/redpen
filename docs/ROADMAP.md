# Redpen roadmap

| NN | Module | Goal | Tier | Depends on | Status |
|---|---|---|---|---|---|
| 00 | scaffold | Workspace, `verify`, hygiene test, CI, golden harness, the fake transport, a tray icon with Quit | process | none | Done (2026-10-27) |
| 01 | harper-check | Check a text locally: findings with spans, categories, dialects, fixes | load-bearing | 00 | Planned |
| 02 | provider-layer | One OpenAI-compatible client with own key, local (Ollama) and fake providers, cache, errors | load-bearing | 00 | Planned |
| 03 | publik | The Publik contract in full (`ai.chat` and `search`): provisioning, status, errors, consent strings, the search helper, tray balance | load-bearing (money path) | 02 | Planned |
| 04 | rewrite-engine | The six rewrite actions with versioned prompts, validation, redaction, language keeping, the guard, word diff | load-bearing | 01, 02, 03 | Planned |
| 05 | capture-and-replace-windows | Read and replace a selection in any Windows app, refuse secure fields, restore the clipboard | load-bearing (privacy) | 00 | Planned |
| 06 | popup-and-settings-ui | The popup, Settings, welcome, consent and first-run card; the rewrite flow end to end | interface | 03, 04, 05 | Planned |
| 07 | capture-linux | Capture, refusals and replacement on Linux X11, and Wayland's limits stated | load-bearing (privacy) | 05, 06 | Planned |
| 08 | dictionary-styles-and-languages | Personal dictionary in Harper and Settings, import and export, rewrite style presets, languages | interface and core | 01, 04, 06 | Planned |
| 09 | plagiarism-check | Up to 8 exact-phrase web searches per check, snippet matching on this computer, sources and citations | load-bearing (privacy, money path) | 03, 04, 06 | Planned |
| 10 | packaging-and-release | Installers for Windows and Linux, autostart, update check, tag-driven release, listing update | process | all | Planned |

**Ordering.** Module 00 is the scaffold and proves the tooling. Harper checking (01) comes first
because it is the free, local half of the product and needs nothing else. The provider layer (02)
and the Publik module (03) come next, before the rewrite engine (04), the first module that makes a
Publik call, so the money path (chat and web search) is finished and tested once rather than
retrofitted; the live Publik path of 03 needs the app token, which the maintainer obtains with
Publik's setup skill once 03 is done and a release build works, so until then 03 is completed and
tested against the fake server, and later modules continue on fakes. Windows capture (05) is
independent of AI and could run in parallel with 02 to 04, but modules are built one at a time. The
popup (06) joins checks, rewrites and capture into the first complete product. Linux (07) comes
after the Windows product is complete, so it ports a finished design; there is no macOS build,
because GSpot already covers macOS (docs/PRODUCT.md §2). The dictionary and presets (08) refine
features that already work. The plagiarism check (09) is the second feature that leaves the
computer, so it comes once capture, redaction and the popup are proven.
Packaging and release (10) is last.
