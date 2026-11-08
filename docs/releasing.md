# Releasing Maine

Maine ships as an unsigned, ad-hoc signed macOS disk image built by CI from a
git tag. One tag push runs the tests, builds the app, packages the DMG and
publishes the GitHub Release. The workflow is
[`.github/workflows/release.yml`](../.github/workflows/release.yml).

## Cutting a release

Merge everything you want in the release to `main` first, then tag that commit:

```sh
git checkout main
git pull
git tag v1.2.3
git push origin v1.2.3
```

The tag is the only version source. The workflow then:

1. rejects any tag that is not `vX.Y.Z` or `vX.Y.Z-suffix`,
2. selects the newest installed Xcode and runs `swift test` in `apps/macos`,
3. builds the app with `scripts/build-app.sh --release --version 1.2.3`
   (the build number is the commit count of the tagged commit),
4. packages `Maine-1.2.3.dmg` plus `Maine-1.2.3.dmg.sha256` with
   `scripts/make-dmg.sh`,
5. creates the release with generated notes and both assets attached.

The in-app updater only offers a release that has both assets, so never publish
one by hand without the checksum file, and never edit a published version's
files in place.

## Pre-releases

A tag with a hyphen suffix (`v1.2.3-rc.1`, `v1.2.3-beta.2`) is published as a
GitHub pre-release. GitHub's "latest release" endpoint never returns
pre-releases, so the updater ignores them. Use one for the first run of the
workflow, or for a build you want to try before it reaches everyone.

## Local dry run

Before tagging, build and package the same way CI will:

```sh
cd apps/macos
scripts/build-app.sh --release --version 1.2.3
scripts/make-dmg.sh 1.2.3
hdiutil verify build/Maine-1.2.3.dmg
(cd build && shasum -a 256 -c Maine-1.2.3.dmg.sha256)
```

The DMG contains `Maine.app` and an `Applications` symlink. Install it by
dragging the app over, or run `scripts/install.sh`. `--build N` overrides the
build number; it must keep increasing across releases, so let it default unless
you are re-running a failed build of the same commit.

## First launch and Gatekeeper

Maine has no Developer ID and is not notarized: provisioned signing was tried
and deliberately rolled back (AGENTS.md invariant 7). The first launch of a
downloaded copy may be blocked. Either right-click the app in Finder and choose
**Open**, or clear the download flag:

```sh
xattr -dr com.apple.quarantine /Applications/Maine.app
```

Updates that the app installs itself are copied from the mounted disk image by
Maine and launched locally, so the prompt normally does not appear again. If
macOS does ask, the same right-click rule applies.

Do not add entitlements or a signing identity to make this go away; that
decision is settled and revisiting it means re-signing every week.

## What the checksum proves

The updater trusts HTTPS to GitHub plus the SHA-256 published in the same
release. That catches a truncated or corrupted download and tampering in
transit. It does **not** prove authorship: there is no Ed25519 signature (that
was considered and declined), and anyone who can publish a release to this
repository can publish an update the app will accept. Protect repository and
release access accordingly.

To check a download yourself, compare its hash with the release page, or the
file against a local rebuild of the tag:

```sh
shasum -a 256 Maine-1.2.3.dmg
cat Maine-1.2.3.dmg.sha256
```

## Version rules

- Versions are semver without the leading `v` in the app and in asset names.
- A release is only offered when it is strictly newer than the running app.
  `1.10.0` is newer than `1.9.0`; an equal or lower tag is ignored.
- The build number is only informational; comparisons use the version.

## When a release is broken

Publish a patch release with the fix. Do not delete and re-upload a version
that people may already have: the installed app compares versions, so a
re-published lower version is never offered and an installed newer build is
not downgraded. If you need the release page to point somewhere else, mark a
different release as latest (`gh release edit v1.2.2 --latest`) and say so in
its notes; users on a newer version still will not be moved backwards.
