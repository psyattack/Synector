# Releasing Synector

How a release is cut: which files change, in what order, and what automation publishes. Releases are
cut by maintainers. Contributors land changes through pull requests; they do not bump the version in
`Cargo.toml`, edit `CHANGELOG.md` or push `v*` tags.

## Version source

The version is single-sourced in `Cargo.toml` (`version = "X.Y.Z"`). `winres` derives both the
numeric `FILEVERSION`/`PRODUCTVERSION` resource fields and the string version Explorer shows from
`CARGO_PKG_VERSION`; `build.rs` supplies only the icon, manifest and descriptive strings, so do not
add a second version there.

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
2. Update `Cargo.toml` as described in [Version source](#version-source); `Cargo.lock` follows.
3. Write the `CHANGELOG.md` section per [Changelog entry](#changelog-entry).
4. Run `cargo check`, `cargo test` and `cargo clippy -- -D warnings` on the release commit.
5. Commit with the release message used in this repository, for example
   `chore(release): bump version to X.Y.Z`, including `CHANGELOG.md`, `Cargo.toml` and `Cargo.lock`.
6. Tag the commit `vX.Y.Z` and push the tag. Pushing `v*` is what starts the release workflow; the
   tag name must equal `v<Cargo.toml version>` or the workflow aborts.
7. Watch `.github/workflows/release.yml` and verify the published release.

## What CI publishes

`.github/workflows/release.yml` runs on a `v*` tag. A manual `workflow_dispatch` must select the tag
too: the workflow aborts unless `GITHUB_REF_TYPE` is `tag` and the ref name equals
`v<Cargo.toml version>`. That gate exports `APP_VERSION` for every later step. The job runs on
`windows-latest`:

1. `cargo build --release --locked`, then the executable's `FileVersion` and `ProductVersion`
   resources are asserted to equal `<version>.0` before anything is packaged.
2. Portable archive `Synector-v<version>-Windows-x64.zip`: `Synector.exe` plus `LICENSE` and
   `NOTICES.md`. The version comes from the tag gate (`APP_VERSION`), not from a second Cargo.toml
   parse and not from a tag-name fallback.
3. Inno Setup 7.1.1 is downloaded from a pinned URL and accepted only when its SHA-256 matches the
   pinned `INNO_SETUP_SHA256`; a mismatch aborts the release before the installer runs. The compiler
   is extracted without installing, then `installer/synector.iss` is compiled with
   `/DAppVersion=<version>` into `Synector-Setup-<version>-Windows-x64.exe`. The installer places
   `LICENSE` and `NOTICES.md` next to the executable.
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
- A storage-format change needs a migration note in the new `CHANGELOG.md` section and a user-facing
  passage in `docs/FAQ.md`; otherwise readers on an older build see no warning.

## If the workflow fails after the tag is pushed

The tag is the trigger, so re-running the job is enough while the tag is correct. If the tag itself
points at the wrong commit, delete the remote tag, fix the release commit locally and push the tag
again; the workflow overwrites the release for that tag.
