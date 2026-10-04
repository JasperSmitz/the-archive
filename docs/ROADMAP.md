# The Archive and The Librarian

## Product boundary

**The Archive remembers. The Librarian understands.**

The Archive is the canonical source of truth for information explicitly curated by its owners: people, character identity and franchise, descriptions/biographies, person–character associations, artwork, character–image memberships, and artist/source metadata. Its manual workflow remains: add a character, write a useful description, associate them with either/both people, and archive images. It must remain useful without AI.

The Librarian is the retrieval, organization, and eventually interpretation layer over that material. Discord is its first additional interface, not a separate database/product. A future model is one tool available to The Librarian, not its identity. The first implementation uses ordinary application functions and SQL only.

Future derived knowledge—embeddings, inferred roles/motifs, image descriptions, similarities, clusters, analyses—must remain separate from canonical records. It must record the source record/version or content hash, derivation method/framework/model version, and supporting evidence where relevant, so it can be invalidated, revised, or regenerated. Never silently replace a curated description or association. These are design requirements for future milestones, not tables to create now.

## Current Archive: implemented

Repository foundation: M1 catalog, M2 image archive, M3 private access/deployment readiness (commit `030eb22`). The owner reports successful hosted operation. This document does not independently certify hosted backup/restore or memory measurements.

- Rust/Axum/Tokio/Askama monolith, ordinary CSS/forms, SQLx runtime queries, PostgreSQL migrations.
- People, one franchise per character, optional plain-text character description up to 10,000 characters, extensible association types; multiple associations per person/character.
- Case-insensitive name uniqueness; character names are unique within a franchise, not globally.
- Images on local disk with metadata, manually selected uploader person, multiple character memberships, exact SHA-256 duplicates, paginated galleries, and orphan maintenance.
- Provisioned login accounts distinct from people; PostgreSQL sessions; protected pages and original files.
- Single-instance Railway deployment direction, Neon database, persistent filesystem image volume, maintenance mode, backup/restore scripts and operator documentation.
- Optional Librarian Clerk M1 adds exact read-only Discord HTTP retrieval in the existing process. No semantic retrieval or AI is implemented. Real Discord setup/registration/hosted acceptance remain operator-unverified.

Keep filling descriptions, associations, images, and source metadata. Broader variants/crossovers, description provenance, better navigation, thumbnails, and remote-use performance may be added when concrete use warrants them. Existing review follow-ups remain in [technical-debt.md](../technical-debt.md); deployment operation remains in [deployment.md](../deployment.md).

### Archive improvement: bulk image upload — implemented locally

The website now provides a multiple-file picker with shared uploader/character selection, sequential bounded uploads, and individual pending/uploading/uploaded/duplicate/failed/unconfirmed results. Each new image receives all selected characters; mixed character collections require separate batches. It retains independent per-image commits and existing validation/storage logic. Duplicates link to existing records without changing their metadata or memberships. Small ordinary browser JavaScript is justified for progress and retry; no batch service, schema, or infrastructure is needed.

