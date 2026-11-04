// Test drive: put a 1:64 die-cast car on the track and let it run. The car follows the centreline of its
// chain (from the engine's `track.centreline` op) as a point mass: gravity along the track, rolling
// friction on the floor, rubbing on the walls in curves, a little air drag. Where the track would have to
// pull rather than push (the top of a loop taken too slowly, a crest taken too fast) the car leaves it and
// flies; it lands again on any track it comes down on, or ends on the ground.
import type { Tool, ToolContext } from '../../plannr/web/src/tools/base';
import type { PluginHost } from '../../plannr/web/src/plugins';
import type * as THREE_NS from 'three';

type V3 = THREE_NS.Vector3;
const THREE = (window as any).plannr.THREE as typeof THREE_NS;
const G = 9.81;
/** 1:64 scale: real speed of the full-size car. */
export const scaleKmh = (v: number) => Math.abs(v) * 64 * 3.6;

// ---- the track's centreline --------------------------------------------------------------------
export interface LinePiece { id: string; label: string; kind: string; s0: number; s1: number }
export interface LineChain { pieces: LinePiece[]; length: number; closed: boolean; frames: number[][] }
export interface Line { profile: { width: number; floor: number; wall: number; wallThickness: number }; chains: LineChain[] }

/** One chain as flat arrays, with the curvature of each segment. */
class Track {
  n: number; s: Float64Array; c: Float64Array; t: Float64Array; u: Float64Array; k: Float64Array;
  length: number; closed: boolean; pieces: LinePiece[];
  private hint = 0;
  constructor(ch: LineChain) {
    const f = ch.frames; const n = f.length; this.n = n; this.length = ch.length; this.closed = ch.closed; this.pieces = ch.pieces;
    this.s = new Float64Array(n); this.c = new Float64Array(n * 3); this.t = new Float64Array(n * 3); this.u = new Float64Array(n * 3); this.k = new Float64Array(Math.max(0, n - 1) * 3);
    f.forEach((r, i) => { this.s[i] = r[0]; for (let a = 0; a < 3; a++) { this.c[i * 3 + a] = r[1 + a]; this.t[i * 3 + a] = r[4 + a]; this.u[i * 3 + a] = r[7 + a]; } });
    // curvature vector dT/ds per segment (zero on straights)
    for (let i = 0; i < n - 1; i++) { const ds = Math.max(1e-9, this.s[i + 1] - this.s[i]); for (let a = 0; a < 3; a++) this.k[i * 3 + a] = (this.t[(i + 1) * 3 + a] - this.t[i * 3 + a]) / ds; }
  }
  /** The segment holding `s` (s clamped to the chain). */
  seg(s: number): number {
    let i = Math.min(Math.max(this.hint, 0), this.n - 2);
    while (i > 0 && this.s[i] > s) i--;
    while (i < this.n - 2 && this.s[i + 1] < s) i++;
    this.hint = i; return i;
  }
  /** Frame at `s`: centre (floor underside), forward, up, left, curvature. */
  at(s: number, o: Frame): Frame {
    const i = this.seg(s); const s0 = this.s[i], s1 = this.s[i + 1]; const w = Math.min(1, Math.max(0, (s - s0) / Math.max(1e-9, s1 - s0)));
    const L = (arr: Float64Array, v: V3) => v.set(arr[i * 3] + (arr[i * 3 + 3] - arr[i * 3]) * w, arr[i * 3 + 1] + (arr[i * 3 + 4] - arr[i * 3 + 1]) * w, arr[i * 3 + 2] + (arr[i * 3 + 5] - arr[i * 3 + 2]) * w);
    L(this.c, o.c); L(this.t, o.t).normalize(); L(this.u, o.u);
    o.u.addScaledVector(o.t, -o.u.dot(o.t)).normalize();
    o.l.crossVectors(o.u, o.t);
    o.k.set(this.k[i * 3], this.k[i * 3 + 1], this.k[i * 3 + 2]);
    o.seg = i; return o;
  }
  piece(s: number): LinePiece | undefined { return this.pieces.find(p => s >= p.s0 - 1e-9 && s <= p.s1 + 1e-9) ?? this.pieces[this.pieces.length - 1]; }
  wrap(s: number) { return this.closed ? ((s % this.length) + this.length) % this.length : s; }
}
interface Frame { c: V3; t: V3; u: V3; l: V3; k: V3; seg: number }
const newFrame = (): Frame => ({ c: new THREE.Vector3(), t: new THREE.Vector3(), u: new THREE.Vector3(), l: new THREE.Vector3(), k: new THREE.Vector3(), seg: 0 });

// ---- physics -----------------------------------------------------------------------------------
export interface Params { speed: number; friction: number; drag: number }
export const DEFAULT_PARAMS: Params = { speed: 2, friction: 0.03, drag: 0.004 };
export interface Outcome { ok: boolean; text: string; at?: V3 }

/** The car: on the track (chain, s, v) or in the air (p, vel). */
export class Sim {
  tracks: Track[]; prof: Line['profile']; par: Params;
  mode: 'track' | 'air' | 'done' = 'track';
  chain = 0; s = 0; v = 0; start = { chain: 0, s: 0, dir: 1 };
  /** Which way the car faces along its chain (a car rolling back still faces forward). */
  face: 1 | -1 = 1; lapBase = 0;
  p = new THREE.Vector3(); vel = new THREE.Vector3(); fwd = new THREE.Vector3(1, 0, 0); up = new THREE.Vector3(0, 1, 0);
  time = 0; dist = 0; airTime = 0; laps = 0; lapTime = 0; lastLap = 0;
  g = 1; maxG = 0; maxSide = 0;
  /** Left the track somewhere other than an open end (a loop's top, a crest). */
  fell: { label: string; kind: string; at: V3; need: number; had: number } | null = null;
  offEnd: LinePiece | null = null; offEndForward = false;
  /** Where it last left the track mid-piece (a crest taken fast), and how fast. */
  flewFrom: { label: string; v: number } | null = null;
  jumps = 0; landedOn = '';
  outcome: Outcome | null = null;
  events: string[] = [];
  private f = newFrame();
  constructor(line: Line, par: Params) { this.tracks = line.chains.map(c => new Track(c)); this.prof = line.profile; this.par = { ...par }; }

