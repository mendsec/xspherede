# XSphereDE 🌐🟢

**XSphereDE** é um ambiente de trabalho tridimensional esférico (3D Desktop Environment) escrito em **Rust** com renderização acelerada por GPU via **wgpu** e **winit**, inspirado no clássico **SphereXP** e na estética volumétrica de **Matrix**.

Em vez de limitar as janelas a um plano 2D tradicional, o XSphereDE posiciona o usuário no centro de uma esfera 3D de 360°, onde as janelas acompanham a curvatura esférica, possuem profundidade radial ajustável $(\theta, \phi, r)$ e são emolduradas por corredores tridimensionais de códigos Matrix pelos quais é possível voar espacialmente.

---

## ✨ Funcionalidades Atuais (Protótipo v0.1)

- **Espaço de Trabalho Esférico 360° (`SphereXP Grid`):**
  - Grade geodésica 3D com meridianos, paralelos, linha do equador destacada e anéis polares superior e inferior.
  - Câmera espacial suave (*damping*) com rotação orbital em 360° e translação 3D livre pelo interior da esfera.
- **Chuva Volumétrica 3D de Código Matrix (`Matrix Code Volume`):**
  - Centenas de colunas/fileiras 3D de código Matrix geradas proceduralmente via shader WGSL (glifos estilo Katakana/binário em matriz `5x7`, cabeça brilhante branco-esverdeada e rastro fosforescente).
  - Ao avançar pelo espaço 3D (`Scroll` ou `WASD`), a câmera atravessa fisicamente as fileiras de código como no filme *Matrix*.
- **Campo de Visão Limpa Mínimo & Escudo Angular por Janela:**
  - As fileiras de código Matrix respeitam um raio mínimo de trabalho limpo e um **cone frontal de leitura** dinâmico.
  - Cada janela aberta projeta um **escudo angular 3D** até o observador, garantindo que códigos Matrix nunca passem pela frente de uma janela em uso, independentemente da profundidade em que ela esteja.
- **Janelas 3D Curvas com Profundidade Radial (`SphericalWindow`):**
  - Malhas subdivididas (`16x12`) que acompanham a curvatura exata da esfera no raio $r$ de cada janela.
  - *Raycasting* esférico exato para foco, *hover*, arraste em coordenadas $(\theta, \phi)$ e ajuste de profundidade radial $r$ (`4.0m` a `24.5m`).
  - Arquitetura desacoplada via trait `WindowSurfaceProvider`, pronta para integração com captura de janelas reais (PipeWire, XComposite ou compositor Wayland via Smithay/DMA-BUF).
- **Mini-Globo 3D Interativo e Expansível (`MiniGlobeHud`):**
  - Visão externa 3D da esfera no canto inferior direito mostrando todas as janelas abertas nas suas **profundidades radiais reais** dentro do globo.
  - Permite girar o globo livremente, alterar a profundidade das janelas com o `Scroll`, **expandir o globo para o centro da tela** (modo *Overview* do SphereXP) e clicar em qualquer janela ou coordenada para a câmera principal viajar suavemente até lá.

---

## 🎮 Controles e Atalhos

| Ação | Controle / Atalho |
| :--- | :--- |
| **Girar a visão 360° na esfera** | Arrastar com **Botão Direito** (ou **Botão Esquerdo** no fundo da esfera) |
| **Alternar modo *Free-Look* contínuo** | `Espaço` ou `Tab` |
| **Voar pelo espaço 3D (atravessar códigos Matrix)** | **Roda do Mouse (`Scroll`)** no espaço vazio ou `W`, `A`, `S`, `D` (`Q`/`E` sobe/desce, `Shift` = turbo) |
| **Mover uma janela na superfície da esfera** | Clicar e arrastar a janela com o **Botão Esquerdo** |
| **Alterar a profundidade 3D de uma janela ($r$)** | Apontar para a janela (na visão principal ou no Mini-Globo) e usar **`Scroll`** (ou `Shift + Scroll`) |
| **Girar o Mini-Globo** | Arrastar com o mouse ou usar **`Scroll`** sobre o espaço vazio do Mini-Globo |
| **Ir para um ponto/janela pelo Mini-Globo** | **Clique Esquerdo** no ponto ou na janela dentro do Mini-Globo |
| **Expandir / Recolher o Mini-Globo na tela** | **Clique Direito** (ou **Botão do Meio**) sobre o Mini-Globo, ou tecla **`G`** |
| **Recentralizar no núcleo da esfera `(0, 0, 0)`** | `R` ou `Home` |
| **Alternar velocidade dos códigos Matrix** | `M` |
| **Alternar Tela Cheia (*Fullscreen*)** | `F11` ou `F` |
| **Recolher Mini-Globo / Sair** | `Esc` |

---

## 🛠️ Arquitetura do Projeto

```text
xspherede/
├── Cargo.toml          # Dependências (wgpu, winit, glam, bytemuck, pollster)
└── src/
    ├── main.rs         # Inicialização wgpu/winit, loop de eventos e pipelines de renderização
    ├── camera.rs       # Câmera 3D espacial (SphereCamera), interpolação suave e raycasting NDC
    ├── minimap.rs      # Mini-Globo 3D interativo e expansível (MiniGlobeHud)
    ├── sphere_env.rs   # Geração da grade geodésica SphereXP e do volume 3D de códigos Matrix
    ├── window3d.rs     # Janelas 3D curvas (SphericalWindow), raycasting esférico e trait WindowSurfaceProvider
    └── shader.wgsl     # Shaders WGSL (grade esférica, glifos Matrix volumétricos + campo limpo, janelas 3D)
```

---

## 🚀 Como Compilar e Executar

### Pré-requisitos
- **Rust** (`rustup`, `cargo`, `rustc` 1.80+)
- Drivers gráficos com suporte a **Vulkan** ou **OpenGL / GLES** no Linux (Wayland ou X11), Windows ou macOS.

### Executando em modo otimizado (`release`)

```bash
git clone https://github.com/mendsec/xspherede.git
cd xspherede
cargo run --release
```

---

## 🗺️ Roadmap

- [x] Esfera 3D wireframe estilo SphereXP com navegação 360° e voo espacial.
- [x] Shader volumétrico 3D de código Matrix com Campo de Visão Limpa Mínimo e escudo por janela.
- [x] Janelas 3D curvas com *raycasting* esférico, arraste em $(\theta, \phi)$ e ajuste de profundidade radial ($r$).
- [x] Mini-Globo 3D interativo e expansível no centro da tela com janelas em profundidade real.
- [ ] Captura de janelas reais no Linux via PipeWire / XComposite implementando `WindowSurfaceProvider`.
- [ ] Ícones de atalho 3D flutuantes e *dock* polar nos anéis superior/inferior da esfera.
- [ ] Integração com **Smithay** para execução como sessão de compositor Wayland nativa com texturas zero-copy (`DMA-BUF`).

---

## 📄 Licença

Distribuído sob a licença **MIT**. Veja o arquivo [LICENSE](LICENSE) para mais detalhes.
