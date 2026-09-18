# API token hardening

This plan covers Dreamscroll-issued API tokens only. It is intentionally separate from
Cloud Tasks webhook authentication and Google OIDC workload identity.

## Current state

`src/auth/jwt.rs` issues and validates one-day HS256 bearer tokens using the existing
`jsonwebtoken` crate. The token claims currently include `sub`, `username`, `is_admin`,
`storage_shard`, `iat`, and `exp`. The REST API uses these tokens through
`JwtAxumLayer`.

This is a reasonable small first-party API-token design, but the validation and
lifecycle policy should be made more explicit before the API becomes more widely used.

## Recommended hardening

1. Rename or document `JwtConfig` as the Dreamscroll API-token configuration and keep
   its key material private to that domain.
2. Add explicit `iss` and `aud` claims to newly issued API tokens and require them on
   verification. Use stable configured values such as `dreamscroll-api` and a versioned
   service identifier. Do not use the Google webhook audience for user tokens.
3. Decide whether API tokens should remain one-day bearer tokens. If account or admin
   changes must take effect immediately, move authorization decisions to the database
   or shorten token lifetime; do not treat embedded `is_admin` as a live permission
   source.
4. Replace `JwtConfig::from_secret`'s short-secret assertion with fallible startup
   validation. Configuration errors should fail startup cleanly rather than panic.
5. Add tests for wrong algorithm, missing issuer/audience, expired tokens, invalid
   claim types, malformed subjects, and other claim-validation failures.
6. Keep error responses generic and never log access tokens. The token endpoint should
   also avoid logging usernames at warning level if usernames are sensitive.
7. Pin compatible versions deliberately. The project currently locks
   `google-cloud-auth` 1.9.x and `jsonwebtoken` 10.3.x. Upgrade these together when
   necessary and review the lockfile rather than independently jumping majors.

These are hardening steps, not a reason to replace the current JWT implementation with
another library.

## Scope boundary

This plan does not cover:

- Browser session cookies, which remain handled by `axum-login` and Tower Sessions.
- Google-signed Cloud Tasks ID-token verification, which belongs to the webhook OIDC
  design in `plan/webhook_oidc_same_service.md`.
- Interactive login with an external OIDC provider.

If external interactive OIDC or multiple identity providers are added later, reassess
whether `openidconnect` is appropriate at that time.
