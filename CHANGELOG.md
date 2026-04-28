# Changelog

## [0.6.2](https://github.com/alohaninja/shift/compare/v0.6.1...v0.6.2) (2026-04-28)


### Bug Fixes

* rename opencode-shift-proxy to @shift-preflight/opencode-plugin ([d72a6a1](https://github.com/alohaninja/shift/commit/d72a6a15c7b279b03763904f06262ecc7d44cea6))

## [0.6.1](https://github.com/alohaninja/shift/compare/v0.6.0...v0.6.1) (2026-04-28)


### Bug Fixes

* **opencode-plugin:** clarify startup timing in README ([0aa5041](https://github.com/alohaninja/shift/commit/0aa5041ab1275059632700dd9547c05beafbe227))

## [0.6.0](https://github.com/alohaninja/shift/compare/v0.5.1...v0.6.0) (2026-04-28)


### Features

* add opencode-shift-proxy plugin for automatic image optimization ([#20](https://github.com/alohaninja/shift/issues/20)) ([901426c](https://github.com/alohaninja/shift/commit/901426c61b1b983c0b3bce50534ac17025417da4))

## [0.5.1](https://github.com/alohaninja/shift/compare/v0.5.0...v0.5.1) (2026-04-24)


### Bug Fixes

* strip content-encoding from proxied responses to prevent ZlibError ([#18](https://github.com/alohaninja/shift/issues/18)) ([4a5696f](https://github.com/alohaninja/shift/commit/4a5696f8b68b449cdcbfc70b61fb518c8b73267f))

## [0.5.0](https://github.com/alohaninja/shift/compare/v0.4.1...v0.5.0) (2026-04-24)


### Features

* add @shift-ai/runtime — AI SDK middleware + HTTP proxy ([#16](https://github.com/alohaninja/shift/issues/16)) ([6f7c8ed](https://github.com/alohaninja/shift/commit/6f7c8ede894138ccabaf2c97b95aa432f0afc870))

## [0.4.1](https://github.com/alohaninja/shift/compare/v0.4.0...v0.4.1) (2026-04-24)


### Bug Fixes

* auto-update docs version on release + use shift-ai.dev custom domain ([#14](https://github.com/alohaninja/shift/issues/14)) ([844d724](https://github.com/alohaninja/shift/commit/844d72499f3156d7b7ad92b479e646dc4444db21))

## [0.4.0](https://github.com/alohaninja/shift/compare/v0.3.0...v0.4.0) (2026-04-24)


### Features

* add GitHub Pages site with RTK-style landing page and guide ([#12](https://github.com/alohaninja/shift/issues/12)) ([cbc7f76](https://github.com/alohaninja/shift/commit/cbc7f76a21b7298fdee05f55e616e651793ce504))

## [0.3.0](https://github.com/alohaninja/shift/compare/v0.2.0...v0.3.0) (2026-04-24)


### Features

* RTK-inspired gain dashboard with sparkline and auto-purge ([#9](https://github.com/alohaninja/shift/issues/9)) ([6f84e27](https://github.com/alohaninja/shift/commit/6f84e27cf9bdb28b3cfea19f35a50ffc2eeaef5f))

## [0.2.0](https://github.com/alohaninja/shift/compare/v0.1.4...v0.2.0) (2026-04-24)


### Features

* publish shift-ai-preflight skill to skills.sh ([2003a05](https://github.com/alohaninja/shift/commit/2003a05a7f223efd341e2c914c0f9768e8c85463))


### Bug Fixes

* revert to 0.1.4 and add shift-cli dep version to release-please extra-files ([71d3f90](https://github.com/alohaninja/shift/commit/71d3f90f75f389c0ab6611a7f400746a71143b4d))
* update shift-preflight dependency to ^0.2 (unblocks release build) ([93bce7f](https://github.com/alohaninja/shift/commit/93bce7f65e20b8bf23203eaaef3ff7c344b9d67d))

## [0.1.4](https://github.com/alohaninja/shift/compare/v0.1.3...v0.1.4) (2026-04-23)


### Bug Fixes

* configure release-please for Cargo workspace version inheritance ([0dd2789](https://github.com/alohaninja/shift/commit/0dd278967577e766ed9694257d4d6f07a736f423))
* preserve JPEG format when resizing images ([#5](https://github.com/alohaninja/shift/issues/5)) ([ae7b693](https://github.com/alohaninja/shift/commit/ae7b693dc9ba8d6cfab14879bfe3e6809fb3c5c7))
* switch release-please to simple type to avoid Cargo workspace parsing ([4e994d4](https://github.com/alohaninja/shift/commit/4e994d497ed0cfdecd19a365351370c708ea3544))
