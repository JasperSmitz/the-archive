# The Librarian — Clerk M1

The Archive is canonical curated data. The Librarian is its retrieval/organization interface; an AI model is not The Librarian. Clerk M1 uses exact PostgreSQL queries, with no model, Gateway connection, automatic collection, or Archive mutations. It runs inside the existing Rust application. No new service, database, hosting provider, or public image domain is required.

Implementation and local tests are recorded below. **Portal installation, real registration, Railway delivery, and both-owner hosted acceptance are operator work and have not been performed by the implementer.** The owner reports that the existing website is hosted and functional; this does not verify Discord.

## Runtime configuration

Default `DISCORD_ENABLED=false` makes POST `/discord/interactions` return 404. Enabled startup validates all five runtime settings before connecting/serving:

| Setting | Value supplied by owner |
| --- | --- |
| `DISCORD_ENABLED` | Exactly `true` to enable; `false` to disable |
| `DISCORD_APPLICATION_ID` | Application ID, positive decimal snowflake |
| `DISCORD_PUBLIC_KEY` | Portal public Ed25519 key, exactly 64 hex characters |
| `DISCORD_GUILD_ID` | The one private server's positive decimal ID |
| `DISCORD_ALLOWED_USER_IDS` | `<first-user-id>,<second-user-id>`; no empty entries |
| `DISCORD_BOT_TOKEN` | Secret for explicit registration only; not required/used by the server |

Allowlist whitespace around IDs is accepted; duplicates collapse. The list is bounded to 100 IDs. IDs are parsed as u64 decimal strings, never floating point. Enabled configuration does not infer IDs from names or catalog people. Disabled operation and database/account/orphan CLI commands do not require Discord configuration. Production still requires HTTPS `APP_ORIGIN`; links use that configured origin, never request/forwarding headers.

Keep the current Railway/Neon/volume configuration: `PORT=3000`, `LISTEN_ADDR=0.0.0.0:3000`, `IMAGE_STORAGE_DIR=/data/images`, the same direct TLS database URL, `/healthz`, and one instance. Enabled Discord startup bounds `APP_ORIGIN` to 300 bytes so full protected links fit the response fields. No new migration is needed: existing character/image relationships express these read-only queries.

## Owner portal, installation, and registration steps

