# Octopus's API

Notes for coding agents adding or changing Octopus queries, moved from AGENTS.md.

- `rateLimitInfo { fieldSpecificRateLimits(first: 100) { edges { node { field rate ttl isBlocked } } } }` lists each
  limited field's rate, whether the caller is blocked, and `ttl`, when the block or window ends, as a unix time. A
  limited request fails with GraphQL error KT-CT-1199, not HTTP 429.
- Measured on 25 October 2026: `smartMeterTelemetry` 125 an hour, a fixed hour from the first request;
  `PropertyType.measurements` 20 a minute; 500,000 points an hour.
- `smartMeterTelemetry` at TEN_SECONDS gives 12 and 13 hours, an empty list with no error for 18 hours, and KT-GB-4039
  for 24, so the app asks for at most 12 hours at a time, `MAX_REQUEST_SECONDS` in `src/octopus/telemetry.rs`.
