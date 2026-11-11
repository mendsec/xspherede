# XSphereDE 🌐🟢

**XSphereDE** is a 3D Spherical Desktop Environment written in **Rust** with GPU-accelerated rendering powered by **wgpu** and **winit**, inspired by the classic **SphereXP** desktop and the volumetric digital rain aesthetics of **The Matrix**.

Instead of confining windows to a traditional flat 2D plane, XSphereDE places the user at the center of a 360° 3D sphere where windows conform to the spherical curvature, feature adjustable radial depth $(\theta, \phi, r)$, and are surrounded by three-dimensional corridors of falling Matrix code that you can physically fly through.

---

## ✨ Current Features (Prototype v0.0.1)

- **360° Spherical Workspace (`SphereXP Grid`):**
  - 3D geodesic wireframe grid with meridians, parallels, a highlighted equator line, and upper/lower polar rings.
  - Smooth damped spatial camera with 360° orbital rotation and free 3D translation inside the sphere.
- **3D Volumetric Matrix Code Rain (`Matrix Code Volume`):**
  - Hundreds of 3D Matrix code ribbons procedurally generated in a WGSL shader (`5x7` stylized half-width Katakana/binary glyph grid, bright greenish-white leading drop head, and phosphorescent trail).
  - Moving through 3D space (`Scroll` or `WASD`) lets the camera physically fly between the code columns.
- **Minimum Clear Field of View & Per-Window Angular Shield:**
  - Matrix code columns respect a minimum clear workspace radius and a dynamic **frontal reading cone**.
  - Every open window projects a **3D angular shield** toward the observer, ensuring Matrix code streams never obscure a window in use, regardless of its radial depth.
- **Curved 3D Windows with Radial Depth (`SphericalWindow`):**
  - Subdivided curved meshes (`16x12`) that match the exact curvature of the sphere at each window's radius $r$.
  - Exact spherical raycasting for focus, hover, dragging across $(\theta, \phi)$ coordinates, and radial depth adjustment $r$ (`4.0m` to `24.5m`).
  - Decoupled architecture via the `WindowSurfaceProvider` trait, ready for real window capture integration (PipeWire, XComposite, or native Wayland compositor surfaces via Smithay/DMA-BUF).
- **Interactive & Expandable 3D Mini-Globe (`MiniGlobeHud`):**
  - External 3D view of the sphere in the bottom-right corner displaying all open windows at their **true radial depths** inside the globe.
  - Supports free globe rotation, window depth adjustment via `Scroll`, **expanding the globe to the center of the screen** (SphereXP *Overview* mode), and clicking any window or spherical coordinate to smoothly fly the main camera there.
- **Hardened Security Architecture (`secureblue` + `Qubes OS` — [`src/security.rs`](src/security.rs) & [`SECURITY.md`](SECURITY.md)):**
  - **Unforgeable 3D Trust Borders (`TrustDomain`):** Modeled after *Qubes OS*, every 3D window is bound to an immutable security domain (`[DOM0 // CORE-VAULT]` Violet, `[TRUSTED // VERIFIED]` Emerald, `[ISOLATED // SANDBOX]` Cyan, `[DISP-RED // QUARANTINE]` Crimson Red) whose outer frame is forcefully rendered by the compositor GPU shader (`fs_window`), preventing visual window spoofing even by compromised client surfaces.
  - **Inter-Domain Clipboard Vault (`SecureClipboardVault`):** Isolated copy/paste via the compositor's trusted path (`Ctrl+Shift+C` / `Ctrl+Shift+V`) with Mandatory Access Control (blocking untrusted domains from pasting into `CoreVault`) and immediate *single-use auto-wipe*.
  - **`secureblue` Policies Enforced by Default:** `Xwayland` disabled, automatic media thumbnailers prohibited in the *Spatial File Navigator* (`No-Thumbnails`), third-party shell extensions disabled, *Zero-SUID* (`run0`) enforcement, `#![forbid(unsafe_code)]`, `overflow-checks = true` in `release`, **SHA-256** cryptographic attestation of [`src/shader.wgsl`](src/shader.wgsl) before GPU compilation, and **Instant Sphere Lockdown Mode (`L`)**.

---

## 🎮 Controls & Shortcuts