1. In the [Discord Developer Portal](https://discord.com/developers/applications), create a dedicated application named **The Librarian**. Record its Application ID and Public Key from General Information. The key is public; the bot token is secret.
2. Enable Discord **User Settings → Advanced → Developer Mode**. Right-click your private server to copy its server ID. Copy both owners' user IDs, not usernames or catalog person IDs.
3. In the portal **Installation** page, enable **Guild Install** and disable **User Install**. Select the `applications.commands` scope. Do not request Administrator, privileged/message-content intents, or a Gateway connection. Application commands can be authorized independently of the `bot` scope; a bot user need not join the guild. Discord's general first-bot tutorial illustrates both scopes because it builds a bot. If the current portal's default-link UI adds `bot`, choose a **Custom URL** install link using this documented commands-only authorization URL, substituting the three IDs:

   ```text
   https://discord.com/oauth2/authorize?client_id=<application-id>&scope=applications.commands&integration_type=0&guild_id=<guild-id>&disable_guild_select=true
   ```

   The server authorization must be performed by someone with Manage Server (`MANAGE_GUILD`). Guild command visibility is not access permission: the server enforces its independent user allowlist. See [command authorization](https://docs.discord.com/developers/interactions/application-commands), [OAuth2 scopes](https://docs.discord.com/developers/topics/oauth2), and [installation UI](https://docs.discord.com/developers/quick-start/getting-started). Portal labels may change; do not add unnecessary permissions to work around a UI difference.
4. Add the five runtime values above to the **existing** Railway service, then deploy this prepared code yourself. Set the portal **Interactions Endpoint URL** to `https://<your-existing-origin>/discord/interactions`. Saving must succeed with a signed PING; PING also works while `MAINTENANCE_MODE=true`. No browser cookie/Origin header is needed there. Invalid signatures must fail validation with 401. `/healthz` remains unchanged.
5. Install/authorize the app in that one guild using the link above. Obtain/reset the bot token privately from the portal's **Bot** page for registration. Creating a bot token does not require a Gateway connection or granting guild bot permissions. Run registration explicitly from a private local environment or Railway administrative shell. The server never registers commands at startup.

   ```sh
   # Offline manifest inspection: no DB/storage/Discord credentials needed.
   cargo run --locked -- discord commands print
   # In a PRIVATE bash terminal, read the token without echo or command-line arguments:
   export DISCORD_APPLICATION_ID='<application-id>'
   export DISCORD_GUILD_ID='<guild-id>'
   read -r -s -p 'Registration bot token: ' DISCORD_BOT_TOKEN
   printf '\n'
   export DISCORD_BOT_TOKEN
   cargo run --locked -- discord commands register
   unset DISCORD_BOT_TOKEN
   ```

   Railway alternative: enter `railway ssh` yourself, then run the same private `bash` prompt/export and `/app/scripts/container-entrypoint.sh discord commands register`. Use `/app/scripts/container-entrypoint.sh discord commands print` to inspect remotely. Neither command initializes PostgreSQL or image storage. Avoid shell tracing, shared terminals, token-containing arguments/history, and verbose third-party HTTP logs. Remove any registration token from Railway Variables afterward; normal delivery uses each signed interaction's short-lived token. If registration fails partway, some commands may already be updated; inspect and rerun. Individual POST upserts update only our four CHAT_INPUT names in this guild, preserving unrelated commands. No global or bulk overwrite/delete requests are used.
6. Verify the four guild commands appear, set maintenance false for functional testing, and complete the acceptance checklist below as both allowlisted users and an unlisted user. Saving the portal endpoint verifies real signed PING; local generated-key tests do not replace that check. Redeploy once and rerun commands. Website sessions and image access must remain protected.

## Commands and exact semantics

| Command | Result |
| --- | --- |
| `/character character:Link franchise:Zelda` | Name, franchise/ID, curated description excerpt, person/type associations and protected full-record link; no tags |
| `/images character:Link franchise:Zelda page:1` | Five newest images, IDs/filename/artist/dimensions, protected details/gallery, at most one eligible preview from this page |
| `/random-image character:Venti` | One random membership from the entire exact character image set, with metadata/link and eligible attachment |
| `/characters person:Alice association:favorite franchise:Genshin page:1` | Ten alphabetic characters matching all supplied filters |

`character` is required for the first three commands. Omit optional filters to browse all characters. Page defaults to 1, with a maximum of 100,000 and checked offsets. Rerun with the next page and the same options; there are no buttons/autocomplete in M1.

Names are trimmed exact case-insensitive PostgreSQL `lower(name)` equality, matching existing uniqueness; `%` and `_` are literal. No fuzzy/substring matching or separate Unicode normalization is applied. Association uses its stable key, with outer whitespace/case normalized (e.g. `kin`, `favorite`, a custom key). Unknown references produce an error, never a broader query. Person+association must match the **same** association row; person alone matches any type, association alone any person. Each character appears once with stable alphabetic/ID ordering.

Reserve **`#<digits>`** for positive Archive IDs, including leading zeros; other strings remain names. `#0` and overflowing IDs are invalid. For duplicate names across franchises, supply `franchise` or `#ID`; up to five candidates and a more-matches notice are returned, never an arbitrary first match. A supplied franchise must agree with an ID selector. The Discord five-image page number is not passed to the website's 24-image gallery; that link starts the character-filtered gallery at its first page.

Descriptions/associations and display metadata are bounded excerpts, not generated biographies. Truncation points to the full protected record. Text is neutralized for Discord Markdown/mentions/automatic links. Every message/edit sets `allowed_mentions.parse=[]`. All responses are ephemeral; edits inherit the original ephemeral deferral. Website links still require a website login.

## Security, timing, failures, and maintenance

The endpoint caps raw bodies at 1 MiB, verifies Ed25519 over **unmodified timestamp header bytes followed by raw body bytes** before JSON parsing, and rejects missing/malformed/invalid signatures with 401. Timestamps older than five minutes or more than 30 seconds ahead fail. Keep the host clock synchronized. PING also verifies the application ID, then returns PONG without DB/file/network work or guild/user checks.

Commands independently require the configured app, guild, and `member.user.id`. DMs, unlisted users, other guilds/apps, missing identity, unsupported command types/options are rejected. Signed access denials are generic ephemeral notices before domain access. This endpoint alone sits outside browser session/Origin middleware; browser login/logout, catalog, upload, and original content protections are unchanged.

An in-memory replay cache holds at most 4,096 interaction IDs for six minutes, covering the accepted signature timestamp interval including its inclusive second boundary. It expires entries during admissions. A duplicate cannot run another task; capacity exhaustion returns a busy notice. At most four jobs are admitted without waiting. A deferred type-5/flags-64 acknowledgement body is yielded before starting domain work; the private result edits the original interaction via PATCH. No normal channel messages are sent.

Domain/file preparation has a 35-second deadline; individual file reads have five seconds; outbound connect/request limits are three/ten seconds. A job has 55 seconds plus at most five seconds for a private error edit (60 total). An explicit 429 can retry once when `retry_after` is 0–5 seconds; other statuses, timeouts, and ambiguous file deliveries are not blindly retried. Tasks are supervised; graceful shutdown closes admission, drains ten seconds, then cancels children (an already-running error edit may take at most five more seconds). Restarts can interrupt deferred delivery: rerun the read-only command; there is no durable queue/replay store. Logs contain safe command/interaction ID/outcome only, not raw bodies, signatures, tokens, webhook URLs, or image bytes/response bodies.

During Archive maintenance, authorized commands receive an immediate ephemeral pause notice without domain/file queries. Signed PING remains usable and health remains live. For coordinated backups keep maintenance enabled, wait for in-flight retrieval jobs to finish (up to 60 seconds) or stop the application, and continue to stop all canonical writers/admin commands as in deployment.md. Retrieval is read-only but should not race a destructive file restore/cleanup.

Discord IDs are independent access principals. **Disabling/resetting a website account does not revoke Discord access.** Remove its Discord user ID and redeploy, or set `DISCORD_ENABLED=false` and redeploy. Remove the portal endpoint to stop callbacks. Existing registered commands can be removed manually through Discord's command API/management tooling by the owner; registration deliberately never deletes unrelated commands. An unavailable/disabled endpoint makes invocations fail rather than retrieve private data.

## Image-copy privacy and limits

One original is copied through `LocalStorage.open` into an ephemeral multipart edit with a generated `archive-image-<id>.<detected-extension>` filename. Delivery uses the smaller of **8 MiB** and the signed `attachment_size_limit`. Absent limit uses 8 MiB; malformed/nonpositive limits disable attachments for that request. Both metadata size and actual reads are bounded, including a replaced file. Missing, unreadable, unsupported stored format, oversized, or timed-out files retain metadata/protected links and a fallback notice. `/images` may try another entry within its five-item page; `/random-image` keeps its one selection and never resamples for size. No source URLs are fetched and no images are resized.

**Ephemeral limits message visibility; it still uploads a copy to Discord.** Recipients can download/forward it. Copied/CDN attachment URLs are not controlled by Archive sessions; website logout, allowlist removal, or later Archive deletion cannot revoke a delivered copy. The Archive adds no anonymous original route or share token. See [multipart attachment protocol](https://docs.discord.com/developers/reference#uploading-files) and [response timing/edit protocol](https://docs.discord.com/developers/interactions/receiving-and-responding).

## Verification and operator acceptance

Local tests use generated signing keys, isolated SQLx databases, temporary image directories, and a loopback fake API injected into the client. Production has a fixed Discord API v10 destination, no configurable outbound base URL. Tests never contact real Discord or use real tokens.

```sh
docker compose up -d --wait
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
DATABASE_URL=postgres://archive:archive_local@127.0.0.1:5432/postgres cargo test
# Focused protocol/domain/fake-HTTP tests:
DATABASE_URL=postgres://archive:archive_local@127.0.0.1:5432/postgres cargo test --test discord
```

PostgreSQL test credentials require LOGIN/CREATEDB; full recovery tests also need PostgreSQL 17 client tools (README/ deployment.md). No tunnel is needed for local automated verification. If the owner chooses an HTTPS tunnel for portal testing, target the ordinary authenticated application and a disposable private test guild/configuration; do not add an auth bypass or expose original files separately. The implementer has not created a tunnel.

Owner checklist:

- Portal signed PING saves in normal and maintenance modes; bad signatures are rejected.
- Both allowlisted owners retrieve characters, relational lists, image pages, and random artwork; an unlisted user cannot retrieve data.
- Create Link in two franchises: bare `Link` asks for disambiguation; exact franchise and `#ID` work; wrong franchise/unknown filters do not broaden results.
- Link has Alice/Kin and Bob/Favorite; Venti has Alice/Favorite. Alice+Favorite returns only Venti. Multiple types do not duplicate characters.
- Shared Link+Venti artwork appears from either character; page previews belong to that page. Test no images, no description, next page, missing/too-large preview fallback, and random fallback without resampling.
- Logged-out website originals remain 401, mutations remain protected, website detail links require login.
- Maintenance blocks command retrieval; redeploy preserves Archive data, and new commands work afterward. Interrupted jobs can be rerun.
- No actual Archive table is mutated by a command. Backups/restore/orphan constraints from M2/M3 remain unchanged.

Actual hosted command/file acceptance remains unverified until the owner records these results. Container availability and local check results are recorded in technical-debt.md; no local measurement certifies the deployment memory tier.
