// The Jess hero: a dashboard hula girl whose base is a reactor, rendered live with three.js behind
// the landing page's header.
//
// The scene is a truck's dashboard at night. A full-screen shader is the windshield: a blue-black
// sky over a faint horizon, rain running down the glass, and the lights of oncoming traffic
// sliding out to the edges and past. On the vinyl stands the ornament. Its base is a containment
// vessel in miniature: a plinth, a steel vessel with a glass band, fuel rods inside it glowing
// Cherenkov blue-violet under a pool whose surface moves, a trefoil on the front plate, a gauge
// whose needle jumps on bumps, and two cooling towers behind it breathing vapor. On the pool's
// surface stands the hula girl: lei, flower, raised arms, and a grass skirt of glowing strands
// that trail her hips. She is lit from below by the core, which is what makes her radioactive.
//
// She sways on springs, as the dashboard kind does: the road sends a bump every few seconds, the
// pointer crossing the hero nudges her, and a press on the canvas is a pothole, which also flares
// the core and kicks the gauge. Hips swing one way, torso leans the other, the head follows late,
// the skirt follows later. Nothing is keyframed; the springs are integrated every frame.
//
// Rendering follows the other DreamWeave heroes: a half-float scene target with multisampling,
// a bright pass and two blur levels for the bloom, and a composite that tone-maps (ACES), darkens
// away from the ornament so the words stay calm, and dithers. Colours come from the site's CSS
// tokens, so sass/brand.sass stays their owner. The canvas is inert until the hero is on screen,
// stops in a hidden tab, lowers its resolution when frames run slow, and under
// prefers-reduced-motion draws one frame and stops. Until the first frame, and without WebGL 2 or
// after the context is lost, the still that sass/brand.sass draws in the stage stays.
//
// The template gives the hero a [data-jess-hero-stage] beside the text; the ornament is laid
// out to land inside it, so the text column never sits over the model.

import * as THREE from './vendor/three.module.min.js';

function cssColor(name, fallback) {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const color = new THREE.Color(fallback);
  if (raw) {
    try { color.set(raw); } catch { /* an unparsable token keeps the fallback */ }
  }
  return color;
}

const TAU = Math.PI * 2;
const ARM_UPPER = 0.2;
const ARM_FORE = 0.19;

// Shaders ------------------------------------------------------------------------------------

const FULLSCREEN_VERTEX = /* glsl */ `
  varying vec2 vUv;
  void main() {
    vUv = uv;
    gl_Position = vec4(position.xy, 0.0, 1.0);
  }
`;

const NOISE = /* glsl */ `
  float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
  }
  float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + vec2(1.0, 0.0));
    float c = hash21(i + vec2(0.0, 1.0));
    float d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
  }
`;

// The windshield. Lights come from a vanishing point a little above the dashboard and run out
// to the sides, growing; rain runs down in thin threads that catch them.
const SKY_FRAGMENT = /* glsl */ `
  uniform vec3 uTop;
  uniform vec3 uBottom;
  uniform vec3 uAccent;
  uniform vec3 uCherenkov;
  uniform vec2 uResolution;
  uniform vec2 uDrift;
  uniform vec2 uCore;
  uniform float uTime;
  uniform float uGlow;
  varying vec2 vUv;
  ${NOISE}
  void main() {
    float aspect = uResolution.x / max(uResolution.y, 1.0);
    vec2 uv = vUv + uDrift;
    vec2 p = vec2((uv.x - 0.5) * aspect, uv.y - 0.5);

    // Sky: darker overhead, a cold band at the horizon, bruised violet low on the left.
    float horizon = 0.46;
    vec3 color = mix(uBottom, uTop, smoothstep(horizon - 0.3, horizon + 0.6, uv.y));
    color += uAccent * 0.025 * exp(-abs(uv.y - horizon) * 9.0);
    color += uAccent * 0.018 * exp(-length(p - vec2(-0.55 * aspect, -0.1)) * 2.2);

    // Oncoming traffic: pairs of lights that leave the vanishing point and pass by.
    vec2 vanish = vec2(0.08 * aspect, horizon - 0.5 + 0.03);
    for (int i = 0; i < 7; i++) {
      float fi = float(i);
      float speed = 0.055 + 0.03 * hash21(vec2(fi, 3.0));
      float phase = fract(uTime * speed + hash21(vec2(fi, 7.0)));
      float side = hash21(vec2(fi, 11.0)) > 0.5 ? 1.0 : -1.0;
      float lane = 0.25 + 0.5 * hash21(vec2(fi, 13.0));
      // Perspective: distance shrinks as the light approaches, position grows as 1 / distance.
      float depth = 1.0 / (0.08 + phase * phase * 1.6);
      vec2 at = vanish + vec2(side * lane * 0.9, -0.22) * (1.0 / depth) * 2.4;
      float size = 0.004 + 0.05 / depth;
      float warm = hash21(vec2(fi, 17.0));
      vec3 lamp = mix(vec3(0.95, 0.9, 1.0), uAccent * 1.6, warm * 0.6);
      float fade = smoothstep(0.0, 0.18, phase) * (1.0 - smoothstep(0.82, 1.0, phase));
      float d = length(p - at);
      float beam = exp(-d * d / (size * size * 2.0)) * 0.55 + exp(-d / (size * 3.0)) * 0.12;
      color += lamp * beam * fade * (0.45 + 0.55 * (1.0 - phase));
    }

    // Rain: threads pulled down the glass, brighter where the light behind them is.
    vec2 rp = vec2(uv.x * 160.0, uv.y * 6.0 + uTime * 1.6);
    float thread = vnoise(rp) * vnoise(rp * 2.3 + 11.0);
    float rain = smoothstep(0.6, 0.85, thread) * 0.022;
    color += rain * (0.5 + 1.5 * smoothstep(0.1, 0.0, length(color)));

    // The reactor's light on the glass and the dashboard below it.
    vec2 c = vec2((uCore.x - 0.5) * aspect, uCore.y - 0.5);
    float core = exp(-length(p - c) * 5.5);
    color += uCherenkov * core * 0.14 * uGlow;

    // Dashboard vinyl along the foot of the view, below the horizon of the dash itself.
    float dash = 1.0 - smoothstep(0.0, 0.22, uv.y);
    color = mix(color, uBottom * 0.35 + uCherenkov * core * 0.05 * uGlow, dash * 0.85);

    float grain = (hash21(gl_FragCoord.xy + fract(uTime) * 11.0) - 0.5) * 0.015;
    gl_FragColor = vec4(color + grain, 1.0);
  }
`;

const BRIGHT_FRAGMENT = /* glsl */ `
  uniform sampler2D tInput;
  uniform float uThreshold;
  varying vec2 vUv;
  void main() {
    vec3 color = texture2D(tInput, vUv).rgb;
    color = max(color - vec3(0.0), 0.0);
    color = min(color, vec3(8.0));
    float luma = dot(color, vec3(0.2126, 0.7152, 0.0722));
    float keep = smoothstep(uThreshold, uThreshold + 0.5, luma);
    gl_FragColor = vec4(color * keep, 1.0);
  }
`;

const BLUR_FRAGMENT = /* glsl */ `
  uniform sampler2D tInput;
  uniform vec2 uDirection;
  varying vec2 vUv;
  void main() {
    vec3 sum = texture2D(tInput, vUv).rgb * 0.2270270270;
    sum += texture2D(tInput, vUv + uDirection * 1.3846153846).rgb * 0.3162162162;
    sum += texture2D(tInput, vUv - uDirection * 1.3846153846).rgb * 0.3162162162;
    sum += texture2D(tInput, vUv + uDirection * 3.2307692308).rgb * 0.0702702703;
    sum += texture2D(tInput, vUv - uDirection * 3.2307692308).rgb * 0.0702702703;
    gl_FragColor = vec4(sum, 1.0);
  }
`;

const COPY_FRAGMENT = /* glsl */ `
  uniform sampler2D tInput;
  varying vec2 vUv;
  void main() { gl_FragColor = vec4(texture2D(tInput, vUv).rgb, 1.0); }
`;

