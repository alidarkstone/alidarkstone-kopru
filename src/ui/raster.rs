//! Yalnızca testler için: egui çıktısını CPU'da rasterleştirip PNG'ye yazan küçük yazılım çizici.
//! GPU ya da pencere gerektirmez; arayüzün ekran görüntülerini başsız üretmek için.

use std::collections::HashMap;

use egui::{epaint::Primitive, ImageData, TextureId};

pub struct Raster {
    textures: HashMap<TextureId, (usize, usize, Vec<[f32; 4]>)>,
}

impl Raster {
    pub fn new() -> Self {
        Raster {
            textures: HashMap::new(),
        }
    }

    /// Doku güncellemelerini uygular (her karenin `textures_delta.set` değeri).
    pub fn apply(&mut self, delta: &egui::TexturesDelta) {
        for (id, d) in &delta.set {
            let (w, h, px): (usize, usize, Vec<[f32; 4]>) = match &d.image {
                ImageData::Color(c) => (c.size[0], c.size[1], c.pixels.iter().map(to_f).collect()),
                ImageData::Font(f) => (
                    f.size[0],
                    f.size[1],
                    f.srgba_pixels(None).map(|c| to_f(&c)).collect(),
                ),
            };
            match d.pos {
                None => {
                    self.textures.insert(*id, (w, h, px));
                }
                Some([x0, y0]) => {
                    if let Some((tw, _th, tex)) = self.textures.get_mut(id) {
                        for y in 0..h {
                            for x in 0..w {
                                let i = (y0 + y) * *tw + x0 + x;
                                if let Some(dst) = tex.get_mut(i) {
                                    *dst = px[y * w + x];
                                }
                            }
                        }
                    }
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// Sahneyi `w×h` piksellik RGB tampona çizer.
    pub fn render(
        &self,
        prims: &[egui::ClippedPrimitive],
        ppp: f32,
        w: usize,
        h: usize,
        clear: egui::Color32,
    ) -> Vec<u8> {
        let c = to_f(&clear);
        let mut fb = vec![[c[0], c[1], c[2], 1.0]; w * h];
        for cp in prims {
            let Primitive::Mesh(mesh) = &cp.primitive else {
                continue;
            };
            let tex = self.textures.get(&mesh.texture_id);
            let clip = cp.clip_rect;
            let cx0 = (clip.min.x * ppp).max(0.0).floor() as i32;
            let cy0 = (clip.min.y * ppp).max(0.0).floor() as i32;
            let cx1 = ((clip.max.x * ppp).ceil() as i32).min(w as i32);
            let cy1 = ((clip.max.y * ppp).ceil() as i32).min(h as i32);
            for tri in mesh.indices.chunks_exact(3) {
                let v: Vec<&egui::epaint::Vertex> = tri
                    .iter()
                    .filter_map(|&i| mesh.vertices.get(i as usize))
                    .collect();
                if v.len() != 3 {
                    continue;
                }
                let p: Vec<(f32, f32)> = v.iter().map(|v| (v.pos.x * ppp, v.pos.y * ppp)).collect();
                let area = edge(p[0], p[1], p[2]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let minx = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor() as i32).max(cx0);
                let maxx = (p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as i32).min(cx1);
                let miny = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor() as i32).max(cy0);
                let maxy = (p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as i32).min(cy1);
                let cols: Vec<[f32; 4]> = v.iter().map(|v| to_f(&v.color)).collect();
                for y in miny..maxy {
                    for x in minx..maxx {
                        let q = (x as f32 + 0.5, y as f32 + 0.5);
                        let e0 = edge(p[1], p[2], q) * area.signum();
                        let e1 = edge(p[2], p[0], q) * area.signum();
                        let e2 = edge(p[0], p[1], q) * area.signum();
                        // ortak kenardaki piksel iki üçgene birden yazılmasın (yarı açık kural)
                        if e0 < 0.0 || e1 < 0.0 || e2 < 0.0 || (e0 == 0.0 && e1 == 0.0) {
                            continue;
                        }
                        if (e0 == 0.0 && !owns(p[1], p[2], area))
                            || (e1 == 0.0 && !owns(p[2], p[0], area))
                            || (e2 == 0.0 && !owns(p[0], p[1], area))
                        {
                            continue;
                        }
                        let w0 = e0 / area.abs();
                        let w1 = e1 / area.abs();
                        let w2 = 1.0 - w0 - w1;
                        let mut s = [0f32; 4];
                        for k in 0..4 {
                            s[k] = cols[0][k] * w0 + cols[1][k] * w1 + cols[2][k] * w2;
                        }
                        if let Some((tw, th, tp)) = tex {
                            let u = v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2;
                            let vv = v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2;
                            let t = sample(*tw, *th, tp, u, vv);
                            for k in 0..4 {
                                s[k] *= t[k];
                            }
                        }
                        // premultiplied "over"
                        let d = &mut fb[y as usize * w + x as usize];
                        for k in 0..3 {
                            d[k] = s[k] + d[k] * (1.0 - s[3]);
                        }
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(w * h * 3);
        for px in fb {
            for k in 0..3 {
                out.push((px[k].clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        out
    }
}

fn to_f(c: &egui::Color32) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// Üst-sol kuralı: kenar üzerindeki pikseli yalnızca "üst" ya da "sol" kenara sahip üçgen çizer.
fn owns(a: (f32, f32), b: (f32, f32), area: f32) -> bool {
    let (dx, dy) = ((b.0 - a.0) * area.signum(), (b.1 - a.1) * area.signum());
    (dy == 0.0 && dx < 0.0) || dy > 0.0
}

fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Çift doğrusal örnekleme.
fn sample(w: usize, h: usize, px: &[[f32; 4]], u: f32, v: f32) -> [f32; 4] {
    if w == 0 || h == 0 {
        return [1.0; 4];
    }
    let x = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let g = |xx: usize, yy: usize| px.get(yy * w + xx).copied().unwrap_or([1.0; 4]);
    let (a, b, c, d) = (g(x0, y0), g(x1, y0), g(x0, y1), g(x1, y1));
    let mut o = [0f32; 4];
    for k in 0..4 {
        o[k] = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy;
    }
    o
}

/// Bağımlılıksız PNG yazıcı (sıkıştırmasız deflate blokları).
pub fn write_png(path: &std::path::Path, w: usize, h: usize, rgb: &[u8]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for y in 0..h {
        raw.push(0u8);
        raw.extend_from_slice(&rgb[y * w * 3..(y + 1) * w * 3]);
    }
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        let n = chunk.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}
