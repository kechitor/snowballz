//! Reading zip archives (the maps are zip files).  Stored and deflated
//! entries are supported.

use crate::inflate;
use std::collections::HashMap;

pub struct Zip {
    data: Vec<u8>,
    entries: HashMap<String, (usize, usize, usize, u16)>, // name -> (local header, csize, usize, method)
}

fn le16(b: &[u8], p: usize) -> usize {
    b[p] as usize | (b[p + 1] as usize) << 8
}
fn le32(b: &[u8], p: usize) -> usize {
    le16(b, p) | le16(b, p + 2) << 16
}

impl Zip {
    pub fn open(path: &str) -> Result<Zip, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {}", path, e))?;
        Zip::from_bytes(data).map_err(|e| format!("{}: {}", path, e))
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Zip, String> {
        // find the end of central directory record
        if data.len() < 22 {
            return Err("not a zip file".into());
        }
        let mut eocd = None;
        let lo = data.len().saturating_sub(22 + 65535);
        let mut p = data.len() - 22;
        loop {
            if &data[p..p + 4] == b"PK\x05\x06" {
                eocd = Some(p);
                break;
            }
            if p == lo {
                break;
            }
            p -= 1;
        }
        let eocd = eocd.ok_or("not a zip file (no central directory)")?;
        let count = le16(&data, eocd + 10);
        let mut cd = le32(&data, eocd + 16);
        let mut entries = HashMap::new();
        for _ in 0..count {
            if cd + 46 > data.len() || &data[cd..cd + 4] != b"PK\x01\x02" {
                return Err("bad zip central directory".into());
            }
            let method = le16(&data, cd + 10) as u16;
            let csize = le32(&data, cd + 20);
            let usize_ = le32(&data, cd + 24);
            let nlen = le16(&data, cd + 28);
            let elen = le16(&data, cd + 30);
            let clen = le16(&data, cd + 32);
            let local = le32(&data, cd + 42);
            let name = String::from_utf8_lossy(&data[cd + 46..cd + 46 + nlen]).into_owned();
            entries.insert(name, (local, csize, usize_, method));
            cd += 46 + nlen + elen + clen;
        }
        Ok(Zip { data, entries })
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let &(local, csize, usize_, method) =
            self.entries.get(name).ok_or_else(|| format!("{} is not in the archive", name))?;
        let d = &self.data;
        if local + 30 > d.len() || &d[local..local + 4] != b"PK\x03\x04" {
            return Err("bad zip local header".into());
        }
        let start = local + 30 + le16(d, local + 26) + le16(d, local + 28);
        if start + csize > d.len() {
            return Err("truncated zip entry".into());
        }
        let raw = &d[start..start + csize];
        match method {
            0 => Ok(raw.to_vec()),
            8 => inflate::inflate(raw, usize_),
            m => Err(format!("unsupported zip compression method {}", m)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps() {
        for m in ["whiteforest", "iceland", "crystalcrises", "smallcrystalrush"] {
            let z = Zip::open(&format!("maps/{}.zip", m)).unwrap();
            let ini = String::from_utf8(z.read("settings.ini").unwrap()).unwrap();
            assert!(ini.contains("[map]"));
            let img = crate::image::Image::decode(&z.read("regions.png").unwrap()).unwrap();
            assert!(img.w >= 100);
        }
    }
}
