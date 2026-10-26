# Changelog

## [0.6.0](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.5.0...v0.6.0) (2026-10-26)


### Fixes

* Fix glass-label loading when an NDPI acquisition mask reports RGB but contains only one channel. An unreadable auxiliary image no longer hides other embedded images.

### Features

* Open NDPI and NDPIS files directly from Windows, with installer registration for Open With and Default Apps.
* Automatically load the opened slide's folder in the library.

### Performance

* Load two visible previews concurrently, reuse cached previews, and decode small previews once within a bounded pixel budget.

## [0.5.0](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.4.1...v0.5.0) (2026-10-24)


### Features

* Remember tracking minimap size across slides and application restarts.
* Cycle library slides with Tab and Shift+Tab while preserving text-field editing.
* Configure and remember arrow-key movement in the Navigation menu.
* Show higher visited zoom levels with darker tracking colors and a legend.

### Performance

* Improve low-magnification detail with filtered intermediate tiles and display-scale-aware rendering while retaining bounded tile caches.

### Fixes

* Display areas in mm² from 1,000,000 µm².

## [0.4.1](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.4.0...v0.4.1) (2026-10-23)


### Fixes

* display lengths in millimeters from 1000 micrometers ([#10](https://github.com/Mancuerna/ndpiviewer-rs/issues/10)) ([f1e6259](https://github.com/Mancuerna/ndpiviewer-rs/commit/f1e6259b2237398e4002d25824edef2d6a9fcf47))

## [0.4.0](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.3.0...v0.4.0) (2026-10-22)


### Features

* add left-hand shortcuts and microscope magnification presets ([#8](https://github.com/Mancuerna/ndpiviewer-rs/issues/8)) ([4a3a916](https://github.com/Mancuerna/ndpiviewer-rs/commit/4a3a91682280cd8ce65d7ed16ee4b3025d1e65c9))

## [0.3.0](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.2.1...v0.3.0) (2026-10-22)


### Features

* place circles and rectangles with exact measurements ([#6](https://github.com/Mancuerna/ndpiviewer-rs/issues/6)) ([bd88946](https://github.com/Mancuerna/ndpiviewer-rs/commit/bd889463b797bb753d8e63f387c533779dc90074))

## [0.2.1](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.2.0...v0.2.1) (2026-10-21)


### Fixes

* add a custom Windows application icon ([#4](https://github.com/Mancuerna/ndpiviewer-rs/issues/4)) ([e8b7842](https://github.com/Mancuerna/ndpiviewer-rs/commit/e8b7842ac24f3ce5291f51190f2fe8c8be817336))

## [0.2.0](https://github.com/Mancuerna/ndpiviewer-rs/compare/v0.1.0...v0.2.0) (2026-10-21)


### Features

* notify about new releases at startup ([fb4424a](https://github.com/Mancuerna/ndpiviewer-rs/commit/fb4424ad89e8b70b4558d3f4f0b08d8f92266cb1))

## 0.1.0 (2026-10-21)

First Windows release.

- Explore NDPI slides and NDPIS fluorescence with tiled zoom, pan and Z controls.
- Compare two views with synchronized navigation and independent channels.
- Adjust channel visibility, colors, brightness, contrast and gamma.
- Draw and edit NDPA annotations with calibrated measurements and undo/redo.
- Navigate with an always-visible overview and per-plane visited-area tracking.
- Export views, full-resolution regions and measurement reports.
- Configure a bounded tile cache and retain per-image display preferences.
- Install without Java or additional runtimes; inspect privacy-conscious local error logs.

Windows x64 installer is unsigned. No sample slides or personal data are included.
