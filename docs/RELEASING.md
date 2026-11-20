# Releasing Synector

How a release is cut: which files change, in what order, and what automation publishes. The
user-facing result is described in the README and in `CHANGELOG.md`; this document covers the
mechanics only.

Releases are cut by maintainers. Contributors land changes through pull requests; they do not bump the
version in `Cargo.toml` or `build.rs`, edit `CHANGELOG.md` or push `v*` tags.

## Version source

The version is not derived; it is written in two places and they must match:

| File | What changes | Example for 2.9.2 |
|---|---|---|
| `Cargo.toml` | `version = "X.Y.Z"` | `version = "2.9.2"` |
| `build.rs` | `FILEVERSION` and `PRODUCTVERSION`, packed as `0x00AA00BB00CC0000` | `0x0002000900020000` |

`build.rs` values are what Explorer shows in the executable's file properties. Each dotted component
fills one 16-bit field, so `2.9.2` is `0x0002 0009 0002 0000` and the fourth field stays zero;
`2.10.0` would be `0x0002 000A 0000 0000`.

`Cargo.lock` records the package version, so it changes on the same commit.

## Changelog entry

`CHANGELOG.md` is read by the release workflow, so the newest section must be complete before the tag
is pushed. Entry shape:

```markdown
## [X.Y.Z](https://github.com/psyattack/Synector/compare/v<previous-tag>...vX.Y.Z) - YYYY-MM-DD

### Added
- Short, user-readable line

### Changed
- ...

### Fixed
- ...

<details><summary>Detailed Changelog</summary>

### Fixed
- **scope:** commit-level line (by @author in abc1234)

</details>
```

- The summary sections are written for users, one line per change, no emojis.
- The `<details>` block keeps the commit-level history and author handles.
- The workflow extracts everything between the first `## [` heading and the next one as the GitHub
  release body, so the section must be self-contained and `<previous-tag>` must match the tag of the
  release before this one.

## Steps

1. Pick the SemVer bump: breaking change — major, new feature — minor, fixes only — patch.
2. Update `Cargo.toml` and `build.rs` as described in [Version source](#version-source).
3. Write the `CHANGELOG.md` section per [Changelog entry](#changelog-entry).
4. Run `cargo check`, `cargo test` and `cargo clippy -- -D warnings` on the release commit.
5. Commit with the release message used in this repository, for example
   `chore(release): bump version to 2.9.2`, including `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock` and
   `build.rs`.
6. Tag the commit `vX.Y.Z` and push the tag. Pushing `v*` is what starts the release workflow.
7. Watch `.github/workflows/release.yml` and verify the published release.

## What CI publishes

`.github/workflows/release.yml` runs on a `v*` tag (or manually through `workflow_dispatch`) on
`windows-latest`:

1. `cargo build --release`.
2. Portable archive `Synector-v<version>-Windows-x64.zip`: `Synector.exe` plus `LICENSE` and
   `NOTICES.md`. The version is read from `Cargo.toml`, falling back to the tag name.
3. Inno Setup 7.1.1 compiler is downloaded and extracted without installing it, then
   `installer/synector.iss` is compiled with `/DAppVersion=<version>` into
   `Synector-Setup-<version>-Windows-x64.exe`. The installer places `LICENSE` and `NOTICES.md` next
   to the executable.
4. `SHA256SUMS.txt`: lowercase SHA-256 hashes, one per artifact, sorted by file name.
5. Release notes extracted from the first `CHANGELOG.md` section, with a generic fallback if the
   heading is missing.
6. A GitHub Release named `Synector vX.Y.Z` with every file from `dist_artifacts/`.

The GitHub API digest of each asset is what the in-app updater verifies; `SHA256SUMS.txt` exists for
manual checks. Both are described in
[docs/ARCHITECTURE.md#installation-forms-and-updates](ARCHITECTURE.md#installation-forms-and-updates)
and in the [FAQ](FAQ.md#how-do-updates-work).

## Documentation touch points

- `README.md` and `README.ru.md` use `<version>` placeholders for artifact names, so a version bump
  does not require an edit there.
- A storage-format change needs a migration note in the new `CHANGELOG.md` section and an entry in
  the FAQ's upgrade questions; otherwise readers on an older build see no warning.
- `SECURITY.md` supports only the latest published release — nothing to change unless that policy
  changes.

## If the workflow fails after the tag is pushed

The tag is the trigger, so re-running the job is enough while the tag is correct. If the tag itself
points at the wrong commit, delete the remote tag, fix the release commit locally and push the tag
again; the workflow overwrites the release for that tag.
