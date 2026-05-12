# Changelog

## [Unreleased]

- feat!(spotify, ymusic): paginate every list endpoint and expose an
  `ApiCalibration` constant per service so callers can record the
  upstream API revision their reads were grounded against
- fix(spotify): switch `PUT`/`DELETE /me/library` to the canonical
  query-string `?uris=<csv>` shape with internal 40-URI chunking

## [0.8.1]

- fix(token): add cross-process access-token cache and refresh lock (#80)

