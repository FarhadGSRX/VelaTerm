//! Translate parser-selected prose ranges while retaining the original Markdown source as a template.

use std::ops::Range;
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair { pub original: String, pub translation: String }

#[derive(Clone)]
struct Slot { range:Range<usize>, text:String }
#[derive(Clone)]
struct Reference { owner:Range<usize>, at:usize, suffix:String }
pub struct Protected {
    pub lines: Vec<String>,
    pub(super) tokens: Vec<(String, String)>,
    original: String,
    slots: Vec<Slot>,
    references: Vec<Reference>,
}

fn options() -> Options { Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_GFM }

/// Streaming translation commits complete top-level blocks, never partial lists, tables, or fences.
pub fn stable_text(text: &str, streaming: bool) -> &str {
    if !streaming { return text; }
    let mut depth=0usize;let mut block_start=0;let mut last_start=0;let mut boundary=0;
    for (event,range) in Parser::new_ext(text,options()).into_offset_iter() {
        match event {
            Event::Start(_)=>{if depth==0 {block_start=range.start;}depth+=1;},
            Event::End(_)=>{depth-=1;if depth==0 {last_start=block_start;boundary=range.end;}},
            _ if depth==0=>{last_start=range.start;boundary=range.end;},
            _=>{}
        }
    }
    if boundary==0 {return "";}
    // A trailing block may still grow even after a newline; the next block establishes its boundary.
    let end=if text[boundary..].contains('\n') && text[..boundary].ends_with('\n') {boundary} else {last_start};
    &text[..end]
}

fn token(output: &mut String, tokens: &mut Vec<(String, String)>, prefix: &str, value: &str) {
    let key=format!("{prefix}{}__",tokens.len());output.push_str(&key);tokens.push((key,value.into()));
}

/// Diff payloads without fences are still source artifacts, not prose or Markdown lists.
fn diff_ranges(text: &str) -> Vec<Range<usize>> {
    let lines:Vec<_>=text.split_inclusive('\n').collect();let mut result=Vec::new();let mut start=None;let mut offset=0;let mut binary=false;
    for (index,line) in lines.iter().enumerate() {
        let line_end=offset+line.len();let raw=line.trim_end_matches(['\r','\n']);
        let header=raw.starts_with("diff --git ") || raw.starts_with("Index: ")
            || raw.starts_with("--- ") && lines.get(index+1).is_some_and(|next|next.starts_with("+++ "))
            || raw.starts_with("@@ -") && raw.contains(" @@");
        let body=raw.starts_with([' ','+','-']) || raw.starts_with("\\ No newline") || raw.starts_with("@@")
            || ["index ","old mode ","new mode ","deleted file mode ","new file mode ","similarity index ","dissimilarity index ","rename from ","rename to ","copy from ","copy to ","Binary files ","GIT binary patch","literal ","delta ","===="].iter().any(|prefix|raw.starts_with(prefix));
        if header && start.is_none() {start=Some(offset);}
        if start.is_some() && raw=="GIT binary patch" {binary=true;}
        let binary_separator=binary && raw.is_empty() && lines.get(index+1).is_some_and(|next|next.starts_with("literal ") || next.starts_with("delta "));
        if start.is_some() && !header && !body && !(binary && !raw.is_empty()) && !binary_separator {
            result.push(start.take().unwrap()..offset);binary=false;
        }
        offset=line_end;
    }
    if let Some(start)=start {result.push(start..text.len());}
    result
}

fn protect_prose(text: &str,prefix: &str,tokens: &mut Vec<(String,String)>) -> String {
    let mut output=String::new();let mut i=0;
    while i<text.len() {
        let rest=&text[i..];let boundary=i==0 || text[..i].chars().next_back().is_some_and(|c|c.is_whitespace() || "(<[\"'".contains(c));
        if boundary {
            let len=rest.find(|c:char|c.is_whitespace() || ")]<>\"'".contains(c)).unwrap_or(rest.len());
            let candidate=rest[..len].trim_end_matches([',','.',';','!','?']);
            let technical=candidate.contains("://") || candidate.starts_with("www.")
                || candidate.starts_with('/') || candidate.starts_with("./") || candidate.starts_with("../") || candidate.starts_with("~/")
                || candidate.contains('\\') || candidate.starts_with("--") || candidate.contains('_') || candidate.contains('/')
                || candidate.is_ascii() && (candidate.contains('.') || candidate.trim_start_matches(['(','[','{']).chars().skip(1).any(|c|c.is_ascii_uppercase())
                    || ["git","cargo","pnpm","npm","node","python","python3","bash","zsh","curl","ssh"].contains(&candidate));
            if technical && !candidate.is_empty() {token(&mut output,tokens,prefix,candidate);i+=candidate.len();continue;}
        }
        let ch=rest.chars().next().unwrap();output.push(ch);i+=ch.len_utf8();
    }
    output
}

