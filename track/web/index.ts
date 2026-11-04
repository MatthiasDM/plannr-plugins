// Car Track plugin — browser side: lay 1:64 die-cast track (Hot Wheels-compatible profile) piece by piece,
// edit pieces in Entity Info, add connectors and supports, and send the pieces to the 3D Print plugin.
// Engine side: track/engine (track/README.md). Runtime deps come from window.plannr (no imports).
import type { Tool, ToolContext } from '../../plannr/web/src/tools/base';
import type { Plugin, PluginHost } from '../../plannr/web/src/plugins';
import type { CustomEntity, Document, Entity, Op, Vec3 } from '../../plannr/web/src/model';
import type * as THREE_NS from 'three';
import { makeDrive } from './drive';

const P = (window as any).plannr as { registerPlugin(p: Plugin): void; THREE: typeof THREE_NS; print3d?: { download(format: string, opts: any, name?: string): Promise<any>; settings: { plate: number[]; printer: string } } };
const THREE = P.THREE;
const ACCENT = '#ff7a1a';
const T_PIECE = 'track.piece', T_CONN = 'track.connector', T_SUP = 'track.support';

interface Piece { kind: 'straight' | 'curve' | 'ramp' | 'loop'; start: Vec3; heading: number; pitch: number; length: number; radius: number; angle: number; rise: number; endPitch: number; offset?: number; prev?: string; name?: string }
interface Profile { preset: string; width: number; floor: number; wall: number; wallThickness: number; lead: number; holes: boolean; holeLength: number; holeWidth: number; holeInset: number; connectorThickness: number; clearance: number; segment: number }
interface Info { pieces: { id: string; label: string; kind: string; prev?: string; length: number; sizeMm: [number, number, number]; fits: boolean; maxHeight: number }[]; chains: { pieces: string[]; length: number; closed: boolean; gapMm: number; headingErrorDeg: number }[]; connectors: number; supports: number; warnings: string[] }

const KEY = 'plannr.track';
const state = (() => {
  const d = { straight: 200, radius: 150, angle: 90, rise: 40, rampLength: 300, loopRadius: 90, showInfo: true, profileOpen: false };
  try { return { ...d, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') }; } catch { return d; }
})();
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(state)); } catch { /* storage blocked */ } };

