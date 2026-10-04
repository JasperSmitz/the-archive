# The Librarian — Clerk retrieval and artwork viewer

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
| `/character character:Link franchise:Zelda` | Name, franchise/ID, curated description excerpt, person/type associations, protected full-record link, and one random associated artwork with artist/detail attribution; no tags |
| `/images character:Link franchise:Zelda` | One newest-first image with ID/filename/artist/dimensions, protected details/gallery, and shared Previous/Next buttons |
| `/random-image character:Venti` | One random membership from the entire exact character image set, with metadata/link and eligible attachment |
| `/characters person:Alice association:favorite franchise:Genshin page:1` | Ten alphabetic characters matching all supplied filters |

`character` is required for the first three commands. Omit optional filters to browse all characters. Only `/characters` accepts `page`: it defaults to 1, with a maximum of 100,000 and checked offsets. Rerun that command with the next page and the same options. `/images` no longer accepts `page`; it uses shared buttons. No autocomplete is added.

Names are trimmed exact case-insensitive PostgreSQL `lower(name)` equality, matching existing uniqueness; `%` and `_` are literal. No fuzzy/substring matching or separate Unicode normalization is applied. Association uses its stable key, with outer whitespace/case normalized (e.g. `kin`, `favorite`, a custom key). Unknown references produce an error, never a broader query. Person+association must match the **same** association row; person alone matches any type, association alone any person. Each character appears once with stable alphabetic/ID ordering.

Reserve **`#<digits>`** for positive Archive IDs, including leading zeros; other strings remain names. `#0` and overflowing IDs are invalid. For duplicate names across franchises, supply `franchise` or `#ID`; up to five candidates and a more-matches notice are returned, never an arbitrary first match. A supplied franchise must agree with an ID selector. The protected website gallery link starts the character-filtered gallery at its first page; Discord navigation does not reuse website pagination.

Descriptions/associations and display metadata are bounded excerpts, not generated biographies. Truncation points to the full protected record. Text is neutralized for Discord Markdown/mentions/automatic links. Every message/edit sets `allowed_mentions.parse=[]`. Accepted retrieval commands are public in the channel, including subsequent lookup and delivery errors. Access denials, maintenance, malformed requests, and busy notices remain ephemeral. Website links still require a website login.

## Security, timing, failures, and maintenance

The endpoint caps raw bodies at 1 MiB, verifies Ed25519 over **unmodified timestamp header bytes followed by raw body bytes** before JSON parsing, and rejects missing/malformed/invalid signatures with 401. Timestamps older than five minutes or more than 30 seconds ahead fail. Keep the host clock synchronized. PING also verifies the application ID, then returns PONG without DB/file/network work or guild/user checks.

Commands and every button click independently require the configured app, guild, and `member.user.id`. DMs, unlisted users, other guilds/apps, missing identity, unsupported command types/options are rejected. Signed access denials are generic ephemeral notices before domain access. This endpoint alone sits outside browser session/Origin middleware; browser login/logout, catalog, upload, and original content protections are unchanged.