  place(chain: number, s: number, dir: 1 | -1) {
    this.mode = 'track'; this.chain = chain; this.s = s; this.v = this.par.speed * dir; this.start = { chain, s, dir }; this.face = dir; this.lapBase = 0;
    this.time = this.dist = this.airTime = this.laps = this.lapTime = this.lastLap = 0; this.maxG = this.maxSide = 0; this.g = 1;
    this.fell = null; this.offEnd = null; this.offEndForward = false; this.flewFrom = null; this.jumps = 0; this.landedOn = ''; this.outcome = null; this.events = [];
    this.pose();
  }
  track() { return this.tracks[this.chain]; }
  /** Position (on top of the floor) and orientation from the state. */
  pose() {
    if (this.mode !== 'track') return;
    const f = this.track().at(this.s, this.f);
    this.p.copy(f.c).addScaledVector(f.u, this.prof.floor);
    this.fwd.copy(f.t).multiplyScalar(this.face); this.up.copy(f.u);
  }
  piece() { return this.track().piece(this.s); }

  step(dt: number) {
    if (this.mode === 'done') return;
    this.time += dt;
    if (this.mode === 'air') return this.stepAir(dt);
    const tr = this.track(); const f = tr.at(this.s, this.f); const v = this.v;
    // what the track must push with (per unit mass) to keep the car on it: up out of the floor and sideways
    const kv2u = f.k.dot(f.u) * v * v, kv2l = f.k.dot(f.l) * v * v;
    const Fu = kv2u + G * f.u.y, Fl = kv2l + G * f.l.y;
    this.g = Math.hypot(Fu, Fl) / G; this.maxG = Math.max(this.maxG, this.g); this.maxSide = Math.max(this.maxSide, Math.abs(Fl) / G);
    if (Fu < -0.2) { // the floor would have to pull: the car leaves it
      const pc = tr.piece(this.s);
      if (pc && pc.kind === 'loop' && !this.fell) {
        // the speed it needs at the very top of the loop: v² ≥ g·r there
        let need = 0; let uy = 1;
        for (let i = 0; i < tr.n - 1; i++) { if (tr.s[i] < pc.s0 || tr.s[i] > pc.s1) continue; const y = tr.u[i * 3 + 1]; if (y < uy) { uy = y; const k = Math.hypot(tr.k[i * 3], tr.k[i * 3 + 1], tr.k[i * 3 + 2]); need = Math.sqrt(G * -y / Math.max(1e-6, k)); } }
        this.fell = { label: pc.label, kind: pc.kind, at: f.c.clone(), need, had: Math.abs(v) };
        this.events.push(`Fell off ${pc.label} at ${this.time.toFixed(2)} s`);
      }
      this.takeOff(f, null); return;
    }
    const sgn = v !== 0 ? Math.sign(v) : -Math.sign(f.t.y);
    const mu = this.par.friction;
    const fric = mu * Math.max(Fu, 0) + 4 * mu * Math.abs(Fl);
    const grav = -G * f.t.y;
    if (v === 0) {
      if (Math.abs(grav) <= mu * Math.max(Fu, 0) + 1e-9) return this.stop(f);
      this.v = (grav - sgn * fric) * dt; return this.advance(dt, f);
    }
    const nv = v + (grav - sgn * fric - this.par.drag * v * Math.abs(v)) * dt;
    if (nv * v <= 0) { // it comes to a halt: it stays if the slope can't beat the friction, else rolls back
      if (Math.abs(grav) <= mu * Math.max(Fu, 0) + 1e-9) { this.v = 0; return this.stop(f); }
      this.v = 0; return;
    }
    this.v = nv; this.advance(dt, f);
  }
  private advance(dt: number, f: Frame) {
    const tr = this.track(); const ds = this.v * dt;
    this.s += ds; this.dist += Math.abs(ds);
    if (tr.closed) {
      this.lapTime += dt;
      if (this.dist - this.lapBase >= (this.laps + 1) * tr.length && !this.fell) { this.laps++; this.lastLap = this.lapTime; this.lapTime = 0; this.events.push(`Lap ${this.laps} in ${this.lastLap.toFixed(2)} s`); }
      this.s = tr.wrap(this.s);
    } else if (this.s > tr.length || this.s < 0) {
      // off an open end
      const end = this.s > tr.length ? tr.pieces[tr.pieces.length - 1] : tr.pieces[0];
      this.offEnd = end; this.offEndForward = (this.s > tr.length) === (this.start.dir > 0) && this.chain === this.start.chain || this.jumps > 0;
      this.s = Math.min(Math.max(this.s, 0), tr.length);
      this.takeOff(tr.at(this.s, f), end); return;
    }
    this.pose();
  }
  private takeOff(f: Frame, end: LinePiece | null) {
    this.mode = 'air'; this.airTime = 0;
    if (!end) this.flewFrom = { label: this.track().piece(this.s)?.label ?? 'the track', v: Math.abs(this.v) };
    this.p.copy(f.c).addScaledVector(f.u, this.prof.floor + 0.0005);
    this.vel.copy(f.t).multiplyScalar(this.v);
  }
  private stop(f: Frame) {
    this.mode = 'done'; this.pose();
    const pc = this.track().piece(this.s);
    if (this.laps) this.finish(true, `Went round ${this.laps} time${this.laps === 1 ? '' : 's'}${this.lastLap ? ` (last lap ${this.lastLap.toFixed(2)} s)` : ''}, then stopped on ${pc?.label ?? 'the track'}`, f.c.clone());
    else this.finish(false, `Stopped on ${pc?.label ?? 'the track'} after ${(this.dist * 100).toFixed(0)} cm: it needs more speed`, f.c.clone());
  }
  private stepAir(dt: number) {
    this.airTime += dt;
    this.vel.y -= G * dt; this.vel.addScaledVector(this.vel, -this.par.drag * this.vel.length() * dt);
    const prev = this.p.clone();
    this.p.addScaledVector(this.vel, dt);
    if (this.vel.lengthSq() > 1e-8) { const d = this.vel.clone().normalize(); if (d.dot(this.fwd) < 0) d.negate(); this.fwd.lerp(d, Math.min(1, dt * 8)).normalize(); }
    if (this.airTime > 0.015 && this.land(prev)) return;
    if (this.p.y <= 0 || this.airTime > 8) {
      this.p.y = Math.max(0, this.p.y); this.mode = 'done'; this.up.set(0, 1, 0);
      const flat = this.fwd.clone().setY(0); if (flat.lengthSq() > 1e-6) this.fwd.copy(flat.normalize());
      const spd = Math.hypot(this.vel.x, this.vel.z);
      if (this.fell) this.finish(false, '', this.fell.at);
      else if (this.offEnd && this.offEndForward) this.finish(true, `Made it to the end${this.jumps ? ` (${this.jumps} jump${this.jumps === 1 ? '' : 's'})` : ''}: off ${this.offEnd.label} at ${spd.toFixed(2)} m/s after ${this.time.toFixed(2)} s`, this.p.clone());
      else if (this.offEnd) this.finish(false, `Rolled back off ${this.offEnd.label}: not enough speed to make it`, this.p.clone());
      else this.finish(false, `Flew off ${this.flewFrom?.label ?? 'the track'} at ${(this.flewFrom?.v ?? spd).toFixed(2)} m/s and came down beside the track`, this.p.clone());
    }
  }
  /** Coming down onto any track from above: back on it, keeping the speed along it. */
  private land(prev: V3): boolean {
    const hw = this.prof.width / 2 - this.prof.wallThickness; const f = this.f;
    for (let ci = 0; ci < this.tracks.length; ci++) {
      const tr = this.tracks[ci];
      for (let i = 0; i < tr.n - 1; i++) {
        const ax = tr.c[i * 3], ay = tr.c[i * 3 + 1], az = tr.c[i * 3 + 2];
        const dx = this.p.x - ax, dy = this.p.y - ay, dz = this.p.z - az;
        const seg = tr.s[i + 1] - tr.s[i];
        if (dx * dx + dy * dy + dz * dz > (seg + 0.06) ** 2) continue;
        const along = dx * tr.t[i * 3] + dy * tr.t[i * 3 + 1] + dz * tr.t[i * 3 + 2];
        if (along < -1e-6 || along > seg + 1e-6) continue;
        tr.at(tr.s[i] + along, f);
        const top = f.c.clone().addScaledVector(f.u, this.prof.floor);
        const h = this.p.clone().sub(top).dot(f.u), h0 = prev.clone().sub(top).dot(f.u);
        const lat = this.p.clone().sub(f.c).dot(f.l);
        if (Math.abs(lat) > hw || h > 0 || h < -0.02 || h0 < -0.002 || this.vel.dot(f.u) >= 0) continue;
        if (ci !== this.chain) { this.lapBase = this.dist; this.laps = 0; this.lapTime = 0; }
        this.mode = 'track'; this.chain = ci; this.s = tr.s[i] + along; this.v = this.vel.dot(f.t); this.face = this.fwd.dot(f.t) >= 0 ? 1 : -1;
        const pc = tr.piece(this.s); this.landedOn = pc?.label ?? ''; this.jumps++;
        this.events.push(`Jumped and landed on ${this.landedOn}`);
        if (this.offEnd) this.offEnd = null; // over a gap onto more track
        this.pose(); return true;
      }
    }
    return false;
  }
  private finish(ok: boolean, text: string, at?: V3) {
    // falling off a loop is what went wrong, whatever happened after
    if (this.fell) { ok = false; at = this.fell.at; text = `Fell off ${this.fell.label} near the top, going ${this.fell.had.toFixed(2)} m/s: it needs at least ${this.fell.need.toFixed(2)} m/s at the top. Launch faster (try Find min speed)`; }
    this.outcome = { ok, text, at };
  }
  /** Run until done (or `limit` seconds); a closed chain is done after `laps` laps. */
  runHeadless(limit = 30, laps = 1): Outcome {
    const dt = 0.001;
    while (this.mode !== 'done' && this.time < limit) {
      this.step(dt);
      if (this.fell) return { ok: false, text: `Fell off ${this.fell.label}`, at: this.fell.at };
      if (this.laps >= laps) return { ok: true, text: `Lap in ${this.lastLap.toFixed(2)} s` };
    }
    return this.outcome ?? { ok: false, text: 'Still going' };
  }
}