let host: PluginHost;
const drive = makeDrive(() => host, (op, data) => top(op, data), ACCENT);
let info: Info | null = null; let infoVersion = -1; let infoPending = false; let infoPlate = '';
const esc = (s: string) => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]!));
const top = (op: string, data: unknown): Op => ({ op: 'custom', type: `track.${op}`, data } as Op);
const customs = (doc: Document, type: string) => doc.entities.filter((e): e is CustomEntity => e.kind === 'custom' && e.type === type);
const pieceOf = (e: CustomEntity) => e.data as Piece;
const profile = (doc: Document): Profile => ({ preset: 'die-cast', width: 0.042, floor: 0.002, wall: 0.008, wallThickness: 0.002, lead: 0.02, holes: true, holeLength: 0.008, holeWidth: 0.012, holeInset: 0.008, connectorThickness: 0.0016, clearance: 0.0002, segment: 0.008, ...(((doc.extensions?.track as any)?.profile) ?? {}) });
const mm = (m: number) => +(m * 1000).toFixed(1);
const isTrack = (doc: Document, id: string) => doc.entities.some(e => e.id === id && e.kind === 'custom' && e.type.startsWith('track.'));
function label(p: Piece): string {
  if (p.name?.trim()) return p.name;
  switch (p.kind) {
    case 'straight': return `Straight ${mm(p.length)} mm`;
    case 'curve': return `Curve ${Math.abs(p.angle)}° ${p.angle >= 0 ? 'left' : 'right'} R${mm(p.radius)}`;
    case 'ramp': return `Ramp ${p.rise >= 0 ? '+' : ''}${mm(p.rise)} mm`;
    case 'loop': return `Loop R${mm(p.radius)}`;
  }
}
/** The piece a new one chains onto: the selected piece, else the end of the most recent chain. */
function anchor(ctx: ToolContext): string | 'last' | null {
  const doc = ctx.doc(); const sel = [...ctx.selection()].map(id => id.split('#')[0]);
  const picked = customs(doc, T_PIECE).filter(e => sel.includes(e.id));
  if (picked.length === 1) return picked[0].id;
  return customs(doc, T_PIECE).length ? 'last' : null;
}
async function addPiece(ctx: ToolContext, data: Record<string, unknown>, label: string) {
  const after = anchor(ctx);
  try {
    const r = await ctx.ops([top('add_piece', after ? { ...data, after } : data)], label);
    const id = r[0]?.created?.[0]; if (id) ctx.select([id]);
    const w = (r[0]?.entities?.[0]?.data as any)?.warning; if (w) ctx.toast(w);
  } catch (e: any) { ctx.toast(e.message ?? String(e)); }
}
const PALETTE: { id: string; label: string; title: string; data: () => Record<string, unknown>; icon: string }[] = [
  { id: 'straight', label: 'Straight', title: 'A straight of the length below', data: () => ({ kind: 'straight', length: state.straight / 1000 }), icon: '<path d="M4 12h16" stroke-width="4"/>' },
  { id: 'left', label: 'Curve ⟲', title: 'A curve turning left', data: () => ({ kind: 'curve', radius: state.radius / 1000, angle: Math.abs(state.angle) }), icon: '<path d="M5 20V13a8 8 0 0 1 8-8h6" stroke-width="4" fill="none"/>' },
  { id: 'right', label: 'Curve ⟳', title: 'A curve turning right', data: () => ({ kind: 'curve', radius: state.radius / 1000, angle: -Math.abs(state.angle) }), icon: '<path d="M19 20V13a8 8 0 0 0-8-8H5" stroke-width="4" fill="none"/>' },
  { id: 'up', label: 'Ramp up', title: 'Up by the rise below, level at both ends', data: () => ({ kind: 'ramp', length: state.rampLength / 1000, rise: Math.abs(state.rise) / 1000 }), icon: '<path d="M3 19h5c5 0 6-10 11-10h2" stroke-width="4" fill="none"/>' },
  { id: 'down', label: 'Ramp down', title: 'Down by the rise below, level at both ends', data: () => ({ kind: 'ramp', length: state.rampLength / 1000, rise: -Math.abs(state.rise) / 1000 }), icon: '<path d="M3 9h2c5 0 6 10 11 10h5" stroke-width="4" fill="none"/>' },
  { id: 'loop', label: 'Loop', title: 'A loop-the-loop of the radius below; its exit is shifted sideways so it passes its entry', data: () => ({ kind: 'loop', radius: state.loopRadius / 1000 }), icon: '<path d="M2 20h9a6 6 0 1 0-6-6v6m6 0h11" stroke-width="3" fill="none"/>' },
];

