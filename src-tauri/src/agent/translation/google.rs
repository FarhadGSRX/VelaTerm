//! Anonymous Google translation with bounded batches, protected HTML, and process-wide throttling.

use super::*;
use std::sync::Condvar;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/t?client=gtx&dt=t&format=html";
const CHUNK_CHARS: usize = 2000;
const BATCH_BYTES: usize = 32_000;
static LIMITER: OnceLock<Limiter> = OnceLock::new();

#[derive(Default)]
struct Throttle { active:usize, next:Option<Instant>, cooldown:Option<Instant> }
#[derive(Default)]
struct Limiter { state:Mutex<Throttle>, changed:Condvar }
struct Permit<'a>(&'a Limiter);
impl Drop for Permit<'_> {
    fn drop(&mut self) { self.0.state.lock().unwrap().active-=1;self.0.changed.notify_all(); }
}
impl Limiter {
    fn acquire(&self,deadline:Instant) -> Result<Permit<'_>,String> {
        let mut state=self.state.lock().unwrap();
        loop {
            let now=Instant::now();
            if state.cooldown.is_some_and(|until|until>now) { return Err("translation:http:429".into()); }
            if now>=deadline { return Err("translation:timeout".into()); }
            let next=state.next.unwrap_or(now);
            if state.active<4 && next<=now {
                state.active+=1;state.next=Some(now+Duration::from_millis(150));return Ok(Permit(self));
            }
            let until=if state.active>=4 {deadline} else {next.min(deadline)};
            state=self.changed.wait_timeout(state,until.saturating_duration_since(now)).unwrap().0;
        }
    }
    fn rate_limited(&self,retry_after:Option<&str>) {
        let seconds=retry_after.and_then(|s|s.parse::<u64>().ok()).filter(|n|*n>0).unwrap_or(60).min(300);
        let mut state=self.state.lock().unwrap();let until=Instant::now()+Duration::from_secs(seconds);
        state.cooldown=Some(state.cooldown.map_or(until,|old|old.max(until)));
        drop(state);self.changed.notify_all();
    }
}

struct Fragment { line:usize, leading:String, trailing:String, text:String, query:String }

fn html_escape(text: &str) -> String {
    text.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&#39;")
}

// Decode one layer only: a literal source entity such as &lt; must survive the round trip.
fn html_decode(text: &str) -> String {
    let mut output=String::new();let mut rest=text;
    while let Some(start)=rest.find('&') {
        output.push_str(&rest[..start]);rest=&rest[start..];
        let entity=rest.find(';').filter(|end|*end<=12).and_then(|end| {
            let name=&rest[1..end];
            let ch=match name {
                "amp"=>Some('&'),"lt"=>Some('<'),"gt"=>Some('>'),"quot"=>Some('"'),"apos"=>Some('\''),"nbsp"=>Some('\u{a0}'),
                _=>name.strip_prefix("#x").or_else(||name.strip_prefix("#X")).and_then(|n|u32::from_str_radix(n,16).ok())
                    .or_else(||name.strip_prefix('#').and_then(|n|n.parse::<u32>().ok())).and_then(char::from_u32),
            };
            ch.map(|ch|(end,ch))
        });
        if let Some((end,ch))=entity {output.push(ch);rest=&rest[end+1..];}
        else {output.push('&');rest=&rest[1..];}
    }
    output.push_str(rest);output
}