/** The lowest launch speed that gets round (closed) or to the end (open), and up to where it keeps working. */
export async function minSpeed(line: Line, par: Params, chain: number, s: number, dir: 1 | -1, onProgress?: (t: string) => void): Promise<{ min: number | null; max: number | null; why: string }> {
  const test = (v: number) => { const sim = new Sim(line, { ...par, speed: v }); sim.place(chain, s, dir); return sim.runHeadless(40, 1); };
  const top = 8; let lo: number | null = null; let hi: number | null = null; let firstFail = ''; let tooFast = '';
  for (let v = 0; v <= top + 1e-9; v += 0.1) {
    const o = test(+v.toFixed(2));
    if (o.ok && lo === null) lo = v; if (o.ok) hi = v;
    if (!o.ok && lo === null) firstFail = o.text;
    if (!o.ok && lo !== null && !tooFast) { tooFast = o.text; break; }
    if (Math.round(v * 10) % 10 === 0) { onProgress?.(`Testing ${v.toFixed(1)} m/s…`); await new Promise(r => setTimeout(r)); }
  }
  if (lo === null) return { min: null, max: null, why: firstFail };
  // narrow the lower edge to a centimetre per second
  let a = Math.max(0, lo - 0.1), b = lo;
  if (lo > 0) for (let k = 0; k < 7; k++) { const m = (a + b) / 2; if (test(m).ok) b = m; else a = m; }
  return { min: b, max: tooFast ? hi : null, why: tooFast };
}

