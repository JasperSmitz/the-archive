# Librarian milestone: character artwork and interactive image viewer

Inspect the repository and applicable instructions before editing. Read the current Discord endpoint/client/response code, application retrieval functions, image storage validation, tests, and docs/DISCORD.md. Implement this focused milestone in the existing Rust monolith, following its conventions. No AI, Gateway, new service, queue, or image schema changes.

## Product behavior

1. `/character character:... [franchise:...]` retains curated description, franchise, associations, and protected detail link. Add one randomly selected associated image to the embed using the existing safe attachment-copy mechanism. Include compact image ID/artist attribution and protected image detail link. Sample once over all associated images; if it cannot be attached, keep the chosen image metadata and explain the fallback rather than silently resampling. No images must still produce a useful character reply. Unknown/ambiguous selectors retain existing exact behavior.
2. `/images character:... [franchise:...]` shows one image, beginning with the newest (`created_at DESC, id DESC`, matching the Archive), and Previous/Next buttons beneath it. Each click edits that same public message; it must not create another image message. Show character identity, image ID, filename, artist, dimensions, protected detail/gallery links, and useful position/end information. Remove the old five-image page UX and the `page` option from this command only; `/characters` pagination stays unchanged. Update command manifest/parser/documentation and tell the owner registration must be rerun.
3. Both allowlisted owners may navigate the shared message, regardless of who invoked it. The updated image is visible to channel viewers. Unlisted users receive only an ephemeral denial and cannot alter the message. Keep public accepted slash replies and private access/maintenance/busy notices.
4. Disable Previous at the newest end and Next at the oldest end. Do not wrap. No images: useful empty reply without active navigation. One image: both directions disabled. Missing/unreadable/oversized selected files: keep metadata and a fallback notice, with navigation still usable. Never skip the selected archive entry solely because its file cannot be displayed.

## Navigation design

- Use ordinary embed + legacy action row/buttons; no Components V2 migration is needed. Consult current official Discord component/interaction docs before implementation.
- Handle signed MESSAGE_COMPONENT (type 3) button interactions on the existing endpoint. Authorize application, configured guild, and invoking member user on every click before Archive access, with the same maintenance, replay, admission, deadlines, logging, and shutdown policies as commands.
- Acknowledge authorized navigation promptly with DEFERRED_UPDATE_MESSAGE (type 6), then edit the original clicked message through that click's fresh interaction token. Do not reuse/store the initial slash interaction token: it expires. Immediate denials are ephemeral type 4 and do not edit the shared viewer.
- Use compact versioned custom IDs (within Discord's 100-character limit) containing character ID, current image cursor and direction. Prefer stable keyset navigation over mutable offsets. Validate syntax, positive IDs, supported version/direction, button component type, and the signed message's application/author identity and matching viewer controls. A custom ID is navigation data, never authorization. Resolve character/image membership in application logic, not only the component parser.
- Keep navigation usable after restart without persistent viewer/session tables or extra secrets. Derive state from the signed component/message and Archive. Use a bounded per-message in-flight guard so concurrent clicks cannot deliver edits out of order; immediately return a private busy notice for a second in-flight click. Release guards on success, errors, timeout, and shutdown. Avoid an unbounded map of every historical message.
- Treat repeated clicks from the same cursor as the same intended move; never multiply one click into several moves. Document that navigation is shared and uses the state carried by the clicked controls rather than a private cursor for each person.
- Additions/deletions between clicks must be safe and predictable: preserve newest-first order, verify memberships, and return a recoverable stale-view notice when the cursor/character is removed or unlinked. Do not expose unrelated images or panic. Exact count/position may change with Archive edits; avoid promising snapshot consistency. Explain how to reopen `/images` when necessary.
- Keep SQL and membership/navigation logic in application retrieval functions. No duplicate Discord-specific database layer or generic repository abstraction.

## Attachments and security

- Reuse LocalStorage.open and existing file/metadata/read-timeout checks and min(8 MiB, signed interaction attachment limit). Never fetch source URLs or expose anonymous Archive image routes.
- Replace the previous attachment and embed image reference on each edit. Explicitly remove old attachments; do not accumulate them. Navigating to a fallback entry must clear the previous image so the UI cannot misattribute it. Use generated filenames and correct MIME types.
- Clear/update components and embed fields consistently with the selected image. Preserve bounded embeds, text neutralization, allowed_mentions.parse=[], safe errors, and sanitized logs.
- Component errors after type-6 acknowledgement should not destroy the current viewer. Preserve the prior image/controls when preparation fails; if needed, use a bounded ephemeral follow-up for the user. Describe any unavoidable uncertain-delivery outcome without blindly retrying attachments. Do not introduce a public error edit that overwrites the entire viewer.
- Existing website authentication, Origin checks, maintenance handling, and protected originals must remain intact. Discord receives image copies visible to channel viewers; current allowlist gates invocation/navigation, not reading a public message.

## Tests and acceptance

- Test exact character resolution with random artwork, empty collection, attachment fallback, and preserved description/associations. Verify selected artwork belongs to the character and that random sampling does not retry around oversized files.
- Test initial newest image, forward/backward navigation, deterministic ordering when timestamps tie, zero/one/many image cases, endpoints, and shared artwork memberships. Cover additions, deletion/unlinking of cursor and character removal.
- Extend signed interaction tests for type-3 parsing, valid type-6 acknowledgement, wrong guild/app/user/message, malformed/unsupported custom IDs, forged membership, maintenance, replay, and bounded admission. Preserve private denials and public command behavior.
- Fake Discord HTTP integration tests must prove edits target the clicked message via its fresh interaction token, attachments are replaced/cleared, controls update, and neither accepted navigation nor attachment fallback creates a new public message.
- Exercise simultaneous clicks, timeout/error paths and guard cleanup; restart-style testing should show component navigation needs no initial token or in-memory viewer session. Keep test network calls local; no real tokens or production database.
- Update docs/DISCORD.md, README where useful, and roadmap: command syntax, newest-first ordering, shared navigation, fallback/stale behavior, image-copy visibility, and exact operator steps. Rerun explicit guild command registration after deploying to remove `/images page` and update its description. No new Railway variables or installation scopes are expected; document any justified deviation before adding them.
- Run formatting, Clippy, relevant tests and the existing regression suite. Report checks, limitations, architectural questions, and owner hosted acceptance steps. Do not deploy, register actual commands, or push without explicit owner authorization.

## Out of scope

Bulk uploads are a separate Archive milestone. No Discord archival/context menus, autoplay, random-image buttons, private per-user galleries, image transforms/thumbnails, bot permissions expansion, semantic search, AI, or changes to `/random-image` and `/characters` beyond shared helpers genuinely required here.

Official protocol references:
- https://docs.discord.com/developers/components/reference
- https://docs.discord.com/developers/interactions/receiving-and-responding
