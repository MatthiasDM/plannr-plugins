// 3D Print plugin — browser side: a tray panel that checks the selection (or the whole model) for printing
// and downloads it as 3MF or STL in millimetres for a slicer. The engine side (plugins/print3d/engine)
// welds, repairs and checks the meshes; it is served at GET /api/projects/:id/export/{3mf,stl}.
import type { ToolContext } from '../../plannr/web/src/tools/base';
import type { Plugin, PluginHost } from '../../plannr/web/src/plugins';

const P = (window as any).plannr as { registerPlugin(p: Plugin): void; app?: { projectId?: string | null; projectName?: string } };
const ACCENT = '#7a4fd6';

/** Build volumes in mm (width × depth × height). */
const PRINTERS: [string, string, [number, number, number]][] = [
  ['bambu', 'Bambu Lab X1 / P1 / A1', [256, 256, 256]], ['bambu-h2d', 'Bambu Lab H2D', [325, 320, 325]], ['bambu-mini', 'Bambu Lab A1 mini', [180, 180, 180]],
  ['prusa-mk4', 'Prusa MK4 / MK3', [250, 210, 220]], ['prusa-core', 'Prusa CORE One', [250, 220, 270]], ['prusa-mini', 'Prusa MINI', [180, 180, 180]], ['prusa-xl', 'Prusa XL', [360, 360, 360]],
  ['ender3', 'Creality Ender-3 V3', [220, 220, 250]], ['k1', 'Creality K1 / K1C', [220, 220, 250]], ['k1max', 'Creality K1 Max', [300, 300, 300]],
  ['elegoo-neptune4', 'Elegoo Neptune 4', [225, 225, 265]], ['elegoo-cc2', 'Elegoo Centauri Carbon 2 / 2 Combo', [256, 256, 256]], ['voron350', 'Voron 2.4 350', [350, 350, 340]], ['custom', 'Custom…', [200, 200, 200]],
];
const SCALES: [string, number][] = [['1:1 (real size)', 1], ['2:1', 2], ['1:2', 0.5], ['1:5', 0.2], ['1:10', 0.1], ['1:20', 0.05], ['1:43', 1 / 43], ['1:50', 0.02], ['1:64', 1 / 64], ['1:87 (H0)', 1 / 87], ['1:100', 0.01], ['1:200', 0.005], ['1:500', 0.002]];

interface Settings { scope: 'selection' | 'all'; scale: number; printer: string; plate: [number, number, number]; arrange: boolean; merge: boolean; orient: boolean }
const KEY = 'plannr.print3d';
const load = (): Settings => {
  const d: Settings = { scope: 'selection', scale: 1, printer: 'bambu', plate: [256, 256, 256], arrange: true, merge: false, orient: true };
  try { return { ...d, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') }; } catch { return d; }
};
const st = load();
const save = () => { try { localStorage.setItem(KEY, JSON.stringify(st)); } catch { /* storage blocked */ } };
const plate = (): [number, number, number] => st.printer === 'custom' ? st.plate : (PRINTERS.find(p => p[0] === st.printer)?.[2] ?? [256, 256, 256]);

interface Orientation { down: [number, number, number]; turned: boolean; supportMm2: number; baseMm2: number; keptSupportMm2: number }
interface ObjReport { id: string; name: string; size: [number, number, number]; triangles: number; shells: number; watertight: boolean; openEdges: number; nonManifoldEdges: number; surfaces: number; volumeCm3: number; fits: boolean; plate: number; color?: string | null; orientation?: Orientation; repaired: Record<string, number> }
interface Report { format: string; scale: number; plate: number[]; objects: ObjReport[]; size: number[]; watertight: boolean; volumeCm3: number; solidGramsPla: number; plates: number; warnings: string[]; notes?: string[]; oriented?: boolean; triangles: number }

let host: PluginHost;
let report: Report | null = null; let reportKey = ''; let busy = false; let error = '';
const esc = (s: string) => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]!));
const owner = (): string => new URLSearchParams(location.search).get('owner') ?? (() => { try { return localStorage.getItem('plannr.owner') ?? ''; } catch { return ''; } })();
const projectId = (): string | null => P.app?.projectId ?? new URLSearchParams(location.search).get('project');