// ---- tool ---------------------------------------------------------------------------------------
type V3 = THREE_NS.Vector3;
const GROUND_PLANE = () => ({ origin: new THREE.Vector3(), normal: new THREE.Vector3(0, 1, 0) });
/** Lay track: click the start, click the direction — a straight to that point. */
type LayTool = Tool & { start: V3 | null; shown: string; lastDir: V3 | null; op(end: V3, ctx: ToolContext, length?: number): Op | null; place(op: Op, ctx: ToolContext): Promise<void> };
const layTool: LayTool = {
  id: 'tr-lay', label: 'Lay track', cursor: 'crosshair', start: null, shown: '', lastDir: null,
  hint: 'Click where the track starts, then click where the first straight ends (or type its length in mm). Then add curves, ramps and loops from the Car Track panel: each one goes on the end of the selected piece.',
  icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round"><path d="M3 17 21 7" stroke="#ff7a1a" stroke-width="5"/><path d="M3 17 21 7" stroke="#ffd2ad" stroke-width="1.4"/><circle cx="3" cy="17" r="2" fill="#fff"/></svg>`,
  activate(ctx) { this.start = null; ctx.setHint(this.hint); ctx.setVCB('', 'Length (mm)'); },
  deactivate(ctx) { this.start = null; this.shown = ''; ctx.preview([]); ctx.scene.clearOverlay(); },
  onMove(ev, ctx) {
    const s = ctx.snap(ev, { plane: GROUND_PLANE(), from: this.start ?? undefined, lock: ctx.lock, ignoreIf: id => !ctx.inDocument(id) });
    ctx.scene.clearOverlay(); ctx.scene.addDot(s.point, 0xff7a1a, 8);
    if (!this.start) return;
    const d = s.point.clone().sub(this.start).setY(0); if (d.length() > 1e-3) this.lastDir = d.normalize();
    const op = this.op(s.point, ctx); if (!op) return;
    const L = (op as any).data.length as number;
    ctx.scene.addLabel(this.start.clone().lerp(s.point, 0.5), `${(L * 1000).toFixed(0)} mm`);
    const k = JSON.stringify(op); if (k !== this.shown) { this.shown = k; ctx.preview([op]); }
  },
  op(end, ctx, length) {
    if (!this.start) return null; const d = end.clone().sub(this.start); d.y = 0; if (d.length() < 0.01 && !length) return null;
    const L = length ?? Math.max(0.05, Math.round(d.length() * 100) / 100); // 10 mm steps
    const heading = d.length() < 1e-6 ? 0 : +(Math.atan2(-d.z, d.x) * 180 / Math.PI).toFixed(3);
    const y = this.start.y;
    void ctx; return top('add_piece', { kind: 'straight', start: [+this.start.x.toFixed(4), +y.toFixed(4), +this.start.z.toFixed(4)], heading, length: L });
  },
  async onDown(ev, ctx) {
    if (ev.button !== 0) return;
    const s = ctx.snap(ev, { plane: GROUND_PLANE(), from: this.start ?? undefined, lock: ctx.lock, ignoreIf: id => !ctx.inDocument(id) });
    if (!this.start) { this.start = s.point.clone(); return; }
    const op = this.op(s.point, ctx); if (!op) return;
    await this.place(op, ctx);
  },
  async place(op, ctx) {
    this.start = null; this.shown = ''; ctx.preview([]); ctx.scene.clearOverlay();
    const r = await ctx.ops([op], 'Lay track'); const id = r[0]?.created?.[0]; if (id) ctx.select([id]);
    ctx.setTool('select'); ctx.toast('Track started. Add pieces from the Car Track panel (each goes on the end of the selected piece).');
  },
  onVCB(v, ctx) {
    const n = parseFloat(v); if (!this.start || !(n > 0)) return;
    const end = this.start.clone().add(this.lastDir ?? new THREE.Vector3(1, 0, 0));
    const op = this.op(end, ctx, n / 1000); if (op) this.place(op, ctx);
  },
  onKey(ev, ctx) { if (ev.key === 'Escape' && this.start) { this.start = null; this.shown = ''; ctx.preview([]); ctx.scene.clearOverlay(); return true; } return false; },
};

// ---- panel --------------------------------------------------------------------------------------
async function refreshInfo(ctx: ToolContext, force = false) {
  const v = ctx.doc().version; const pk = JSON.stringify(plateOf());
  if (infoPending || (!force && infoVersion === v && infoPlate === pk)) return;
  if (!customs(ctx.doc(), T_PIECE).length) { info = null; infoVersion = v; infoPlate = pk; return; }
  infoPending = true;
  try { const r = await ctx.ops([top('info', { plate: plateOf() })], 'Track info'); info = (r[0]?.entities?.[0]?.data as Info) ?? null; infoVersion = v; infoPlate = pk; } catch { info = null; }
  infoPending = false; host.refresh();
  // the document moved on while we asked: ask again for the current one
  if (ctx.doc().version !== v) refreshInfo(ctx);
}
const plateOf = (): number[] => (P.print3d?.settings as any) ? (() => { const s: any = P.print3d!.settings; const known: Record<string, number[]> = { bambu: [256, 256, 256], 'bambu-h2d': [325, 320, 325], 'bambu-mini': [180, 180, 180], 'prusa-mk4': [250, 210, 220], 'prusa-core': [250, 220, 270], 'prusa-mini': [180, 180, 180], 'prusa-xl': [360, 360, 360], ender3: [220, 220, 250], k1: [220, 220, 250], k1max: [300, 300, 300], 'elegoo-neptune4': [225, 225, 265], voron350: [350, 350, 340] }; return s.printer === 'custom' ? s.plate : known[s.printer] ?? [256, 256, 256]; })() : [256, 256, 256];
const fits = (s: number[], pl: number[]) => s[2] <= pl[2] && ((s[0] <= pl[0] && s[1] <= pl[1]) || (s[1] <= pl[0] && s[0] <= pl[1]));

