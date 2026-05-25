# unshroud

A reactive diagnostic agent for Unix systems.

`unshroud` is a lightweight daemon that collects metrics and events from the host and external plugins, evaluates them against threshold rules, and produces compressed forensic bundles when anomalies occur. It operates on the same principle as network IDS/IPS — passive observation, rule-based reaction, evidence capture.

> **Status:** v0.1.0 — early release. Core pipeline is stable. Plugin runtime and Lua scripting are not implemented yet.

---

## What it is

- **Unix-domain socket listener** accepting NDJSON from external plugins.
- **Internal collector** reading `/proc/stat` and computing CPU utilization deltas.
- **Threshold engine** with per-metric rules and cooldown suppression.
- **Forensic bundler** that dumps the in-memory ring buffer and event log into a zstd-compressed binary file on trigger.
- **Systemd socket activation** compatible (falls back to standalone binding on any other init system).

## What it is not

- Not a metrics time-series database. Buffers are ring-shaped and overwritten.
- Not a dashboard. Output is a binary `.zst` file, intended for post-incident analysis.
- Not a replacement for Prometheus, Grafana, Zabbix, or similar.
- Not yet extensible via Lua or WASM. This is on the roadmap.

## Build

```bash
cargo build --release
