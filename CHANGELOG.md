# Changelog

All notable changes to this project are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Continuous integration: formatting, lints, tests, minimum supported Rust version check, dependency advisory audit, and cross-compilation checks for FreeBSD and NetBSD.
- Contribution guide, code of conduct, issue forms, and pull request template.
- `Cargo.lock` is now tracked for reproducible builds.
- Screenshot of the prototype in the README.

### Changed

- README rewritten with an explicit project status, platform support table, and roadmap.
- Minimum supported Rust version declared as 1.88.
- Updated `glam` to 0.34, `wgpu` to 30, and `pollster` to 1.0.

### Fixed

- A lost rendering surface is now recreated instead of only being reconfigured.
- A lost GPU device no longer leaves the renderer unusable: all GPU resources are rebuilt on a new device and the session (camera, window placement, Mini-Globe, lockdown, clipboard vault) carries over.

## [0.0.1]

### Added

- Spherical workspace with geodesic grid, damped camera, and free flight.
- Curved windows with spherical raycasting, dragging, and adjustable radial depth.
- Interactive, expandable Mini-Globe overview.
- Volumetric code rain shader with reading cone and per-window shield.
- Security engine: trust domains, inter-domain clipboard vault, lockdown mode, surface validation, shader integrity check, and host audit.
