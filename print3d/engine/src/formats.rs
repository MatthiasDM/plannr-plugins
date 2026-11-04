//! File writers: binary STL and 3MF (a zip with the 3D model XML, millimetres, one object per part).

use crate::mesh::{Mesh, V3};

pub struct Part<'a> { pub name: &'a str, pub mesh: &'a Mesh, pub color: Option<&'a str>, /// Where the build item goes (the mesh is around its own origin).
    pub offset: V3 }

pub fn stl(parts: &[Part]) -> Vec<u8> {
    let n: usize = parts.iter().map(|p| p.mesh.t.len()).sum();
    let mut out = Vec::with_capacity(84 + n * 50);
    let mut header = [0u8; 80];
    let h = b"Plannr print3d binary STL, units: mm";
    header[..h.len()].copy_from_slice(h);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    for p in parts {
        for t in &p.mesh.t {
            let [a, b, c] = t.map(|i| { let v = p.mesh.v[i as usize]; [v[0] + p.offset[0], v[1] + p.offset[1], v[2] + p.offset[2]] });
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]]; let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let nrm = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
            let l = (nrm[0] * nrm[0] + nrm[1] * nrm[1] + nrm[2] * nrm[2]).sqrt().max(1e-30);
            for x in [nrm[0] / l, nrm[1] / l, nrm[2] / l] { out.extend_from_slice(&(x as f32).to_le_bytes()); }
            for v in [a, b, c] { for x in v { out.extend_from_slice(&(x as f32).to_le_bytes()); } }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.chars().map(|c| match c { '&' => "&amp;".into(), '<' => "&lt;".into(), '>' => "&gt;".into(), '"' => "&quot;".into(), '\'' => "&apos;".into(), c if (c as u32) < 0x20 => " ".into(), c => c.to_string() }).collect()
}
fn num(x: f64) -> String {
    let s = format!("{:.4}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}

pub fn threemf(parts: &[Part], title: &str) -> Vec<u8> {
    use std::fmt::Write;
    let mut m = String::new();
    m.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n");
    let _ = writeln!(m, " <metadata name=\"Title\">{}</metadata>\n <metadata name=\"Application\">Plannr</metadata>", xml_escape(title));
    m.push_str(" <resources>\n");
    let colored = parts.iter().any(|p| p.color.is_some());
    if colored {
        m.push_str("  <basematerials id=\"1\">\n");
        for p in parts { let _ = writeln!(m, "   <base name=\"{}\" displaycolor=\"{}FF\"/>", xml_escape(p.name), p.color.unwrap_or("#B0B0B0").to_uppercase()); }
        m.push_str("  </basematerials>\n");
    }
    for (k, p) in parts.iter().enumerate() {
        let id = k + 2;
        if colored { let _ = writeln!(m, "  <object id=\"{id}\" type=\"model\" name=\"{}\" pid=\"1\" pindex=\"{k}\">", xml_escape(p.name)); }
        else { let _ = writeln!(m, "  <object id=\"{id}\" type=\"model\" name=\"{}\">", xml_escape(p.name)); }
        m.push_str("   <mesh>\n    <vertices>\n");
        for v in &p.mesh.v { let _ = writeln!(m, "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", num(v[0]), num(v[1]), num(v[2])); }
        m.push_str("    </vertices>\n    <triangles>\n");
        for t in &p.mesh.t { let _ = writeln!(m, "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]); }
        m.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    }
    m.push_str(" </resources>\n <build>\n");
    for (k, p) in parts.iter().enumerate() {
        let _ = writeln!(m, "  <item objectid=\"{}\" transform=\"1 0 0 0 1 0 0 0 1 {} {} {}\"/>", k + 2, num(p.offset[0]), num(p.offset[1]), num(p.offset[2]));
    }
    m.push_str(" </build>\n</model>\n");
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>\n";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/></Relationships>\n";
    zip(&[("[Content_Types].xml", content_types.as_bytes()), ("_rels/.rels", rels.as_bytes()), ("3D/3dmodel.model", m.as_bytes())])
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for i in 0..256u32 { let mut c = i; for _ in 0..8 { c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }; } table[i as usize] = c; }
    let mut crc = 0xFFFF_FFFFu32;
    for b in data { crc = table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8); }
    crc ^ 0xFFFF_FFFF
}

/// A zip archive with the files stored (no compression): every slicer reads it, and it needs no dependency.
pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = vec![];
    let mut central = vec![];
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        // local file header
        le32(&mut out, 0x0403_4b50); le16(&mut out, 20); le16(&mut out, 0); le16(&mut out, 0); le16(&mut out, 0); le16(&mut out, 0x21);
        le32(&mut out, crc); le32(&mut out, data.len() as u32); le32(&mut out, data.len() as u32);
        le16(&mut out, name.len() as u16); le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes()); out.extend_from_slice(data);
        // central directory entry
        le32(&mut central, 0x0201_4b50); le16(&mut central, 20); le16(&mut central, 20); le16(&mut central, 0); le16(&mut central, 0); le16(&mut central, 0); le16(&mut central, 0x21);
        le32(&mut central, crc); le32(&mut central, data.len() as u32); le32(&mut central, data.len() as u32);
        le16(&mut central, name.len() as u16); le16(&mut central, 0); le16(&mut central, 0); le16(&mut central, 0); le16(&mut central, 0);
        le32(&mut central, 0); le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50); le16(&mut out, 0); le16(&mut out, 0); le16(&mut out, files.len() as u16); le16(&mut out, files.len() as u16);
    le32(&mut out, cd_len); le32(&mut out, cd_offset); le16(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc_matches_reference() { assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926); }
}