// ---- the car -----------------------------------------------------------------------------------
function carModel(): { group: THREE_NS.Group; wheels: THREE_NS.Object3D[]; mats: THREE_NS.MeshStandardMaterial[] } {
  const mm = 0.001; const group = new THREE.Group(); const wheels: THREE_NS.Object3D[] = [];
  const paint = new THREE.MeshStandardMaterial({ color: 0xe8262e, metalness: 0.1, roughness: 0.38, emissive: 0x3a0606 });
  const glass = new THREE.MeshStandardMaterial({ color: 0x24324a, metalness: 0, roughness: 0.2 });
  const tyre = new THREE.MeshStandardMaterial({ color: 0x1a1a1a, roughness: 0.8 });
  const rim = new THREE.MeshStandardMaterial({ color: 0xd5dae2, metalness: 0.2, roughness: 0.3 });
  const base = new THREE.MeshStandardMaterial({ color: 0x2b2f36, roughness: 0.6 });
  const side = (pts: number[][], depth: number, mat: THREE_NS.Material) => {
    const sh = new THREE.Shape(); pts.forEach(([x, y], i) => i ? sh.lineTo(x * mm, y * mm) : sh.moveTo(x * mm, y * mm));
    const g = new THREE.ExtrudeGeometry(sh, { depth: depth * mm, bevelEnabled: true, bevelThickness: 0.8 * mm, bevelSize: 0.8 * mm, bevelSegments: 2, curveSegments: 4 });
    g.translate(0, 0, -depth * mm / 2); const m = new THREE.Mesh(g, mat); m.castShadow = true; group.add(m); return m;
  };
  // a low wedge with a cab, nose along +x; y = 0 is the floor the wheels roll on
  side([[-34, 3.2], [33, 3.2], [36, 5.5], [33, 8], [12, 10.5], [3, 16], [-14, 16.5], [-24, 12], [-34, 11.5]], 26, paint);
  side([[2.2, 15.4], [10.6, 10.8], [-22.4, 11.6], [-13.6, 15.8]], 27.8, glass);
  side([[-30, 2.2], [30, 2.2], [30, 3.6], [-30, 3.6]], 22, base);
  for (const x of [-22, 22]) for (const z of [-12.2, 12.2]) {
    const w = new THREE.Group(); w.position.set(x * mm, 5.2 * mm, z * mm);
    const t = new THREE.Mesh(new THREE.CylinderGeometry(5.2 * mm, 5.2 * mm, 4 * mm, 18), tyre); t.rotation.x = Math.PI / 2; t.castShadow = true;
    const r = new THREE.Mesh(new THREE.CylinderGeometry(3.4 * mm, 3.4 * mm, 4.3 * mm, 6), rim); r.rotation.x = Math.PI / 2;
    w.add(t, r); group.add(w); wheels.push(w);
  }
  return { group, wheels, mats: [paint, glass, tyre, rim, base] };
}

// ---- tool, HUD ---------------------------------------------------------------------------------
const KEY = 'plannr.track.drive';
const st = (() => {
  const d = { ...DEFAULT_PARAMS, reverse: false, follow: false, slow: 1 };
  try { return { ...d, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') }; } catch { return d; }
})();
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* storage blocked */ } };

interface Spot { chain: number; s: number }
interface Placed { piece: string; ds: number }