/** Top-level ids the export takes: the selection (Select resolves parts to their object) or everything. */
function ids(ctx: ToolContext): string[] { return st.scope === 'selection' ? [...new Set([...ctx.selection()].map(id => ctx.resolve(id.split('#')[0])))] : []; }

export function exportUrl(format: string, opts: { ids?: string[]; scale?: number; plate?: number[]; arrange?: boolean; merge?: boolean; orient?: boolean; report?: boolean }): string | null {
  const pid = projectId(); if (!pid) return null;
  const q = new URLSearchParams();
  if (opts.ids?.length) q.set('ids', opts.ids.join(','));
  const pl = opts.plate ?? plate();
  q.set('scale', String(opts.scale ?? st.scale)); q.set('plateWidth', String(pl[0])); q.set('plateDepth', String(pl[1])); q.set('plateHeight', String(pl[2]));
  q.set('arrange', String(opts.arrange ?? st.arrange)); q.set('merge', String(opts.merge ?? st.merge)); q.set('orient', String(opts.orient ?? st.orient));
  if (opts.report) q.set('report', '1');
  const o = owner(); if (o) q.set('owner', o);
  return `/api/projects/${pid}/export/${format}?${q}`;
}
async function fetchJson(url: string) {
  const r = await fetch(url, { headers: { 'X-Plannr-Owner': owner() } }); const t = await r.text(); let j: any; try { j = JSON.parse(t); } catch { j = { error: t }; }
  if (!r.ok) throw new Error(j?.error ?? `${r.status}`); return j;
}
/** Download a print file; returns its report (from the response header) or throws. */
export async function download(format: string, opts: Parameters<typeof exportUrl>[1], name?: string) {
  const url = exportUrl(format, opts); if (!url) throw new Error('no project open');
  const r = await fetch(url, { headers: { 'X-Plannr-Owner': owner() } });
  if (!r.ok) { const t = await r.text(); let m = t; try { m = JSON.parse(t).error ?? t; } catch { /* plain text */ } throw new Error(m); }
  const blob = await r.blob();
  const cd = r.headers.get('content-disposition') ?? ''; const fname = name ?? cd.match(/filename="([^"]+)"/)?.[1] ?? `model.${format}`;
  const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = fname; document.body.appendChild(a); a.click(); a.remove(); setTimeout(() => URL.revokeObjectURL(a.href), 10000);
  try { return JSON.parse(r.headers.get('x-plannr-report') ?? 'null') as Report | null; } catch { return null; }
}

async function check(ctx: ToolContext) {
  const sel = ids(ctx);
  if (st.scope === 'selection' && !sel.length) { report = null; error = 'Select what to print, or switch to the whole model.'; host.refresh(); return; }
  const url = exportUrl('3mf', { ids: sel, report: true }); if (!url) return;
  busy = true; error = ''; host.refresh();
  try { report = await fetchJson(url); reportKey = key(ctx); } catch (e: any) { report = null; error = e.message ?? String(e); }
  busy = false; host.refresh();
}
const key = (ctx: ToolContext) => JSON.stringify([ctx.doc().version, ids(ctx), st.scale, plate(), st.arrange, st.merge, st.orient, st.scope]);
const mm = (v: number[]) => v.map(x => x >= 100 ? x.toFixed(0) : x.toFixed(1)).join(' × ') + ' mm';

