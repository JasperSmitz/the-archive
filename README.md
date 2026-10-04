# The Archive

A private, locally runnable character catalog. M1 supports people, franchises, characters, tags, and extensible association types. All operations use server-rendered HTML and ordinary forms; JavaScript is unnecessary.

## Prerequisites and startup

Install stable Rust (validated with Rust 1.96.0), Docker with the Compose plugin, and optionally `psql` for database inspection. The database uses PostgreSQL 17; the application runs on the host.

```sh
cp .env.example .env
docker compose up -d --wait
cargo run --locked
```

Open <http://127.0.0.1:3000>. Create franchises and people, then characters. Character pages let you add multiple person/type associations and tags. Create extra tags or association types from the navigation. Association keys are immutable; labels can be edited. Character filters combine with AND, and person/type filters match the same association row. Results are alphabetically ordered with stable tie breakers.

`DATABASE_URL` is required. `LISTEN_ADDR` defaults to `127.0.0.1:3000`; `APP_ORIGIN` defaults to the HTTP origin of that listener. Set both if using another address. `RUST_LOG` controls logs. `.env` is loaded without overriding exported environment variables. Startup connects to PostgreSQL and applies all pending ordered migrations before listening; connection or migration failure exits with an error. Migrations seed only the five standard association types. Compilation does not need PostgreSQL, and migration edits trigger recompilation.

Stop the application with Ctrl-C (or SIGTERM). `docker compose stop` stops PostgreSQL without deleting its named volume. Restart with `docker compose up -d --wait` and `cargo run --locked`; catalog data persists. `docker compose down` also preserves the volume. **Do not use `down -v` unless you intend to erase all data.**

## Tests and checks

Integration tests require a running real PostgreSQL server. SQLx creates separate uniquely named test databases, applies migrations to each, and cleans up successful tests. Tests never truncate or clear the catalog database. The test login needs `LOGIN` and `CREATEDB`, and access to the `postgres` maintenance database. The local Compose user is a superuser suitable for these local tests; do not reuse these credentials outside local development. Failed SQLx test databases may be retained for inspection.

```sh
docker compose up -d --wait
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://archive:archive_local@127.0.0.1:5432/postgres cargo test
```

Tests cover migration seeds, character-count validation, case-insensitive uniqueness and franchise scoping, association multiplicity, idempotent joins, foreign keys/deletion, combined filters and same-row matching, and a PostgreSQL-backed HTTP workflow with escaping, validation, conflicts, and missing records. If PostgreSQL is unavailable, integration tests fail rather than silently skip.

## Boundaries and request protection

This is a local-only application with **no authentication**. Anyone who can access the listener can read and change its data. Both the default application listener and Compose's database port bind to loopback. Do not expose either to a public network.

Unsafe requests must have an `Origin` header matching `APP_ORIGIN`. If Origin is absent, a Referer URL with the matching origin is accepted. A missing, malformed, `null`, or mismatched header is rejected with 403; an invalid Origin never falls back to Referer. Ordinary browsers send these headers for forms. Privacy configurations that strip both will prevent form submissions. Scripts must explicitly send a matching header, for example:

```sh
curl -i -H 'Origin: http://127.0.0.1:3000' \
  -d 'name=Zelda' http://127.0.0.1:3000/franchises
```

GET reads and POST mutates. Successful mutations redirect with 303. Validation returns 422, missing records 404, duplicate entities and restricted deletions 409, and unexpected failures 500 with details confined to server logs. Character and tag deletion cascades memberships; people, franchises, and association types in use cannot be deleted. Deletion requires an explicit confirmation page.

## Structure

One Rust package contains library and binary targets. `config` and `main` handle startup; `app` contains ordinary-input application functions and validation; `models` and `error` define data and typed failures. `web` handles forms, routes, and Askama rendering. SQLx runtime queries use explicit columns and bound values; dynamic table names come only from the internal catalog kind enum. PostgreSQL owns uniqueness, foreign keys, lengths, timestamps, and deletion rules. `templates`, `static`, and `migrations` contain HTML, CSS, and schema respectively. There is no parallel JSON API, frontend build, authentication, image storage, or cloud infrastructure.
