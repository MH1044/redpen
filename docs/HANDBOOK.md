# Redpen handbook

The user guide. It grows with the app: each module adds the pages for what you can now do, and
every example in it has been run.

## Version 0.1.0

Nothing checks or rewrites yet. Building from source (see `README.md`) gives:

- a tray icon with one menu item, **Quit**; no window is shown;
- the `redpen` command-line tool, which answers `redpen --version --json` with
  `{"name":"redpen","version":"0.1.0"}` and refuses an unknown argument with
  `error: unknown argument <arg>. Run redpen --help.`

## Coming next

In the order of `docs/ROADMAP.md`: local checks with Harper (findings with fixes, in your English
dialect), then the providers (publik API, your own key, a local model), the rewrite actions,
reading and replacing a selection in any Windows app, the popup and Settings, Linux, the personal
dictionary and style presets, the plagiarism check, and installers.