| Action | Control / Shortcut |
| :--- | :--- |
| **Rotate 360° view inside the sphere** | Drag with **Right Mouse Button** (or **Left Mouse Button** on the background) |
| **Toggle continuous *Free-Look* mode** | `Space` or `Tab` |
| **Fly through 3D space (pass through Matrix code)** | **Mouse Wheel (`Scroll`)** on empty space or `W`, `A`, `S`, `D` (`Q`/`E` down/up, `Shift` = boost) |
| **Move a window across the sphere surface** | Click and drag the window with **Left Mouse Button** |
| **Adjust a window's 3D radial depth ($r$)** | Hover over the window (in the main view or inside the Mini-Globe) and use **`Scroll`** (or `Shift + Scroll`) |
| **Rotate the Mini-Globe** | Drag with the mouse or use **`Scroll`** over empty space inside the Mini-Globe |
| **Jump to a point or window via the Mini-Globe** | **Left Click** on the point or window inside the Mini-Globe |
| **Expand / Collapse the Mini-Globe on screen** | **Right Click** (or **Middle Click**) on the Mini-Globe, or press **`G`** |
| **Seal focused window content into Clipboard Vault (Qubes)** | `Ctrl + Shift + C` |
| **Paste from Clipboard Vault with MAC check + Auto-Wipe** | `Ctrl + Shift + V` |
| **Toggle Instant Sphere Lockdown Mode (Blank/Encrypt Windows)** | `L` |
| **Run Live Kernel & Environment Security Audit** | `F12` |
| **Recenter at the sphere core `(0, 0, 0)`** | `R` or `Home` |
| **Toggle Matrix code rain speed** | `M` |
| **Toggle Fullscreen** | `F11` or `F` |
| **Collapse Mini-Globe / Exit** | `Esc` |

---

## 🛠️ Project Architecture

```text
xspherede/
├── Cargo.toml          # Hardened manifest (unsafe_code = "forbid", overflow-checks, LTO, panic = "abort")
├── SECURITY.md         # Comprehensive security architecture & policies (secureblue, Qubes OS, GrapheneOS)
└── src/
    ├── main.rs         # wgpu/winit initialization, event loop, Lockdown mode, and render pipelines
    ├── camera.rs       # 3D spatial camera (SphereCamera), smooth damping, and NDC raycasting
    ├── minimap.rs      # Interactive and expandable 3D Mini-Globe HUD (MiniGlobeHud)
    ├── security.rs     # Security engine: TrustDomain, SecureClipboardVault, SHA-256, kernel/env audit
    ├── sphere_env.rs   # SphereXP geodesic grid and 3D Matrix code volume generation
    ├── window3d.rs     # Curved 3D windows (SphericalWindow), spherical raycasting, and WindowSurfaceProvider trait
    └── shader.wgsl     # WGSL shaders (spherical grid, volumetric Matrix glyphs, unforgeable 3D domain borders)
```

---

## 🚀 Building & Running

### Prerequisites
- **Rust** (`rustup`, `cargo`, `rustc` 1.80+)
- Graphics drivers with **Vulkan**, **Metal**, **DX12**, or **OpenGL / GLES** support on Linux (Wayland or X11), Windows, or macOS.

### Run in optimized release mode

```bash
git clone https://github.com/mendsec/xspherede.git
cd xspherede
cargo run --release
```

---

## 🗺️ Roadmap

- [x] SphereXP-style 3D wireframe sphere with 360° navigation and spatial flight.
- [x] 3D volumetric Matrix code shader with Minimum Clear Field of View and per-window angular shield.
- [x] Curved 3D windows with spherical raycasting, $(\theta, \phi)$ dragging, and radial depth ($r$) adjustment.
- [x] Interactive 3D Mini-Globe expandable to the center of the screen with windows rendered at true radial depth.
- [ ] Live Linux window capture via PipeWire / XComposite implementing `WindowSurfaceProvider`.
- [ ] Floating 3D desktop shortcut icons and polar docks on the upper/lower sphere rings.
- [ ] **Smithay** integration to run as a native Wayland compositor session with zero-copy textures (`DMA-BUF`).

---

## 🛡️ Security

Please do **not** open public GitHub issues for security vulnerabilities. Refer to [SECURITY.md](SECURITY.md) for private reporting instructions.

---

## 📄 License

Distributed under the **MIT** License. See [LICENSE](LICENSE) for details.