An in-memory replay cache holds at most 4,096 interaction IDs for six minutes, covering the accepted signature timestamp interval including its inclusive second boundary. It expires entries during admissions. A duplicate cannot run another task; capacity exhaustion returns a busy notice. At most four command/navigation jobs are admitted without waiting. Slash commands yield a deferred public type-5 acknowledgement before domain work. Viewer buttons yield type 6 (`DEFERRED_UPDATE_MESSAGE`) before work, then PATCH the clicked message via `@original` with that click’s fresh token. Both are edits, not new public messages. These response types and private follow-ups follow the [official interaction protocol](https://docs.discord.com/developers/interactions/receiving-and-responding). Results edit the interaction response rather than sending separate channel messages.

Domain/file preparation has a 35-second deadline; individual file reads have five seconds; outbound connect/request limits are three/ten seconds. A job has 55 seconds plus at most five seconds for an error notice (60 total). Slash-command delivery failures retain the existing public error-edit behavior. Component preparation/staleness errors leave the viewer unchanged and use a bounded ephemeral follow-up; component delivery failures/timeouts attempt only a private uncertainty notice, never an error PATCH overwriting the viewer. An explicit 429 can retry once when `retry_after` is 0–5 seconds; other statuses, timeouts, and ambiguous file deliveries are not blindly retried. Tasks are supervised; graceful shutdown closes admission, drains ten seconds, then cancels children (an already-running error edit may take at most five more seconds). Restarts can interrupt deferred delivery: check the shared message, then click again or reopen `/images` as needed; there is no durable queue/replay store. Logs contain safe command/interaction ID/outcome only, not raw bodies, signatures, tokens, webhook URLs, or image bytes/response bodies.

During Archive maintenance, authorized commands and valid viewer clicks receive an immediate ephemeral pause notice without domain/file queries. Signed PING remains usable and health remains live. For coordinated backups keep maintenance enabled, wait for in-flight retrieval jobs to finish (up to 60 seconds) or stop the application, and continue to stop all canonical writers/admin commands as in deployment.md. Retrieval is read-only but should not race a destructive file restore/cleanup.

Discord IDs are independent access principals. **Disabling/resetting a website account does not revoke Discord access.** Remove its Discord user ID and redeploy, or set `DISCORD_ENABLED=false` and redeploy. Remove the portal endpoint to stop callbacks. Existing registered commands can be removed manually through Discord's command API/management tooling by the owner; registration deliberately never deletes unrelated commands. An unavailable/disabled endpoint makes invocations fail rather than retrieve private data.

## Shared artwork navigation

`/images` opens the newest associated image ordered by archival `created_at DESC, id DESC`. Previous moves toward newer artwork; Next moves toward older artwork, without wrapping. The current image shows its live position/count and newest/oldest-end information. Zero images gives an ordinary empty reply without controls; one image disables both directions. Missing/unreadable/oversized images keep their metadata/detail link and usable navigation, never silently skipping an entry. Each edit replaces the attachment list and embed: fallback clears the previous image so it cannot be attributed to the wrong entry.

Either allowlisted owner can navigate the same public message, independent of its original invoker. Channel viewers see updates; unlisted users get a private denial and cannot edit it. There is no per-user/private cursor. Buttons use the signed clicked message's current cursor: repeated clicks carrying the same cursor request the same neighbour, rather than multiplying moves. Counts/positions may change with Archive edits; the viewer is not a snapshot of the archive across clicks. New/deleted neighbours are resolved in newest-first order. If the character or cursor image was deleted/unlinked, an ephemeral stale-view notice leaves the old viewer intact; reopen `/images` to recover. If a formerly disabled edge needs refreshing after new artwork, reopen `/images`.

Legacy action rows/buttons accompany the normal embed; no Components V2 flag is used ([official component reference](https://docs.discord.com/developers/components/reference)). Versioned custom IDs (`av1:<character-id>:<image-id>:p|n`, under 100 characters) carry only navigation data. The parser checks positive bigint IDs, version/direction/button type, app-owned message application/author identity, channel identity, and both matching viewer controls. Database logic verifies character/cursor membership before selecting an image. IDs do not authorize access.

Only in-flight message IDs are guarded, at most four globally. A second in-flight click on the same message gets a private busy notice; there is no growing map of past viewers. Guards release on acknowledgement drop, success, failure, timeout, panic/cancellation, and shutdown. Each click uses its own signed interaction token and type-6 acknowledgement; the initial slash token is neither stored nor reused. Old controls continue after an application restart if their character/cursor still exists. If a delivery response is lost, the edit may already have succeeded: check the current shared message or reopen `/images`, rather than assuming delivery failed. No automatic retry follows an ambiguous attachment edit.

## Operator update for this milestone

No Railway variables, database migrations, install scopes, permissions, Gateway, or provider resources are added. The owner reports the existing integration is live; this implementation does not independently verify the new hosted viewer.

1. Deploy the prepared code yourself using the existing single-instance service/volume configuration.
2. In a private terminal, use the hidden token prompt from the registration instructions above. Run `cargo run --locked -- discord commands print` to verify that `/images` now has only `character` and `franchise` options. **Rerun `cargo run --locked -- discord commands register`** (or `/app/scripts/container-entrypoint.sh discord commands register` in the existing administrative shell). The individual guild upsert removes the obsolete `/images page` option and updates its description; other three commands and unrelated guild commands remain intact. No registration occurs at startup.
3. Remove/unset the registration token after use, leave maintenance for functional testing, and run the owner checklist below. Do not reuse an old five-entry response as a viewer: invoke `/images` again. Current viewers retain versioned buttons across redeploys without a stored session.

## Image-copy privacy and limits

One original is copied through `LocalStorage.open` into a public multipart edit with a generated `archive-image-<id>.<detected-extension>` filename. Delivery uses the smaller of **8 MiB** and the signed `attachment_size_limit`. Absent limit uses 8 MiB; malformed/nonpositive limits disable attachments for that request. Both metadata size and actual reads are bounded, including a replaced file. Missing, unreadable, unsupported stored format, oversized, or timed-out files retain metadata/protected links and a fallback notice. `/character` and `/random-image` sample once over all associated images, retaining that selected metadata on fallback instead of resampling. `/images` retains exactly its selected entry, with working navigation even if the file cannot be delivered. Replaced files are checked against actual read length too. No source URLs are fetched and no images are resized.

**Retrieval replies and image copies are visible to everyone who can view the channel.** Recipients can download/forward it. Copied/CDN attachment URLs are not controlled by Archive sessions; website logout, allowlist removal, or later Archive deletion cannot revoke a delivered copy. The Archive adds no anonymous original route or share token. See [multipart attachment protocol](https://docs.discord.com/developers/reference#uploading-files) and [response timing/edit protocol](https://docs.discord.com/developers/interactions/receiving-and-responding).

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
- Both allowlisted owners see each other’s retrieval replies in the channel and retrieve characters with artwork, relational lists, shared viewers, and random artwork; an unlisted user cannot retrieve data.
- Create Link in two franchises: bare `Link` asks for disambiguation; exact franchise and `#ID` work; wrong franchise/unknown filters do not broaden results.
- Link has Alice/Kin and Bob/Favorite; Venti has Alice/Favorite. Alice+Favorite returns only Venti. Multiple types do not duplicate characters.
- Shared Link+Venti artwork appears from either character. `/character` retains descriptions/associations with a random member and attribution, including empty/missing/too-large fallback without resampling. `/images` begins at newest; both owners use Previous/Next on the same public message, without new public messages. Test zero/one/many images, disabled ends, tied archival timestamps, missing/too-large entries clearing the old attachment, and `/characters page` unchanged.
- Logged-out website originals remain 401, mutations remain protected, website detail links require login.
- Click as an unlisted user: only a private denial, no shared edit. Click concurrently as both owners: the second gets a private busy notice. Delete/unlink a cursor: only a private stale notice, then reopen `/images`. Test new/deleted neighbours; no unrelated artwork is exposed.
- Maintenance blocks commands and button retrieval; signed PING remains live. Redeploy, then navigate an existing viewer as both owners without rerunning its original slash command. Check the channel state before retrying an interrupted edit.
- No actual Archive table is mutated by a command. Backups/restore/orphan constraints from M2/M3 remain unchanged.

Actual hosted command/file acceptance remains unverified until the owner records these results. Container availability and local check results are recorded in technical-debt.md; no local measurement certifies the deployment memory tier.