/// Form fields are plain text. Keep field boundaries, line breaks and inline code out of provider prose.
pub fn protect_fields(fields: &[String]) -> Protected {
    let prefix=format!("__VLXT_{}_",uuid::Uuid::new_v4().simple());
    let mut tokens=Vec::new();let mut lines=Vec::new();let mut slots=Vec::new();
    for field in fields {
        let mut line=String::new();let mut rest=field.as_str();
        while !rest.is_empty() {
            let end=rest.find(['`','\n','\r']).unwrap_or(rest.len());
            line.push_str(&protect_prose(&rest[..end],&prefix,&mut tokens));rest=&rest[end..];
            if rest.is_empty() {break;}
            let end=if rest.starts_with('`') {rest[1..].find('`').map(|i|i+2).unwrap_or(1)} else {1};
            token(&mut line,&mut tokens,&prefix,&rest[..end]);rest=&rest[end..];
        }
        slots.push(Slot {range:0..field.len(),text:field.clone()});lines.push(line);
    }
    Protected {lines,tokens,original:String::new(),slots,references:Vec::new()}
}

pub fn protect(text: &str) -> Protected {
    let prefix=format!("__VLXT_{}_",uuid::Uuid::new_v4().simple());
    let mut tokens=Vec::new();let mut slots=Vec::new();let mut lines=Vec::new();let mut stack=Vec::new();let mut references=Vec::new();
    let diffs=diff_ranges(text);
    for (event,range) in Parser::new_ext(text,options()).into_offset_iter() {
        match event {
            Event::Start(tag)=>{
                if let Tag::Link {link_type,id,..} | Tag::Image {link_type,id,..}=&tag {
                    // Translated labels must keep resolving to the original reference definition.
                    match link_type {
                        LinkType::Collapsed=>references.push(Reference {owner:range.clone(),at:range.end-1,suffix:id.to_string()}),
                        LinkType::Shortcut=>references.push(Reference {owner:range.clone(),at:range.end,suffix:format!("[{id}]")}),
                        _=>{}
                    }
                }
                let protected=matches!(tag,Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_)
                    | Tag::Link {link_type:LinkType::Autolink | LinkType::Email,..});
                stack.push(stack.last().copied().unwrap_or(false) || protected);
            },
            Event::End(_)=>{stack.pop();},
            Event::Text(value) if !stack.last().copied().unwrap_or(false)=>{
                if diffs.iter().any(|diff|range.start<diff.end && diff.start<range.end) {continue;}
                let raw=&text[range.clone()];
                // Escapes and entities retain their exact source spelling; they are syntax, not prose.
                if raw!=value.as_ref() || !raw.chars().any(char::is_alphabetic) {continue;}
                let start=range.start+raw.len()-raw.trim_start().len();let end=range.end-(raw.len()-raw.trim_end().len());
                if start>=end {continue;}
                let prose=&text[start..end];
                if prose.contains(['\n','\r']) {continue;}
                slots.push(Slot {range:start..end,text:prose.into()});lines.push(protect_prose(prose,&prefix,&mut tokens));
            },
            _=>{}
        }
    }
    Protected {lines,tokens,original:text.into(),slots,references}
}

fn escape_prose(text: &str) -> String {
    let mut output=String::new();
    for ch in text.chars() {
        if "\\`*_[]<>|~#".contains(ch) {output.push('\\');}
        output.push(ch);
    }
    // A translated sentence must not create a new list or heading in its source text slot.
    if ["- ","+ ","! "].iter().any(|prefix|output.starts_with(prefix)) {output.insert(0,'\\');}
    let digits=output.bytes().take_while(u8::is_ascii_digit).count();
    if digits>0 && output.as_bytes().get(digits).is_some_and(|c|b".)".contains(c)) && output.as_bytes().get(digits+1)==Some(&b' ') {output.insert(digits,'\\');}
    output
}

