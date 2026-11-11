//! Security and Hardening Engine for **XSphereDE**.
//!
//! Enforces strict Desktop Environment (DE) and 3D Compositor security policies directly
//! modeled after the defense-in-depth architectures of:
//! - **secureblue** (<https://secureblue.dev/features>):
//!   - Complete disablement of `Xwayland` by default (strict Wayland isolation, eliminating X11 global snooping).
//!   - Disablement of automatic file/media thumbnailers (preventing zero-click RCE exploits in media parsers).
//!   - Prohibition of third-party shell extensions/plugins inside the privileged compositor address space.
//!   - *Zero-SUID* policy (`run0` instead of `sudo`/`su`/`pkexec`) and `ptrace` restrictions.
//!   - Runtime detection and compatibility with `hardened_malloc` (GrapheneOS / secureblue).
//!   - Removal of risky legacy attack surface (`fuse2`, unauthenticated IPC).
//! - **Qubes OS** (<https://doc.qubes-os.org/en/r4.3/user/downloading-installing-upgrading/install-security.html>):
//!   - **Trust Domains (`TrustDomain`)** with **unforgeable** colored 3D window borders rendered
//!     exclusively by the trusted compositor shader (impossible for a client application to spoof).
//!   - **Inter-Domain Clipboard Vault (`SecureClipboardVault`)** triggered solely via the compositor's
//!     trusted path (`Ctrl+Shift+C` / `Ctrl+Shift+V`) with Mandatory Access Control (Biba integrity
//!     blocking untrusted domains from pasting into `CoreVault`) and immediate *single-use auto-wipe*.
//!   - **SHA-256 Cryptographic Attestation** of WGSL shaders prior to GPU compilation.
//!   - **Trusted Input Path** and instant **Sphere Lockdown Mode (`L`)**.

use std::fmt;
use std::fs;

/// Maximum allowed texture dimension per window (mitigates VRAM exhaustion / GPU DoS attacks).
pub const MAX_WINDOW_TEXTURE_DIM: u32 = 4096;

/// Maximum allowed byte length for a window RGBA surface buffer (64 MiB).
pub const MAX_SURFACE_BUFFER_BYTES: usize = (MAX_WINDOW_TEXTURE_DIM as usize)
    .saturating_mul(MAX_WINDOW_TEXTURE_DIM as usize)
    .saturating_mul(4);

/// Expected SHA-256 cryptographic digest of the unified WGSL shader pipeline (`src/shader.wgsl`).
/// Any tampering with the shader source (e.g., attempting to bypass unforgeable domain borders)
/// immediately aborts compositor initialization before uploading code to the GPU.
pub const EXPECTED_SHADER_SHA256: &str =
    "0dbf47c26f90c678aec5b50e273bd12eda2e3334f148af48b29fc1a8d7e1516d";

/// Cryptographically verifies the SHA-256 integrity of the WGSL shader source before GPU compilation.
pub fn verify_shader_source_integrity(shader_source: &str) -> Result<String, SecurityViolation> {
    let actual_hex = sha256_hex(shader_source.as_bytes());
    if actual_hex != EXPECTED_SHADER_SHA256 {
        return Err(SecurityViolation::IntegrityCheckFailed {
            asset_name: "src/shader.wgsl",
            expected_hex: EXPECTED_SHADER_SHA256,
            actual_hex,
        });
    }
    Ok(actual_hex)
}

/// Compartmentalized security trust domains modeled after **Qubes OS**.
///
/// Variant order reflects ascending integrity/trust level:
/// `DisposableRed < UntrustedNet < IsolatedApp < TrustedWork < CoreVault`.
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrustDomain {
    /// Ephemeral disposable quarantine (`[DISP-RED]`): unforgeable crimson red border.
    /// Intended for untrusted inputs; destroyed on exit; prohibited from exporting data to high-trust domains.
    DisposableRed = 0,
    /// Untrusted networked domain (`[UNTRUSTED-NET]`): amber/orange border.
    /// Exposed to external networks (e.g., SELinux/seccomp-confined Trivalent browser).
    UntrustedNet = 1,
    /// Sandboxed application domain (`[ISOLATED-APP]`): sapphire/cyan border.
    /// Air-gapped from direct network access, no automatic thumbnailers, restricted filesystem access.
    IsolatedApp = 2,
    /// Cryptographically verified workspace (`[TRUSTED-WORK]`): Matrix emerald green border.
    TrustedWork = 3,
    /// Core administrative / Vault domain (`[DOM0: VAULT]`): royal violet/purple border.
    /// Strictly air-gapped (zero network access, isolated against clipboard paste from lower-trust domains).
    CoreVault = 4,
}

