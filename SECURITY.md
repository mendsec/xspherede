# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.0.x   | :white_check_mark: |

## Scope

XSphereDE is an early prototype and does not yet provide an isolation boundary; see the project status section of the [README](README.md#project-status). Reports are still welcome for any defect in the security engine, input handling, surface validation, rendering pipeline, build configuration, or dependency chain.

## Reporting a Vulnerability

We take the security of **XSphereDE** seriously. If you discover a security vulnerability, please **do not** report it via public GitHub issues, discussions, or pull requests.

### Private Reporting Channels

1. **GitHub Private Vulnerability Reporting (Preferred):**
   Use the [Report a vulnerability](https://github.com/mendsec/xspherede/security/advisories/new) button under the **Security** tab of this repository to open a confidential security advisory.
2. **Encrypted / Direct Email:**
   Send a detailed report to **fabiomendes@mailfence.com** with:
   - Description of the vulnerability and affected component (`camera`, `minimap`, `sphere_env`, `window3d`, `shader.wgsl`, or Wayland/PipeWire surface handling).
   - Steps to reproduce or proof-of-concept (PoC).
   - Potential impact and suggested mitigation (if any).

### Response Timeline & Governance

- **Acknowledgment:** Within **48 hours** of receipt.
- **Triage & Assessment:** Within **7 days**.
- **Coordinated Disclosure:** Security audits, threat models, hardening policies, and remediation tracking are maintained in our dedicated **private security repository** (`mendsec/xspherede-security`) until a fix is verified and released.