function panel(ctx: ToolContext, el: HTMLElement) {
  const doc = ctx.doc(); const prof = profile(doc); const ps = customs(doc, T_PIECE);
  refreshInfo(ctx);
  const a = anchor(ctx); const anchorPiece = a && a !== 'last' ? ps.find(p => p.id === a) : null;
  const pl = plateOf();
  const num = (k: string, v: number, lbl: string, step = 1, unit = 'mm') => `<label>${lbl}</label><div class="row"><input type="number" step="${step}" data-st="${k}" value="${v}" style="width:72px"><span class="muted" style="align-self:center">${unit}</span></div>`;
  const pf = (k: keyof Profile, lbl: string) => `<label>${lbl}</label><div class="row"><input type="number" step="0.1" data-pf="${k}" value="${mm(prof[k] as number)}" style="width:72px"><span class="muted" style="align-self:center">mm</span></div>`;
  el.innerHTML = `
    <div class="muted" style="margin:4px 0 6px;font-size:11px">${ps.length ? (anchorPiece ? `Next piece goes after <b>${esc(label(pieceOf(anchorPiece)))}</b>` : 'Next piece goes on the end of the last chain') : 'No track yet: the first piece starts at the origin, heading along red (or use Lay track)'}</div>
    <div class="track-palette" style="display:grid;grid-template-columns:repeat(3,1fr);gap:4px">${PALETTE.map(p => `<button class="btn" data-add="${p.id}" title="${esc(p.title)}" style="justify-content:center;flex-direction:column;padding:6px 2px;gap:2px"><svg viewBox="0 0 24 24" width="22" height="22" stroke="${ACCENT}" stroke-linecap="round">${p.icon}</svg><span style="font-size:11px">${p.label}</span></button>`).join('')}</div>
    <div class="kv" style="margin-top:8px">${num('straight', state.straight, 'Straight length', 10)}${num('radius', state.radius, 'Curve radius', 5)}${num('angle', state.angle, 'Curve angle', 15, '°')}${num('rise', state.rise, 'Ramp rise', 5)}${num('rampLength', state.rampLength, 'Ramp length', 10)}${num('loopRadius', state.loopRadius, 'Loop radius', 5)}</div>
    <div class="btn-row"><button class="btn" data-act="drive" title="Put a car on the track and see whether it makes it round: loops, ramps, jumps (Test drive tool)" ${ps.length ? '' : 'disabled'}>▶ Test drive</button></div>
    <div class="btn-row"><button class="btn" data-act="connectors" title="A connector under every joint between pieces (prints separately, in blue)">+ Connectors</button><button class="btn" data-act="supports" title="Supports under raised track every 15 cm">+ Supports</button><button class="btn primary" data-act="print" title="Download every piece, connector and support as one 3MF, laid out on the plate">Print all (3MF)</button></div>
    ${info ? infoHtml(info, pl) : ''}
    <details style="margin-top:10px" ${state.profileOpen ? 'open' : ''} data-details><summary class="muted" style="font-size:11px;cursor:default">Track profile · ${esc(prof.preset)} · ${mm(prof.width)} mm wide</summary>
      <div class="kv" style="margin-top:6px"><label>Preset</label><select data-preset><option value="die-cast" ${prof.preset === 'die-cast' ? 'selected' : ''}>1:64 die-cast (orange track)</option><option value="wide" ${prof.preset === 'wide' ? 'selected' : ''}>Wide &amp; deep</option><option value="sturdy" ${prof.preset === 'sturdy' ? 'selected' : ''}>Sturdy</option>${prof.preset === 'custom' ? '<option value="custom" selected>Custom</option>' : ''}</select>
        ${pf('width', 'Width')}${pf('floor', 'Floor')}${pf('wall', 'Wall height')}${pf('wallThickness', 'Wall thickness')}${pf('lead', 'Straight ends')}
        <label>Connector holes</label><input type="checkbox" data-pf="holes" ${prof.holes ? 'checked' : ''}>
        ${pf('holeLength', 'Hole length')}${pf('holeWidth', 'Hole width')}${pf('holeInset', 'Hole from end')}${pf('connectorThickness', 'Connector plate')}${pf('clearance', 'Peg clearance')}</div>
      <div class="muted" style="font-size:11px;margin-top:4px">The defaults follow the orange 1:64 track (≈ 42 mm wide). Measure a piece of yours with calipers and print a short straight and a connector first to check the fit.</div></details>`;
  el.querySelectorAll<HTMLElement>('[data-add]').forEach(b => b.onclick = () => { const p = PALETTE.find(x => x.id === b.dataset.add)!; addPiece(ctx, p.data(), `Add ${p.label.toLowerCase()}`); });
  el.querySelectorAll<HTMLInputElement>('[data-st]').forEach(i => i.onchange = () => { const v = parseFloat(i.value); if (isFinite(v)) { (state as any)[i.dataset.st!] = v; save(); } });
  el.querySelector<HTMLElement>('[data-details]')!.addEventListener('toggle', e => { state.profileOpen = (e.target as HTMLDetailsElement).open; save(); });
  el.querySelector<HTMLSelectElement>('[data-preset]')!.onchange = e => { const v = (e.target as HTMLSelectElement).value; if (v !== 'custom') ctx.ops([top('set_profile', { preset: v })], 'Track profile').catch(x => ctx.toast(x.message)); };
  el.querySelectorAll<HTMLInputElement>('[data-pf]').forEach(i => i.onchange = () => { const k = i.dataset.pf!; const v = i.type === 'checkbox' ? i.checked : parseFloat(i.value) / 1000; ctx.ops([top('set_profile', { [k]: v })], 'Track profile').catch(x => ctx.toast(x.message)); });
  el.querySelector<HTMLElement>('[data-act="connectors"]')!.onclick = async () => { const r = await ctx.ops([top('add_connectors', {})], 'Add connectors'); const n = r[0]?.created?.length ?? 0; ctx.toast(n ? `${n} connector${n === 1 ? '' : 's'} added under the joints` : 'Every joint already has a connector'); };
  el.querySelector<HTMLElement>('[data-act="supports"]')!.onclick = async () => { const r = await ctx.ops([top('auto_supports', {})], 'Add supports'); const n = r[0]?.created?.length ?? 0; ctx.toast(n ? `${n} support${n === 1 ? '' : 's'} under the raised track` : 'No track is raised enough to need supports'); };
  el.querySelector<HTMLElement>('[data-act="print"]')!.onclick = () => printAll(ctx);
  el.querySelector<HTMLElement>('[data-act="drive"]')!.onclick = () => ctx.setTool('tr-drive');
  el.querySelector<HTMLElement>('[data-act="split"]')?.addEventListener('click', async () => {
    try { const r = await ctx.ops([top('split_to_fit', { plate: plateOf() })], 'Split to fit the printer'); const n = r[0]?.created?.filter(id => customs(ctx.doc(), T_PIECE).some(p => p.id === id)).length ?? 0; const w = (r[0]?.entities?.[0]?.data as any)?.warning; ctx.toast(w ?? (n ? `${n} piece${n === 1 ? '' : 's'} added by splitting` : 'Everything fits already')); }
    catch (e: any) { ctx.toast(e.message ?? String(e)); }
  });
  el.querySelectorAll<HTMLElement>('[data-piece]').forEach(r => r.onclick = () => ctx.select([r.dataset.piece!]));
}
function infoHtml(i: Info, pl: number[]) {
  const byId = new Map(i.pieces.map(p => [p.id, p]));
  const chains = i.chains.map((c, n) => `<div class="muted" style="margin:10px 0 4px;font-size:11px">Track ${i.chains.length > 1 ? n + 1 : ''} · ${c.pieces.length} piece${c.pieces.length === 1 ? '' : 's'} · ${(c.length * 100).toFixed(0)} cm${c.closed ? ' · <b style="color:#1e8e3e">closed circuit</b>' : c.pieces.length > 2 && c.gapMm < 300 ? ` · ends ${c.gapMm.toFixed(0)} mm from the start` : ''}</div>` +
    c.pieces.map(id => { const p = byId.get(id)!; const ok = p.fits ?? fits(p.sizeMm, pl); return `<div class="list-row" data-piece="${id}" title="${ok ? 'Fits' : 'Too big for'} the ${pl[0]}×${pl[1]} mm plate"><span class="swatch" style="background:${ACCENT}"></span><span class="name">${ok ? '' : '⚠ '}${esc(p.label)} <span class="muted">${p.sizeMm.map(x => x.toFixed(0)).join('×')} mm</span></span></div>`; }).join('')).join('');
  const big = i.pieces.filter(p => !(p.fits ?? fits(p.sizeMm, pl))).length;
  return chains + (big ? `<div style="margin-top:6px;font-size:11px;color:#b06d00">⚠ ${big} piece${big === 1 ? ' is' : 's are'} too big for the ${pl[0]}×${pl[1]}×${pl[2]} mm printer <button class="btn" data-act="split" title="Split them into shorter pieces that fit, keeping the track's shape (closed circuits stay closed)">Split to fit</button></div>` : '') + `<div class="muted" style="font-size:11px;margin-top:6px">${i.connectors} connectors · ${i.supports} supports · printer from the 3D Print panel</div>` + i.warnings.map(w => `<div class="muted" style="font-size:11px;margin-top:4px;color:#b06d00">⚠ ${esc(w)}</div>`).join('');
}
async function printAll(ctx: ToolContext) {
  const doc = ctx.doc(); const ids = doc.entities.filter(e => e.kind === 'custom' && e.type.startsWith('track.')).map(e => e.id);
  if (!ids.length) { ctx.toast('No track to print yet'); return; }
  if (!P.print3d) { ctx.toast('The 3D Print plugin is not loaded'); return; }
  try {
    const r = await P.print3d.download('3mf', { ids, scale: 1, arrange: true, merge: false }, 'track.3mf');
    if (r) ctx.toast(`track.3mf saved · ${r.objects.length} parts${r.plates > 1 ? ` on ${r.plates} plates` : ''}${r.warnings.length ? ` · ${r.warnings.length} warning${r.warnings.length === 1 ? '' : 's'} (see the 3D Print panel)` : ' · ready to slice'}`);
  } catch (e: any) { ctx.toast(`Export failed: ${e.message ?? e}`); }
}

