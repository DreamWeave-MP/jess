const hero = document.querySelector('[data-jess-hero]');
const canvas = document.querySelector('[data-jess-hero-canvas]');

const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

// The rest of the page earns its animation by staying quiet until it enters the viewport.
const reveals = document.querySelectorAll('.jess-reveal');
if (reducedMotion.matches || !('IntersectionObserver' in window)) {
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

if (!hero || !(canvas instanceof HTMLCanvasElement) || reducedMotion.matches) {
  // CSS renders a static fallback. Nothing about the page depends on WebGL.
} else {
  const gl = canvas.getContext('webgl2', {
    alpha: true,
    antialias: false,
    depth: false,
    powerPreference: 'high-performance',
    premultipliedAlpha: true,
  });

  if (gl) {
    const vertexSource = `#version 300 es
      precision highp float;
      out vec2 v_uv;
      void main() {
        vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
        v_uv = p * 0.5;
        gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
      }
    `;

    const fragmentSource = `#version 300 es
      precision highp float;
      in vec2 v_uv;
      out vec4 out_color;

      uniform vec2 u_resolution;
      uniform vec2 u_pointer;
      uniform float u_time;

      const float PI = 3.141592653589793;

      float hash21(vec2 p) {
        p = fract(p * vec2(123.34, 456.21));
        p += dot(p, p + 45.32);
        return fract(p.x * p.y);
      }

      float noise(vec2 p) {
        vec2 i = floor(p);
        vec2 f = fract(p);
        f = f * f * (3.0 - 2.0 * f);
        float a = hash21(i);
        float b = hash21(i + vec2(1.0, 0.0));
        float c = hash21(i + vec2(0.0, 1.0));
        float d = hash21(i + vec2(1.0, 1.0));
        return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
      }

      float fbm(vec2 p) {
        float value = 0.0;
        float amplitude = 0.5;
        for (int i = 0; i < 4; i++) {
          value += amplitude * noise(p);
          p = mat2(1.6, 1.2, -1.2, 1.6) * p + 0.17;
          amplitude *= 0.5;
        }
        return value;
      }

      float sdBox(vec2 p, vec2 b) {
        vec2 q = abs(p) - b;
        return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0);
      }

      float line(float d, float width) {
        return 1.0 - smoothstep(width, width * 2.4, abs(d));
      }

      void main() {
        vec2 uv = v_uv;
        vec2 p = uv * 2.0 - 1.0;
        p.x *= u_resolution.x / max(u_resolution.y, 1.0);

        // Pointer motion bends the field instead of moving the object. It should feel heavy.
        vec2 pointer = (u_pointer * 2.0 - 1.0) * vec2(0.10, -0.07);
        p += pointer * (0.45 + 0.55 * (1.0 - smoothstep(0.0, 1.4, length(p))));

        float t = u_time * 0.16;
        float r = length(p);
        float a = atan(p.y, p.x);

        // The outside is noisy and contradictory. Toward the boundary it resolves into structure.
        float turbulence = fbm(p * 2.1 + vec2(t * 0.35, -t * 0.22));
        float haze = (1.0 - smoothstep(0.10, 1.55, r)) * (0.11 + turbulence * 0.16);

        // Concentric protocol/ring traces. They compress near the core like a machine becoming ordered.
        float ringPhase = r * 18.0 - t * 2.2 + turbulence * 1.5;
        float rings = pow(max(0.0, 0.5 + 0.5 * cos(ringPhase * PI)), 18.0);
        rings *= (1.0 - smoothstep(0.15, 1.45, r)) * smoothstep(0.18, 0.44, r);

        // Radial package shards: detached pieces still obey the same runtime geometry.
        float sectors = 18.0;
        float cell = floor((a + PI) / (2.0 * PI) * sectors);
        float sectorRand = hash21(vec2(cell, floor(r * 8.0)));
        float localA = fract((a + PI) / (2.0 * PI) * sectors) - 0.5;
        float shardBand = abs(fract(r * 7.0 - t * (0.12 + sectorRand * 0.22)) - 0.5);
        float shards = (1.0 - smoothstep(0.015, 0.10, abs(localA))) * (1.0 - smoothstep(0.02, 0.18, shardBand));
        shards *= (1.0 - smoothstep(0.34, 1.65, r)) * smoothstep(0.22, 0.50, r);
        shards *= 0.35 + sectorRand * 0.65;

        // The privileged core is literally crossed out and fractured into a narrow boundary.
        vec2 q = p;
        q *= mat2(cos(0.08), -sin(0.08), sin(0.08), cos(0.08));
        float box = sdBox(q, vec2(0.26, 0.17));
        float coreFrame = line(box, 0.007);
        float coreGlow = exp(-18.0 * abs(box));
        float crossA = line(q.y - q.x * 0.48, 0.007) * (1.0 - smoothstep(0.29, 0.36, abs(q.x)));
        float crossB = line(q.y + q.x * 0.48, 0.007) * (1.0 - smoothstep(0.29, 0.36, abs(q.x)));
        float split = max(crossA, crossB);

        // Thin horizontal/vertical data lanes survive where the old monolith used to be.
        float laneX = line(fract((q.x + 0.52) * 8.0 + t * 0.08) - 0.5, 0.035);
        float laneY = line(fract((q.y + 0.41) * 11.0 - t * 0.06) - 0.5, 0.028);
        float lanes = max(laneX * (1.0 - smoothstep(0.08, 0.32, abs(q.y))), laneY * (1.0 - smoothstep(0.10, 0.43, abs(q.x))));
        lanes *= (1.0 - smoothstep(0.24, 0.40, r));

        // Sparse distant telemetry, never foreground enough to become starfield wallpaper.
        vec2 grid = floor((p + 2.0) * 22.0);
        vec2 gf = fract((p + 2.0) * 22.0) - 0.5;
        float dotNoise = step(0.965, hash21(grid));
        float telemetry = dotNoise * (1.0 - smoothstep(0.02, 0.13, length(gf)));
        telemetry *= smoothstep(0.40, 1.35, r) * (1.0 - smoothstep(0.9, 1.65, r));

        vec3 black = vec3(0.010, 0.006, 0.018);
        vec3 bruise = vec3(0.18, 0.055, 0.30);
        vec3 violet = vec3(0.55, 0.20, 0.88);
        vec3 ultraviolet = vec3(0.82, 0.56, 1.00);
        vec3 white = vec3(0.97, 0.94, 1.00);

        vec3 color = black;
        color += bruise * haze;
        color += violet * rings * 0.62;
        color += ultraviolet * shards * 0.48;
        color += ultraviolet * coreGlow * 0.32;
        color += mix(ultraviolet, white, 0.55) * coreFrame * 0.78;
        color += white * split * 0.78;
        color += ultraviolet * lanes * 0.52;
        color += violet * telemetry * 0.32;

        // Very restrained CRT persistence/noise: the page should feel dangerous, not unreadable.
        float scan = 0.985 + 0.015 * sin(gl_FragCoord.y * 1.35 + u_time * 2.0);
        float grain = (hash21(gl_FragCoord.xy + floor(u_time * 17.0)) - 0.5) * 0.018;
        color = color * scan + grain;

        float vignette = (1.0 - smoothstep(0.38, 1.75, r));
        color *= 0.42 + vignette * 0.70;
        out_color = vec4(color, 0.96);
      }
    `;

    function compile(type, source) {
      const shader = gl.createShader(type);
      if (!shader) return null;
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        console.warn('[jess] hero shader compile failed:', gl.getShaderInfoLog(shader));
        gl.deleteShader(shader);
        return null;
      }
      return shader;
    }

    const vertex = compile(gl.VERTEX_SHADER, vertexSource);
    const fragment = compile(gl.FRAGMENT_SHADER, fragmentSource);
    const program = vertex && fragment ? gl.createProgram() : null;

    if (program && vertex && fragment) {
      gl.attachShader(program, vertex);
      gl.attachShader(program, fragment);
      gl.linkProgram(program);
      gl.deleteShader(vertex);
      gl.deleteShader(fragment);

      if (gl.getProgramParameter(program, gl.LINK_STATUS)) {
        const resolutionLocation = gl.getUniformLocation(program, 'u_resolution');
        const pointerLocation = gl.getUniformLocation(program, 'u_pointer');
        const timeLocation = gl.getUniformLocation(program, 'u_time');
        const pointer = { x: 0.5, y: 0.5, tx: 0.5, ty: 0.5 };
        let visible = true;
        let raf = 0;
        let started = performance.now();

        const resize = () => {
          const rect = canvas.getBoundingClientRect();
          const dpr = Math.min(window.devicePixelRatio || 1, 1.5);
          const width = Math.max(1, Math.round(rect.width * dpr));
          const height = Math.max(1, Math.round(rect.height * dpr));
          if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width;
            canvas.height = height;
          }
          gl.viewport(0, 0, width, height);
        };

        const frame = (now) => {
          if (!visible || document.hidden) {
            raf = 0;
            return;
          }
          resize();
          pointer.x += (pointer.tx - pointer.x) * 0.032;
          pointer.y += (pointer.ty - pointer.y) * 0.032;
          gl.useProgram(program);
          gl.uniform2f(resolutionLocation, canvas.width, canvas.height);
          gl.uniform2f(pointerLocation, pointer.x, pointer.y);
          gl.uniform1f(timeLocation, (now - started) / 1000);
          gl.drawArrays(gl.TRIANGLES, 0, 3);
          raf = requestAnimationFrame(frame);
        };

        const ensureFrame = () => {
          if (!raf && visible && !document.hidden) {
            raf = requestAnimationFrame(frame);
          }
        };

        hero.addEventListener('pointermove', (event) => {
          const rect = hero.getBoundingClientRect();
          pointer.tx = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
          pointer.ty = Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height));
        }, { passive: true });

        hero.addEventListener('pointerleave', () => {
          pointer.tx = 0.5;
          pointer.ty = 0.5;
        }, { passive: true });

        if ('IntersectionObserver' in window) {
          const heroObserver = new IntersectionObserver(([entry]) => {
            visible = entry.isIntersecting;
            if (visible) ensureFrame();
          }, { rootMargin: '120px 0px' });
          heroObserver.observe(hero);
        }

        document.addEventListener('visibilitychange', ensureFrame);
        window.addEventListener('resize', resize, { passive: true });
        canvas.classList.add('is-ready');
        resize();
        raf = requestAnimationFrame(frame);
      } else {
        console.warn('[jess] hero shader link failed:', gl.getProgramInfoLog(program));
        gl.deleteProgram(program);
      }
    }
  }
}
