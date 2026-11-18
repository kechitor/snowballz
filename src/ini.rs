//! INI files, read and written the way Python's RawConfigParser does it
//! (keys are lower-cased, `=` or `:` separate keys and values, `#` and `;`
//! start comment lines, sections and keys keep their order).

#[derive(Default, Clone, Debug)]
pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn new() -> Ini {
        Ini::default()
    }

    pub fn parse(text: &str) -> Ini {
        let mut ini = Ini::new();
        let mut cur: Option<usize> = None;
        let mut last_key: Option<usize> = None;
        for raw in text.lines() {
            let line = raw.trim_end();
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                last_key = None;
                continue;
            }
            if trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            // continuation line
            if line.starts_with(|c: char| c.is_whitespace()) {
                if let (Some(s), Some(k)) = (cur, last_key) {
                    let v = &mut ini.sections[s].1[k].1;
                    v.push('\n');
                    v.push_str(trimmed);
                    continue;
                }
            }
            if trimmed.starts_with('[') {
                if let Some(end) = trimmed.find(']') {
                    let name = trimmed[1..end].to_string();
                    cur = Some(match ini.sections.iter().position(|(n, _)| *n == name) {
                        Some(i) => i,
                        None => {
                            ini.sections.push((name, Vec::new()));
                            ini.sections.len() - 1
                        }
                    });
                    last_key = None;
                    continue;
                }
            }
            let Some(s) = cur else { continue };
            let sep = trimmed.find(['=', ':']);
            let (k, v) = match sep {
                Some(i) => (trimmed[..i].trim(), trimmed[i + 1..].trim()),
                None => (trimmed, ""),
            };
            let k = k.to_lowercase();
            let items = &mut ini.sections[s].1;
            match items.iter().position(|(kk, _)| *kk == k) {
                Some(i) => {
                    items[i].1 = v.to_string();
                    last_key = Some(i);
                }
                None => {
                    items.push((k, v.to_string()));
                    last_key = Some(items.len() - 1);
                }
            }
        }
        ini
    }

    pub fn has_section(&self, s: &str) -> bool {
        self.sections.iter().any(|(n, _)| n == s)
    }

    pub fn add_section(&mut self, s: &str) {
        if !self.has_section(s) {
            self.sections.push((s.to_string(), Vec::new()));
        }
    }

    pub fn remove_section(&mut self, s: &str) {
        self.sections.retain(|(n, _)| n != s);
    }

    pub fn remove(&mut self, s: &str, k: &str) {
        let k = k.to_lowercase();
        if let Some((_, items)) = self.sections.iter_mut().find(|(n, _)| n == s) {
            items.retain(|(kk, _)| *kk != k);
        }
    }

    pub fn get(&self, s: &str, k: &str) -> Option<&str> {
        let k = k.to_lowercase();
        self.sections
            .iter()
            .find(|(n, _)| n == s)
            .and_then(|(_, items)| items.iter().find(|(kk, _)| *kk == k))
            .map(|(_, v)| v.as_str())
    }

    pub fn get_int(&self, s: &str, k: &str) -> Option<i64> {
        self.get(s, k).and_then(|v| v.trim().parse().ok())
    }

    pub fn set(&mut self, s: &str, k: &str, v: &str) {
        self.add_section(s);
        let k = k.to_lowercase();
        let items = &mut self.sections.iter_mut().find(|(n, _)| n == s).unwrap().1;
        match items.iter_mut().find(|(kk, _)| *kk == k) {
            Some(item) => item.1 = v.to_string(),
            None => items.push((k, v.to_string())),
        }
    }

    pub fn items(&self, s: &str) -> Vec<(String, String)> {
        self.sections.iter().find(|(n, _)| n == s).map(|(_, i)| i.clone()).unwrap_or_default()
    }

    pub fn write(&self) -> String {
        let mut out = String::new();
        for (name, items) in &self.sections {
            out.push_str(&format!("[{}]\n", name));
            for (k, v) in items {
                out.push_str(&format!("{} = {}\n", k, v.replace('\n', "\n\t")));
            }
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_settings() {
        let ini = Ini::parse(
            "[map]\nname = Crystal Crises\ndescription: A 6 way race.\n\
             required_plugins: powerup-v1.1, gather_victory-v1\n\n[ai]\n;80 as in 80%\n\
             gather_force = 80\n[igloos]\n1 = 20,125\n2 = 81,135\n",
        );
        assert_eq!(ini.get("map", "description"), Some("A 6 way race."));
        assert_eq!(ini.get("map", "Name"), Some("Crystal Crises"));
        assert_eq!(ini.get_int("ai", "gather_force"), Some(80));
        let ig = ini.items("igloos");
        assert_eq!(ig[1], ("2".to_string(), "81,135".to_string()));
        let again = Ini::parse(&ini.write());
        assert_eq!(again.items("igloos"), ig);
    }
}