// ---- Entity Info --------------------------------------------------------------------------------
function renderEntity(ctx: ToolContext, e: Entity, el: HTMLElement): boolean {
  if (e.kind !== 'custom' || !e.type.startsWith('track.')) return false;
  const doc = ctx.doc();
  if (e.type !== T_PIECE) {
    const d: any = e.data; const host = customs(doc, T_PIECE).find(p => p.id === d.piece);
    el.innerHTML = `<div class="kv"><label>Type</label><div><b>${e.type === T_CONN ? 'Connector' : 'Support'}</b></div><label>Under</label><div>${host ? esc(label(pieceOf(host))) : '—'}${e.type === T_CONN ? ` (${d.at === 'end' ? 'end' : 'start'})` : ` at ${mm(d.at)} mm`}</div></div>
      <div class="muted" style="font-size:11px;margin-top:6px">${e.type === T_CONN ? 'Prints on its own (blue); slide it under the joint so its pegs go up into both pieces.' : 'Follows the piece. Print it standing on its foot.'}</div>
      <div class="btn-row">${host ? '<button class="btn" data-host>Select piece</button>' : ''}<button class="btn" data-erase>Erase</button></div>`;
    el.querySelector<HTMLElement>('[data-host]')?.addEventListener('click', () => ctx.select([d.piece]));
    el.querySelector<HTMLElement>('[data-erase]')!.onclick = () => ctx.ops([{ op: 'erase', ids: [e.id] }], 'Erase').then(() => ctx.select([]));
    return true;
  }
  const p = pieceOf(e); const prev = p.prev ? customs(doc, T_PIECE).find(x => x.id === p.prev) : null;
  const n = (k: string, v: number, lbl: string, step = 1, unit = 'mm', scale = 1000) => `<label>${lbl}</label><div class="row"><input type="number" step="${step}" data-f="${k}" data-scale="${scale}" value="${+(v * scale).toFixed(2)}" style="width:80px"><span class="muted" style="align-self:center">${unit}</span></div>`;
  let body = `<div class="kv"><label>Type</label><select data-f="kind">${['straight', 'curve', 'ramp', 'loop'].map(k => `<option value="${k}" ${p.kind === k ? 'selected' : ''}>${k[0].toUpperCase() + k.slice(1)}</option>`).join('')}</select>
    <label>Name</label><input type="text" data-f="name" value="${esc(p.name ?? '')}" placeholder="${esc(label({ ...p, name: undefined }))}">`;
  if (p.kind === 'straight') body += n('length', p.length, 'Length', 10);
  if (p.kind === 'curve') body += n('radius', p.radius, 'Radius', 5) + n('angle', Math.abs(p.angle), 'Angle', 15, '°', 1) + `<label>Turns</label><select data-f="direction"><option value="left" ${p.angle >= 0 ? 'selected' : ''}>left</option><option value="right" ${p.angle < 0 ? 'selected' : ''}>right</option></select>`;
  if (p.kind === 'ramp') body += n('length', p.length, 'Length', 10) + n('rise', p.rise, 'Rise', 5) + n('endPitch', p.endPitch, 'End pitch', 5, '°', 1);
  if (p.kind === 'loop') body += n('radius', p.radius, 'Radius', 5) + n('offset', p.offset ?? profile(doc).width + 0.004, 'Side shift', 1);
  body += `<label>Follows</label><div>${prev ? `<a href="#" data-prev>${esc(label(pieceOf(prev)))}</a>` : '<span class="muted">nothing (placed on its own)</span>'}</div>`;
  if (!prev) body += n('heading', p.heading, 'Heading', 15, '°', 1) + n('pitch', p.pitch, 'Pitch', 5, '°', 1);
  body += `</div><div class="muted" style="font-size:11px;margin-top:6px">Pieces after it follow when it changes. The pieces in the Car Track panel go on the end of the selected piece.</div>
    <div class="btn-row"><button class="btn" data-remove title="Remove it; the next piece joins up with the one before">Remove &amp; close gap</button>${prev ? '<button class="btn" data-detach title="Keep it where it is, no longer following the piece before">Detach</button>' : ''}<button class="btn" data-support>+ Support here</button></div>`;
  el.innerHTML = body;
  el.querySelectorAll<HTMLInputElement | HTMLSelectElement>('[data-f]').forEach(inp => inp.onchange = () => {
    const k = inp.dataset.f!; let v: any = inp.value;
    if (inp.type === 'number') { v = parseFloat(inp.value) / parseFloat(inp.dataset.scale ?? '1'); if (!isFinite(v)) return; }
    const patch: any = k === 'angle' ? { angle: p.angle < 0 ? -Math.abs(v) : Math.abs(v) } : k === 'direction' ? { direction: v } : k === 'name' ? { name: v || null } : { [k]: v };
    ctx.ops([top('update_piece', { id: e.id, patch })], 'Track piece').catch(x => ctx.toast(x.message));
  });
  el.querySelector<HTMLElement>('[data-prev]')?.addEventListener('click', ev => { ev.preventDefault(); ctx.select([p.prev!]); });
  el.querySelector<HTMLElement>('[data-remove]')!.onclick = () => ctx.ops([top('remove_pieces', { ids: [e.id] })], 'Remove track piece').then(() => ctx.select([]));
  el.querySelector<HTMLElement>('[data-detach]')?.addEventListener('click', () => ctx.ops([top('update_piece', { id: e.id, patch: { prev: null } })], 'Detach piece'));
  el.querySelector<HTMLElement>('[data-support]')!.onclick = () => ctx.ops([top('add_support', { piece: e.id, t: 0.5 })], 'Add support').catch(x => ctx.toast(x.message));
  return true;
}