const COMPOSITE_FRAGMENT = /* glsl */ `
  uniform sampler2D tScene;
  uniform sampler2D tBloomNear;
  uniform sampler2D tBloomFar;
  uniform vec2 uCenter;
  uniform vec2 uHalf;
  uniform float uScrim;
  uniform float uTime;
  varying vec2 vUv;
  vec3 aces(vec3 x) {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
  }
  float dither(vec2 p) {
    return fract(sin(dot(p + fract(uTime), vec2(12.9898, 78.233))) * 43758.5453) - 0.5;
  }
  vec3 scrub(vec3 c) {
    c = max(c, vec3(0.0));
    return min(c, vec3(16.0));
  }
  void main() {
    vec3 color = scrub(texture2D(tScene, vUv).rgb);
    color += scrub(texture2D(tBloomNear, vUv).rgb) * 0.45 + scrub(texture2D(tBloomFar, vUv).rgb) * 0.35;
    // Keep the words calm: darken what lies well away from the ornament.
    vec2 p = (vUv - uCenter) / max(uHalf, vec2(1e-3));
    float away = smoothstep(1.0, 2.1, length(p * vec2(0.9, 1.0)));
    color *= 1.0 - away * uScrim;
    color = aces(color * 0.92);
    color = pow(color, vec3(1.0 / 2.2));
    color += dither(gl_FragCoord.xy) / 255.0;
    gl_FragColor = vec4(color, 1.0);
  }
`;

// The pool's surface: water over the rods, its caustics drifting, brighter at the middle.
const POOL_FRAGMENT = /* glsl */ `
  uniform vec3 uColor;
  uniform float uTime;
  uniform float uGlow;
  varying vec2 vUv;
  ${NOISE}
  void main() {
    vec2 p = (vUv - 0.5) * 2.0;
    float r = length(p);
    if (r > 1.0) discard;
    vec2 q = p * 4.0;
    float a = vnoise(q + vec2(uTime * 0.35, -uTime * 0.2));
    float b = vnoise(q * 1.9 - vec2(uTime * 0.27, uTime * 0.31) + 5.0);
    float caustic = pow(max(0.0, 1.0 - abs(a - b) * 3.2), 3.0);
    float body = (0.35 + 0.65 * (1.0 - r * r)) * uGlow;
    vec3 color = uColor * (body * 0.9 + caustic * 1.4 * uGlow);
    float alpha = 0.72 - 0.25 * r;
    gl_FragColor = vec4(color, alpha);
  }
`;

// A skirt strand: hangs from the hip, bends with the sway, flutters a little on its own, and
// fades toward its tip. Instanced; each strand's matrix places its root on the hip ring.
const STRAND_VERTEX = /* glsl */ `
  uniform vec2 uSway;
  uniform float uTime;
  attribute float aSeed;
  varying float vAlong;
  varying float vSeed;
  void main() {
    vAlong = -position.y;
    vSeed = aSeed;
    float t = clamp(-position.y / 0.46, 0.0, 1.0);
    vec3 p = position;
    // The root follows the hip; the tip lags and overshoots, so the bend grows down the strand.
    float bend = t * t;
    p.x += uSway.x * bend * 0.38 + sin(uTime * 2.3 + aSeed * 9.0) * 0.012 * t;
    p.z += uSway.y * bend * 0.3 + cos(uTime * 1.9 + aSeed * 7.0) * 0.012 * t;
    // Hanging: the farther the sway, the more the strand lifts, like a skirt in a spin.
    p.y += length(uSway) * bend * 0.08;
    vec4 world = instanceMatrix * vec4(p, 1.0);
    gl_Position = projectionMatrix * modelViewMatrix * world;
  }
`;

const STRAND_FRAGMENT = /* glsl */ `
  uniform vec3 uColor;
  uniform vec3 uTip;
  uniform float uGlow;
  varying float vAlong;
  varying float vSeed;
  void main() {
    float t = clamp(vAlong / 0.46, 0.0, 1.0);
    vec3 color = mix(uColor, uTip, t) * (0.55 + 0.45 * vSeed);
    float alpha = (1.0 - t * 0.85) * 0.95;
    gl_FragColor = vec4(color * uGlow, alpha);
  }
`;

// Vapor from the towers: sprites that rise, spread, and thin out.
const VAPOR_VERTEX = /* glsl */ `
  uniform float uTime;
  uniform float uPixelRatio;
  attribute float aSeed;
  attribute vec3 aOrigin;
  varying float vLife;
  void main() {
    float speed = 0.09 + 0.05 * fract(aSeed * 7.31);
    float life = fract(uTime * speed + aSeed);
    vLife = life;
    float spread = life * (0.08 + 0.12 * fract(aSeed * 3.17));
    vec3 p = aOrigin;
    p.y += life * 0.55;
    p.x += sin(aSeed * 31.0 + uTime * 0.4) * spread + 0.03 * life;
    p.z += cos(aSeed * 17.0 + uTime * 0.3) * spread;
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    gl_Position = projectionMatrix * mv;
    gl_PointSize = (12.0 + 34.0 * life) * uPixelRatio * (2.4 / max(-mv.z, 0.1));
  }
`;

const VAPOR_FRAGMENT = /* glsl */ `
  uniform vec3 uColor;
  varying float vLife;
  void main() {
    float d = length(gl_PointCoord - 0.5) * 2.0;
    float soft = smoothstep(1.0, 0.1, d);
    float alpha = soft * (1.0 - vLife) * smoothstep(0.0, 0.12, vLife) * 0.11;
    gl_FragColor = vec4(uColor, alpha);
  }
`;

// Textures ---------------------------------------------------------------------------------

function canvasTexture(size, draw) {
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext('2d');
  draw(context, size);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = 4;
  return texture;
}

// The trefoil, to the usual proportions: a disc of radius r and three blades from 1.5r to 5r,
// each 60 degrees wide, with the first pointing down.
function trefoilTexture(accent, plate) {
  return canvasTexture(256, (context, size) => {
    context.fillStyle = plate;
    context.fillRect(0, 0, size, size);
    const cx = size / 2;
    const cy = size / 2;
    const r = size / 14;
    context.fillStyle = accent;
    context.beginPath();
    context.arc(cx, cy, r, 0, TAU);
    context.fill();
    for (let i = 0; i < 3; i++) {
      const start = Math.PI / 2 - Math.PI / 6 + (i * TAU) / 3;
      context.beginPath();
      context.arc(cx, cy, r * 5, start, start + Math.PI / 3);
      context.arc(cx, cy, r * 1.5, start + Math.PI / 3, start, true);
      context.closePath();
      context.fill();
    }
    context.fillStyle = accent;
    context.font = `700 ${Math.round(size * 0.075)}px oxanium, ui-monospace, monospace`;
    context.textAlign = 'center';
    context.fillText('JESS', cx, size * 0.93);
  });
}

// A gauge face: an arc of ticks, with the last quarter marked.
function gaugeTexture(accent, plate, ink) {
  return canvasTexture(256, (context, size) => {
    context.fillStyle = plate;
    context.fillRect(0, 0, size, size);
    const cx = size / 2;
    const cy = size * 0.62;
    const radius = size * 0.4;
    const from = Math.PI * 1.15;
    const to = Math.PI * 1.85;
    context.strokeStyle = ink;
    context.lineWidth = 3;
    context.beginPath();
    context.arc(cx, cy, radius, from, to);
    context.stroke();
    context.strokeStyle = accent;
    context.lineWidth = 7;
    context.beginPath();
    context.arc(cx, cy, radius, from + (to - from) * 0.74, to);
    context.stroke();
    for (let i = 0; i <= 12; i++) {
      const angle = from + ((to - from) * i) / 12;
      const long = i % 3 === 0;
      context.strokeStyle = i >= 9 ? accent : ink;
      context.lineWidth = long ? 4 : 2;
      context.beginPath();
      context.moveTo(cx + Math.cos(angle) * (radius - (long ? 22 : 12)), cy + Math.sin(angle) * (radius - (long ? 22 : 12)));
      context.lineTo(cx + Math.cos(angle) * (radius - 4), cy + Math.sin(angle) * (radius - 4));
      context.stroke();
    }
    context.fillStyle = ink;
    context.font = `600 ${Math.round(size * 0.085)}px oxanium, ui-monospace, monospace`;
    context.textAlign = 'center';
    context.fillText('mSv/h', cx, cy + size * 0.2);
  });
}

