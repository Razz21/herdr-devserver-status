# Changelog

## [0.2.0]

### Fixed

- `has_errors` stuck `true` after recovery once the error needle scrolled out of the read window — recompute every `Running` tick instead of gating on needle presence.
- `status` regressed `Running` → `Starting` once the `Ready` needle/URL scrolled out of the read window — `Running` now sticky, cleared only on process restart.

### Changed

- `ToolDetector::match_output` — added `previous_status: ToolStatus` param.
- `signal_matching::derive_status` — added `previous_status: ToolStatus` param.

## [0.1.0]

### Added
- initial release: framework spec loader, vite detector, daemon discovery/worker loop