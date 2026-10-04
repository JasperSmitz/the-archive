# Developer prompt: Librarian Clerk M1

Implement a small, private, deterministic Discord retrieval interface for The Archive. Read this whole prompt, inspect the current repository and applicable AGENTS.md instructions, and read README.md, deployment.md, technical-debt.md, testing.md, and docs/ROADMAP.md before editing. Preserve unrelated working-tree changes. The architect observed pending changes to .env.example/deployment.md and an untracked tests/health.rs; inspect the actual state yourself.

The owner reports that the app is deployed and functional. Do not deploy, register real commands, send Discord messages, create external resources, or request production credentials yourself. Prepare code, tests, placeholders, and exact operator instructions; the owner performs portal/registration/Railway actions.

## Product and architecture

The Archive is canonical curated data. The Librarian is its interface/organizer and future interpretive layer; an AI model is not The Librarian. This milestone is **Clerk: exact read-only retrieval**, with no model involvement.

Continue Rust/Tokio/Axum/SQLx/PostgreSQL/Askama in one application. Reuse the existing catalog, association, character, image, and LocalStorage application operations. HTTP/Discord protocol concerns stay outside core application/query functions. Add small exact-resolution, bounded pagination, image count/page, and random-image operations in src/app as needed; do not duplicate SQL inside Discord handlers. Keep website semantics working, including same-association-row person/type filtering.

Suggested boundaries: src/discord/{mod,verification,commands,responses,client}.rs and small additions in src/app/{characters,images,catalog}. Use judgment on file size. No generic repository framework or separate Librarian service/crate is needed. Use minimal maintained crypto/HTTP dependencies; no Gateway client or event runtime. No domain migrations are expected for this milestone; explain any concrete need before adding one.

## Command surface

Register four guild-scoped CHAT_INPUT commands with one shared manifest used by the parser/registration tests:

1. `/character character:<string> [franchise:<string>]`
   - character is required: exact trimmed case-insensitive name, or #<positive Archive ID>.
   - Show name, franchise, a bounded excerpt of the curated plain-text description, person/type associations, and a website detail link. An absent description is an ordinary empty state. Omit tags. Indicate truncation and link to the full record.
2. `/images character:<string> [franchise:<string>] [page:<integer>]`
   - Same exact selector. Page defaults to 1. Five entries per page ordered by created_at DESC, id DESC, with IDs, concise filenames/artist metadata, protected image detail links, and a protected character-gallery link.
   - Attach at most one eligible image from that page as a preview. State which image it is. Do not imply other entries were attached. If no preview can be delivered, retain the entries/links and explain why.
3. `/random-image character:<string> [franchise:<string>]`
   - Randomly select one image from all archived image memberships of the exact character. ORDER BY random() LIMIT 1 is acceptable at this scale. Do not fetch all image bytes or filter the random population by Discord-deliverable size.
   - Show metadata/detail link and attach it if deliverable. If not, explain the fallback; do not repeatedly resample until a smaller image appears.
4. `/characters [person:<string>] [association:<string>] [franchise:<string>] [page:<integer>]`
   - person and franchise resolve by exact trimmed case-insensitive name. association resolves by stable lookup key, e.g. kin, associated, favorite, wife, husband, or a custom key; normalize outer whitespace/case.
   - AND filters; person+association must match the SAME association row. Person alone means any association; type alone means any person. No hardcoded owner names.
   - Return 10 alphabetically ordered characters per page with franchise/ID and protected detail links, plus next-page guidance. No filters means all characters, paginated. No tag option.

Name resolution must use PostgreSQL semantics consistent with existing lower(name) uniqueness, not approximate matching or an independent Rust Unicode normalization rule. Treat %/_ as literal name characters. If a name exists in multiple franchises, return up to five candidates plus a more-matches notice and ask for franchise or #ID. Never choose the first. If #ID and franchise are both supplied, validate they agree. Reserve and document #<digits> for IDs; other strings remain names. Validate positive IDs and bounded inputs/pages; use checked offsets (a maximum page of 100,000 is fine). Unknown references must not silently remove a filter or broaden results.