impl TrustDomain {
    /// Unambiguous domain badge displayed by the Trusted Window Manager.
    pub const fn badge_label(self) -> &'static str {
        match self {
            Self::DisposableRed => "DISP-RED // QUARANTINE",
            Self::UntrustedNet => "UNTRUSTED // NETWORK",
            Self::IsolatedApp => "ISOLATED // SANDBOX",
            Self::TrustedWork => "TRUSTED // VERIFIED",
            Self::CoreVault => "DOM0 // CORE-VAULT",
        }
    }

    /// Unforgeable RGB border color enforced directly by the compositor GPU shader (`fs_window`).
    /// No client application surface can overwrite or spoof this frame.
    pub const fn border_color_rgb(self) -> [f32; 3] {
        match self {
            Self::DisposableRed => [1.00, 0.16, 0.22], // Crimson Red (Qubes Untrusted / Disposable)
            Self::UntrustedNet => [1.00, 0.68, 0.12],  // Amber / Orange (Qubes Yellow / Orange)
            Self::IsolatedApp => [0.14, 0.78, 1.00],   // Sapphire / Cyan (Qubes App Sandbox)
            Self::TrustedWork => [0.18, 1.00, 0.52],   // Matrix Emerald Green (Trusted)
            Self::CoreVault => [0.76, 0.32, 1.00],     // Royal Violet / Purple (Dom0 / Vault)
        }
    }

    /// Indicates whether this trust domain is permitted to access external network interfaces.
    pub const fn allows_network(self) -> bool {
        matches!(
            self,
            Self::DisposableRed | Self::UntrustedNet | Self::TrustedWork
        )
    }

    /// Mandatory Access Control (MAC) policy for inter-domain clipboard transfers.
    ///
    /// Modeled after the **Biba Integrity Model** and **Qubes OS**:
    /// - `CoreVault` (`Dom0`) **never** accepts clipboard data from lower-integrity domains.
    /// - `DisposableRed` is prohibited from pasting into `TrustedWork` or `CoreVault`.
    /// - Explicit transfers (`Ctrl+Shift+C` -> `Ctrl+Shift+V`) between permitted domains
    ///   immediately wipe the vault upon completion (*single-use auto-wipe*).
    pub const fn can_transfer_clipboard_to(self, destination: Self) -> bool {
        match (self, destination) {
            (src, Self::CoreVault) => matches!(src, Self::CoreVault),
            (Self::DisposableRed, Self::TrustedWork) => false,
            _ => true,
        }
    }
}

impl fmt::Display for TrustDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.badge_label())
    }
}

/// Hardened Desktop Environment security policies modeled after **secureblue** and **Qubes OS**.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecurityPolicy {
    /// Completely disables Xwayland (prevents X11 global keylogging and client snooping).
    pub disable_xwayland: bool,
    /// Disables automatic file/image/video thumbnailing in the file navigator.
    pub disable_thumbnailers: bool,
    /// Prohibits loading third-party extensions/plugins inside the compositor process.
    pub disable_shell_extensions: bool,
    /// Prohibits legacy unmaintained FUSE2 / unconfined AppImage execution.
    pub disable_legacy_fuse2: bool,
    /// Enforces a Zero-SUID policy (`run0` over `sudo`/`su`/`pkexec`).
    pub enforce_run0_zero_suid: bool,
    /// Restricts `ptrace` attachment and core dumps (`PR_SET_DUMPABLE = 0`).
    pub restrict_ptrace_and_coredumps: bool,
    /// Isolates clipboard transfers across domains (`Ctrl+Shift+C` / `Ctrl+Shift+V` with auto-wipe).
    pub inter_domain_clipboard_lock: bool,
    /// Routes keyboard/mouse events strictly to the focused window (Trusted Input Path).
    pub strict_input_routing: bool,
    /// Blocks unauthenticated cross-window screen capture / screencasting.
    pub block_unauthenticated_screencast: bool,
    /// Requires SHA-256 cryptographic verification of WGSL shaders prior to GPU compilation.
    pub verify_shader_integrity: bool,
}