fn fragments(protected: &text::Protected) -> Vec<Fragment> {
    let mut output=Vec::new();
    for (line_index,line) in protected.lines.iter().enumerate() {
        let ranges:Vec<_>=protected.tokens.iter().filter_map(|(key,_)|line.find(key).map(|start|start..start+key.len())).collect();
        let mut start=0;
        while start<line.len() {
            let tail=&line[start..];
            let mut end=tail.char_indices().nth(CHUNK_CHARS).map_or(line.len(),|(n,_)|start+n);
            if end<line.len() {
                if let Some(space)=line[start..end].rfind(char::is_whitespace) {
                    if space>=(end-start)/2 {end=start+space+line[start+space..].chars().next().unwrap().len_utf8();}
                }
                if let Some(range)=ranges.iter().find(|range|range.start<end && end<range.end) {end=range.start.max(start);if end==start {end=range.end;}}
            }
            let chunk=&line[start..end];let trimmed=chunk.trim();
            let leading=&chunk[..chunk.len()-chunk.trim_start().len()];
            let trailing=if trimmed.is_empty() {""} else {&chunk[chunk.trim_end().len()..]};
            let mut prose=trimmed.to_string();let mut html=html_escape(trimmed);
            for (key,_) in &protected.tokens {
                prose=prose.replace(key,"");
                html=html.replace(key,&format!("<span translate=\"no\">{key}</span>"));
            }
            output.push(Fragment { line:line_index,leading:leading.into(),trailing:trailing.into(),text:trimmed.into(),
                query:if prose.trim().is_empty() {String::new()} else {html} });
            start=end;
        }
    }
    output
}

fn request(agent: &ureq::Agent,endpoint: &str,body: &str,limiter: &Limiter,deadline:Instant) -> Result<Value,String> {
    let _permit=limiter.acquire(deadline)?;
    let remaining=deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {return Err("translation:timeout".into());}
    let response=agent.post(endpoint).timeout(remaining.min(Duration::from_secs(10))).set("Content-Type","application/x-www-form-urlencoded")
        .send_string(body).map_err(|error|match error {
            ureq::Error::Status(code,response)=>{
                if code==429 {limiter.rate_limited(response.header("Retry-After"));}
                format!("translation:http:{code}")
            },
            _=>"translation:network".into(),
        })?;
    let mut raw=String::new();response.into_reader().take(2_000_001).read_to_string(&mut raw).map_err(|_|"translation:network")?;
    if raw.len()>2_000_000 {return Err("translation:structure".into());}
    serde_json::from_str(&raw).map_err(|_|"translation:structure".into())
}

pub(super) fn translate(protected: &text::Protected,source: &str,target: &str) -> Result<(Vec<String>,String),String> {
    translate_at(protected,source,target,ENDPOINT,LIMITER.get_or_init(Limiter::default))
}

