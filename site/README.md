# Rift's website

The marketing site and docs for Rift, in `site/` of the Rift repo. Astro, static output, plain
CSS. Deployed to GitHub Pages by `.github/workflows/site.yml` (at the repo's root), on a push that
changes `site/`, `docs/`, `README.md` or the keymap.

## Develop

```sh
pnpm install      # pnpm downloads the Node version pinned in package.json (devEngines)
pnpm dev          # http://localhost:4321 (syncs the docs pages first)
pnpm check        # type-check (syncs the docs pages first)
pnpm test         # the docs transforms and key tables
pnpm build        # static site in dist/ (syncs the docs pages first)
```

`scripts/gates.sh site` (from the repo's root) runs install, test, check and build.

## Release data

Download links, the version and the release date come from the latest release of
[botjaeger/rift-releases](https://github.com/botjaeger/rift-releases/releases) at build time
(`src/lib/release.ts`). Asset names carry the version, so the site rebuilds daily, on push, by hand,
or on a `repository_dispatch` event of type `release` to the Rift repo. The build fails if the API or an expected
asset is missing.

## Docs

`/docs/` renders the Markdown in `src/content/docs/`, with a search box over every section
(`/docs/search.json`, built from the same files).

- `getting-started.md`, `features.md`, `configuration.md` and `keyboard-shortcuts.md` are built from
  the repo's `docs/` and `README.md` by `scripts/sync-docs.mjs`, which `pnpm dev`, `check` and `build`
  run first. They are not committed (`.gitignore`): change the repo's docs, never these.
- The sync turns links to unpublished pages into plain text, drops "see …" pointers to them and
  bullets about Rift's own test switches, and stops with an error on a private path, a link to the
  private repo, or a reference it does not know how to clean up. Extend the lists at the top of
  `scripts/docs-transform.mjs` when it does; `pnpm test` checks those rules.
- In key tables (first column "Shortcut" or "Key") the sync writes a Windows and Linux version
  beside each macOS shortcut (`scripts/os-keys.mjs`, following the app's own rule: an action's
  chord without ⌘, its own Windows/Linux key where it has one, else ⌘ as Super). The keyboard
  shortcuts page shows a toggle for them. The
  sync also reads `crates/rift-core/src/keybindings.rs` from the checkout and stops if any Linux
  shortcut differs from the one the app shows.
- `phone.md` is written by hand from the Rift docs (Remote access); the sync never touches it.
- `/features/` shows the rows of the synced `features.md` table, grouped by the `prefixes` in
  `src/pages/features.astro`. The build fails when a row fits no group, so a new README feature
  needs a prefix there.

## Assets

- The site is served at https://botjaeger.github.io/rift/, so every internal link and asset goes
  through `url()` in `src/lib/url.ts` (Markdown links get the base path from `astro.config.mjs`).
  `pnpm build` ends with `scripts/check-links.mjs`, which fails on a link outside `/rift/` or to a
  page the build did not produce.
- For a custom domain, put it in `public/CNAME` and set the same domain under Settings › Pages ›
  Custom domain (deploys from Actions ignore the file); the site then moves to the domain's root.
- `node scripts/icons.mjs` regenerates the logo sizes and favicons from `scripts/rift-1024.png`.
- `scripts/og.html` is the source of `public/og.png` (screenshot it at 1200×630).

## Placeholders

`[APP STORE LINK]`, `[GOOGLE PLAY LINK]` and `[LICENSE]` are still to be filled in.
