//! Parse note references without treating fenced code or inline code as links.
use super::*;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub target: String,
    pub label: String,
    pub embed: bool,
    pub wiki: bool,
    #[serde(skip)]
    pub start: usize,
    #[serde(skip)]
    pub end: usize,
}
pub fn parse(content: &str) -> Vec<Link> {
    let mut links = Vec::new();
    let mut offset = 0;
    let mut fence = String::new();
    let mut frontmatter = false;
    for (line_index, line) in content.split_inclusive('\n').enumerate() {
        let trim = line.trim();
        if line_index == 0 && trim == "---" {
            frontmatter = true;
            offset += line.len();
            continue;
        }
        if frontmatter {
            if trim == "---" {
                frontmatter = false;
            }
            offset += line.len();
            continue;
        }
        if trim.starts_with("```") || trim.starts_with("~~~") {
            let marker = &trim[..3];
            if fence.is_empty() {
                fence = marker.into();
            } else if fence == marker {
                fence.clear();
            }
            offset += line.len();
            continue;
        }
        if !fence.is_empty() || line.starts_with("    ") || line.starts_with('\t') {
            offset += line.len();
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if bytes[i] == b'`' {
                let count = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                let needle = "`".repeat(count);
                i += count;
                if let Some(end) = line[i..].find(&needle) {
                    i += end + count;
                }
                continue;
            }
            if bytes[i] != b'[' {
                i += 1;
                continue;
            }
            let embed = i > 0 && bytes[i - 1] == b'!';
            if bytes.get(i + 1) == Some(&b'[') {
                if let Some(end) = line[i + 2..].find("]]") {
                    let inside = &line[i + 2..i + 2 + end];
                    let (target, label) = inside.split_once('|').unwrap_or((inside, inside));
                    if !target.is_empty() {
                        links.push(Link {
                            target: target.into(),
                            label: label.into(),
                            embed,
                            wiki: true,
                            start: offset + i + 2,
                            end: offset + i + 2 + target.len(),
                        });
                    }
                    i += end + 4;
                    continue;
                }
            } else if let Some(end) = line[i + 1..].find("](") {
                let label = &line[i + 1..i + 1 + end];
                let start = i + end + 3;
                let mut at = start;
                let mut depth = 0;
                while at < bytes.len() {
                    if bytes[at] == b'\\' {
                        at += 2;
                        continue;
                    }
                    if bytes[at] == b'(' {
                        depth += 1;
                    }
                    if bytes[at] == b')' {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    at += 1;
                }
                if at < bytes.len() {
                    let dest = &line[start..at];
                    let (trimmed, skip) = if dest.starts_with('<') {
                        (
                            dest.strip_prefix('<')
                                .and_then(|x| x.split_once('>').map(|p| p.0))
                                .unwrap_or(dest),
                            1,
                        )
                    } else {
                        (dest.split(" \"").next().unwrap_or(dest), 0)
                    };
                    if !trimmed.is_empty() {
                        links.push(Link {
                            target: trimmed.into(),
                            label: label.into(),
                            embed,
                            wiki: false,
                            start: offset + start + skip,
                            end: offset + start + skip + trimmed.len(),
                        });
                    }
                    i = at + 1;
                    continue;
                }
            }
            i += 1;
        }
        offset += line.len();
    }
    links
}
fn decode(raw: &str) -> String {
    let b = raw.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(a), Some(c)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            ) {
                result.push((a * 16 + c) as u8);
                i += 3;
                continue;
            }
        }
        result.push(b[i]);
        i += 1;
    }
    String::from_utf8(result).unwrap_or_else(|_| raw.into())
}
fn normalized(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            x => parts.push(x),
        }
    }
    Some(parts.join("/"))
}
pub fn resolve(source: &str, link: &Link, paths: &[String]) -> Option<String> {
    let raw = decode(link.target.split('#').next().unwrap_or(""));
    if raw.is_empty() {
        return Some(source.into());
    }
    if raw.contains(':') || raw.starts_with(['/', '\\']) {
        return None;
    }
    let dir = source.rsplit_once('/').map(|p| p.0).unwrap_or("");
    let relative = normalized(&format!("{dir}/{raw}"));
    let absolute = normalized(&raw);
    let candidates = if link.wiki {
        [absolute, relative]
    } else {
        [relative, absolute]
    };
    for candidate in candidates.into_iter().flatten() {
        for p in [&candidate, &format!("{candidate}.md")] {
            if paths.contains(p) {
                return Some(p.clone());
            }
        }
    }
    if link.wiki || !raw.contains('/') {
        let matches: Vec<_> = paths
            .iter()
            .filter(|p| {
                p.as_str() == raw
                    || p.ends_with(&format!("/{raw}"))
                    || p.ends_with(&format!("/{raw}.md"))
                    || p.as_str() == format!("{raw}.md")
            })
            .collect();
        if matches.len() == 1 {
            return Some(matches[0].clone());
        }
    }
    None
}
fn relative_link(source: &str, target: &str) -> String {
    let src: Vec<_> = source.split('/').collect();
    let dest: Vec<_> = target.split('/').collect();
    let dir = &src[..src.len().saturating_sub(1)];
    let common = dir.iter().zip(&dest).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; dir.len() - common];
    parts.extend_from_slice(&dest[common..]);
    parts.join("/")
}
pub fn rewrite(
    content: &str,
    old_source: &str,
    new_source: &str,
    paths: &[String],
    from: &str,
    to: &str,
) -> String {
    let mut result = content.to_string();
    for link in parse(content).into_iter().rev() {
        let Some(target) = resolve(old_source, &link, paths) else {
            continue;
        };
        let moved = target == from || target.starts_with(&format!("{from}/"));
        if !moved && old_source == new_source {
            continue;
        }
        let new_target = if moved {
            format!("{to}{}", &target[from.len()..])
        } else {
            target
        };
        let fragment = link
            .target
            .split_once('#')
            .map(|(_, s)| format!("#{s}"))
            .unwrap_or_default();
        let raw = if link.wiki {
            new_target
                .strip_suffix(".md")
                .unwrap_or(&new_target)
                .to_string()
        } else {
            relative_link(new_source, &new_target)
                .replace('%', "%25")
                .replace(' ', "%20")
                .replace('#', "%23")
                .replace('(', "%28")
                .replace(')', "%29")
        };
        result.replace_range(link.start..link.end, &format!("{raw}{fragment}"));
    }
    result
}
pub fn outline(content: &str) -> Vec<Value> {
    let mut fence = false;
    content.lines().enumerate().filter_map(|(line,text)| {
        if text.trim_start().starts_with("```") || text.trim_start().starts_with("~~~") { fence=!fence; return None; }
        if fence { return None; }
        let level=text.chars().take_while(|&c|c=='#').count();
        if !(1..=6).contains(&level) || !text[level..].starts_with(' ') { return None; }
        Some(json!({"text":text[level..].trim().trim_end_matches('#').trim(),"level":level,"line":line}))
    }).collect()
}
pub fn tags(content: &str) -> Vec<String> {
    let mut result = std::collections::BTreeSet::new();
    for word in content.split_whitespace() {
        if let Some(tag) = word
            .strip_prefix('#')
            .filter(|s| !s.is_empty() && !s.starts_with('#'))
        {
            let tag = tag.trim_end_matches(|c: char| {
                !c.is_alphanumeric() && c != '_' && c != '-' && c != '/'
            });
            if !tag.is_empty()
                && tag
                    .chars()
                    .all(|c| c.is_alphanumeric() || "_-/".contains(c))
            {
                result.insert(tag.to_string());
            }
        }
    }
    if let Some(header) = content
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---").map(|p| p.0))
    {
        let mut in_tags = false;
        for line in header.lines() {
            if let Some(value) = line.strip_prefix("tags:") {
                in_tags = value.trim().is_empty();
                for tag in value.trim().trim_matches(['[', ']']).split(',') {
                    let tag = tag.trim().trim_matches(['\'', '"', '#']);
                    if !tag.is_empty() {
                        result.insert(tag.into());
                    }
                }
            } else if in_tags && line.trim_start().starts_with("- ") {
                let tag = line.trim_start()[2..].trim().trim_matches(['\'', '"', '#']);
                if !tag.is_empty() {
                    result.insert(tag.into());
                }
            } else {
                in_tags = false;
            }
        }
    }
    result.into_iter().collect()
}