impl SecurityPolicy {
    /// Returns the immutable maximum hardened security profile for XSphereDE.
    pub const fn maximum_hardened() -> Self {
        Self {
            disable_xwayland: true,
            disable_thumbnailers: true,
            disable_shell_extensions: true,
            disable_legacy_fuse2: true,
            enforce_run0_zero_suid: true,
            restrict_ptrace_and_coredumps: true,
            inter_domain_clipboard_lock: true,
            strict_input_routing: true,
            block_unauthenticated_screencast: true,
            verify_shader_integrity: true,
        }
    }

    /// Returns `true` only if all mandatory `secureblue` + `Qubes OS` locks are enabled.
    pub const fn is_fully_enforced(&self) -> bool {
        self.disable_xwayland
            && self.disable_thumbnailers
            && self.disable_shell_extensions
            && self.disable_legacy_fuse2
            && self.enforce_run0_zero_suid
            && self.restrict_ptrace_and_coredumps
            && self.inter_domain_clipboard_lock
            && self.strict_input_routing
            && self.block_unauthenticated_screencast
            && self.verify_shader_integrity
    }
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self::maximum_hardened()
    }
}

/// Security violations detected and blocked by the XSphereDE compositor.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecurityViolation {
    /// Window RGBA buffer dimensions or byte size violate GPU memory safety bounds.
    InvalidSurfaceBuffer {
        width: u32,
        height: u32,
        actual_len: usize,
    },
    /// Attempted to paste (`Ctrl+Shift+V`) when the isolated clipboard vault is empty.
    ClipboardVaultEmpty,
    /// Mandatory Access Control (MAC) policy blocked an inter-domain clipboard transfer.
    CrossDomainClipboardBlocked {
        source: TrustDomain,
        destination: TrustDomain,
    },
    /// SHA-256 cryptographic attestation failed for a critical compositor asset.
    IntegrityCheckFailed {
        asset_name: &'static str,
        expected_hex: &'static str,
        actual_hex: String,
    },
    /// Operation rejected because XSphereDE is currently in Lockdown mode (`Key L`).
    SystemLockedDown,
}

impl fmt::Display for SecurityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSurfaceBuffer {
                width,
                height,
                actual_len,
            } => write!(
                f,
                "Rejected surface buffer ({}x{}, {} bytes): outside GPU safety bounds",
                width, height, actual_len
            ),
            Self::ClipboardVaultEmpty => {
                write!(
                    f,
                    "Clipboard vault is empty (press Ctrl+Shift+C on a source window first)"
                )
            }
            Self::CrossDomainClipboardBlocked {
                source,
                destination,
            } => write!(
                f,
                "BLOCKED BY QUBES MAC POLICY: Cannot paste from [{}] into [{}]",
                source.badge_label(),
                destination.badge_label()
            ),
            Self::IntegrityCheckFailed {
                asset_name,
                expected_hex,
                actual_hex,
            } => write!(
                f,
                "CRYPTOGRAPHIC INTEGRITY VIOLATION in '{}': expected SHA-256 {}, got {}",
                asset_name, expected_hex, actual_hex
            ),
            Self::SystemLockedDown => {
                write!(
                    f,
                    "Operation blocked: XSphereDE is in LOCKDOWN mode (press 'L' to unlock)"
                )
            }
        }
    }
}

/// Sealed entry inside the Qubes-style Inter-Domain Clipboard Vault.
#[derive(Clone, Debug)]
struct ClipboardEntry {
    source_domain: TrustDomain,
    source_window_title: String,
    payload: String,
}

