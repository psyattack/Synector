## Summary

<!-- What does this change and why? Link the issue if there is one. -->

## Type

- [ ] Bug fix
- [ ] Feature
- [ ] Refactor or cleanup
- [ ] Documentation
- [ ] Build, CI or release

## Testing

<!-- Commands you ran and any manual steps needed to verify the change. -->

```powershell
cargo check
cargo test
cargo clippy -- -D warnings
```

## Checklist

- [ ] `cargo check`, `cargo test` and `cargo clippy -- -D warnings` pass locally.
- [ ] New behavior is covered by a unit test next to the code, or the PR explains why it cannot be.
- [ ] User-visible strings are in both `src/i18n/en.rs` and `src/i18n/ru.rs`.
- [ ] The in-app guide was updated in both language branches if builder behavior changed.
- [ ] Documentation was updated in the owning file (README/README.ru, ARCHITECTURE, RULE_AUTHORING,
      FAQ, EXAMPLES, RELEASING); new or renamed documents are registered in `docs/README.md`.
- [ ] `README.md` and `README.ru.md` still match, if either changed.
- [ ] `NOTICES.md` is updated if dependencies changed.
- [ ] Logs contain identifiers and outcomes only, no scripts, payloads, rule JSON or secrets.
- [ ] Line endings are LF.

## Screenshots

<!-- For UI changes: before/after or a short clip. Delete this section otherwise. -->
