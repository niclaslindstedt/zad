# Changelog

## [Unreleased]

- fix(spotify): tolerate `id: null` on local-file playlist items so
  the whole `/playlists/{id}/items` page no longer fails to decode

## [0.8.2]

- feat!(spotify, ymusic): paginate list endpoints + ApiCalibration constants (#82)
- fix(gcal): add cross-process access-token cache and refresh lock (#81)

