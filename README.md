# portfolio-mcp

Admin-only [MCP](https://modelcontextprotocol.io) server for managing portfolio content, built on the
official Rust SDK ([`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 3.5). Every HTTP request
must carry an OAuth 2.1 access token issued by portfolio **core** for an `admin` user.

## Architecture

```
MCP client ──OAuth 2.1──▶ core (Node): /oauth/* authorization server  ──ES256 JWT──▶ client
MCP client ──Bearer JWT──▶ portfolio-mcp /mcp ──X-Internal-Scopes (in-cluster)──▶ core REST API ──▶ MySQL
```

Core stays the single source of truth (Joi validation, tag sync, MySQL). This server never talks to MySQL.

| Module | Layer | Responsibility |
|---|---|---|
| `domain.rs` | domain | closed set of operations (`Op`, `Entity`), id/page/slug/reorder validation |
| `application.rs` | application | `PortfolioApi` port + safe `AppError` categories |
| `core_client.rs` | infrastructure | `PortfolioApi` over core's REST API (internal scopes header, no credentials) |
| `config.rs` | infrastructure | `config.toml` parsing (unknown keys rejected), fail-closed validation |
| `mcp/tools.rs` | MCP | `portfolio_*` tool schemas and handlers (parse → port → response) |
| `mcp/auth.rs` | MCP | OAuth resource-server middleware (JWKS, ES256, iss/aud/exp/scope/role) |
| `mcp/server.rs` | MCP | streamable-HTTP wiring + RFC 9728 metadata |

Core additions (Node, one file per function in the usual routes → controllers → usecases → data-access layers): `usecases/oauth`, `controllers/oauth`, `data-access/oauth-{clients,authorization-codes,refresh-tokens}`, `exceptions/oauth.error.js`, `routes/oauth-routes.js`,
`utils/rate-limit.js`, `config.oauth`, migration `20261009000001-create-oauth-tables.js`. Existing
endpoints and the existing login/JWT flow are unchanged.

## Tools

All tools return `{"data": ...}` or an error result `{"error": <code>, "message": ...}` with `isError: true`
(`validation_failed`, `not_found`, `conflict`, `unavailable`, `internal`). Updates send only supplied fields.

| Entity | Tools |
|---|---|
| skills | `portfolio_skills_{list,get,create,update,delete}` — `name`, `categoryIds[]`, `level`, `notes`, `sortOrder`, `references[{type: article\|experience\|project, referenceId}]` |
| categories | `portfolio_categories_{list,get,create,update,delete}` — `name`, `description`, `sortOrder` (`includeSkills` on list) |
| articles | `portfolio_articles_{list,get,create,update,delete}`, `portfolio_articles_set_published` — `title`, `content`, `authorName` (all required on create), `tags[]`, `excerpt`, `coverImage`, `authorAvatar`, `status`, `publishedAt` |
| tags | `portfolio_tags_{list,get,create,update,delete}` — `name`, `group`, `color`, `description`, `isSeoEnabled`, `isInternal` |
| experiences | `portfolio_experiences_{list,get,create,update,delete,set_published,reorder}` — `role`, `company` (required on create), `slug`, `companyUrl`, `logoUrl`, `location`, `employmentType`, `startDate`/`endDate` (YYYY-MM-DD), `isCurrent`, `summary`, `description`, `responsibilities[]`, `achievements[]`, `technologies[]`, `tags[]`, `skillIds[]`, `links[{label, url}]`, `sortOrder`, `status` (`status` filter on list) |
| projects | `portfolio_projects_{list,get,create,update,delete,set_published,reorder}` — `title` (required on create), `slug`, `summary`, `description`, `problem`, `solution`, `technicalDecisions`, `outcomes`, `technologies[]`, `tags[]`, `skillIds[]`, `links[{type: repository\|demo\|documentation\|other, label, url}]`, `coverImage`, `images[{url, alt}]`, `isFeatured`, `sortOrder`, `status` (`status`, `featured`, `tag` filters on list) |
| resumes | `portfolio_resumes_{list,get_active,activate,delete}` (PDF upload is deliberately not exposed) |

**Content detail.** A project's `description` is its complete case study: one full GitHub-flavoured markdown document
(tables, task lists, code, ```` ```mermaid ```` diagrams, images) rendered as the project page body with an automatic table of
contents; `problem`/`solution`/`technicalDecisions`/`outcomes` are optional extra sections. The create/update tool descriptions and
server instructions tell clients to write full, specific detail, send the whole rewritten document on updates (never a fragment),
and ask the user for missing facts instead of inventing them.

Lists accept `page` (≥1), `pageSize` (1–500) and `search`. Experience/project `list` and `get` use core's
`admin/*` read routes, so drafts are included; `get` takes exactly one of `id` or `slug` (slug validated locally
against `^[a-z0-9]+(?:-[a-z0-9]+)*$`, max 160). `reorder` takes a non-empty list of unique ids and sets each one's
`sortOrder` to its position. Array fields in updates replace the whole list; because unset/null fields are
dropped, an update cannot clear a nullable field. Full JSON schemas: `tools/list`.
Not exposed: users, roles, profile, git sync, raw SQL/HTTP/filesystem access.

## Authentication & authorization

* **HTTP transport:** OAuth 2.1 per the MCP authorization spec. Core is the authorization server
  (`/.well-known/oauth-authorization-server`, `/oauth/authorize` (admin login page), `/oauth/token`,
  `/oauth/jwks`). There is **no dynamic registration**: clients are predefined rows in `oauth_clients`
  (confidential, `client_secret_basic`/`client_secret_post`; grants: authorization_code + refresh_token). PKCE `S256` is mandatory, redirect URIs must match
  exactly (https or loopback only), codes are single-use for 60 s, refresh tokens rotate and reuse revokes the
  family, and the RFC 8707 `resource` must equal the MCP resource URI (`server.resource_uri`).
* Only users with core's `admin` role can approve; access tokens are ES256 JWTs (10 min) signed with a key
  separate from core's existing HS256 login tokens, so neither token type works on the other's endpoints.
* **This server** validates *every* request (initialize, tools/list, tools/call): signature (ES256 only, JWKS
  cached, refetch rate-limited), `iss`, `aud`, `exp`/`nbf`, scope `portfolio:admin` and role `admin`.
  Missing/invalid → `401`, insufficient → `403`, both with `WWW-Authenticate: Bearer resource_metadata=...`.
  `/.well-known/oauth-protected-resource` is public; nothing else is.
* The caller's token is **not** forwarded to core (no token passthrough). Calls to core are **internal
  in-cluster calls**: the server sends `X-Internal-Scopes: portfolio:admin` and no credentials. Core maps that
  scope to its `service` role, accepted only on the content routes (skills, categories, articles, tags,
  experiences, projects, resumes); users, roles, applications and auth stay human-admin only. The public `service-request-router`
  strips `X-Internal-Scopes` from every request it forwards (covered by a Go test).
* **stdio transport** (`server.transport = "stdio"`): no OAuth; the trust boundary is the spawning local process.

## OAuth clients

Clients live in core's `oauth_clients` table (`client_id` UUID, `client_name`, `client_secret_hash` (scrypt),
`redirect_uris` JSON, `is_active`). Migration `20261009000002-seed-oauth-clients.js` creates **`webapp`** and
**`mcp`** with random UUID ids and secrets and **prints each secret once** (only the hash is stored). Set the
exact redirect URIs at migration time with `OAUTH_WEBAPP_REDIRECT_URIS` / `OAUTH_MCP_REDIRECT_URIS`
(comma-separated) or later via `UPDATE oauth_clients SET redirect_uris = '["https://..."]' WHERE client_name = 'mcp'`.
Add further clients with a new migration/insert (hash with `utils/client-secret.js#hashSecret`); disable one with
`is_active = 0`. Use the `mcp` credentials in your MCP client, e.g. `claude mcp add --transport http portfolio
http://127.0.0.1:8765/mcp --client-id <id> --client-secret`.

## Configuration

All settings live in a TOML file: `portfolio-mcp --config <path>` (default `./config.toml`). No environment
variables are read.

* [`config.toml`](config.toml) — the **deployed** configuration, committed and baked into the Docker image at
  `/app/config.toml` (owned by the non-root user). Change it and push to redeploy.
* [`config.example.toml`](config.example.toml) — local template; copy to `config.local.toml` (git-ignored).

| Key | Required | Notes |
|---|---|---|
| `server.transport` | no (`http`) | `http` (OAuth-protected) or `stdio` (local process only) |
| `server.bind` | no (`127.0.0.1:8765`) | non-loopback needs `allow_remote = true` **and** `behind_tls_proxy = true` |
| `server.allow_internal_http` | no (`false`) | permits http for `core.base_url` and `oauth.jwks_url` only |
| `server.resource_uri` | http | public URL of `/mcp` (token audience); https unless loopback |
| `server.log_level` | no (`info`) | tracing filter |
| `oauth.issuer` / `oauth.jwks_url` | http / no | core's public issuer (https unless loopback); JWKS defaults to `<issuer>/oauth/jwks` |
| `core.base_url` | yes | core API is `<base_url>/apis/<service_name>/` (`service_name` defaults to `core`) |

Unknown keys, wrong types and missing security settings stop startup with an error that names the key/line but
never echoes values. Core's OAuth settings have defaults in `core/config/backend-config.js` (issuer `https://apis.jeetul.com`,
resource `https://mcp.jeetul.com/mcp`, key id, and a signing seed from which the ES256 key is derived; a PEM
key is also accepted). Each can be overridden with its `OAUTH_*` env var. Run the OAuth migrations in
`migrations/`.

## Run locally

```bash
# 1. core: set the OAUTH_* variables, run the migration, start core (port 3000)
cp config.example.toml config.local.toml   # edit, then:
cargo run --release -- --config ./config.local.toml
```

Connect a client (it performs discovery, registration, and the browser sign-in itself):

```bash
claude mcp add --transport http portfolio http://127.0.0.1:8765/mcp
# stdio alternative:
claude mcp add portfolio -- /path/to/portfolio-mcp --config /path/to/stdio-config.toml   # transport = "stdio"
```

## Deployment notes

* Keep it private: loopback or a private network. Do not route `/mcp` through the public portfolio site.
* Remote access: terminate TLS in a reverse proxy (HTTPS), set `server.allow_remote`, `server.behind_tls_proxy`,
  an https `server.resource_uri`, and restrict by firewall/VPN. The server validates the `Host` header against
  the resource URI's host.
* Behind a proxy set Express `trust proxy` in core so its per-IP rate limits see client IPs.
* Rotate the signing seed by changing both `signingPrivateKey` and `keyId` in core's config (clients refetch
  JWKS on an unknown `kid`; existing tokens expire within 10 minutes).

## Docker & CI/CD

* `Dockerfile`: cross-compiles a static musl binary (`tonistiigi/xx`, no QEMU) and ships it on
  `distroless/static:nonroot` (~6 MB); entrypoint `portfolio-mcp --config /app/config.toml` (the committed `config.toml`). `docker buildx build --platform linux/arm64 -t portfolio-mcp .`
* `.github/workflows/cicd-build-push-gitops.yml` (same flow as core): on PRs runs fmt/clippy/tests; on push to
  `main` it also bumps the crate version (`tag-minor` / `tag-major` in the commit message, else patch), builds and
  pushes the arm64 image to the org registry, writes the tag into `deploy/services.json`, regenerates
  `deploy/ymls` and commits them for ArgoCD. Uses the same org vars/secrets as core
  (`ORG_DOCKER_REGISTRY`, `ORG_DOCKER_NAMESPACE`, `ORG_IMAGE_NAMESPACE`, `ORG_DOCKER_REGISTRY_USERNAME/PASSWORD`).
* `deploy/services.json`: ClusterIP service on 8080 in namespace `portfolio`; no ConfigMaps or Secrets needed —
  the configuration is inside the image.
  Public URLs: issuer `https://apis.jeetul.com` (core), resource `https://mcp.jeetul.com/mcp`.
* `service-request-router/config.json` routes `mcp.jeetul.com/*` → this service, and
  `apis.jeetul.com/.well-known/oauth-authorization-server` + `apis.jeetul.com/oauth/*` → core. Restrict
  `mcp.jeetul.com` at the TLS edge (e.g. Cloudflare Access / IP allow-list) so only you can reach it.

## Tests

```bash
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test   # Rust
cd ../core && npm test                                                                      # core OAuth
```

## Limitations

* **Trust boundary for internal calls:** core accepts `X-Internal-Scopes` from anything that can reach it
  directly. The router strips it, but core's ClusterIP Service is reachable from every pod in the cluster
  unless a NetworkPolicy restricts it, and any other ingress/port-forward bypasses the router. Any such
  path gets content-admin access without credentials.

* Core OAuth is intentionally minimal: confidential clients only, one scope, in-process rate limiting (use a shared
  store with multiple core replicas), errors on `/oauth/authorize` render an error page rather than redirecting.
* Core records MCP changes as the internal caller (`sub: internal`), not as the signed-in admin.
* Remote TLS is delegated to a reverse proxy; there is no in-process TLS.
* No live end-to-end run against MySQL was performed; see the report for what was verified.
