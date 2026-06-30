*Read this in [Русский](CONTRIBUTING.ru.md).*

# Contributing to Cortiq Gateway

Thank you for your interest in the project! This is an open gateway and we welcome contributions.

## Getting started

1. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — design overview and roadmap.
2. Review [docs/PROTOCOLS.md](docs/PROTOCOLS.md) and [docs/ROUTING.md](docs/ROUTING.md).
3. The current phase is **Phase 0** (skeleton). Good entry-point tasks are labeled
   `good first issue`.

## How to add...

- **A new inbound protocol** — implement the `InboundProtocol` trait in `src/protocols/`,
  translate to/from the canonical model (`ChatRequest`/`ChatResponse`), and add the
  corresponding flag in `[protocols]`. Do not touch the providers.
- **A new provider** — implement the `Provider` trait in `src/providers/`. Most local
  servers are OpenAI-compatible, so reusing `openai.rs` is often sufficient.
- **A new routing policy** — extend `src/routing.rs` and the `[routing.policy]` section.

Core principle: N protocols × M providers communicate **only** through the canonical model —
no direct protocol↔provider translations.

## Style and checks

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

- Match the style of the surrounding code.
- Secrets go only through `*_env` variables — never commit them to the repository.
- New functionality must be accompanied by tests and updates to `docs/`.

## Pull requests

- Keep PRs small and focused. One PR = one logical unit of change.
- In the description: what, why, and how it was tested.
- Link the related issue if one exists.

## License

Contributions are accepted under [Apache-2.0](LICENSE).
