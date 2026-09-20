# Changelog

All notable changes to Tapas are recorded here.

## [0.7.14](https://github.com/nkootstra/tapas/compare/v0.7.13...v0.7.14) - 2026-09-20

### Patch changes

- reject grouped rg decompression flags ([#58](https://github.com/nkootstra/tapas/pull/58))

## [0.7.13](https://github.com/nkootstra/tapas/compare/v0.7.12...v0.7.13) - 2026-09-20

### Patch changes

- preserve diff hunk lines that resemble file metadata ([#56](https://github.com/nkootstra/tapas/pull/56))

## [0.7.12](https://github.com/nkootstra/tapas/compare/v0.7.11...v0.7.12) - 2026-09-20

### Patch changes

- preserve nextest failure details ([#54](https://github.com/nkootstra/tapas/pull/54))

## [0.7.11](https://github.com/nkootstra/tapas/compare/v0.7.10...v0.7.11) - 2026-09-20

### Patch changes

- preserve ESLint paths with spaces ([#52](https://github.com/nkootstra/tapas/pull/52))

## [0.7.10](https://github.com/nkootstra/tapas/compare/v0.7.9...v0.7.10) - 2026-09-20

### Patch changes

- distinguish duplicate GitHub job names ([#50](https://github.com/nkootstra/tapas/pull/50))

## [0.7.9](https://github.com/nkootstra/tapas/compare/v0.7.8...v0.7.9) - 2026-09-20

### Patch changes

- preserve facts beside Zig summaries ([#48](https://github.com/nkootstra/tapas/pull/48))

## [0.7.8](https://github.com/nkootstra/tapas/compare/v0.7.7...v0.7.8) - 2026-09-20

### Patch changes

- reject overflowing build numbers ([#46](https://github.com/nkootstra/tapas/pull/46))

## [0.7.7](https://github.com/nkootstra/tapas/compare/v0.7.6...v0.7.7) - 2026-09-20

### Patch changes

- preserve commit hook preambles ([#44](https://github.com/nkootstra/tapas/pull/44))

## [0.7.6](https://github.com/nkootstra/tapas/compare/v0.7.5...v0.7.6) - 2026-09-20

### Patch changes

- avoid invented diffstat counts ([#42](https://github.com/nkootstra/tapas/pull/42))

## [0.7.5](https://github.com/nkootstra/tapas/compare/v0.7.4...v0.7.5) - 2026-09-20

### Patch changes

- preserve multiple tree roots ([#40](https://github.com/nkootstra/tapas/pull/40))

## [0.7.4](https://github.com/nkootstra/tapas/compare/v0.7.3...v0.7.4) - 2026-09-19

### Patch changes

- preserve indented uv package changes ([#38](https://github.com/nkootstra/tapas/pull/38))

## [0.7.3](https://github.com/nkootstra/tapas/compare/v0.7.2...v0.7.3) - 2026-09-19

### Patch changes

- reject short AWS table rows without panicking ([#36](https://github.com/nkootstra/tapas/pull/36))

## [0.7.2](https://github.com/nkootstra/tapas/compare/v0.7.1...v0.7.2) - 2026-09-19

### Patch changes

- preserve verbose remote branch details ([#34](https://github.com/nkootstra/tapas/pull/34))

## [0.7.1](https://github.com/nkootstra/tapas/compare/v0.7.0...v0.7.1) - 2026-09-19

### Patch changes

- mark clean working tree in git status output ([#26](https://github.com/nkootstra/tapas/pull/26))

## [0.7.0](https://github.com/nkootstra/tapas/compare/v0.6.0...v0.7.0) - 2026-08-16

### Minor changes

- add trusted process filter plugins ([#24](https://github.com/nkootstra/tapas/pull/24))

## [0.6.0](https://github.com/nkootstra/tapas/compare/v0.5.0...v0.6.0) - 2026-08-14

### Minor changes

- automate trusted release pull request merges ([#21](https://github.com/nkootstra/tapas/pull/21))

### Patch changes

- repair release automation ([#22](https://github.com/nkootstra/tapas/pull/22))

## [0.5.0](https://github.com/nkootstra/tapas/compare/v0.4.0...v0.5.0) - 2026-08-13

### Minor changes

- expand shape-aware command coverage ([#19](https://github.com/nkootstra/tapas/pull/19))

### Patch changes

- allow release tags across workflow changes ([#18](https://github.com/nkootstra/tapas/pull/18))
- skip preparation when the release is already prepared ([#17](https://github.com/nkootstra/tapas/pull/17))

## [0.4.0](https://github.com/nkootstra/tapas/compare/v0.3.0...v0.4.0) - 2026-08-11

### Added

- *(catalog)* expand default CLI support ([#12](https://github.com/nkootstra/tapas/pull/12))

### Minor changes

- automate signed release preparation and publishing ([#13](https://github.com/nkootstra/tapas/pull/13))

### Performance

- apply internal optimalizations ([#9](https://github.com/nkootstra/tapas/pull/9))

## [0.3.0] - 2026-08-05

### Added

- Established the maintained Tapas Rust release line with signed, checksummed binaries.
