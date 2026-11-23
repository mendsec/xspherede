#!/usr/bin/env python3
"""Generate the Spheradian logo SVGs.

The mark is a wireframe sphere seen from slightly above. A curved window sits
on its surface, and a cone runs from the viewer at the center of the sphere to
the window's corners: the solid angle (in steradians) that the window takes up.
Two smaller windows carry the other trust-domain colors.

Run: python3 docs/brand/generate_logo.py
"""

from math import cos, radians, sin
from pathlib import Path

OUT = Path(__file__).resolve().parent

# Palette taken from the renderer (src/security.rs, src/sphere_env.rs).
BG = "#04140b"
GRID = "#2eff85"  # TrustedWork emerald, also the sphere wireframe
VIOLET = "#c252ff"  # CoreVault
CYAN = "#24c7ff"  # IsolatedApp
CENTER = "#eafff2"  # the viewer
EMERALD = "#2eff85"  # TrustedWork

SIZE = 512
C = SIZE / 2
R = 176
TILT = radians(-18)  # camera above the equator, looking slightly down
YAW = radians(0)


def project(lat_deg, lon_deg, r=R):
    """Orthographic projection of a sphere point. Returns (x, y, depth)."""
    lat, lon = radians(lat_deg), radians(lon_deg + 0) + YAW
    x = r * cos(lat) * sin(lon)
    y = r * sin(lat)
    z = r * cos(lat) * cos(lon)
    # Rotate around the X axis (tilt).
    y2 = y * cos(TILT) - z * sin(TILT)
    z2 = y * sin(TILT) + z * cos(TILT)
    return C + x, C - y2, z2


def polyline(points):
    return " ".join(f"{x:.1f},{y:.1f}" for x, y, _ in points)


def split_by_visibility(points):
    """Split a sampled curve into front (z >= 0) and back runs."""
    runs, cur, vis = [], [], None
    for p in points:
        v = p[2] >= 0
        if vis is None or v == vis:
            cur.append(p)
        else:
            cur.append(p)
            runs.append((vis, cur))
            cur = [p]
        vis = v
    if cur:
        runs.append((vis, cur))
    return runs


def wire(points, width, front_op, back_op, color=GRID):
    out = []
    for vis, run in split_by_visibility(points):
        if len(run) < 2:
            continue
        op = front_op if vis else back_op
        if op <= 0:
            continue
        out.append(
            f'<polyline points="{polyline(run)}" fill="none" stroke="{color}" '
            f'stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round" '
            f'opacity="{op}"/>'
        )
    return out


def window_patch(lat0, lat1, lon0, lon1, steps=12):
    """Curved quad on the sphere bounded by two parallels and two meridians."""
    pts = [project(lat1, lon0 + (lon1 - lon0) * i / steps) for i in range(steps + 1)]
    pts += [project(lat1 + (lat0 - lat1) * i / steps, lon1) for i in range(1, steps + 1)]
    pts += [project(lat0, lon1 + (lon0 - lon1) * i / steps) for i in range(1, steps + 1)]
    pts += [project(lat0 + (lat1 - lat0) * i / steps, lon0) for i in range(1, steps)]
    return pts


def mark(background=True, detail=True):
    global GRID, EMERALD, CENTER
    s = []
    if background:
        s.append(f'<rect width="{SIZE}" height="{SIZE}" fill="{BG}"/>')
        s.append(
            '<radialGradient id="glow" cx="50%" cy="50%" r="50%">'
            f'<stop offset="0" stop-color="{GRID}" stop-opacity="0.16"/>'
            f'<stop offset="1" stop-color="{GRID}" stop-opacity="0"/></radialGradient>'
            f'<circle cx="{C}" cy="{C}" r="{R + 40}" fill="url(#glow)"/>'
        )

    # Wireframe: parallels and meridians, back half faint.
    if detail:
        for lat in (-50, -25, 25, 50):
            s += wire([project(lat, l) for l in range(0, 361, 4)], 3, 0.32, 0.10)
        for lon in (30, 90, 150):
            s += wire([project(a, lon) for a in range(-90, 91, 3)] +
                      [project(a, lon + 180) for a in range(90, -91, -3)], 3, 0.32, 0.10)
    else:
        s += wire([project(a, 90) for a in range(-90, 91, 3)] +
                  [project(a, 270) for a in range(90, -91, -3)], 5, 0.45, 0.0)
    # Equator, highlighted as in the renderer.
    s += wire([project(0, l) for l in range(0, 361, 3)], 6 if detail else 9, 0.85, 0.22)

    # Secondary windows: violet (CoreVault) and cyan (IsolatedApp).
    for (la0, la1, lo0, lo1, col) in ((-54, -28, -62, -34, VIOLET), (-54, -28, -28, 2, CYAN)):
        pts = window_patch(la0, la1, lo0, lo1)
        s.append(f'<polygon points="{polyline(pts)}" fill="{col}" fill-opacity="0.22" '
                 f'stroke="{col}" stroke-width="{7 if detail else 10}" stroke-linejoin="round"/>')

    # Primary window and its solid-angle cone back to the viewer at the center.
    lat0, lat1, lon0, lon1 = 10, 42, 8, 50
    pts = window_patch(lat0, lat1, lon0, lon1)
    corners = [project(lat1, lon0), project(lat1, lon1), project(lat0, lon1), project(lat0, lon0)]
    s.append(
        f'<polygon points="{C},{C} {polyline([corners[3], corners[0], corners[1]])}" '
        f'fill="{EMERALD}" fill-opacity="0.10"/>'
    )
    for x, y, _ in corners:
        s.append(f'<line x1="{C}" y1="{C}" x2="{x:.1f}" y2="{y:.1f}" stroke="{EMERALD}" '
                 f'stroke-width="{3 if detail else 5}" stroke-dasharray="{"7 7" if detail else "none"}" '
                 f'opacity="0.75"/>')
    s.append(f'<polygon points="{polyline(pts)}" fill="{EMERALD}" fill-opacity="0.28" '
             f'stroke="{EMERALD}" stroke-width="{8 if detail else 12}" stroke-linejoin="round"/>')

    # Outline of the sphere and the viewer at its center.
    s.append(f'<circle cx="{C}" cy="{C}" r="{R}" fill="none" stroke="{GRID}" '
             f'stroke-width="{10 if detail else 14}"/>')
    s.append(f'<circle cx="{C}" cy="{C}" r="{12 if detail else 16}" fill="{CENTER}"/>')
    s.append(f'<circle cx="{C}" cy="{C}" r="{22 if detail else 28}" fill="none" '
             f'stroke="{EMERALD}" stroke-width="3" opacity="0.6"/>')
    return s


def write(name, body):
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" '
           f'width="{SIZE}" height="{SIZE}">\n<title>Spheradian</title>\n'
           + "\n".join(body) + "\n</svg>\n")
    (OUT / name).write_text(svg)


if __name__ == "__main__":
    write("spheradian-logo.svg", mark(background=True, detail=True))
    write("spheradian-logo-transparent.svg", mark(background=False, detail=True))
    write("spheradian-logo-small.svg", mark(background=True, detail=False))
    # Variant for light backgrounds: deeper green so the wireframe keeps contrast.
    GRID = EMERALD = CENTER = "#0a9e4c"
    write("spheradian-logo-light.svg", mark(background=False, detail=True))