impl Drop for ClipboardEntry {
    fn drop(&mut self) {
        // Overwrite payload bytes in memory before deallocation (mitigates data remanence)
        // in 100% safe Rust without `unsafe`.
        let len = self.payload.len();
        self.payload.clear();
        self.payload.push_str(&"\0".repeat(len.min(4096)));
        self.payload.clear();
    }
}

/// Qubes-style Isolated Inter-Domain Clipboard Vault.
///
/// Prevents background windows from silently reading or writing a global shared clipboard:
/// 1. The user presses `Ctrl+Shift+C` to seal data from the focused window into the vault.
/// 2. The user focuses the target window and presses `Ctrl+Shift+V`.
/// 3. The compositor checks the MAC policy (`TrustDomain::can_transfer_clipboard_to`) and,
///    if approved, delivers the payload and **immediately wipes** the vault (*single-use auto-wipe*).
#[derive(Default)]
pub struct SecureClipboardVault {
    sealed: Option<ClipboardEntry>,
    pub total_transfers_approved: u64,
    pub total_transfers_blocked: u64,
}

impl SecureClipboardVault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seals data from the currently focused window into the clipboard vault (`Ctrl+Shift+C`).
    pub fn seal_from_window(
        &mut self,
        source_domain: TrustDomain,
        source_window_title: &str,
        payload: String,
    ) {
        self.sealed = Some(ClipboardEntry {
            source_domain,
            source_window_title: source_window_title.to_string(),
            payload,
        });
    }

    /// Returns the source `TrustDomain` if a sealed entry is currently pending in the vault.
    #[allow(dead_code)]
    pub fn pending_source_domain(&self) -> Option<TrustDomain> {
        self.sealed.as_ref().map(|e| e.source_domain)
    }

    /// Unseals and consumes (*auto-wipes*) the clipboard vault into the target window (`Ctrl+Shift+V`).
    pub fn unseal_to_window(
        &mut self,
        destination_domain: TrustDomain,
    ) -> Result<(TrustDomain, String, String), SecurityViolation> {
        let Some(entry) = self.sealed.take() else {
            return Err(SecurityViolation::ClipboardVaultEmpty);
        };

        if !entry
            .source_domain
            .can_transfer_clipboard_to(destination_domain)
        {
            self.total_transfers_blocked = self.total_transfers_blocked.saturating_add(1);
            // Even when blocked, `take()` has already wiped the vault to prevent retry probing
            return Err(SecurityViolation::CrossDomainClipboardBlocked {
                source: entry.source_domain,
                destination: destination_domain,
            });
        }

        self.total_transfers_approved = self.total_transfers_approved.saturating_add(1);
        Ok((
            entry.source_domain,
            entry.source_window_title.clone(),
            entry.payload.clone(),
        ))
    }

    /// Immediately wipes any residual data from the clipboard vault (e.g., on Lockdown).
    pub fn wipe(&mut self) {
        self.sealed = None;
    }
}

/// Strictly validates a window RGBA surface buffer prior to GPU upload (`wgpu::Queue`),
/// preventing integer overflows, zero-sized dimensions, or VRAM exhaustion attacks.
pub fn validate_surface_buffer(
    width: u32,
    height: u32,
    rgba_bytes: &[u8],
) -> Result<(), SecurityViolation> {
    if width == 0
        || height == 0
        || width > MAX_WINDOW_TEXTURE_DIM
        || height > MAX_WINDOW_TEXTURE_DIM
    {
        return Err(SecurityViolation::InvalidSurfaceBuffer {
            width,
            height,
            actual_len: rgba_bytes.len(),
        });
    }

    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4));

    match expected_len {
        Some(len) if len == rgba_bytes.len() && len <= MAX_SURFACE_BUFFER_BYTES => Ok(()),
        _ => Err(SecurityViolation::InvalidSurfaceBuffer {
            width,
            height,
            actual_len: rgba_bytes.len(),
        }),
    }
}