function panel(ctx: ToolContext, el: HTMLElement) {
  const sel = ids(ctx); const pl = plate(); const stale = report && reportKey !== key(ctx);
  const scaleKnown = SCALES.some(s => Math.abs(s[1] - st.scale) < 1e-9);
  el.innerHTML = `
    <div class="kv"><label>Print</label><select data-s="scope"><option value="selection" ${st.scope === 'selection' ? 'selected' : ''}>the selection (${sel.length})</option><option value="all" ${st.scope === 'all' ? 'selected' : ''}>the whole model</option></select>
      <label>Scale</label><select data-s="scale">${SCALES.map(([l, v]) => `<option value="${v}" ${Math.abs(v - st.scale) < 1e-9 ? 'selected' : ''}>${l}</option>`).join('')}${scaleKnown ? '' : `<option value="${st.scale}" selected>× ${st.scale}</option>`}<option value="custom">Custom…</option></select>
      <label>Printer</label><select data-s="printer">${PRINTERS.map(([k, l, v]) => `<option value="${k}" ${st.printer === k ? 'selected' : ''}>${l}${k === 'custom' ? '' : ` · ${v[0]}×${v[1]}×${v[2]}`}</option>`).join('')}</select>
      ${st.printer === 'custom' ? `<label>Build volume</label><div class="row">${[0, 1, 2].map(i => `<input type="number" min="20" step="1" data-plate="${i}" value="${st.plate[i]}" style="width:56px">`).join('')}<span class="muted" style="align-self:center">mm</span></div>` : ''}
      <label>Lay out</label><div class="row" style="align-items:center;gap:6px"><input type="checkbox" data-s="arrange" ${st.arrange ? 'checked' : ''}><span style="font-size:12px">side by side on the plate</span></div>
      ${st.arrange ? `<label>Stand</label><div class="row" style="align-items:center;gap:6px"><input type="checkbox" data-s="orient" ${st.orient ? 'checked' : ''}><span style="font-size:12px" title="A U-shaped track piece printed opening-up needs no support; on its side the whole floor hangs in the air. Each object is turned onto the side that prints best.">each object the way it prints best</span></div>` : ''}
      <label>Objects</label><div class="row" style="align-items:center;gap:6px"><input type="checkbox" data-s="merge" ${st.merge ? 'checked' : ''}><span style="font-size:12px">merge into one</span></div></div>
    <div class="btn-row"><button class="btn" data-act="check" ${busy ? 'disabled' : ''}>${busy ? 'Checking…' : stale ? 'Check again' : 'Check'}</button><button class="btn primary" data-act="3mf" title="3MF: millimetres, one object per group / component / track piece, with colours. Best for Bambu Studio, PrusaSlicer, OrcaSlicer, Cura">Download 3MF</button><button class="btn" data-act="stl" title="Binary STL in millimetres, Z up, all objects in one file">STL</button></div>
    ${error ? `<div class="empty" style="color:#d93025">${esc(error)}</div>` : ''}
    ${report ? reportHtml(report, !!stale, pl) : `<div class="muted" style="font-size:11px;margin-top:6px">Every group, component and plugin object (a track piece…) becomes its own object in the slicer. Faces need thickness: Push/Pull a face into a solid before printing it.</div>`}`;
  el.querySelectorAll<HTMLInputElement | HTMLSelectElement>('[data-s]').forEach(inp => inp.onchange = async () => {
    const k = inp.dataset.s as keyof Settings;
    if (k === 'scale') {
      if (inp.value === 'custom') { const v = await host.prompt('Print size ÷ model size (e.g. 0.01 for 1:100, or type 1:100)', String(st.scale)); if (v) { const m = v.match(/^\s*(\d*\.?\d+)\s*:\s*(\d*\.?\d+)\s*$/); const n = m ? parseFloat(m[1]) / parseFloat(m[2]) : parseFloat(v); if (n > 0 && isFinite(n)) st.scale = n; } }
      else st.scale = parseFloat(inp.value);
    } else if (k === 'arrange' || k === 'merge' || k === 'orient') (st as any)[k] = (inp as HTMLInputElement).checked;
    else (st as any)[k] = inp.value;
    save(); host.refresh(); if (report) check(ctx);
  });
  el.querySelectorAll<HTMLInputElement>('[data-plate]').forEach(inp => inp.onchange = () => { const v = parseFloat(inp.value); if (v > 0) { st.plate[+inp.dataset.plate!] = v; save(); host.refresh(); } });
  el.querySelector<HTMLElement>('[data-act="check"]')!.onclick = () => check(ctx);
  for (const f of ['3mf', 'stl']) el.querySelector<HTMLElement>(`[data-act="${f}"]`)!.onclick = async () => {
    const s = ids(ctx); if (st.scope === 'selection' && !s.length) { ctx.toast('Select what to print, or switch to the whole model'); return; }
    try { const r = await download(f, { ids: s }); if (r) { report = r; reportKey = key(ctx); host.refresh(); const w = r.warnings.length; ctx.toast(`${f.toUpperCase()} saved · ${mm(r.size)} · ${r.objects.length} object${r.objects.length === 1 ? '' : 's'}${w ? ` · ${w} warning${w === 1 ? '' : 's'}` : ' · ready to slice'}`); } }
    catch (e: any) { ctx.toast(`Export failed: ${e.message ?? e}`); }
  };
  el.querySelectorAll<HTMLElement>('[data-obj]').forEach(r => r.onclick = () => { const id = r.dataset.obj!; if (id && !id.startsWith('__') && id !== 'model') ctx.select([id]); });
}

