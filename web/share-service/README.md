# Sketch3D artwork share service

This is a Cloudflare Workers + R2 candidate for the host-independent viewer. It is local code only; it has not been deployed and does not include account credentials. A production account owner and monthly cost ceiling are still required before deployment.

## Local checks

Run the provider-independent Worker tests with:

```sh
node --test web/share-service/test/worker.test.mjs
```

They use an in-memory R2 fake and a single Durable Object instance. They do not prove remote Cloudflare behavior, billing limits, or device/browser support.

## API

- `POST /api/v1/models` accepts one validated binary glTF 2.0 GLB (maximum 10 MiB). It returns a share URL and a 256-bit management capability once.
- `PUT /api/v1/models/{id}` replaces the GLB after `Authorization: Bearer <capability>` validation.
- `DELETE /api/v1/models/{id}` immediately removes the public object after the same authorization check.
- `GET /models/{id}.glb` serves the public GLB without caching.

The public ID and management capability are generated independently. Only the SHA-256 hash of the capability is stored in private R2 metadata. Capability bytes are never put in the URL or response logs. The global Durable Object serializes writes and enforces at most 900 active shares and 9 GiB total stored GLBs. Each share expires 30 days after its latest publish/update; an expired URL returns 404 immediately, even while R2 lifecycle cleanup is pending.

## R2 lifecycle setup

Before deployment:

1. In the Cloudflare dashboard, open **R2 Object Storage → sketch3d-shares → Settings → Object lifecycle rules**.
2. Add an expiration rule scoped to the `shares/` prefix and set it to expire objects after 30 days.
3. Verify the rule is enabled and keep this service's `expiresAt` check in place. The service check is what makes public links return 404 at their expiry time; lifecycle removal is only physical cleanup.

Authenticated DELETE removes an object immediately. Cloudflare lifecycle deletion can lag by up to 24 hours, so this rule does not promise immediate physical erasure. Recheck the current provider behavior and account pricing before launch.

## Deployment prerequisites

The Wrangler configuration binds an R2 bucket, a SQLite-backed Durable Object, and the static `web/share-viewer` assets. Deployment remains gated on a named billing/account owner, an approved monthly cost ceiling, lifecycle and abuse-response ownership, and account configuration. Cloudflare free allowances and budget alerts are not a hard bill cap for R2; the code quotas bound active stored bytes and objects, but request/operation volume and provider/account policies still require monitoring.