No autocomplete, components, DMs, random unfiltered image command, or message-context archival in M1. Users rerun commands with the supplied page or selector. Bound message/embed fields and the full payload to Discord's limits; escape/neutralize Discord Markdown where necessary, suppress unintended link previews, and set allowed_mentions.parse to [] on every generated message/edit. Never manufacture biography text.

## Endpoint, verification, and authorization

Add POST /discord/interactions, disabled/404 unless explicitly enabled. It is reachable without browser cookies and is exempt ONLY at this route from browser Origin/Referer checks. Do not weaken website authentication, image-content protection, same-origin rules, or /healthz behavior.

Use a small request-body cap (e.g. 1 MiB). Verify X-Signature-Ed25519 over the exact concatenation of X-Signature-Timestamp bytes and raw request-body bytes using DISCORD_PUBLIC_KEY before JSON parsing or domain access. Missing/malformed/invalid signatures return 401; no trimming/reserializing signed bytes. Use a maintained Ed25519 verifier and validate key/signature lengths.

Add a documented timestamp policy: reject timestamps older than five minutes or more than 30 seconds ahead; use wall-clock UTC and bounded arithmetic. Add a bounded, expiring interaction-ID deduplication cache for command dispatch, covering the accepted timestamp interval; duplicates must not run a second task/send multiple deliveries. No durable replay store is necessary for read-only M1. Reject gracefully if capacity is exhausted rather than growing unboundedly.

After signature verification, verify application_id for all supported payloads. PING (type 1) returns PONG (type 1) immediately, without user/guild checks, database access, or outbound calls, including maintenance mode. Discord sends signature-validation probes; do not bypass verification for PING.

For commands independently require DISCORD_GUILD_ID and an invoking member.user.id in DISCORD_ALLOWED_USER_IDS. Reject DMs, other guilds, mismatched apps, unsupported command types, and missing/invalid identity fields. Never derive permission from submitted person names, guild admin permissions, roles, installation alone, or a username. Snowflakes are validated decimal strings/u64, never floating-point.

Valid signed command denials return a generic ephemeral interaction message without Archive queries. Every success, error, busy notice and maintenance notice is ephemeral. Apply application and user/guild authorization before any private lookup. In maintenance, authorized commands receive an immediate ephemeral maintenance notice and perform no domain/file queries. Distinguish signatures (request authenticity) from allowlists (access permission).

These Discord IDs are access principals independent of catalog people and website accounts. Document that disabling a website account does not revoke Discord access; revoke by removing that Discord ID/redeploying, or disable the integration. No account-linking schema yet.

## Timing and task lifecycle

Discord requires the first response within three seconds. For accepted commands, immediately return type 5 DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE with flags 64 (EPHEMERAL), before database/file/network work. Complete by PATCHing the original interaction webhook response, not a normal channel message. Denials/maintenance can use immediate type 4 with flags 64.

Use bounded in-process admission (e.g. at most four active command jobs), finite DB/file/outbound timeouts, and a tracked task lifecycle. Busy requests get an immediate ephemeral notice. Do not wait for a semaphore before acknowledging. Supervise tasks so panic/error releases admission and is logged safely; drain/cancel tasks within a bounded graceful-shutdown window. No queue service, worker process, or durable job table.

Set a short total job budget, such as 60 seconds, well within the 15-minute interaction-token lifetime. On ordinary failure, attempt a short private error edit. Respect Discord 429 retry_after with at most a bounded retry that fits the deadline; do not blindly retry ambiguous attachment delivery. Process restart can interrupt a deferred job: document rerunning the read-only command, rather than promising durable delivery.

