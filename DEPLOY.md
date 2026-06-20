# Deploying Loupe (unsigned Homebrew Cask)

Loupe ships as an **ad-hoc signed** macOS app (no Apple Developer account / no
$99). Distribution is a personal **Homebrew tap** + a GitHub Release that holds
the `.dmg`. The Cask strips the quarantine flag on install so the app launches
without the "unidentified developer" wall.

> GitHub account: **`JiwonKKang`**. (The app's bundle id stays
> `com.jiwon.loupe` regardless — that's a separate identifier.)

## You create two GitHub repos

1. **`JiwonKKang/loupe`** — this source repo. Holds the app + `.github/workflows/release.yml`.
2. **`JiwonKKang/homebrew-loupe`** — the tap. Holds **`Casks/loupe.rb`** (copy it from this repo's `Casks/loupe.rb`).

## One-time setup

```bash
# 1. push the source
git remote add origin git@github.com:JiwonKKang/loupe.git
git push -u origin main

# 2. create the tap repo and add the cask
#    (locally: a folder with Casks/loupe.rb, then push to JiwonKKang/homebrew-loupe)
mkdir -p homebrew-loupe/Casks
cp Casks/loupe.rb homebrew-loupe/Casks/loupe.rb
cd homebrew-loupe && git init && git add -A \
  && git commit -m "loupe 0.1.0" \
  && git remote add origin git@github.com:JiwonKKang/homebrew-loupe.git \
  && git push -u origin main
```

## Cut a release

```bash
# from the source repo — tag a version, push the tag
git tag v0.1.0
git push origin v0.1.0
```

That fires `.github/workflows/release.yml`: GitHub Actions builds a **universal**
(.dmg) on a `macos-14` runner and publishes a Release named `Loupe v0.1.0` with
`Loupe_0.1.0_universal.dmg` attached.

For each new version: bump `version` in `src-tauri/tauri.conf.json` **and**
`Casks/loupe.rb` (push the cask change to the tap), then tag + push.

## Install (end users)

```bash
brew install --cask JiwonKKang/loupe/loupe
```

`brew` taps `JiwonKKang/homebrew-loupe`, downloads the `.dmg` from the Release, copies
`Loupe.app` to `/Applications`, and the cask's `postflight` runs
`xattr -dr com.apple.quarantine` so it opens immediately.

Manual install (no Homebrew): open the `.dmg`, drag to Applications, then on
first launch **right-click → Open** (or run
`xattr -dr com.apple.quarantine /Applications/Loupe.app`).

## Local verification (no repos needed)

```bash
npm run tauri build          # release build → src-tauri/target/release/bundle/
# .app  → bundle/macos/Loupe.app
# .dmg  → bundle/dmg/Loupe_0.1.0_*.dmg
```

The app needs the `claude` CLI on the user's PATH at runtime (it shells out for
clustering/labeling and the codebase-aware thread Q&A) and a model token set in
the onboarding screen.

## Later: real signing/notarization

When you get an Apple Developer account, set `bundle.macOS.signingIdentity` to
your "Developer ID Application" cert and add notarization env vars — the Cask's
`postflight` xattr line becomes unnecessary. One-line switch.