fn translate_at(protected: &text::Protected,source: &str,target: &str,endpoint: &str,limiter: &Limiter) -> Result<(Vec<String>,String),String> {
    let deadline=Instant::now()+Duration::from_secs(90);
    let mut url=url::Url::parse(endpoint).map_err(|_|"translation:url")?;
    url.query_pairs_mut().append_pair("sl",source).append_pair("tl",if target=="pt-BR" {"pt"} else {target});
    let agent=ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).redirects(0).build();
    let mut fragments=fragments(protected);
    let pending:Vec<_>=fragments.iter().enumerate().filter_map(|(index,f)|(!f.query.is_empty()).then_some(index)).collect();
    let mut detected=source.to_string();let mut offset=0;
    while offset<pending.len() {
        let mut batch=Vec::new();let mut body=String::new();
        while offset<pending.len() && batch.len()<50 {
            let index=pending[offset];
            let encoded=url::form_urlencoded::Serializer::new(String::new()).append_pair("q",&fragments[index].query).finish();
            if !batch.is_empty() && body.len()+encoded.len()+1>BATCH_BYTES {break;}
            if !body.is_empty() {body.push('&');}body.push_str(&encoded);batch.push(index);offset+=1;
        }
        let value=request(&agent,url.as_str(),&body,limiter,deadline)?;
        let rows=value.as_array().filter(|rows|rows.len()==batch.len()).ok_or("translation:structure")?;
        for (index,row) in batch.into_iter().zip(rows) {
            let raw=if source=="auto" {
                let entry=row.as_array().filter(|v|v.len()==2).ok_or("translation:structure")?;
                let language=entry[1].as_str().filter(|s|!s.is_empty() && s.len()<32 && s.chars().all(|c|c.is_ascii_alphanumeric() || c=='-')).ok_or("translation:language")?;
                if detected=="auto" {detected=language.into();}
                entry[0].as_str().ok_or("translation:structure")?
            } else {row.as_str().ok_or("translation:structure")?};
            let html=raw.replace("<span translate=\"no\">","").replace("</span>","");
            if html.contains(['<','>']) {return Err("translation:structure".into());}
            let decoded=html_decode(&html);
            if decoded.contains(['\n','\r']) {return Err("translation:structure".into());}
            fragments[index].text=decoded.trim().to_string();
        }
    }
    let mut lines=vec![String::new();protected.lines.len()];
    for fragment in fragments {lines[fragment.line].push_str(&format!("{}{}{}",fragment.leading,fragment.text,fragment.trailing));}
    Ok((lines,detected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_and_long_chunks_preserve_tokens_whitespace_and_unicode() {
        let original=format!("# Hello `request_id` & <value>\n\n{} {} end.\n```rs\nlet x = 1;\n```\n", "你好 ".repeat(800),"a".repeat(1980));
        let protected=text::protect(&original);let fragments=fragments(&protected);
        assert!(fragments.iter().any(|f|fragments.iter().filter(|other|other.line==f.line).count()>1));
        let mut lines=vec![String::new();protected.lines.len()];
        for f in fragments {
            let decoded=html_decode(&f.query.replace("<span translate=\"no\">","").replace("</span>",""));
            let body=if f.query.is_empty() {f.text} else {decoded};
            lines[f.line].push_str(&format!("{}{}{}",f.leading,body,f.trailing));
        }
        assert_eq!(protected.restore(&lines).unwrap().iter().map(|p|p.translation.as_str()).collect::<Vec<_>>().join("\n"),original);
        assert_eq!(html_decode("&amp;lt; &quot; &#39; &#x1f642;"),"&lt; \" ' 🙂");
    }

    #[test]
    fn anonymous_batches_parse_detection_and_reject_incomplete_responses() {
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr();
        let worker=std::thread::spawn(move || {for incomplete in [false,true] {
            let mut req=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
            assert_eq!(req.method(),&tiny_http::Method::Post);assert!(req.url().contains("sl=auto&tl=pt"));
            assert!(!req.headers().iter().any(|h|h.field.equiv("Authorization") || h.field.equiv("Cookie")));
            let mut raw=String::new();req.as_reader().read_to_string(&mut raw).unwrap();
            let queries=url::form_urlencoded::parse(raw.as_bytes()).map(|(key,text)|{assert_eq!(key,"q");text.into_owned()}).collect::<Vec<_>>();
            assert_eq!(queries.len(),3);assert!(queries.iter().all(|q|!q.contains("request_id") && !q.contains("let x")));
            let rows:Vec<_>=queries.iter().take(if incomplete {1} else {queries.len()}).map(|q|json!([q.replace("Hello","Olá"),"en"])).collect();
            req.respond(tiny_http::Response::from_string(json!(rows).to_string())).unwrap();
        }});
        let protected=text::protect("Hello `request_id` & literal &lt;\n\nHello again.\n```rs\nlet x = 1;\n```\n");
        let limiter=Limiter::default();let endpoint=format!("http://{address}/translate_a/t?client=gtx");
        let (lines,language)=translate_at(&protected,"auto","pt-BR",&endpoint,&limiter).unwrap();
        assert_eq!(language,"en");assert!(protected.restore(&lines).unwrap()[0].translation.starts_with("Olá `request_id` & literal &lt;"));
        assert_eq!(translate_at(&protected,"auto","pt-BR",&endpoint,&limiter).unwrap_err(),"translation:structure");worker.join().unwrap();
    }

    #[test]
    fn rate_limit_cooldown_blocks_follow_up_requests() {
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr();
        let worker=std::thread::spawn(move || {
            let req=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
            req.respond(tiny_http::Response::empty(429).with_header(tiny_http::Header::from_bytes("Retry-After","2").unwrap())).unwrap();
            assert!(server.recv_timeout(Duration::from_millis(300)).unwrap().is_none());
        });
        let limiter=Limiter::default();let protected=text::protect("Hello.");let endpoint=format!("http://{address}/translate_a/t");
        for _ in 0..2 {assert_eq!(translate_at(&protected,"en","de",&endpoint,&limiter).unwrap_err(),"translation:http:429");}
        worker.join().unwrap();
    }
}