impl Protected {
    pub fn restore_fields(&self,lines: &[String]) -> Result<Vec<String>,String> {
        if lines.len()!=self.lines.len() {return Err("translation:structure".into());}
        self.lines.iter().zip(lines).map(|(source,translated)| {
            if translated.contains(['\n','\r']) || translated.trim().is_empty() {return Err("translation:structure".into());}
            let mut decoded=translated.trim().to_string();
            for (key,value) in &self.tokens {
                if translated.matches(key).count()!=source.matches(key).count() {return Err("translation:structure".into());}
                decoded=decoded.replace(key,value);
            }
            if decoded.contains("__VLXT_") {return Err("translation:structure".into());}
            Ok(decoded)
        }).collect()
    }
    pub fn fragment(&self,index:usize) -> &str { &self.slots[index].text }
    pub fn fragment_tokens(&self,index:usize) -> Vec<(String,String)> {
        self.tokens.iter().filter(|(key,_)|self.lines[index].contains(key)).cloned().collect()
    }
    pub fn select(&self,indices:&[usize]) -> Self {
        Self {lines:indices.iter().map(|i|self.lines[*i].clone()).collect(),tokens:self.tokens.clone(),original:self.original.clone(),
            slots:indices.iter().map(|i|self.slots[*i].clone()).collect(),references:self.references.clone()}
    }
    pub fn restore(&self, lines: &[String]) -> Result<Vec<Pair>, String> {
        if lines.len()!=self.lines.len() {return Err("translation:structure".into());}
        let mut edits=Vec::new();
        for ((source,translated),slot) in self.lines.iter().zip(lines).zip(&self.slots) {
            if translated.contains(['\n','\r']) || translated.trim().is_empty() {return Err("translation:structure".into());}
            let mut decoded=translated.trim().to_string();
            for (key,value) in &self.tokens {
                if translated.matches(key).count()!=source.matches(key).count() {return Err("translation:structure".into());}
                decoded=decoded.replace(key,value);
            }
            if decoded.contains("__VLXT_") {return Err("translation:structure".into());}
            if decoded!=slot.text {
                let mut replacement=String::new();
                // Escape only provider prose. Protected technical fragments are copied byte for byte.
                let mut rest=translated.trim();
                while !rest.is_empty() {
                    let next=self.tokens.iter().filter_map(|(key,value)|rest.find(key).map(|at|(at,key,value))).min_by_key(|(at,_,_)|*at);
                    if let Some((at,key,value))=next {replacement.push_str(&escape_prose(&rest[..at]));replacement.push_str(value);rest=&rest[at+key.len()..];}
                    else {replacement.push_str(&escape_prose(rest));break;}
                }
                edits.push((slot.range.clone(),replacement));
            }
        }
        for reference in &self.references {
            if edits.iter().any(|(range,_)|reference.owner.start<=range.start && range.end<=reference.owner.end) {
                edits.push((reference.at..reference.at,reference.suffix.clone()));
            }
        }
        edits.sort_by_key(|(range,_)|(range.start,range.end));
        let mut output=String::new();let mut cursor=0;
        for (range,replacement) in edits {
            if range.start<cursor {return Err("translation:structure".into());}
            output.push_str(&self.original[cursor..range.start]);output.push_str(&replacement);cursor=range.end;
        }
        output.push_str(&self.original[cursor..]);
        let original:Vec<_>=self.original.split('\n').collect();let translated:Vec<_>=output.split('\n').collect();
        if original.len()!=translated.len() {return Err("translation:structure".into());}
        Ok(original.into_iter().zip(translated).map(|(original,translation)|Pair {original:original.into(),translation:translation.into()}).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_technical_restoration_and_rejection() {
        let original = "## Read **git** with sessionId and API_KEY.\nRead `x = 1` and [docs](https://host/a(b)).\n```sh\ncargo test --lib\n```\nOpen /tmp/file and request_id.\n";
        let protected = protect(original);
        let pairs = protected.restore(&protected.lines).unwrap();
        assert_eq!(pairs.iter().map(|p|p.original.as_str()).collect::<Vec<_>>().join("\n"), original);
        assert_eq!(pairs.iter().map(|p|p.translation.as_str()).collect::<Vec<_>>().join("\n"), original);
        let mut bad = protected.lines.clone(); bad[0].push_str(&protected.tokens[0].0);
        assert!(protected.restore(&bad).is_err());
    }
    #[test]
    fn streaming_boundaries_keep_unfinished_fences_and_lines() {
        assert_eq!(stable_text("Ready.\n```rs\nlet x", true), "Ready.\n");
        assert_eq!(stable_text("Ready.\n```rs\nlet x\n```\nNext", true), "Ready.\n```rs\nlet x\n```\n");
        assert_eq!(stable_text("last", false), "last");
    }
}