/// Potentially dangerous environment variables that can be abused for code injection
/// into GUI toolkits/processes (`GTK_MODULES`, `QT_PLUGIN_PATH`, `LD_AUDIT`, etc.).
pub const DANGEROUS_ENV_VARS: &[&str] = &[
    "LD_AUDIT",
    "LD_DEBUG",
    "LD_PROFILE",
    "GTK_MODULES",
    "GTK3_MODULES",
    "GIO_EXTRA_MODULES",
    "QT_PLUGIN_PATH",
    "QML2_IMPORT_PATH",
    "PYTHONINSPECT",
    "GCONV_PATH",
    "HOSTALIASES",
];

/// Runtime Security Audit Report (modeled after `secureblue` and `Qubes OS` checks).
#[derive(Clone, Debug)]
pub struct SecurityAuditReport {
    pub policy_enforced: bool,
    pub hardened_malloc_active: bool,
    pub yama_ptrace_scope: u8,
    pub kptr_restrict: u8,
    pub dmesg_restrict: u8,
    pub no_new_privs: bool,
    pub xwayland_blocked: bool,
    pub thumbnailers_disabled: bool,
    pub extensions_disabled: bool,
    pub shader_sha256_hex: String,
    pub dangerous_env_detected: Vec<String>,
}

impl SecurityAuditReport {
    /// Performs a non-intrusive, 100% memory-safe security audit of the Linux host and DE session.
    pub fn run_audit(policy: &SecurityPolicy, shader_source: &str) -> Self {
        let hardened_malloc_active = Self::detect_hardened_malloc();
        let yama_ptrace_scope =
            Self::read_sysctl_u8("/proc/sys/kernel/yama/ptrace_scope").unwrap_or(0);
        let kptr_restrict = Self::read_sysctl_u8("/proc/sys/kernel/kptr_restrict").unwrap_or(0);
        let dmesg_restrict = Self::read_sysctl_u8("/proc/sys/kernel/dmesg_restrict").unwrap_or(0);
        let no_new_privs = Self::detect_no_new_privs();
        let shader_sha256_hex = sha256_hex(shader_source.as_bytes());

        let mut dangerous_env_detected = Vec::new();
        for &var in DANGEROUS_ENV_VARS {
            if std::env::var_os(var).is_some() {
                dangerous_env_detected.push(var.to_string());
            }
        }

        Self {
            policy_enforced: policy.is_fully_enforced(),
            hardened_malloc_active,
            yama_ptrace_scope,
            kptr_restrict,
            dmesg_restrict,
            no_new_privs,
            xwayland_blocked: policy.disable_xwayland,
            thumbnailers_disabled: policy.disable_thumbnailers,
            extensions_disabled: policy.disable_shell_extensions,
            shader_sha256_hex,
            dangerous_env_detected,
        }
    }

    fn detect_hardened_malloc() -> bool {
        if let Ok(preload) = std::env::var("LD_PRELOAD") {
            if preload.contains("hardened_malloc") {
                return true;
            }
        }
        if let Ok(maps) = fs::read_to_string("/proc/self/maps") {
            if maps.contains("hardened_malloc") {
                return true;
            }
        }
        false
    }

    fn detect_no_new_privs() -> bool {
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("NoNewPrivs:") {
                    return rest.trim() == "1";
                }
            }
        }
        false
    }

    fn read_sysctl_u8(path: &str) -> Option<u8> {
        fs::read_to_string(path).ok()?.trim().parse::<u8>().ok()
    }

    /// Compact single-line summary for audit logging and the DE status bar.
    pub fn summary_line(&self) -> String {
        let short_hash = &self.shader_sha256_hex[..12.min(self.shader_sha256_hex.len())];
        format!(
            "Policy: {} | Xwayland: {} | Thumbnails: {} | Extensions: {} | ptrace: {} | kptr: {} | dmesg: {} | NoNewPrivs: {} | hardened_malloc: {} | SHA-256: {}…",
            if self.policy_enforced { "HARDENED" } else { "PARTIAL" },
            if self.xwayland_blocked { "OFF" } else { "ON" },
            if self.thumbnailers_disabled { "OFF" } else { "ON" },
            if self.extensions_disabled { "OFF" } else { "ON" },
            self.yama_ptrace_scope,
            self.kptr_restrict,
            self.dmesg_restrict,
            if self.no_new_privs { "1" } else { "0" },
            if self.hardened_malloc_active { "ACTIVE" } else { "HOST-DEFAULT" },
            short_hash
        )
    }
}