Interaction tokens and outbound webhook URLs are secrets. Do not log raw bodies, signature headers, bot/session/interaction tokens, webhook URLs, image bytes, or full Discord response bodies. Sanitize HTTP errors because request URLs contain interaction tokens. Logs may include route pattern, command name, interaction ID, and safe outcome/status. Use fixed Discord API v10 destinations assembled from validated IDs/tokens; no arbitrary user-provided outbound URL.

## Image delivery and privacy

Use LocalStorage.open with the record's generated key. Copy one bounded original image into an ephemeral multipart webhook edit (files[0], payload_json, attachment metadata and attachment:// reference if useful). Use a generated safe attachment filename, detected content type, and bounded description. Never pass storage keys, absolute paths, website session cookies, or Discord tokens into user-visible URLs.

Use the smaller of the signed interaction's attachment_size_limit and an application delivery cap of 8 MiB per file. If the field is absent, use the conservative 8 MiB cap; malformed/nonpositive values must not increase access/limits. Bound actual read length too: do not trust only stored byte_size or read an unbounded replacement file. At most one attachment/job; no resizing/re-encoding or thumbnail subsystem now.

If an image is too large, missing, or unreadable, return the metadata/protected detail link with an understandable notice. /images may try another eligible entry within its five-entry page; /random-image retains its single selected record. No source downloads, external URL fetches, public bucket, new anonymous content route, or expiring Archive share token.

Website links use the configured APP_ORIGIN and existing protected numeric-ID paths. For /images use the existing website gallery URL semantics: do not reuse the Discord five-entry page number as the website's 24-entry page number. No Discord scraping of the login-protected website.

Document that ephemeral delivery limits message visibility but uploads a copy to Discord. Recipients can download/forward it; copied/CDN attachment URLs are not governed by Archive sessions. Do not promise that website logout or later Archive deletion revokes a delivered copy.

## Configuration and registration

Add/document:
- DISCORD_ENABLED: explicit boolean, default false.
- DISCORD_APPLICATION_ID: expected application snowflake.
- DISCORD_PUBLIC_KEY: public Ed25519 verification key from the portal.
- DISCORD_GUILD_ID: the single authorized guild.
- DISCORD_ALLOWED_USER_IDS: comma-separated nonempty allowlist; document two IDs.
- DISCORD_BOT_TOKEN: secret needed only by the explicit registration CLI.

Enabled server startup must reject incomplete/malformed Discord configuration. Disabled operation and database/account/orphan CLI commands must not require Discord values. Production continues using HTTPS APP_ORIGIN. Do not require a signing secret or bot token for normal interaction-token delivery.

Provide `discord commands print` and `discord commands register` (or equally explicit equivalents). Print is offline/nonsecret and does not need PostgreSQL, storage, or a token. Register validates app/guild IDs and reads the bot token from environment, works independently of DB/storage initialization, and upserts ONLY these four guild commands using the manifest. No global registration or startup registration; preserve unrelated commands and report safe registration outcomes. Do not bulk-delete existing commands. Handle failed registration/rate limits clearly without logging secrets.

## Owner setup documentation

Add docs/DISCORD.md and link it from README/deployment/roadmap. Record exact manual steps:
1. Create a dedicated application named The Librarian in the Discord Developer Portal; record Application ID/Public Key.
2. Enable Developer Mode in Discord and copy the private guild ID and both user IDs.
3. Choose Guild Install, disable User Install for M1, and use application-command scope. No Administrator permission, privileged/message-content intents, or Gateway connection. A bot token can authorize registration; adding bot scope/permissions is unnecessary solely to use application commands when supported by the portal installation flow. Explain any current portal-specific difference, verified against official docs.
4. Add enabled runtime variables to the existing Railway service, deploy the prepared code, and set Interactions Endpoint URL to https://<origin>/discord/interactions. Saving must pass signed PING even during maintenance.
5. Install/authorize the app in the one guild with the appropriate guild-management authority. Obtain/reset the bot token privately for registration; explain how to run the registration CLI using a private local environment or administrative terminal without echoing the token. The production server need not retain the registration token.
6. Run registration explicitly, verify the four guild commands, leave maintenance for functional tests, and test both allowed users plus an unlisted user. Test signed PING, private file fallback, unknown/ambiguous characters, same-row filters, and a redeploy. Normal Archive maintenance continues to block retrieval.