P.registerPlugin({
  id: 'track', name: 'Car Track', version: '0.2.0', accent: ACCENT,
  tools: [layTool, drive.tool], toolbar: [['tr-lay', 'tr-drive']],
  panels: [{ id: 'main', title: 'Car Track', render: panel }],
  renderEntity,
  /** Moving a whole chain: only the pieces whose predecessor stays put move, the rest follow them. */
  beforeMove(ids, delta, ctx) {
    const doc = ctx.doc(); if (!ids.some(id => isTrack(doc, id))) return null;
    const set = new Set(ids);
    const keep = ids.filter(id => { const e = doc.entities.find(x => x.id === id); if (!e || e.kind !== 'custom') return true; if (e.type === T_PIECE) { const pv = (e.data as Piece).prev; return !(pv && set.has(pv)); } return !(e.type === T_CONN || e.type === T_SUP); });
    return keep.length ? [{ op: 'move', ids: keep, delta }] : [];
  },
  activate(h) {
    host = h; h.on('selection', () => h.refresh()); h.on('changed', () => { if (infoVersion !== h.document().version) refreshInfo(h.ctx); void drive.changed(); });
    // headless test drive for MCP / the WebSocket: { piece?, at?, speed?, reverse?, friction?, findMin? }
    h.registerCommand?.('track_test_drive', a => drive.testDrive(h.ctx, a ?? {}));
  },
});