export function makeDrive(getHost: () => PluginHost, top: (op: string, data: unknown) => any, accent: string) {
  let line: Line | null = null; let lineVersion = -1; let loading: Promise<void> | null = null;
  let sim: Sim | null = null; let placed: Placed | null = null; let running = false;
  let group: THREE_NS.Group | null = null; let car: ReturnType<typeof carModel> | null = null; let ghost: ReturnType<typeof carModel> | null = null;
  let trail: THREE_NS.Line | null = null; const trailPts: number[] = []; let marker: THREE_NS.Mesh | null = null;
  let poser: Sim | null = null; let poserLine: Line | null = null;
  let raf = 0; let last = 0; let wheelTurn = 0; let ctxRef: ToolContext | null = null; let testing = '';
  const hud = document.createElement('div'); hud.className = 'track-drive'; hud.style.display = 'none';

  async function load(ctx: ToolContext) {
    const v = ctx.doc().version; if (line && lineVersion === v) return;
    if (loading) return loading;
    loading = (async () => {
      try { const r = await ctx.ops([top('centreline', {})], 'Track centreline'); line = (r[0]?.entities?.[0]?.data as Line) ?? null; lineVersion = v; }
      catch (e: any) { ctx.toast(e.message ?? String(e)); line = null; }
      loading = null;
    })();
    return loading;
  }
  const chainOf = (piece: string) => line?.chains.findIndex(c => c.pieces.some(p => p.id === piece)) ?? -1;
  const spotOf = (p: Placed): Spot | null => { const ci = chainOf(p.piece); if (ci < 0) return null; const pc = line!.chains[ci].pieces.find(x => x.id === p.piece)!; return { chain: ci, s: Math.min(pc.s1, pc.s0 + p.ds) }; };
  const placedOf = (sp: Spot): Placed => { const ch = line!.chains[sp.chain]; const pc = ch.pieces.find(p => sp.s >= p.s0 - 1e-9 && sp.s <= p.s1 + 1e-9) ?? ch.pieces[0]; return { piece: pc.id, ds: sp.s - pc.s0 }; };

  /** The centreline point nearest `p` in space, on the chains that hold `pieces` (all when empty). */
  function nearest3(p: V3, pieces: Set<string>): Spot | null {
    let best: Spot | null = null; let bd = Infinity;
    line!.chains.forEach((ch, ci) => {
      if (pieces.size && !ch.pieces.some(x => pieces.has(x.id))) return;
      const F = ch.frames;
      for (let i = 1; i < F.length; i++) {
        const a = F[i - 1], b = F[i]; const ex = b[1] - a[1], ey = b[2] - a[2], ez = b[3] - a[3]; const L2 = ex * ex + ey * ey + ez * ez;
        const w = L2 < 1e-12 ? 0 : Math.min(1, Math.max(0, ((p.x - a[1]) * ex + (p.y - a[2]) * ey + (p.z - a[3]) * ez) / L2));
        const d = (a[1] + ex * w - p.x) ** 2 + (a[2] + ey * w - p.y) ** 2 + (a[3] + ez * w - p.z) ** 2;
        if (d < bd) { bd = d; best = { chain: ci, s: a[0] + (b[0] - a[0]) * w }; }
      }
    });
    return best;
  }
  /** The track point under the cursor: on a piece's faces, else the centreline nearest on screen (within 40 px). */
  function nearest(ctx: ToolContext, ev: { clientX: number; clientY: number }): Spot | null {
    if (!line) return null;
    const hit = ctx.scene.pick(ev, { facesOnly: true });
    if (hit) { const id = hit.id.split('#')[0]; if (line.chains.some(c => c.pieces.some(p => p.id === id))) return nearest3(hit.point, new Set([id])); }
    const rect = ctx.scene.renderer.domElement.getBoundingClientRect(); const mx = ev.clientX - rect.left, my = ev.clientY - rect.top;
    const fl = line.profile.floor; let best: Spot | null = null; let bd = 40 * 40;
    const a = new THREE.Vector3(), b = new THREE.Vector3();
    line.chains.forEach((ch, ci) => {
      const F = ch.frames; let pa = ctx.scene.screenPoint(a.set(F[0][1] + F[0][7] * fl, F[0][2] + F[0][8] * fl, F[0][3] + F[0][9] * fl));
      for (let i = 1; i < F.length; i++) {
        const pb = ctx.scene.screenPoint(b.set(F[i][1] + F[i][7] * fl, F[i][2] + F[i][8] * fl, F[i][3] + F[i][9] * fl));
        const ex = pb.x - pa.x, ey = pb.y - pa.y; const L2 = ex * ex + ey * ey;
        const w = L2 < 1e-9 ? 0 : Math.min(1, Math.max(0, ((mx - pa.x) * ex + (my - pa.y) * ey) / L2));
        const dx = pa.x + ex * w - mx, dy = pa.y + ey * w - my; const d = dx * dx + dy * dy;
        if (d < bd) { bd = d; best = { chain: ci, s: F[i - 1][0] + (F[i][0] - F[i - 1][0]) * w }; }
        pa = pb;
      }
    });
    return best;
  }

  function ensureScene(ctx: ToolContext) {
    if (group) return;
    group = new THREE.Group(); group.name = 'track-drive'; ctx.scene.scene.add(group);
    car = carModel(); group.add(car.group); car.group.visible = false;
    ghost = carModel(); ghost.mats.forEach(m => { m.transparent = true; m.opacity = 0.45; m.depthWrite = false; }); ghost.group.traverse(o => (o as any).castShadow = false); group.add(ghost.group); ghost.group.visible = false;
    const g = new THREE.BufferGeometry(); g.setAttribute('position', new THREE.BufferAttribute(new Float32Array(3 * 8000), 3)); g.setDrawRange(0, 0);
    trail = new THREE.Line(g, new THREE.LineBasicMaterial({ color: new THREE.Color(accent), transparent: true, opacity: 0.85, depthTest: false })); trail.renderOrder = 5; trail.frustumCulled = false; group.add(trail);
    marker = new THREE.Mesh(new THREE.SphereGeometry(0.006, 16, 12), new THREE.MeshBasicMaterial({ color: 0xd93025, depthTest: false })); marker.renderOrder = 6; marker.visible = false; group.add(marker);
  }
  function dropScene(ctx: ToolContext) {
    if (!group) return;
    ctx.scene.scene.remove(group);
    group.traverse(o => { const m = o as THREE_NS.Mesh; m.geometry?.dispose?.(); const mt = m.material as any; if (mt) (Array.isArray(mt) ? mt : [mt]).forEach((x: any) => x.dispose?.()); });
    group = car = ghost = null; trail = null; marker = null;
  }
  const basis = new THREE.Matrix4(); const side = new THREE.Vector3();
  function orient(obj: THREE_NS.Object3D, p: V3, f: V3, up: V3) {
    side.crossVectors(f, up).normalize(); const u = new THREE.Vector3().crossVectors(side, f).normalize();
    basis.makeBasis(f, u, side); obj.quaternion.setFromRotationMatrix(basis); obj.position.copy(p);
  }
  function showGhost(sp: Spot | null) {
    if (!ghost || !line) return;
    if (!sp || running) { ghost.group.visible = false; return; }
    if (poserLine !== line) { poser = new Sim(line, st); poserLine = line; }
    poser!.place(sp.chain, sp.s, st.reverse ? -1 : 1);
    orient(ghost.group, poser!.p, poser!.fwd, poser!.up); ghost.group.visible = true;
  }
  function drawCar() {
    if (!car || !sim) return;
    car.group.visible = true;
    orient(car.group, sim.p, sim.fwd, sim.up);
    car.wheels.forEach(w => (w.rotation.z = -wheelTurn));
  }
  function clearTrail() { trailPts.length = 0; if (trail) trail.geometry.setDrawRange(0, 0); if (marker) marker.visible = false; ctxRef?.scene.clearOverlay(); }
  function pushTrail(p: V3) {
    if (!trail) return; const n = trailPts.length / 3;
    if (n && Math.hypot(p.x - trailPts[n * 3 - 3], p.y - trailPts[n * 3 - 2], p.z - trailPts[n * 3 - 1]) < 0.004) return;
    if (n >= 8000) trailPts.splice(0, 3);
    trailPts.push(p.x, p.y + 0.003, p.z);
    const attr = trail.geometry.getAttribute('position') as THREE_NS.BufferAttribute; (attr.array as Float32Array).set(trailPts); attr.needsUpdate = true;
    trail.geometry.setDrawRange(0, trailPts.length / 3);
  }

  function reset(ctx: ToolContext) {
    running = false; cancelAnimationFrame(raf); clearTrail();
    if (!line || !placed) { sim = null; if (car) car.group.visible = false; renderHud(); return; }
    const sp = spotOf(placed); if (!sp) { placed = null; sim = null; if (car) car.group.visible = false; renderHud(); return; }
    sim = new Sim(line, st); sim.place(sp.chain, sp.s, st.reverse ? -1 : 1); wheelTurn = 0; drawCar(); renderHud();
    void ctx;
  }
  function go(ctx: ToolContext) {
    if (!sim) { ctx.toast('Click the track to put the car on it first'); return; }
    if (sim.mode === 'done') reset(ctx);
    if (!sim) return;
    running = true; last = performance.now(); if (ghost) ghost.group.visible = false;
    const tick = (now: number) => {
      if (!running || !sim) return;
      const real = Math.min(0.05, (now - last) / 1000); last = now;
      const before = sim.p.clone();
      let left = real * st.slow; const dt = 0.0005;
      while (left > 1e-9 && sim.mode !== 'done') { const h = Math.min(dt, left); sim.step(h); left -= h; }
      wheelTurn += before.distanceTo(sim.p) / 0.0052 * (sim.mode === 'track' && sim.v * sim.face < 0 ? -1 : 1);
      pushTrail(sim.p); drawCar();
      if (st.follow) { const d = sim.p.clone().sub(before); const cam = ctx.scene.cam; cam.setImmediate(cam.camera.position.clone().add(d), cam.target.clone().add(d), cam.up.clone()); }
      if (sim.mode === 'done') { running = false; showOutcome(ctx); renderHud(); return; }
      renderReadout();
      raf = requestAnimationFrame(tick);
    };
    renderHud(); raf = requestAnimationFrame(tick);
  }
  function pause() { running = false; cancelAnimationFrame(raf); renderHud(); }
  function showOutcome(ctx: ToolContext) {
    const o = sim?.outcome; if (!o) return;
    if (o.at && marker) { marker.position.copy(o.at); marker.material = new THREE.MeshBasicMaterial({ color: o.ok ? 0x1e8e3e : 0xd93025, depthTest: false }); marker.visible = true; }
    if (o.at) ctx.scene.addLabel(o.at.clone().add(new THREE.Vector3(0, 0.01, 0)), o.ok ? '✓ made it' : '✕ ' + (sim?.fell ? 'fell off' : sim?.offEnd ? 'rolled back' : o.text.startsWith('Stopped') ? 'stopped' : 'crashed'));
  }

  async function findMin(ctx: ToolContext) {
    if (!line || !placed) { ctx.toast('Click the track to put the car on it first'); return; }
    const sp = spotOf(placed); if (!sp) return;
    pause(); testing = 'Testing…'; renderHud();
    const r = await minSpeed(line, st, sp.chain, sp.s, st.reverse ? -1 : 1, t => { testing = t; renderReadout(); });
    testing = '';
    const closed = line.chains[sp.chain].closed;
    if (r.min === null) ctx.toast(`It doesn't make it ${closed ? 'round' : 'to the end'} at any launch speed up to 8 m/s${r.why ? `: ${r.why.toLowerCase()}` : ''}`);
    else {
      const need = Math.ceil(r.min * 100) / 100;
      st.speed = r.min === 0 ? 0 : Math.ceil((need + 0.05) * 10) / 10; save();
      ctx.toast(r.min === 0 ? `It rolls ${closed ? 'round' : 'to the end'} on its own from here (gravity is enough)` : `It needs at least ${need.toFixed(2)} m/s (≈ ${scaleKmh(need).toFixed(0)} km/h at scale)${r.max !== null ? `, and goes wrong above ${(r.max + 0.1).toFixed(1)} m/s (${r.why.toLowerCase()})` : ''}. Launch speed set to ${st.speed.toFixed(1)} m/s.`);
    }
    reset(ctx);
  }

  function renderHud() {
    const host = getHost(); const ctx = ctxRef; if (!ctx) { hud.style.display = 'none'; return; }
    hud.style.display = '';
    const has = !!sim;
    ctx.setVCB(String(st.speed), 'Launch (m/s)');
    hud.innerHTML = `<div class="td-row">
      <button class="td-go ${running ? 'on' : ''}" data-go ${has ? '' : 'disabled'} title="${running ? 'Pause (Space)' : 'Launch the car (Space)'}">${running ? '❚❚ Pause' : sim?.mode === 'done' ? '▶ Again' : '▶ Go'}</button>
      <button data-reset ${has ? '' : 'disabled'} title="Back to where you put it (R)">⟲</button>
      <span class="td-sep"></span>
      <label title="Launch speed: how fast the car is pushed off. A launcher gives about 2–4 m/s; 0 lets it roll from rest. Type a number to set it.">Launch <input type="number" min="0" max="15" step="0.1" data-par="speed" value="${st.speed}"> m/s</label>
      <button data-rev class="${st.reverse ? 'on' : ''}" title="Drive the other way along the track (F)">⇄</button>
      <label title="Rolling friction μ: how much speed the car loses rolling (die-cast cars on plastic track ≈ 0.02–0.05). Rubbing the walls in curves costs 4× this.">μ <input type="number" min="0" max="0.5" step="0.005" data-par="friction" value="${st.friction}"></label>
      <select data-slow title="Slow motion">${[1, 0.5, 0.25, 0.1].map(x => `<option value="${x}" ${st.slow === x ? 'selected' : ''}>${x === 1 ? 'Real time' : `${x}×`}</option>`).join('')}</select>
      <button data-follow class="${st.follow ? 'on' : ''}" title="The camera follows the car">Follow</button>
      <button data-min ${has && !testing ? '' : 'disabled'} title="Try launch speeds to find the lowest that gets round the circuit, or to the end of the track, from here">Find min speed</button>
      <span class="td-sep"></span>
      <button data-exit title="Leave test drive (Esc)">✕</button></div>
      <div class="td-read" data-read></div>`;
    hud.querySelector<HTMLElement>('[data-go]')!.onclick = () => running ? pause() : go(ctx);
    hud.querySelector<HTMLElement>('[data-reset]')!.onclick = () => reset(ctx);
    hud.querySelector<HTMLElement>('[data-rev]')!.onclick = () => { st.reverse = !st.reverse; save(); reset(ctx); };
    hud.querySelector<HTMLElement>('[data-follow]')!.onclick = () => { st.follow = !st.follow; save(); renderHud(); };
    hud.querySelector<HTMLElement>('[data-min]')!.onclick = () => findMin(ctx);
    hud.querySelector<HTMLElement>('[data-exit]')!.onclick = () => host.setTool('select');
    hud.querySelector<HTMLSelectElement>('[data-slow]')!.onchange = e => { st.slow = parseFloat((e.target as HTMLSelectElement).value); save(); };
    hud.querySelectorAll<HTMLInputElement>('[data-par]').forEach(i => i.onchange = () => { const v = parseFloat(i.value); if (isFinite(v) && v >= 0) { (st as any)[i.dataset.par!] = v; save(); if (!running) reset(ctx); else if (sim) (sim.par as any)[i.dataset.par!] = v; } });
    renderReadout();
  }
  function renderReadout() {
    const el = hud.querySelector<HTMLElement>('[data-read]'); if (!el) return;
    if (testing) { el.innerHTML = `<span class="muted">${testing}</span>`; return; }
    if (!line?.chains.length) { el.innerHTML = '<span class="muted">No track yet: lay some first.</span>'; return; }
    if (!sim) { el.innerHTML = '<span class="muted">Click the track where the car starts. It drives along the track (⇄ turns it round).</span>'; return; }
    const o = sim.outcome;
    const pc = sim.mode === 'track' ? sim.piece()?.label : sim.mode === 'air' ? 'in the air' : '';
    const spd = sim.mode === 'air' ? sim.vel.length() : Math.abs(sim.v);
    const laps = sim.track().closed ? ` · lap ${sim.laps + 1}${sim.lastLap ? ` (last ${sim.lastLap.toFixed(2)} s)` : ''}` : '';
    const live = `<b>${spd.toFixed(2)} m/s</b> <span class="muted">≈ ${scaleKmh(spd).toFixed(0)} km/h at 1:64</span> · ${sim.mode === 'air' ? '0.0' : sim.g.toFixed(1)} g${sim.time ? ` <span class="muted">(max ${sim.maxG.toFixed(1)})</span>` : ''} · ${sim.time.toFixed(2)} s${laps}${pc ? ` · <span class="muted">${pc}</span>` : ''}`;
    el.innerHTML = o ? `<span class="${o.ok ? 'td-ok' : 'td-bad'}">${o.ok ? '✓' : '✕'} ${o.text}</span>${sim.maxSide > 3 ? ` <span class="td-warn">· hits the wall at ${sim.maxSide.toFixed(1)} g in a curve: a fast, light car may jump out</span>` : ''}` : live;
  }

  const tool: Tool & { busy(): boolean } = {
    id: 'tr-drive', label: 'Test drive', cursor: 'pointer',
    hint: 'Click the track to put a car on it, then Space to launch it. F turns it round, R puts it back, type a number for the launch speed (m/s).',
    icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M2 19h20" stroke="#ff7a1a" stroke-width="3"/><path d="M4 15.5l1.5-3.5h5l2.5-3h4.5l2.5 3.2V15.5z" fill="#d7262e" stroke="#d7262e"/><circle cx="7.5" cy="15.8" r="1.7" fill="#222" stroke="#222"/><circle cx="16.5" cy="15.8" r="1.7" fill="#222" stroke="#222"/></svg>`,
    busy: () => running,
    activate(ctx) {
      ctxRef = ctx; const host = getHost();
      if (!hud.isConnected) host.mount(hud);
      ensureScene(ctx); ctx.setVCB(String(st.speed), 'Launch (m/s)'); ctx.setHint(this.hint);
      load(ctx).then(() => { if (placed) reset(ctx); renderHud(); if (line && !line.chains.length) ctx.toast('No track yet: lay some track first'); });
      renderHud();
    },
    deactivate(ctx) { running = false; cancelAnimationFrame(raf); ctx.scene.clearOverlay(); dropScene(ctx); sim = null; ctxRef = null; hud.style.display = 'none'; },
    onMove(ev, ctx) { if (!running) showGhost(nearest(ctx, ev)); },
    async onDown(ev, ctx) {
      if (ev.button !== 0) return;
      await load(ctx); const sp = nearest(ctx, ev);
      if (!sp) { ctx.toast('Click on the track to put the car there'); return; }
      placed = placedOf(sp); reset(ctx); showGhost(null);
    },
    onKey(ev, ctx) {
      if (ev.type !== 'keydown') return false;
      if (ev.key === ' ') { if (running) pause(); else go(ctx); return true; }
      if (ev.key === 'r' || ev.key === 'R') { reset(ctx); return true; }
      if (ev.key === 'f' || ev.key === 'F') { st.reverse = !st.reverse; save(); reset(ctx); return true; }
      if (ev.key === 'Escape') { if (running) { pause(); return true; } getHost().setTool('select'); return true; }
      return false;
    },
    onVCB(v, ctx) { const n = parseFloat(v); if (isFinite(n) && n >= 0 && n <= 15) { st.speed = n; save(); if (!running) reset(ctx); renderHud(); } },
  };

  /** The document changed (an edit, undo): drive the new track. */
  async function changed() {
    const ctx = ctxRef; if (!ctx || !line || lineVersion === ctx.doc().version) return;
    await load(ctx); reset(ctx);
  }

  /** Headless: drive from a piece (or the first one) and report what happens. For MCP / the WebSocket. */
  async function testDrive(ctx: ToolContext, a: { piece?: string; at?: number; speed?: number; reverse?: boolean; friction?: number; findMin?: boolean }) {
    await load(ctx); if (!line?.chains.length) return { error: 'no track' };
    const ci = a.piece ? chainOf(a.piece) : 0; if (ci < 0) return { error: `no track piece ${a.piece}` };
    const ch = line.chains[ci]; const pc = a.piece ? ch.pieces.find(p => p.id === a.piece)! : ch.pieces[0];
    const s = Math.min(pc.s1, pc.s0 + (a.at ?? 0.03)); const dir = a.reverse ? -1 : 1;
    const par = { ...st, speed: a.speed ?? st.speed, friction: a.friction ?? st.friction };
    const sim = new Sim(line, par); sim.place(ci, s, dir as 1 | -1); const o = sim.runHeadless(60, 1);
    const res: any = { ok: o.ok, result: o.text, time: +sim.time.toFixed(3), distance: +sim.dist.toFixed(3), maxG: +sim.maxG.toFixed(2), maxSideG: +sim.maxSide.toFixed(2), jumps: sim.jumps, events: sim.events, closed: ch.closed };
    if (a.findMin) { const r = await minSpeed(line, par, ci, s, dir as 1 | -1); res.minSpeed = r.min; res.maxSpeed = r.max; res.limit = r.why; }
    return res;
  }

  return { tool, changed, testDrive, busy: () => running };
}