// ============================================================================
// PURE RUST SHA-256 IMPLEMENTATION (FIPS 180-4) FOR ASSET ATTESTATION
// Zero external dependencies and 100% free of `unsafe` blocks.
// ============================================================================

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Computes the SHA-256 cryptographic digest (FIPS 180-4) of a byte slice.
pub fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() + 8) % 64 != 0 {
        padded.push(0x00);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word_bytes) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes([word_bytes[0], word_bytes[1], word_bytes[2], word_bytes[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        let bytes = word.to_be_bytes();
        out[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
    out
}

/// Returns the SHA-256 digest formatted as a 64-character lowercase hexadecimal string.
pub fn sha256_hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = sha256_digest(data);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_nist_vectors() {
        // Official FIPS 180-4 test vector for empty string ""
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // Official FIPS 180-4 test vector for "abc"
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn test_maximum_hardened_policy() {
        let policy = SecurityPolicy::maximum_hardened();
        assert!(policy.is_fully_enforced());
        assert!(policy.disable_xwayland);
        assert!(policy.disable_thumbnailers);
        assert!(policy.disable_shell_extensions);
        assert!(policy.enforce_run0_zero_suid);
        assert!(policy.inter_domain_clipboard_lock);
    }

    #[test]
    fn test_qubes_mac_clipboard_isolation() {
        let mut vault = SecureClipboardVault::new();

        // 1. Pasting from DisposableRed into CoreVault must be BLOCKED by MAC policy
        vault.seal_from_window(
            TrustDomain::DisposableRed,
            "Disposable Untrusted App",
            "malicious_payload".to_string(),
        );
        let res = vault.unseal_to_window(TrustDomain::CoreVault);
        assert_eq!(
            res,
            Err(SecurityViolation::CrossDomainClipboardBlocked {
                source: TrustDomain::DisposableRed,
                destination: TrustDomain::CoreVault,
            })
        );
        // Vault must be automatically wiped even after a blocked attempt
        assert_eq!(vault.pending_source_domain(), None);
        assert_eq!(vault.total_transfers_blocked, 1);

        // 2. Legitimate transfer from TrustedWork into IsolatedApp succeeds once (single-use auto-wipe)
        vault.seal_from_window(
            TrustDomain::TrustedWork,
            "Trusted Terminal",
            "verified_token_123".to_string(),
        );
        let ok_res = vault.unseal_to_window(TrustDomain::IsolatedApp);
        assert!(ok_res.is_ok());
        assert_eq!(vault.total_transfers_approved, 1);

        // Second paste without a new Ctrl+Shift+C must fail because the vault was auto-wiped
        assert_eq!(
            vault.unseal_to_window(TrustDomain::IsolatedApp),
            Err(SecurityViolation::ClipboardVaultEmpty)
        );
    }

    #[test]
    fn test_surface_buffer_validation() {
        let valid_buf = vec![0u8; 64 * 64 * 4];
        assert!(validate_surface_buffer(64, 64, &valid_buf).is_ok());

        // Truncated/tampered buffer must be rejected
        assert!(validate_surface_buffer(64, 64, &valid_buf[..100]).is_err());

        // Zero or oversized dimensions (GPU DoS protection) must be rejected
        assert!(validate_surface_buffer(0, 64, &[]).is_err());
        assert!(validate_surface_buffer(8192, 8192, &valid_buf).is_err());
    }

    #[test]
    fn test_shader_cryptographic_integrity_attestation() {
        let shader_src = include_str!("shader.wgsl");
        assert!(verify_shader_source_integrity(shader_src).is_ok());

        let tampered = format!("{}\n// tampered", shader_src);
        assert!(matches!(
            verify_shader_source_integrity(&tampered),
            Err(SecurityViolation::IntegrityCheckFailed { .. })
        ));
    }
}
