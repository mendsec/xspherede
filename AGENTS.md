# AGENTS.md — mendsec/xspherede

This file defines persistent rules and architectural context for AI coding agents working on `mendsec/xspherede`.

---

## Hard Rules — Never Violate

1. **English Only Across the Entire Project:**
   - All Rust source files (`src/*.rs`), shaders (`src/*.wgsl`), documentation (`README.md`, `SECURITY.md`, etc.), package metadata (`Cargo.toml`), log messages, UI strings, window titles, comments, and commit messages MUST be written strictly in **English**.
2. **All Security Work Goes to the Private Security Repository (`mendsec/xspherede-security`):**
   - Never commit security audit reports, threat models, vulnerability analyses, exploit proofs-of-concept, internal hardening notes, or unpatched security findings to the public `mendsec/xspherede` repository.
   - Anything security-related that we investigate, audit, or track MUST be stored exclusively in the private repository:
     - **GitHub (Private):** `https://github.com/mendsec/xspherede-security`
     - **Local Path:** `/home/fabio/Dev/xspherede-security` (restricted `0700` permissions)
   - Public commits fixing security issues in `mendsec/xspherede` must use clean, functional, defensive descriptions without exposing exploit vectors, internal heuristics, or vulnerability identifiers prior to coordinated disclosure.