Preserve current Railway/Neon/volume/PORT setup. No new service, database, public image domain, or hosting provider is needed. Record precise local test/tunnel instructions if useful, but do not deploy a tunnel yourself or expose the Archive to bypass authentication. Document disabling the integration, changing allowlists, and removing commands manually if needed.

## Tests and checks

Use real isolated PostgreSQL/temp image directories for domain/integration behavior and a local fake outbound HTTP endpoint for Discord edits. Keep any fake transport/test endpoint dependency injected; do not expose an arbitrary production webhook base URL. Tests must not call real Discord or require real tokens.

Cover raw-body signed verification with generated test keys; altered timestamp/body, malformed/missing signatures, stale/future requests; disabled route; signed PING with no cookies/Origin and during maintenance; exact-route browser protection isolation; application/guild/user denials without DB access; unsupported payloads/options; manifest/parser consistency; bounded replay/admission and fast deferral before slow work; outbound error/timeouts/429 handling; safe response lengths/mentions/links; Unicode/exact literal names, unknown/ambiguous/#ID selectors; multiple franchises; structured AND/same-row matching and pagination; image selection/page membership; random selection from the correct set; multipart attachment bytes/type; actual file-size bounds, missing/oversized-file fallbacks; protected website originals still return 401 while logged out. Include shared Link+Venti artwork and duplicate character names across franchises.

Avoid flaky randomness/timing assertions. Query-set correctness can be tested with one candidate or controlled selection; test prompt deferral using an intentionally blocked fake dependency. Verify commands do not mutate canonical tables. Keep existing M1–M3/health/recovery tests passing.

Run cargo fmt --check, cargo clippy --all-targets --all-features -- -D warnings, and cargo test with documented PostgreSQL/client tools. Build the production container if available. Do not claim unavailable checks passed or register a real command to test implementation.

Update docs/ROADMAP.md to reflect implemented versus owner-unverified state, .env.example/README/deployment.md for actual configuration, and technical-debt.md only for concrete findings. Do not mark hosted/operator verification complete without evidence. Preserve pending owner changes.

## Acceptance and exclusions

Done when the owner can configure The Librarian, register the four exact guild commands, and both allowlisted users can retrieve correct characters, relational lists, image pages and a random image without browser credentials on Discord requests or any weakened website protection. Ambiguity, empty sets, maintenance, delivery limits and failures must be understandable. Raw signature checks and independent authorization must be tested.

Exclude LLMs, embeddings/pgvector, semantic/archetype analysis, image understanding, ontology generation, conversational memory, Gateway/event listening, context-menu archival, Archive mutations, public share links, R2, thumbnails, Redis/queues, agent frameworks, tag expansion/removal, and a large browser UI redesign. Retain tags as optional existing metadata; de-emphasizing the website tag UI is a later separate change.

Report implementation changes, architectural choices, commands/configuration, tests actually run, owner setup steps, privacy/delivery limitations, and unresolved issues. You have freedom over low-level module boundaries and protocol libraries within this scope.

## Official protocol references

- [Signature verification and HTTP setup](https://docs.discord.com/developers/interactions/overview)
- [Interaction timing, response types, webhook edits](https://docs.discord.com/developers/interactions/receiving-and-responding)
- [Guild command scope, registration, options](https://docs.discord.com/developers/interactions/application-commands)
- [Multipart attachments and reported upload limits](https://docs.discord.com/developers/reference#uploading-files)

Verify protocol details against official documentation during implementation. Repository behavior remains authoritative for Archive semantics.
