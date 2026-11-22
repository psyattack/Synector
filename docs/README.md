# Synector documentation

This directory holds the detailed documentation. Each topic has one owner; update that file and
link to it instead of copying its content elsewhere.

This index is the only place where the document set is enumerated. Other files link here instead of
keeping their own list.

## Users

| Document | Contents |
|---|---|
| [../README.md](../README.md) | Overview, features, installation, quick-start build commands and configuration. [Русская версия](../README.ru.md). |
| [FAQ.md](FAQ.md) | Common questions and troubleshooting: elevation, updates, logs, rules, overlays, macros and configuration recovery. |
| [RULE_AUTHORING.md](RULE_AUTHORING.md) | Command-line rule authoring: commands, JSON format, quotas, webhook contract, validation, share codes. |
| [SCRIPTING.md](SCRIPTING.md) | Lua runtime, the `syn` API, context tokens, output and error handling. |
| [EXAMPLES.md](EXAMPLES.md) | Complete, ready-to-apply rule recipes. |

## Developers

| Document | Contents |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Two tiers, data flow, rule engine, event sources, overlays, input simulation, updates and design conventions. |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Contribution workflow, code style, documentation rules and pull-request process. |
| [../SECURITY.md](../SECURITY.md) | Supported versions and private vulnerability reporting. |
| [../CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md) | Community standards. |

## Releases

| Document | Contents |
|---|---|
| [RELEASING.md](RELEASING.md) | Version source, changelog entry shape, tag flow and what CI publishes. |
| [../CHANGELOG.md](../CHANGELOG.md) | Release history, one section per version. |
| [../NOTICES.md](../NOTICES.md) | Direct dependencies and their license texts. |

## Website

The project landing page is published from the `gh-pages` branch:
<https://psyattack.github.io/Synector/>.

## Documentation rules

- **One owner per topic.** Behavior belongs to `ARCHITECTURE.md`, CLI and JSON format to
  `RULE_AUTHORING.md`, Lua scripting to `SCRIPTING.md`, recipes to `EXAMPLES.md`, user questions to
  `FAQ.md`, release mechanics to `RELEASING.md`.
- **Link, do not copy.** When a detail is already documented in its owner file, reference it. This
  keeps `README.md` and the docs from drifting.
- **Keep both languages in sync.** `README.md` and `README.ru.md` are mirrors. GUI strings live in
  both i18n catalogs, and the in-app guide has Russian and English branches.
- **Update this index** when a document is added, renamed or removed.
