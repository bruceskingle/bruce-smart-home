# Octopus's API

Notes for coding agents adding or changing Octopus queries, moved from AGENTS.md.

- `rateLimitInfo { fieldSpecificRateLimits(first: 100) { edges { node { field rate ttl isBlocked } } } }` lists each
  limited field's rate, whether the caller is blocked, and `ttl`, when the block or window ends, as a unix time. A
  limited request fails with GraphQL error KT-CT-1199, not HTTP 429.
- Measured on 25 October 2026: `smartMeterTelemetry` 125 an hour, a fixed hour from the first request;
  `PropertyType.measurements` 20 a minute; 500,000 points an hour.
- `smartMeterTelemetry` at TEN_SECONDS gives 12 and 13 hours, an empty list with no error for 18 hours, and KT-GB-4039
  for 24, so the app asks for at most 12 hours at a time, `MAX_REQUEST_SECONDS` in `src/octopus/telemetry.rs`.
- `--profile <profile> --octopusday <YYYY-MM-DD>` asks Octopus for each meter's half hours of a day, with the app's
  GraphQL query (RAW_INTERVAL, and THIRTY_MIN_INTERVAL for the import), and for the import meter from the REST API's
  `consumption` endpoint with the profile's API key, which is not shown, printing only counts, times and values, and
  storing nothing: e.g. on 23 November 2026 all three had only 00:00 of the import meter's 10 November, the export all
  48, so the gap was Octopus's, though its iOS app showed the day, presumably from the Home Mini.