const css = document.createElement('style'); css.textContent = `
.track-drive { position: absolute; left: 50%; bottom: 14px; transform: translateX(-50%); z-index: 6; display: flex; flex-direction: column; gap: 4px; padding: 5px 8px; max-width: calc(100% - 24px);
  background: var(--glass); border: 1px solid var(--glass-border); border-radius: 10px; box-shadow: var(--shadow-md); backdrop-filter: blur(10px) saturate(1.4); font-size: 12px; color: var(--ink); }
.track-drive .td-row { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
.track-drive button { border: 0; background: none; padding: 4px 9px; border-radius: 6px; cursor: default; font-weight: 500; color: var(--ink-2); white-space: nowrap; }
.track-drive button:hover:not(:disabled) { background: var(--surface-3); color: var(--ink); }
.track-drive button:disabled { opacity: .4; }
.track-drive button.on { color: #ff7a1a; background: color-mix(in srgb, #ff7a1a 14%, transparent); }
.track-drive .td-go { background: #ff7a1a; color: #fff; min-width: 74px; }
.track-drive .td-go:hover:not(:disabled) { background: #f06a08; color: #fff; }
.track-drive .td-go.on { background: var(--surface-3); color: var(--ink); }
.track-drive label { display: flex; align-items: center; gap: 4px; color: var(--ink-2); padding: 0 3px; }
.track-drive input[type=number] { width: 62px; }
.track-drive .td-sep { width: 1px; height: 16px; background: var(--line-strong); margin: 0 3px; }
.track-drive .td-read { padding: 0 4px 2px; min-height: 16px; font-variant-numeric: tabular-nums; }
.track-drive .td-ok { color: var(--green); font-weight: 600; }
.track-drive .td-bad { color: var(--red); font-weight: 600; }
.track-drive .td-warn { color: var(--amber); }
.track-drive .muted { color: var(--ink-3); }`;
document.head.appendChild(css);
