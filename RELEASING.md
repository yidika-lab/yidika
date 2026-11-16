# Releasing Yidika

A release is the binaries that `install.sh`, `install.ps1` and `yidi upgrade` download. Once published, people run it, so these rules are strict. `.github/workflows/release.yml` checks every rule that a machine can check, and stops before publishing if one fails.

## Version numbers

Yidika follows [Semantic Versioning 2.0.0](https://semver.org/). Before 1.0:

| Change since the last release | Bump | Example |
|---|---|---|
| Only fixes, no new syntax, no new std function, nothing removed | patch | 0.3.0 to 0.3.1 |
| A new feature, or a change that can break a program that worked | minor | 0.3.1 to 0.4.0 |

From 1.0 on, a change that breaks a working program needs a major version. A version number is never reused: a release that went wrong is fixed by the next patch, never by moving or replacing its tag.

The version lives in one place, `version` in the root `Cargo.toml`. The tag is that version with a `v`: `v0.3.0`.

## The changelog is the release notes

`CHANGELOG.md` follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/). Every pull request that changes what a user sees adds a line under `## [Unreleased]`, in one of `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`. Write what the user can now do or no longer hits, with the issue or PR number, not how the code changed.

At release time, `## [Unreleased]` becomes `## [X.Y.Z] - YYYY-MM-DD` and a new empty `## [Unreleased]` goes above it. The release workflow publishes that section as the GitHub release notes; a tag without its section fails.

## Steps

1. Check that `main` is green: CI, the compat suite on Windows, `cargo audit`.
2. Open a pull request named `release: vX.Y.Z` that only:
   - sets `version` in `Cargo.toml` and refreshes `Cargo.lock` (`cargo build`);
   - moves the `[Unreleased]` entries under `## [X.Y.Z] - <today>`.
3. Merge it once CI passes.
4. Tag the merge commit on `main` and push the tag:

   ```sh
   git switch main && git pull
   git tag -a vX.Y.Z -m "Yidika vX.Y.Z"
   git push origin vX.Y.Z
   ```

5. Watch the `Release` workflow. It refuses the tag unless:
   - the tag equals the `Cargo.toml` version;
   - the tagged commit is on `main`;
   - `CHANGELOG.md` has a dated `## [X.Y.Z]` section.

   It then builds the five targets, publishes `yidi-<target>.tar.gz` or `.zip` with a `.sha256` next to each, and installs the release with both installers on Linux, macOS and Windows to run a program.
6. If the install smoke test fails, mark the release as a pre-release so `releases/latest` points back to the previous one, fix, and ship the next patch.

## What a release must not contain

- Asset names with a version in them: the installers download from `releases/latest/download/yidi-<target>.<ext>`.
- A dependency with an open RustSec advisory.
- Any credit to an AI or a bot, in the notes, the tag message or the commits (see `AGENTS.md`).