// Fine vinyl grain for the dashboard, tiled.
function grainTexture() {
  const texture = canvasTexture(128, (context, size) => {
    const image = context.createImageData(size, size);
    for (let i = 0; i < image.data.length; i += 4) {
      const v = 118 + Math.floor(Math.random() * 28);
      image.data[i] = v;
      image.data[i + 1] = v;
      image.data[i + 2] = v;
      image.data[i + 3] = 255;
    }
    context.putImageData(image, 0, 0);
  });
  texture.colorSpace = THREE.NoColorSpace;
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.repeat.set(14, 6);
  return texture;
}

// The ornament ------------------------------------------------------------------------------

function lathe(points, segments = 48) {
  return new THREE.LatheGeometry(points.map(([x, y]) => new THREE.Vector2(x, y)), segments);
}

function buildOrnament(palette) {
  const group = new THREE.Group();
  const parts = {};

  const steel = new THREE.MeshStandardMaterial({ color: new THREE.Color('#9a94a6').lerp(palette.accent, 0.08), metalness: 0.85, roughness: 0.38 });
  const darkSteel = new THREE.MeshStandardMaterial({ color: new THREE.Color('#2b2733'), metalness: 0.7, roughness: 0.55 });
  const concrete = new THREE.MeshStandardMaterial({ color: new THREE.Color('#5f5a69').lerp(palette.accent, 0.06), metalness: 0.05, roughness: 0.92 });
  const chrome = new THREE.MeshStandardMaterial({ color: new THREE.Color('#cfc9d8'), metalness: 1.0, roughness: 0.22 });

  // Plinth: a wide low disc with a chamfer, and a chrome foot ring.
  const plinth = new THREE.Mesh(lathe([[0, 0], [0.92, 0], [0.98, 0.04], [0.98, 0.1], [0.9, 0.15], [0, 0.15]]), concrete);
  group.add(plinth);
  const footRing = new THREE.Mesh(new THREE.TorusGeometry(0.95, 0.018, 10, 72), chrome);
  footRing.rotation.x = Math.PI / 2;
  footRing.position.y = 0.025;
  group.add(footRing);

  // The vessel: a steel can with a glass band, a shoulder, and an open rim over the pool.
  const vesselLower = new THREE.Mesh(lathe([[0, 0.15], [0.56, 0.15], [0.58, 0.2], [0.58, 0.36], [0.55, 0.37], [0, 0.37]]), steel);
  group.add(vesselLower);
  const vesselUpper = new THREE.Mesh(lathe([[0, 0.66], [0.55, 0.66], [0.58, 0.67], [0.58, 0.8], [0.5, 0.9], [0.42, 0.94], [0.36, 0.94], [0.36, 0.9], [0.44, 0.86], [0.5, 0.78], [0.5, 0.7], [0, 0.7]]), steel);
  group.add(vesselUpper);
  const bandRings = new THREE.Group();
  for (const y of [0.385, 0.645]) {
    const ring = new THREE.Mesh(new THREE.TorusGeometry(0.585, 0.014, 8, 72), chrome);
    ring.rotation.x = Math.PI / 2;
    ring.position.y = y;
    bandRings.add(ring);
  }
  group.add(bandRings);
  const glass = new THREE.Mesh(new THREE.CylinderGeometry(0.575, 0.575, 0.26, 64, 1, true), new THREE.MeshPhysicalMaterial({
    color: palette.cherenkov.clone().lerp(new THREE.Color('#ffffff'), 0.4),
    metalness: 0.0,
    roughness: 0.08,
    transparent: true,
    opacity: 0.22,
    side: THREE.DoubleSide,
    depthWrite: false,
  }));
  glass.position.y = 0.515;
  glass.renderOrder = 4;
  group.add(glass);
  const bolts = new THREE.InstancedMesh(new THREE.SphereGeometry(0.014, 8, 6), chrome, 24);
  const bolt = new THREE.Object3D();
  for (let i = 0; i < 24; i++) {
    const angle = (i / 24) * TAU + 0.1;
    bolt.position.set(Math.cos(angle) * 0.585, i % 2 ? 0.2 : 0.78, Math.sin(angle) * 0.585);
    bolt.updateMatrix();
    bolts.setMatrixAt(i, bolt.matrix);
  }
  group.add(bolts);

  // The core: a hexagonal bundle of rods under the water, each one a light.
  const rodMaterial = new THREE.MeshStandardMaterial({
    color: palette.cherenkov.clone().multiplyScalar(0.25),
    emissive: palette.cherenkov,
    emissiveIntensity: 1.2,
    metalness: 0.1,
    roughness: 0.5,
  });
  const rodPlaces = [];
  for (let q = -2; q <= 2; q++) {
    for (let r = -2; r <= 2; r++) {
      if (Math.abs(q + r) > 2) continue;
      rodPlaces.push([(q + r / 2) * 0.13, r * 0.1126]);
    }
  }
  const rods = new THREE.InstancedMesh(new THREE.CylinderGeometry(0.022, 0.022, 0.5, 12), rodMaterial, rodPlaces.length);
  const rod = new THREE.Object3D();
  rodPlaces.forEach(([x, z], i) => {
    rod.position.set(x, 0.42, z);
    rod.updateMatrix();
    rods.setMatrixAt(i, rod.matrix);
  });
  group.add(rods);
  parts.rodMaterial = rodMaterial;
  const grid = new THREE.Mesh(new THREE.CylinderGeometry(0.5, 0.5, 0.02, 48), darkSteel);
  grid.position.y = 0.165;
  group.add(grid);

  // The pool: a disc of moving water at the rim, lit from the rods beneath it.
  const poolUniforms = {
    uColor: { value: palette.cherenkov },
    uTime: { value: 0 },
    uGlow: { value: 1 },
  };
  const pool = new THREE.Mesh(new THREE.CircleGeometry(0.44, 64), new THREE.ShaderMaterial({
    vertexShader: /* glsl */ `varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: POOL_FRAGMENT,
    uniforms: poolUniforms,
    transparent: true,
    depthWrite: false,
  }));
  pool.rotation.x = -Math.PI / 2;
  pool.position.y = 0.935;
  pool.renderOrder = 3;
  group.add(pool);
  parts.poolUniforms = poolUniforms;

  // The light the core throws on everything else.
  const coreLight = new THREE.PointLight(palette.cherenkov, 3.0, 5.0, 2.0);
  coreLight.position.set(0, 1.0, 0);
  group.add(coreLight);
  parts.coreLight = coreLight;

  // Plate with the trefoil on the vessel's front, and a gauge on the plinth.
  const trefoil = new THREE.Mesh(new THREE.PlaneGeometry(0.3, 0.3), new THREE.MeshStandardMaterial({
    map: trefoilTexture(palette.accentCss, '#15121a'),
    emissive: palette.accent,
    emissiveMap: trefoilTexture(palette.accentCss, '#000000'),
    emissiveIntensity: 0.55,
    roughness: 0.6,
    metalness: 0.2,
  }));
  trefoil.position.set(0, 0.26, 0.583);
  group.add(trefoil);

  const gauge = new THREE.Group();
  const gaugeFace = new THREE.Mesh(new THREE.PlaneGeometry(0.22, 0.22), new THREE.MeshStandardMaterial({
    map: gaugeTexture(palette.accentCss, '#100d14', '#9f93ad'),
    emissive: palette.accent,
    emissiveIntensity: 0.12,
    roughness: 0.5,
  }));
  gauge.add(gaugeFace);
  const bezel = new THREE.Mesh(new THREE.TorusGeometry(0.108, 0.012, 8, 48), chrome);
  gauge.add(bezel);
  const needle = new THREE.Mesh(new THREE.BoxGeometry(0.008, 0.09, 0.004), new THREE.MeshStandardMaterial({ color: '#ffffff', emissive: palette.accent, emissiveIntensity: 0.8 }));
  needle.geometry.translate(0, 0.045, 0);
  needle.position.set(0, -0.026, 0.006);
  gauge.add(needle);
  // Flush on the vessel's lower band, a third of the way round to her left, facing outward.
  const gaugeAngle = 0.95;
  gauge.scale.setScalar(0.82);
  gauge.position.set(Math.sin(gaugeAngle) * 0.588, 0.262, Math.cos(gaugeAngle) * 0.588);
  gauge.rotation.set(0, gaugeAngle, 0);
  group.add(gauge);
  parts.needle = needle;

  // Cooling towers behind, with vapor.
  const towerProfile = [];
  for (let i = 0; i <= 16; i++) {
    const t = i / 16;
    const y = t * 0.78;
    const radius = 0.13 + 0.11 * Math.pow(Math.abs(t - 0.75) / 0.75, 1.6) * (t < 0.75 ? 1 : 0.6);
    towerProfile.push([radius, y]);
  }
  towerProfile.unshift([0, 0]);
  const vaporOrigins = [];
  for (const x of [-0.72, 0.72]) {
    const tower = new THREE.Mesh(lathe(towerProfile, 40), concrete);
    tower.position.set(x, 0.15, -0.42);
    group.add(tower);
    const lip = new THREE.Mesh(new THREE.TorusGeometry(0.157, 0.012, 8, 40), darkSteel);
    lip.rotation.x = Math.PI / 2;
    lip.position.set(x, 0.93, -0.42);
    group.add(lip);
    vaporOrigins.push(new THREE.Vector3(x, 0.93, -0.42));
  }
  const vaporCount = 160;
  const vaporGeometry = new THREE.BufferGeometry();
  const vaporPositions = new Float32Array(vaporCount * 3);
  const vaporSeeds = new Float32Array(vaporCount);
  const vaporOriginArray = new Float32Array(vaporCount * 3);
  for (let i = 0; i < vaporCount; i++) {
    const origin = vaporOrigins[i % vaporOrigins.length];
    const angle = Math.random() * TAU;
    const radius = Math.random() * 0.1;
    vaporOriginArray[i * 3] = origin.x + Math.cos(angle) * radius;
    vaporOriginArray[i * 3 + 1] = origin.y;
    vaporOriginArray[i * 3 + 2] = origin.z + Math.sin(angle) * radius;
    vaporSeeds[i] = Math.random();
  }
  vaporGeometry.setAttribute('position', new THREE.BufferAttribute(vaporPositions, 3));
  vaporGeometry.setAttribute('aOrigin', new THREE.BufferAttribute(vaporOriginArray, 3));
  vaporGeometry.setAttribute('aSeed', new THREE.BufferAttribute(vaporSeeds, 1));
  vaporGeometry.boundingSphere = new THREE.Sphere(new THREE.Vector3(0, 1.2, -0.42), 2);
  const vaporUniforms = {
    uTime: { value: 0 },
    uPixelRatio: { value: 1 },
    uColor: { value: palette.accent.clone().lerp(new THREE.Color('#ffffff'), 0.6) },
  };
  const vapor = new THREE.Points(vaporGeometry, new THREE.ShaderMaterial({
    vertexShader: VAPOR_VERTEX,
    fragmentShader: VAPOR_FRAGMENT,
    uniforms: vaporUniforms,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  }));
  vapor.frustumCulled = false;
  vapor.renderOrder = 6;
  group.add(vapor);
  parts.vaporUniforms = vaporUniforms;

  // The girl --------------------------------------------------------------------------------

  const skin = new THREE.MeshStandardMaterial({
    color: new THREE.Color('#e9d6ff').lerp(palette.accent, 0.25),
    emissive: palette.accent,
    emissiveIntensity: 0.28,
    roughness: 0.55,
    metalness: 0.0,
  });
  const hairMaterial = new THREE.MeshStandardMaterial({ color: '#17121d', roughness: 0.65, metalness: 0.1, emissive: palette.accent, emissiveIntensity: 0.05 });
  const petalMaterial = new THREE.MeshStandardMaterial({ color: '#ffffff', emissive: palette.accent.clone().lerp(new THREE.Color('#ffffff'), 0.35), emissiveIntensity: 0.9, roughness: 0.6 });
  const topMaterial = new THREE.MeshStandardMaterial({ color: palette.accent.clone().multiplyScalar(0.55), emissive: palette.accent, emissiveIntensity: 0.35, roughness: 0.7 });

  const girl = new THREE.Group();
  const girlScale = 0.94;
  girl.scale.setScalar(girlScale);
  girl.position.y = 0.935;
  group.add(girl);

  // The spring base of the ornament: a chrome coil sits on the water, and she stands on it.
  const spring = new THREE.Mesh(new THREE.TorusGeometry(0.075, 0.016, 10, 40), chrome);
  spring.rotation.x = Math.PI / 2;
  spring.position.y = 0.012;
  girl.add(spring);
  const coil = new THREE.Mesh(new THREE.CylinderGeometry(0.05, 0.065, 0.06, 20), chrome);
  coil.position.y = 0.04;
  girl.add(coil);

  // Hierarchy: hips pivot on the spring; torso leans against the hips; head follows the torso.
  const hips = new THREE.Group();
  hips.position.y = 0.07;
  girl.add(hips);

  const legs = new THREE.Mesh(lathe([[0, 0], [0.11, 0], [0.14, 0.12], [0.15, 0.3], [0.13, 0.34], [0, 0.34]], 32), skin);
  hips.add(legs);
  const hipBand = new THREE.Mesh(new THREE.TorusGeometry(0.15, 0.028, 10, 40), topMaterial);
  hipBand.rotation.x = Math.PI / 2;
  hipBand.position.y = 0.3;
  hips.add(hipBand);

  // Skirt: strands rooted around the hip band.
  const strandCount = 150;
  const strandGeometry = new THREE.PlaneGeometry(0.014, 0.46, 1, 8);
  strandGeometry.translate(0, -0.23, 0);
  const strandSeeds = new Float32Array(strandCount);
  for (let i = 0; i < strandCount; i++) strandSeeds[i] = Math.random();
  strandGeometry.setAttribute('aSeed', new THREE.InstancedBufferAttribute(strandSeeds, 1));
  const strandUniforms = {
    uSway: { value: new THREE.Vector2() },
    uTime: { value: 0 },
    uGlow: { value: 1 },
    uColor: { value: palette.accent.clone().lerp(new THREE.Color('#ffffff'), 0.45) },
    uTip: { value: palette.cherenkov.clone().multiplyScalar(0.9) },
  };
  const strands = new THREE.InstancedMesh(strandGeometry, new THREE.ShaderMaterial({
    vertexShader: STRAND_VERTEX,
    fragmentShader: STRAND_FRAGMENT,
    uniforms: strandUniforms,
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
  }), strandCount);
  const strand = new THREE.Object3D();
  for (let i = 0; i < strandCount; i++) {
    const angle = (i / strandCount) * TAU + (Math.random() - 0.5) * 0.04;
    const radius = 0.15 + (i % 3) * 0.012;
    strand.position.set(Math.cos(angle) * radius, 0.3 - (i % 2) * 0.01, Math.sin(angle) * radius);
    strand.rotation.set((Math.random() - 0.5) * 0.12, -angle + Math.PI / 2, Math.cos(angle) * 0.14 + (Math.random() - 0.5) * 0.1);
    strand.rotation.x += -Math.sin(angle) * 0.14;
    strand.updateMatrix();
    strands.setMatrixAt(i, strand.matrix);
  }
  strands.frustumCulled = false;
  strands.renderOrder = 5;
  hips.add(strands);
  parts.strandUniforms = strandUniforms;

  const torso = new THREE.Group();
  torso.position.y = 0.31;
  hips.add(torso);
  const waist = new THREE.Mesh(lathe([[0, 0], [0.115, 0], [0.095, 0.1], [0.11, 0.22], [0.13, 0.3], [0.1, 0.36], [0, 0.36]], 32), skin);
  torso.add(waist);
  const shoulders = new THREE.Mesh(new THREE.SphereGeometry(0.16, 24, 16, 0, TAU, 0, Math.PI / 2), skin);
  shoulders.position.y = 0.33;
  shoulders.scale.set(1, 0.45, 0.75);
  torso.add(shoulders);
  const neck = new THREE.Mesh(new THREE.CylinderGeometry(0.04, 0.05, 0.08, 16), skin);
  neck.position.y = 0.4;
  torso.add(neck);

  // Lei: a loop of flowers behind the neck that hangs down the chest, between her and the uke.
  const lei = new THREE.InstancedMesh(new THREE.SphereGeometry(0.027, 10, 8), petalMaterial, 30);
  const flower = new THREE.Object3D();
  for (let i = 0; i < 30; i++) {
    const angle = (i / 30) * TAU;
    const front = 0.5 + 0.5 * Math.sin(angle);
    flower.position.set(Math.cos(angle) * (0.06 + front * 0.06), 0.43 - front * 0.3, -0.05 + front * 0.19);
    flower.scale.setScalar(0.8 + 0.4 * ((i * 7) % 5) / 5);
    flower.updateMatrix();
    lei.setMatrixAt(i, flower.matrix);
  }
  torso.add(lei);

  // The ukulele, held across her front: neck toward her left, a little up, tilted to the light.
  const wood = new THREE.MeshStandardMaterial({ color: '#b98a5e', roughness: 0.45, metalness: 0.05, emissive: palette.accent, emissiveIntensity: 0.05 });
  const darkWood = new THREE.MeshStandardMaterial({ color: '#3a2417', roughness: 0.6 });
  const ukulele = new THREE.Group();
  const lowerBout = new THREE.Mesh(new THREE.SphereGeometry(0.075, 20, 14), wood);
  lowerBout.scale.set(1, 1, 0.32);
  const upperBout = new THREE.Mesh(new THREE.SphereGeometry(0.058, 20, 14), wood);
  upperBout.position.y = 0.1;
  upperBout.scale.set(1, 1, 0.32);
  const soundhole = new THREE.Mesh(new THREE.CircleGeometry(0.022, 20), darkWood);
  soundhole.position.set(0, 0.035, 0.0245);
  const bridge = new THREE.Mesh(new THREE.BoxGeometry(0.05, 0.01, 0.008), darkWood);
  bridge.position.set(0, -0.03, 0.026);
  const fretboard = new THREE.Mesh(new THREE.BoxGeometry(0.03, 0.23, 0.014), darkWood);
  fretboard.position.set(0, 0.24, 0.012);
  const headstock = new THREE.Mesh(new THREE.BoxGeometry(0.042, 0.055, 0.012), darkWood);
  headstock.position.set(0, 0.375, 0.008);
  headstock.rotation.x = -0.25;
  const strings = new THREE.Mesh(new THREE.BoxGeometry(0.022, 0.32, 0.0015), new THREE.MeshStandardMaterial({ color: '#ddd3e6', emissive: palette.accent, emissiveIntensity: 0.08, roughness: 0.4 }));
  strings.position.set(0, 0.15, 0.0255);
  ukulele.add(lowerBout, upperBout, soundhole, bridge, fretboard, headstock, strings);
  ukulele.position.set(0.0, 0.12, 0.15);
  ukulele.rotation.set(0.5, 0.15, -1.1);
  torso.add(ukulele);
  ukulele.updateMatrix();

  // Arms: upper arm and forearm as two bones from a fixed shoulder, solved each frame toward a
  // hand target in torso space, so the hands stay on the instrument whatever the torso does.
  const arms = [];
  for (const side of [-1, 1]) {
    const upper = new THREE.Mesh(new THREE.CapsuleGeometry(0.03, ARM_UPPER, 6, 12), skin);
    upper.geometry.translate(0, ARM_UPPER / 2, 0);
    const fore = new THREE.Mesh(new THREE.CapsuleGeometry(0.027, ARM_FORE, 6, 12), skin);
    fore.geometry.translate(0, ARM_FORE / 2, 0);
    const hand = new THREE.Mesh(new THREE.SphereGeometry(0.036, 12, 10), skin);
    hand.scale.set(0.75, 1, 0.55);
    torso.add(upper, fore, hand);
    arms.push({
      side,
      upper,
      fore,
      hand,
      shoulder: new THREE.Vector3(side * 0.15, 0.33, 0),
      target: new THREE.Vector3(),
      // Elbows out to the sides and a little back, as arms around an instrument sit.
      pole: side > 0 ? new THREE.Vector3(1.0, -0.3, -0.45) : new THREE.Vector3(-1.5, -0.15, -0.1),
    });
  }
  // Hand targets, in torso space: the fretting hand on the neck, the strumming hand over the
  // soundhole. Both are points on the ukulele, carried through its transform.
  const fretAnchor = new THREE.Vector3(0, 0.27, 0.03).applyMatrix4(ukulele.matrix);
  const strumAnchor = new THREE.Vector3(-0.01, -0.03, 0.05).applyMatrix4(ukulele.matrix);
  parts.anchors = { fret: fretAnchor, strum: strumAnchor };

  const headPivot = new THREE.Group();
  headPivot.position.y = 0.44;
  torso.add(headPivot);
  const head = new THREE.Mesh(new THREE.SphereGeometry(0.105, 28, 20), skin);
  head.position.y = 0.09;
  head.scale.set(0.92, 1, 0.92);
  headPivot.add(head);
  const hair = new THREE.Mesh(new THREE.SphereGeometry(0.113, 28, 20, 0, TAU, 0, Math.PI * 0.62), hairMaterial);
  hair.position.y = 0.1;
  hair.rotation.x = -0.35;
  headPivot.add(hair);
  const hairFall = new THREE.Mesh(lathe([[0, 0], [0.07, 0], [0.1, -0.12], [0.11, -0.3], [0.08, -0.36], [0, -0.36]], 24), hairMaterial);
  hairFall.position.set(0, 0.12, -0.07);
  hairFall.scale.set(1, 1, 0.55);
  headPivot.add(hairFall);
  const hairFlower = new THREE.Group();
  hairFlower.position.set(0.085, 0.14, 0.05);
  for (let i = 0; i < 5; i++) {
    const petal = new THREE.Mesh(new THREE.SphereGeometry(0.02, 10, 8), petalMaterial);
    const angle = (i / 5) * TAU;
    petal.position.set(Math.cos(angle) * 0.022, Math.sin(angle) * 0.022, 0);
    petal.scale.set(1, 1, 0.5);
    hairFlower.add(petal);
  }
  const pistil = new THREE.Mesh(new THREE.SphereGeometry(0.014, 10, 8), topMaterial);
  pistil.position.z = 0.01;
  hairFlower.add(pistil);
  hairFlower.lookAt(0.6, 0.3, 0.6);
  headPivot.add(hairFlower);

  // Her own glow: a soft sprite behind her, so the bloom has something to find at a distance.
  const halo = new THREE.Sprite(new THREE.SpriteMaterial({
    map: canvasTexture(128, (context, size) => {
      const gradient = context.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
      gradient.addColorStop(0, 'rgba(255,255,255,0.55)');
      gradient.addColorStop(0.35, 'rgba(255,255,255,0.16)');
      gradient.addColorStop(1, 'rgba(255,255,255,0)');
      context.fillStyle = gradient;
      context.fillRect(0, 0, size, size);
    }),
    color: palette.cherenkov.clone().lerp(palette.accent, 0.5),
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
    opacity: 0.6,
  }));
  halo.position.set(0, 0.55, -0.05);
  halo.scale.set(1.1, 1.3, 1);
  hips.add(halo);
  parts.halo = halo;

  parts.girl = girl;
  parts.hips = hips;
  parts.torso = torso;
  parts.headPivot = headPivot;
  parts.arms = arms;
  parts.materials = { skin, rodMaterial, petalMaterial, topMaterial };
  return { group, parts };
}

// The dashboard: a long slab of vinyl with a rounded front edge, under the ornament.
function buildDashboard(palette) {
  const shape = new THREE.Shape();
  const w = 7;
  const d = 2.6;
  const r = 0.22;
  shape.moveTo(-w / 2, -d / 2);
  shape.lineTo(w / 2, -d / 2);
  shape.lineTo(w / 2, d / 2 - r);
  shape.quadraticCurveTo(w / 2, d / 2, w / 2 - r, d / 2);
  shape.lineTo(-w / 2 + r, d / 2);
  shape.quadraticCurveTo(-w / 2, d / 2, -w / 2, d / 2 - r);
  shape.closePath();
  const geometry = new THREE.ExtrudeGeometry(shape, { depth: 0.36, bevelEnabled: true, bevelThickness: 0.08, bevelSize: 0.08, bevelSegments: 5 });
  geometry.rotateX(-Math.PI / 2);
  geometry.translate(0, -0.36, 0.3);
  const material = new THREE.MeshStandardMaterial({
    color: new THREE.Color('#171420').lerp(palette.accent, 0.04),
    roughness: 0.78,
    metalness: 0.08,
    bumpMap: grainTexture(),
    bumpScale: 0.6,
  });
  const dash = new THREE.Mesh(geometry, material);
  // A chrome trim line along the front edge, as the better trucks had.
  const trim = new THREE.Mesh(new THREE.BoxGeometry(w - 0.3, 0.02, 0.03), new THREE.MeshStandardMaterial({ color: '#d8d2e0', metalness: 1, roughness: 0.25 }));
  trim.position.set(0, -0.08, d / 2 + 0.33);
  const group = new THREE.Group();
  group.add(dash, trim);
  return group;
}

// Two bones from a shoulder to a hand: the law of cosines gives the angles, the pole says which
// way the elbow goes, and the meshes (which grow along +Y from their origin) are placed on the
// bones. A target out of reach is pulled in along the line to it.
const ikDirection = new THREE.Vector3();
const ikPole = new THREE.Vector3();
const ikElbow = new THREE.Vector3();
const ikBone = new THREE.Vector3();
const ikUp = new THREE.Vector3(0, 1, 0);
function solveArm(arm) {
  const { shoulder, target, pole } = arm;
  ikDirection.subVectors(target, shoulder);
  const reach = Math.min(ikDirection.length(), ARM_UPPER + ARM_FORE - 0.01);
  ikDirection.normalize();
  const cosShoulder = (ARM_UPPER * ARM_UPPER + reach * reach - ARM_FORE * ARM_FORE) / (2 * ARM_UPPER * reach);
  const shoulderAngle = Math.acos(THREE.MathUtils.clamp(cosShoulder, -1, 1));
  ikPole.copy(pole).addScaledVector(ikDirection, -pole.dot(ikDirection)).normalize();
  ikBone.copy(ikDirection).multiplyScalar(Math.cos(shoulderAngle)).addScaledVector(ikPole, Math.sin(shoulderAngle));
  ikElbow.copy(shoulder).addScaledVector(ikBone, ARM_UPPER);
  arm.upper.position.copy(shoulder);
  arm.upper.quaternion.setFromUnitVectors(ikUp, ikBone);
  ikBone.copy(shoulder).addScaledVector(ikDirection, reach).sub(ikElbow).normalize();
  arm.fore.position.copy(ikElbow);
  arm.fore.quaternion.setFromUnitVectors(ikUp, ikBone);
  arm.hand.position.copy(ikElbow).addScaledVector(ikBone, ARM_FORE);
  arm.hand.quaternion.copy(arm.fore.quaternion);
}

// Start ----------------------------------------------------------------------------------------

function start(hero, stage) {
  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const still = stage.querySelector('.jess-hero__still');

  let renderer;
  try {
    renderer = new THREE.WebGLRenderer({ antialias: false, alpha: false, powerPreference: 'high-performance' });
  } catch {
    return; // no WebGL: the still stays
  }
  if (!renderer.capabilities.isWebGL2) {
    renderer.dispose();
    return;
  }
  renderer.autoClear = false;
  renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
  const canvas = renderer.domElement;
  canvas.className = 'jess-hero__canvas';
  canvas.setAttribute('aria-hidden', 'true');
  hero.prepend(canvas);

  const accent = cssColor('--dw-accent', '#cf9bff');
  const bg0 = cssColor('--dw-bg-0', '#07060a');
  const bg1 = cssColor('--dw-bg-1', '#0c0911');
  const palette = {
    accent,
    accentCss: getComputedStyle(document.documentElement).getPropertyValue('--dw-accent').trim() || '#cf9bff',
    // Cherenkov light sits between the brand's violet and the blue it really is.
    cherenkov: accent.clone().lerp(new THREE.Color('#6fc3ff'), 0.5),
  };

  const floatTargets = renderer.extensions.has('EXT_color_buffer_float') || renderer.extensions.has('EXT_color_buffer_half_float');
  const targetType = floatTargets ? THREE.HalfFloatType : THREE.UnsignedByteType;
  const small = Math.min(innerWidth, innerHeight) < 700;
  const sceneTarget = new THREE.WebGLRenderTarget(1, 1, { type: targetType, samples: small ? 0 : 4 });
  const makeTarget = () => new THREE.WebGLRenderTarget(1, 1, { type: targetType, depthBuffer: false });
  const bloomTargets = [makeTarget(), makeTarget(), makeTarget(), makeTarget()];

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(24, 1, 0.1, 60);
  const postCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const quad = new THREE.PlaneGeometry(2, 2);

  const skyUniforms = {
    uTop: { value: bg0.clone().lerp(new THREE.Color('#0a1226'), 0.35) },
    uBottom: { value: bg0.clone().lerp(accent, 0.035) },
    uAccent: { value: accent },
    uCherenkov: { value: palette.cherenkov },
    uResolution: { value: new THREE.Vector2(1, 1) },
    uDrift: { value: new THREE.Vector2() },
    uCore: { value: new THREE.Vector2(0.7, 0.5) },
    uTime: { value: 0 },
    uGlow: { value: 1 },
  };
  const skyMaterial = new THREE.ShaderMaterial({ vertexShader: FULLSCREEN_VERTEX, fragmentShader: SKY_FRAGMENT, uniforms: skyUniforms, depthTest: false, depthWrite: false });
  const skyScene = new THREE.Scene();
  skyScene.add(new THREE.Mesh(quad, skyMaterial));

  // Lights: the sky's cold wash, a dim lamp from the cab, and the core's own light in the model.
  scene.add(new THREE.HemisphereLight(new THREE.Color('#4a5a8a'), bg0, 0.5));
  const cab = new THREE.DirectionalLight(new THREE.Color('#ffffff').lerp(accent, 0.3), 0.75);
  cab.position.set(-2.5, 3.2, 2.6);
  scene.add(cab);
  const rim = new THREE.DirectionalLight(new THREE.Color('#8fb8ff'), 0.6);
  rim.position.set(2, 1.5, -3);
  scene.add(rim);

  const world = new THREE.Group();
  scene.add(world);
  world.add(buildDashboard(palette));
  const ornament = buildOrnament(palette);
  const ornamentRoot = new THREE.Group();
  ornamentRoot.add(ornament.group);
  world.add(ornamentRoot);
  const { parts } = ornament;

  // Post passes.
  const brightMaterial = new THREE.ShaderMaterial({ vertexShader: FULLSCREEN_VERTEX, fragmentShader: BRIGHT_FRAGMENT, uniforms: { tInput: { value: sceneTarget.texture }, uThreshold: { value: 1.0 } }, depthTest: false, depthWrite: false });
  const blurMaterial = new THREE.ShaderMaterial({ vertexShader: FULLSCREEN_VERTEX, fragmentShader: BLUR_FRAGMENT, uniforms: { tInput: { value: null }, uDirection: { value: new THREE.Vector2() } }, depthTest: false, depthWrite: false });
  const copyMaterial = new THREE.ShaderMaterial({ vertexShader: FULLSCREEN_VERTEX, fragmentShader: COPY_FRAGMENT, uniforms: { tInput: { value: null } }, depthTest: false, depthWrite: false });
  const compositeUniforms = {
    tScene: { value: sceneTarget.texture },
    tBloomNear: { value: bloomTargets[0].texture },
    tBloomFar: { value: bloomTargets[2].texture },
    uCenter: { value: new THREE.Vector2(0.7, 0.5) },
    uHalf: { value: new THREE.Vector2(0.25, 0.4) },
    uScrim: { value: 0.62 },
    uTime: { value: 0 },
  };
  const compositeMaterial = new THREE.ShaderMaterial({ vertexShader: FULLSCREEN_VERTEX, fragmentShader: COMPOSITE_FRAGMENT, uniforms: compositeUniforms, depthTest: false, depthWrite: false });
  const postScene = new THREE.Scene();
  const postQuad = new THREE.Mesh(quad, copyMaterial);
  postQuad.frustumCulled = false;
  postScene.add(postQuad);
  function pass(material, target) {
    postQuad.material = material;
    renderer.setRenderTarget(target);
    renderer.render(postScene, postCamera);
  }
  function blur(from, scratch, radius) {
    blurMaterial.uniforms.tInput.value = from.texture;
    blurMaterial.uniforms.uDirection.value.set(radius / from.width, 0);
    pass(blurMaterial, scratch);
    blurMaterial.uniforms.tInput.value = scratch.texture;
    blurMaterial.uniforms.uDirection.value.set(0, radius / from.height);
    pass(blurMaterial, from);
  }

  // Layout: the ornament lands in the stage's box, whatever the grid did with it. The camera
  // sits a little above the dashboard and looks down at it, as a driver would; the model is moved
  // in the plane z = 0 so the stage's centre is where the vessel's middle projects, and scaled so
  // the stage's height is the ornament's.
  let width = 1;
  let height = 1;
  const quality = { level: 1, slow: 0, fast: 0 };
  const stageBox = { x: 0.7, y: 0.5, halfW: 0.25, halfH: 0.4 };
  const CAMERA_DISTANCE = 9;
  const CAMERA_HEIGHT = 3.4;
  const LOOK_HEIGHT = 0.9;
  const ORNAMENT_HEIGHT = 2.05;
  const ORNAMENT_MIDDLE = 0.88;
  const probe = new THREE.Vector3();
  const forward = new THREE.Vector3();
  function layout() {
    width = Math.max(1, hero.clientWidth);
    height = Math.max(1, hero.clientHeight);
    const dpr = Math.min(window.devicePixelRatio || 1, small ? 1.5 : 1.75) * quality.level;
    renderer.setPixelRatio(dpr);
    renderer.setSize(width, height, false);
    const w = Math.max(1, Math.floor(width * dpr));
    const h = Math.max(1, Math.floor(height * dpr));
    sceneTarget.setSize(w, h);
    bloomTargets[0].setSize(Math.max(1, w >> 2), Math.max(1, h >> 2));
    bloomTargets[1].setSize(Math.max(1, w >> 2), Math.max(1, h >> 2));
    bloomTargets[2].setSize(Math.max(1, w >> 3), Math.max(1, h >> 3));
    bloomTargets[3].setSize(Math.max(1, w >> 3), Math.max(1, h >> 3));
    camera.aspect = width / height;
    camera.position.set(0, CAMERA_HEIGHT, CAMERA_DISTANCE);
    camera.lookAt(0, LOOK_HEIGHT, 0);
    camera.updateProjectionMatrix();
    camera.updateMatrixWorld();
    skyUniforms.uResolution.value.set(width, height);
    parts.vaporUniforms.uPixelRatio.value = dpr;

    const heroRect = hero.getBoundingClientRect();
    const rect = stage.getBoundingClientRect();
    stageBox.x = (rect.left + rect.width / 2 - heroRect.left) / width;
    stageBox.y = 1 - (rect.top + rect.height / 2 - heroRect.top) / height;
    stageBox.halfW = rect.width / 2 / width;
    stageBox.halfH = rect.height / 2 / height;
    compositeUniforms.uCenter.value.set(stageBox.x, stageBox.y);
    compositeUniforms.uHalf.value.set(stageBox.halfW, stageBox.halfH);

    // Where the stage's centre lands on the plane z = 0, and how tall that plane shows there.
    probe.set(stageBox.x * 2 - 1, stageBox.y * 2 - 1, 0.5).unproject(camera).sub(camera.position).normalize();
    const along = -camera.position.z / probe.z;
    const centre = probe.multiplyScalar(along).add(camera.position);
    camera.getWorldDirection(forward);
    const depth = centre.clone().sub(camera.position).dot(forward);
    const visibleHeight = 2 * depth * Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
    const fit = Math.min((rect.height / height) * 0.95, (rect.width / width) * camera.aspect * 1.15);
    const scale = (fit * visibleHeight) / ORNAMENT_HEIGHT;
    ornamentRoot.scale.setScalar(scale);
    ornamentRoot.position.set(centre.x, centre.y - ORNAMENT_MIDDLE * scale, 0);
    // The dashboard runs under the whole hero, its top where the ornament stands.
    world.children[0].position.set(0, ornamentRoot.position.y, 0);
    world.children[0].scale.setScalar(Math.max(scale * 1.6, 1.0));
    skyUniforms.uCore.value.set(stageBox.x, stageBox.y);
  }

  // Motion ------------------------------------------------------------------------------------

  const spring = { tilt: 0, tiltV: 0, pitch: 0, pitchV: 0, bounce: 0, bounceV: 0 };
  let flare = 0;
  let nextBump = 2.5 + Math.random() * 3;
  let needleEased = 0;
  let headLag = 0;
  let pitchLag = 0;
  const pointer = new THREE.Vector2();
  const eased = new THREE.Vector2();
  const pointerLast = new THREE.Vector2();
  let pointerKick = 0;

  function bump(strength) {
    const sign = Math.random() > 0.5 ? 1 : -1;
    spring.tiltV += sign * strength * (1.4 + Math.random() * 0.8);
    spring.pitchV += (Math.random() - 0.5) * strength * 1.2;
    spring.bounceV -= strength * 1.1;
    flare = Math.min(1.5, flare + strength * 0.7);
  }

  function integrate(dt) {
    // Lateral sway: slow and barely damped, as a spring under a hollow doll is. Fore and aft a
    // little stiffer; the bounce of the whole thing stiffer still.
    const step = (x, v, frequency, damping, dt) => {
      const k = Math.pow(TAU * frequency, 2);
      const c = 2 * damping * Math.sqrt(k);
      const a = -k * x - c * v;
      v += a * dt;
      x += v * dt;
      return [x, v];
    };
    [spring.tilt, spring.tiltV] = step(spring.tilt, spring.tiltV, 1.35, 0.035, dt);
    [spring.pitch, spring.pitchV] = step(spring.pitch, spring.pitchV, 1.7, 0.05, dt);
    [spring.bounce, spring.bounceV] = step(spring.bounce, spring.bounceV, 3.8, 0.16, dt);
    spring.tilt = THREE.MathUtils.clamp(spring.tilt, -0.6, 0.6);
    spring.pitch = THREE.MathUtils.clamp(spring.pitch, -0.45, 0.45);
    // The road: a steady tremor, and a bump now and then.
    spring.tiltV += (Math.random() - 0.5) * 0.9 * dt;
    spring.pitchV += (Math.random() - 0.5) * 0.5 * dt;
    nextBump -= dt;
    if (nextBump <= 0) {
      bump(0.45 + Math.random() * 0.5);
      nextBump = 3.5 + Math.random() * 5;
    }
    if (pointerKick > 0.0005) {
      spring.tiltV += pointerKick * 2.2;
      pointerKick = 0;
    }
    flare = Math.max(0, flare - flare * 3.2 * dt);
  }

  function pose(elapsed) {
    const tilt = spring.tilt;
    const pitch = spring.pitch;
    headLag += (tilt - headLag) * 0.12;
    pitchLag += (pitch - pitchLag) * 0.1;
    parts.girl.position.y = 0.935 + spring.bounce * 0.035;
    parts.hips.rotation.set(pitch * 0.35, tilt * 1.1 + Math.sin(elapsed * 1.2) * 0.05, tilt * 0.4);
    parts.torso.rotation.set(-pitch * 0.3, -tilt * 0.35, -tilt * 0.6);
    parts.headPivot.rotation.set(pitchLag * 0.25 - 0.04, headLag * 0.4, -headLag * 0.25 + tilt * 0.1);
    // The fretting hand holds the neck and walks a little; the strumming hand crosses the
    // soundhole in time with the sway, faster when the road is rough.
    const strumRate = 2.4 + Math.min(1.5, Math.abs(spring.tiltV)) * 0.4;
    for (const arm of parts.arms) {
      if (arm.side > 0) {
        arm.target.copy(parts.anchors.fret);
        arm.target.y += Math.sin(elapsed * 0.45) * 0.012;
        arm.target.x += Math.sin(elapsed * 0.45) * 0.01;
      } else {
        arm.target.copy(parts.anchors.strum);
        arm.target.y += Math.sin(elapsed * strumRate) * 0.026;
        arm.target.z += 0.01 + Math.cos(elapsed * strumRate) * 0.008;
      }
      solveArm(arm);
    }
    parts.strandUniforms.uSway.value.set(tilt * 1.1 + spring.tiltV * 0.12, pitch * 0.8 + spring.pitchV * 0.08);
    ornamentRoot.rotation.set(0.0, 0, 0);
    ornament.group.position.y = spring.bounce * 0.012;

    // The core breathes; a bump flares it; the gauge shows what it felt.
    const breath = 0.5 + 0.5 * Math.sin(elapsed * 1.4) * Math.sin(elapsed * 0.37);
    const glow = 0.95 + breath * 0.3 + flare * 1.2 + Math.sin(elapsed * 23) * 0.015 * (0.5 + flare);
    parts.rodMaterial.emissiveIntensity = glow * 1.7;
    parts.coreLight.intensity = 5.5 * glow;
    parts.poolUniforms.uGlow.value = glow;
    parts.strandUniforms.uGlow.value = 0.9 + glow * 0.3;
    parts.halo.material.opacity = 0.35 + glow * 0.2;
    parts.materials.skin.emissiveIntensity = 0.22 + glow * 0.12;
    parts.materials.petalMaterial.emissiveIntensity = 0.7 + glow * 0.3;
    skyUniforms.uGlow.value = glow;
    const needleTarget = 0.35 + Math.min(1.6, Math.abs(spring.tiltV) * 0.25 + Math.abs(spring.bounceV) * 0.2 + flare * 1.1);
    needleEased += (needleTarget - needleEased) * 0.2;
    parts.needle.rotation.z = Math.PI * 0.35 - needleEased * 1.25;

    // The cab leans with the pointer, a little; the windshield drifts with it.
    world.rotation.y = eased.x * 0.08;
    world.rotation.x = eased.y * 0.04;
    skyUniforms.uDrift.value.set(-eased.x * 0.015, eased.y * 0.01);
  }

  const clock = new THREE.Clock();
  let elapsed = 0;
  let first = true;
  function frame() {
    running = false;
    let dt = Math.min(clock.getDelta(), 0.05);
    const frameStart = performance.now();
    if (reduceMotion) dt = 0;
    elapsed += dt;
    eased.lerp(pointer, 1 - Math.exp(-dt * 2.2));
    if (!reduceMotion) {
      for (let i = 0; i < 2; i++) integrate(dt / 2);
    }
    pose(elapsed);
    skyUniforms.uTime.value = elapsed;
    parts.poolUniforms.uTime.value = elapsed;
    parts.strandUniforms.uTime.value = elapsed;
    parts.vaporUniforms.uTime.value = elapsed;
    compositeUniforms.uTime.value = elapsed;

    renderer.setRenderTarget(sceneTarget);
    renderer.clear();
    renderer.render(skyScene, postCamera);
    renderer.render(scene, camera);
    pass(brightMaterial, bloomTargets[0]);
    blur(bloomTargets[0], bloomTargets[1], 1.0);
    blur(bloomTargets[0], bloomTargets[1], 2.0);
    copyMaterial.uniforms.tInput.value = bloomTargets[0].texture;
    pass(copyMaterial, bloomTargets[2]);
    blur(bloomTargets[2], bloomTargets[3], 1.5);
    blur(bloomTargets[2], bloomTargets[3], 3.0);
    pass(compositeMaterial, null);

    if (first) {
      first = false;
      canvas.classList.add('is-ready');
      hero.classList.add('is-live');
    }
    // Adaptive resolution: three slow frames in a row drop a level, a long run of fast ones
    // climbs back.
    const cost = performance.now() - frameStart;
    if (cost > 24) {
      quality.slow += 1;
      quality.fast = 0;
      if (quality.slow >= 3 && quality.level > 0.5) {
        quality.level = Math.max(0.5, quality.level - 0.25);
        quality.slow = 0;
        layout();
      }
    } else if (cost < 9) {
      quality.fast += 1;
      quality.slow = 0;
      if (quality.fast > 240 && quality.level < 1) {
        quality.level = Math.min(1, quality.level + 0.25);
        quality.fast = 0;
        layout();
      }
    }
    if (visible && !reduceMotion && !document.hidden) requestFrame();
  }

  let visible = false;
  let running = false;
  let lost = false;
  function requestFrame() {
    if (running || lost) return;
    running = true;
    requestAnimationFrame(frame);
  }

  if (!reduceMotion) {
    hero.addEventListener('pointermove', (event) => {
      const rect = hero.getBoundingClientRect();
      const x = ((event.clientX - rect.left) / rect.width) * 2 - 1;
      const y = ((event.clientY - rect.top) / rect.height) * 2 - 1;
      pointer.set(x, y);
      // A quick pass across the hero nudges her, in the direction it went.
      const dx = x - pointerLast.x;
      pointerLast.set(x, y);
      if (Math.abs(dx) < 0.5) pointerKick = THREE.MathUtils.clamp(pointerKick + dx * 0.4, -0.25, 0.25);
    }, { passive: true });
    hero.addEventListener('pointerleave', () => pointer.set(0, 0), { passive: true });
  }
  canvas.addEventListener('pointerdown', (event) => {
    if (!event.isPrimary) return;
    bump(1.1);
    // Under reduced motion the springs still move, once: a third of a second of road in one
    // frame, then stillness again.
    if (reduceMotion) {
      for (let i = 0; i < 20; i++) integrate(1 / 60);
    }
    requestFrame();
  });
  canvas.style.cursor = 'pointer';

  canvas.addEventListener('webglcontextlost', (event) => {
    event.preventDefault();
    lost = true;
    hero.classList.remove('is-live');
    canvas.classList.remove('is-ready');
  });
  canvas.addEventListener('webglcontextrestored', () => {
    lost = false;
    first = true;
    layout();
    requestFrame();
  });

  layout();
  new ResizeObserver(() => {
    layout();
    requestFrame();
  }).observe(hero);
  if (document.fonts) {
    document.fonts.ready.then(() => {
      layout();
      requestFrame();
    });
  }
  new IntersectionObserver((entries) => {
    visible = entries.some((entry) => entry.isIntersecting);
    if (visible) {
      clock.getDelta();
      requestFrame();
    }
  }, { rootMargin: '80px 0px' }).observe(hero);
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden && visible) {
      clock.getDelta();
      requestFrame();
    }
  });
  if (still) still.setAttribute('aria-hidden', 'true');
}

// The rest of the page earns its motion by staying quiet until it scrolls into view.
const reveals = document.querySelectorAll('.jess-reveal');
if (matchMedia('(prefers-reduced-motion: reduce)').matches || !('IntersectionObserver' in window)) {
  reveals.forEach((node) => node.classList.add('is-visible'));
} else {
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add('is-visible');
      observer.unobserve(entry.target);
    }
  }, { rootMargin: '0px 0px -8% 0px', threshold: 0.08 });
  reveals.forEach((node) => observer.observe(node));
}

const stage = document.querySelector('[data-jess-hero-stage]');
const hero = stage ? stage.closest('[data-jess-hero]') : null;
if (hero && stage) start(hero, stage);