Stop/resume and explicit failed/unconfirmed retry retain files only while the page stays open; duplicates never update memberships. No new configuration or provider resources are needed. Hosted/operator bulk-upload acceptance remains unverified; follow the [manual checklist](../testing.md#bulk-upload-regression-checklist).

This addresses repeated manual uploads after saving artwork locally. Downloading from Discord remains separate; explicit Discord archival is still a later Clerk feature. Implementation brief: [BULK_UPLOAD_PROMPT.md](BULK_UPLOAD_PROMPT.md).

### Tags: retain, de-emphasize

Tags currently have CRUD, character memberships, a main-navigation entry, character-page assignment forms, and a character-list filter. Keep existing records/schema/API behavior intact. They are optional manually curated legacy metadata, not the primary classification workflow or the basis for interpreting either person.

Do not expand categories/ontology, require tags, or expose tag filters in the initial Discord surface. In a separate small Archive UX change, move tag management/filtering/assignment into secondary or collapsed controls and foreground descriptions, associations, and images. Dropping the tables would discard user data for little benefit. Revisit deletion only if the owners later request it.

## Librarian as Clerk — exact Discord retrieval M1: implemented, owner verification pending

Prerequisites: existing Archive data and a reachable HTTPS application; owner-created Discord application, one guild, and two allowlisted Discord user IDs.

Use Discord HTTP interactions in the existing Axum process. No Gateway connection, AI, new infrastructure, or canonical-data migration is expected.

| Command | Behavior |
| --- | --- |
| `/character character:Link [franchise:Zelda]` | Identity, franchise, bounded description, associations, protected website detail link |
| `/images character:Link [franchise:Zelda] [page:1]` | Five newest-first image entries, protected detail/gallery links, and at most one attachment preview |
| `/random-image character:Link [franchise:Zelda]` | Random archived image associated with the exact character, one attachment when within delivery limits, metadata/detail link |
| `/characters [person:Jasper] [association:kin] [franchise:Zelda] [page:1]` | Paginated character list with relational AND filtering and same-row person/type matching |

Character selectors accept an exact case-insensitive trimmed name or `#<Archive ID>`. An optional exact franchise disambiguates names. Multiple matches return a bounded list of names/franchises/IDs; never guess. Person/franchise options use exact names; association uses its stable key. Random selection is explicit sampling over a relationally determined image set, not semantic matching. No-result and invalid-selection states must be useful.

Security: verify every raw signed interaction, independently authorize application/guild/user IDs, use public retrieval responses and ephemeral access/maintenance/busy notices, and keep browser session/Origin checks protecting the website. Reply immediately/defer before querying or reading files. Signed PING works even in maintenance; commands return an ephemeral maintenance notice without querying the Archive.

Images are copied from safe local storage into channel-visible Discord attachments; no new anonymous/signed Archive content URL. Protected website links still require website login. Attachment size is bounded by both a conservative application limit and the interaction's reported limit. Oversized/missing files get metadata and protected links, not resize work or exposed storage paths. Discord stores delivered copies, and recipients can save/share them: website logout cannot revoke a Discord copy.

Registration is an explicit operator CLI action targeting this application's one guild, never a startup action. A bot token is needed only for registration; interaction-token webhook delivery does not require it in normal server operation. Prefer guild install with application-command scope and no extra permissions/intents.

Owner acceptance still required: both allowlisted owners accurately retrieve records/images in the real guild, ambiguity and access denials work, website privacy remains intact, and redeploy/delivery behavior is verified. Local generated-key/PostgreSQL/fake-API tests exercise the protocol/query boundaries; they do not certify real portal installation or hosted delivery. Follow [DISCORD.md](DISCORD.md) for configuration, exact registration commands, copying/privacy limits, and the operator checklist.

Implementation brief: [LIBRARIAN_M1_PROMPT.md](LIBRARIAN_M1_PROMPT.md).

### Clerk follow-ups, after retrieval works

- Explicit message-context **Apps → Archive Image** action, reusing existing validation/duplicate/persistence logic. Design attachment fetching limits, source provenance, attribution and confirmation first. No passive attachment harvesting.
- Autocomplete and navigation components if exact commands become cumbersome.
- Better browsing and thumbnails as separate, evidence-driven Archive improvements.

## Later: Librarian as Searcher — exploratory semantic retrieval

Dependency: a meaningful curated collection with useful descriptions and a specific retrieval question that exact lookup cannot answer.

Explore `/find-character quiet philosophical nature-oriented wanderer` over derived description embeddings. PostgreSQL/pgvector may fit if that feature earns its cost; no commitment yet. Source changes must invalidate derived representations. Search results distinguish retrieved canonical evidence from ranking/similarity, and never write inferred tags into canonical data. Evaluate relevance on owner-selected examples before expanding to images.

## Later: Librarian as Scholar — exploratory interpretation

Dependency: source-rich Archive material, a research/design milestone, and a way to evaluate grounded answers. Semantic search is potentially helpful, not an automatic prerequisite for every analysis.

Investigate narrative roles/functions, folklore motifs, and adapted story patterns using Propp, Aarne–Thompson–Uther, Stith Thompson's Motif-Index, and related traditions. These are research directions, not a ready-made character taxonomy. ATU classifies tales; do not assign tale numbers to modern characters as if they were universal character labels.

Define a small adapted vocabulary only after research. Clearly distinguish framework-backed concepts, adapted narrative categories (e.g. questing hero, hero of destiny, coming-of-age hero, restorer), and freeform interpretation. Preserve evidence, uncertainty, and counterexamples. Avoid pseudo-clinical personality typing or claims about the owners beyond their curated fictional-character associations.

Potential questions: “What archetype is Jasper most associated with?” and “What narrative patterns recur among Ellie's favorites?” Answers should reference characters/descriptions/association sets, explain their basis, and acknowledge incomplete curation. Counting manually entered archetype tags is not the intended scholarly capability.

## Possible later work; not commitments

Image understanding and semantic image search; perceptual duplicate suggestions; derived roles/motifs; similarity/clustering; collection-level pattern discovery; natural-language Discord questions; website Ask The Librarian; richer Discord actions. Keep derived assets separate and preserve original images. No automatic canonical mutation without a later explicit review/approval design.

## Sequencing and scope rule

1. Preserve the functioning private Archive and its recovery procedures.
2. Verify Clerk exact retrieval in the owner's guild and use it while populating the collection.
3. Add small archival/UX capabilities based on use; research narratology in a distinct milestone.
4. Introduce semantic/model capabilities only for an evaluated feature with enough source material.

No dates or promised AI architecture. The next milestone excludes model APIs, pgvector, Python services, vector databases, AI services, Redis, queues, agent frameworks, custom training, local inference, automated ontology generation, conversational memory, and automatic Archive mutation.