function reportHtml(r: Report, stale: boolean, pl: number[]) {
  const ok = r.watertight && r.objects.every(o => o.fits);
  const rep = (o: ObjReport) => { const x = o.repaired; const n = (x.internalFaces ?? 0) + (x.tJunctions ?? 0) + (x.duplicates ?? 0); return n ? ` · repaired ${n}` : ''; };
  const turn = (o: ObjReport) => { const t = o.orientation; if (!t) return ''; const s = t.supportMm2 >= 1 ? ` · ${t.supportMm2.toFixed(0)} mm² support` : ' · no support'; return (t.turned ? ' · turned' : '') + s; };
  return `<div style="margin-top:8px;padding:8px;border-radius:6px;background:${ok ? 'rgba(30,142,62,.10)' : 'rgba(242,183,5,.14)'};${stale ? 'opacity:.6' : ''}">
      <div><b>${ok ? '✓ Ready to slice' : '⚠ Check the warnings'}</b>${stale ? ' <span class="muted">(model changed)</span>' : ''}</div>
      <div class="muted" style="font-size:11px;margin-top:2px">${r.objects.length} object${r.objects.length === 1 ? '' : 's'} · ${mm(r.size)} on ${pl[0]}×${pl[1]} mm${r.plates > 1 ? ` · ${r.plates} plates` : ''} · ${r.volumeCm3} cm³ ≈ ${r.solidGramsPla} g PLA solid</div></div>
    ${r.objects.map(o => `<div class="list-row" data-obj="${esc(o.id)}" title="${o.triangles} triangles · ${o.shells} shell${o.shells === 1 ? '' : 's'}${o.openEdges ? ` · ${o.openEdges} open edges` : ''}${o.nonManifoldEdges ? ` · ${o.nonManifoldEdges} non-manifold edges` : ''}"><span class="swatch" style="background:${o.color ?? '#bbb'}"></span><span class="name">${o.watertight && o.fits ? '✓' : '⚠'} ${esc(o.name)} <span class="muted">${mm(o.size)} · ${o.volumeCm3} cm³${rep(o)}${turn(o)}</span></span></div>`).join('')}
    ${(r.notes ?? []).map(n => `<div class="muted" style="font-size:11px;margin-top:4px">↻ ${esc(n)}</div>`).join('')}
    ${r.warnings.map(w => `<div class="muted" style="font-size:11px;margin-top:4px;color:#b06d00">⚠ ${esc(w)}</div>`).join('')}`;
}

P.registerPlugin({
  id: 'print3d', name: '3D Print', version: '0.1.0', accent: ACCENT,
  panels: [{ id: 'main', title: '3D Print', render: panel }],
  activate(h) {
    host = h;
    h.on('selection', () => h.refresh());
    // let other plugins (the track) and the console reach the exporter
    (window as any).plannr.print3d = { exportUrl, download, settings: st };
    h.registerCommand('print_check', async (a: any) => fetchJson(exportUrl(a?.format ?? '3mf', { ids: a?.ids, scale: a?.scale, report: true })!));
  },
});
