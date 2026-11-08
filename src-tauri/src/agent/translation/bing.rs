//! Bing's anonymous web translator, with expiring page credentials and bounded plain-text requests.

use super::*;
use super::throttle::Limiter;

const BASE: &str = "https://www.bing.com";
const CHUNK_UNITS: usize = 1000;
static CLIENT: OnceLock<Client> = OnceLock::new();

struct Auth { key:String, token:String, ig:String, iid:String, expires:Instant }
#[derive(Default)]
struct Client { limiter:Limiter, auth:Mutex<Option<Auth>> }
struct Fragment { line:usize, text:String, translate:bool }

fn language(code: &str) -> &str {
    match code { "auto"=>"auto-detect", "zh-CN"=>"zh-Hans", "zh-TW"=>"zh-Hant", "pt-BR"=>"pt", _=>code }
}
fn detected_language(code: &str) -> Result<String,String> {
    if code.is_empty() || code.len()>32 || !code.chars().all(|c|c.is_ascii_alphanumeric() || c=='-') {return Err("translation:language".into());}
    Ok(match code { "zh-Hans"=>"zh-CN", "zh-Hant"=>"zh-TW", "pt"=>"pt-BR", _=>code }.into())
}

// Tokens never leave the host. Splitting at their boundaries preserves code, URLs and field newlines.
fn fragments(protected: &text::Protected) -> Vec<Fragment> {
    let mut output=Vec::new();
    for (line,text) in protected.lines.iter().enumerate() {
        let mut rest=text.as_str();
        while !rest.is_empty() {
            let next=protected.tokens.iter().filter_map(|(key,_)|rest.find(key).map(|offset|(offset,key.len()))).min_by_key(|v|v.0);
            if let Some((0,len))=next {
                output.push(Fragment {line,text:rest[..len].into(),translate:false});rest=&rest[len..];continue;
            }
            let end=next.map_or(rest.len(),|v|v.0);let prose=&rest[..end];
            let mut units=0;let mut cut=prose.len();
            for (offset,ch) in prose.char_indices() {
                units+=ch.len_utf16();if units>CHUNK_UNITS {cut=offset;break;}
            }
            if cut<prose.len() {
                if let Some(space)=prose[..cut].rfind(char::is_whitespace) {
                    if space>=cut/2 {cut=space+prose[space..].chars().next().unwrap().len_utf8();}
                }
            }
            let text=&prose[..cut];
            output.push(Fragment {line,text:text.into(),translate:text.chars().any(char::is_alphabetic)});
            rest=&rest[cut..];
        }
    }
    output
}

fn response(result: Result<ureq::Response,ureq::Error>,limiter: &Limiter) -> Result<String,String> {
    let response=result.map_err(|error|match error {
        ureq::Error::Status(code,response)=>{
            if code==429 {limiter.rate_limited(response.header("Retry-After"));}
            format!("translation:http:{code}")
        },
        _=>"translation:network".into(),
    })?;
    let mut raw=String::new();response.into_reader().take(2_000_001).read_to_string(&mut raw).map_err(|_|"translation:network")?;
    if raw.len()>2_000_000 {return Err("translation:structure".into());}Ok(raw)
}

fn quoted_after<'a>(text: &'a str,marker: &str) -> Result<&'a str,String> {
    text.split_once(marker).and_then(|(_,rest)|rest.split_once('"').map(|v|v.0))
        .filter(|s|!s.is_empty() && s.len()<256).ok_or_else(||"translation:structure".into())
}

fn parse_auth(html: &str) -> Result<Auth,String> {
    let raw=html.split_once("params_AbusePreventionHelper").and_then(|(_,rest)|rest.split_once('=').map(|v|v.1))
        .and_then(|rest|rest.split_once(';').map(|v|v.0)).ok_or("translation:structure")?;
    let params:Value=serde_json::from_str(raw.trim()).map_err(|_|"translation:structure")?;
    let key=params[0].as_u64().ok_or("translation:structure")?.to_string();
    let token=params[1].as_str().filter(|s|!s.is_empty() && s.len()<8192).ok_or("translation:structure")?.to_string();
    let ttl=params[2].as_u64().filter(|ttl|*ttl>0).ok_or("translation:structure")?.min(3_600_000).saturating_sub(60_000);
    Ok(Auth {key,token,ig:quoted_after(html,"IG:\"")?.into(),iid:quoted_after(html,"data-iid=\"")?.into(),expires:Instant::now()+Duration::from_millis(ttl)})
}

fn timeout(deadline:Instant) -> Result<Duration,String> {
    let remaining=deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {Err("translation:timeout".into())} else {Ok(remaining.min(Duration::from_secs(10)))}
}

fn request(agent: &ureq::Agent,client: &Client,base: &str,text: &str,source: &str,target: &str,deadline:Instant) -> Result<Value,String> {
    // A rejected page token is refreshed once; other failures are left to an explicit retry.
    for attempt in 0..2 {
        let _permit=client.limiter.acquire(deadline)?;
        let (endpoint,body,token)={
            let mut cached=client.auth.lock().unwrap();
            if cached.as_ref().is_none_or(|auth|auth.expires<=Instant::now()) {
                // Cache-busting avoids a cached page supplying credentials from an earlier visit.
                let page=format!("{base}/translator?_={}",now_ms());
                let html=response(agent.get(&page).timeout(timeout(deadline)?).call(),&client.limiter)?;
                *cached=Some(parse_auth(&html)?);
            }
            let auth=cached.as_ref().unwrap();
            let mut endpoint=url::Url::parse(&format!("{base}/ttranslatev3")).map_err(|_|"translation:url")?;
            endpoint.query_pairs_mut().append_pair("isVertical","1").append_pair("IG",&auth.ig).append_pair("IID",&format!("{}.1",auth.iid));
            let body=url::form_urlencoded::Serializer::new(String::new()).append_pair("fromLang",language(source))
                .append_pair("to",language(target)).append_pair("text",text).append_pair("token",&auth.token).append_pair("key",&auth.key).finish();
            (endpoint,body,auth.token.clone())
        };
        let outcome=response(agent.post(endpoint.as_str()).timeout(timeout(deadline)?).set("Content-Type","application/x-www-form-urlencoded")
            .set("Referer",&format!("{base}/translator")).send_string(&body),&client.limiter).and_then(|raw| {
                let value:Value=serde_json::from_str(&raw).map_err(|_|"translation:structure")?;
                // Bing can report service errors inside an HTTP 200 response.
                if let Some(code)=value.get("statusCode").and_then(Value::as_u64).filter(|code|*code>=400 && *code<=599) {
                    if code==429 {client.limiter.rate_limited(None);}
                    return Err(format!("translation:http:{code}"));
                }
                Ok(value)
            });
        if outcome.as_ref().err().is_some_and(|error|error=="translation:http:401" || error=="translation:http:403") {
            let mut cached=client.auth.lock().unwrap();
            if cached.as_ref().is_some_and(|auth|auth.token==token) {*cached=None;}
            if attempt==0 {continue;}
        }
        return outcome;
    }
    unreachable!()
}

pub(super) fn translate(protected: &text::Protected,source: &str,target: &str) -> Result<(Vec<String>,String),String> {
    translate_at(protected,source,target,BASE,CLIENT.get_or_init(Client::default))
}

fn translate_at(protected: &text::Protected,source: &str,target: &str,base: &str,client: &Client) -> Result<(Vec<String>,String),String> {
    let deadline=Instant::now()+Duration::from_secs(90);
    let agent=ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).redirects(0).build();
    let mut lines=vec![String::new();protected.lines.len()];let mut detected=source.to_string();
    for fragment in fragments(protected) {
        let mut translated=fragment.text.clone();
        if fragment.translate {
            let text=fragment.text.trim();
            let value=request(&agent,client,base,text,source,target,deadline)?;
            // Additional entries may contain source transliteration metadata.
            let rows=value.as_array().filter(|rows|!rows.is_empty()).ok_or("translation:structure")?;
            let translations=rows[0]["translations"].as_array().ok_or("translation:structure")?;
            let entry=translations.iter().find(|entry|entry["to"]==language(target)).ok_or("translation:structure")?;
            let output=entry["text"].as_str().filter(|s|!s.trim().is_empty() && !s.contains(['\n','\r'])).ok_or("translation:structure")?;
            if source=="auto" && detected=="auto" {detected=detected_language(rows[0]["detectedLanguage"]["language"].as_str().ok_or("translation:language")?)?;}
            let leading=&fragment.text[..fragment.text.len()-fragment.text.trim_start().len()];
            let trailing=&fragment.text[fragment.text.trim_end().len()..];
            translated=format!("{leading}{}{trailing}",output.trim());
        }
        lines[fragment.line].push_str(&translated);
    }
    Ok((lines,detected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(token: &str) -> String {
        format!("IG:\"fixture-ig\"; <div data-iid=\"translator.1\"></div><script>var params_AbusePreventionHelper = [123,\"{token}\",3600000];</script>")
    }

    #[test]
    fn token_boundaries_and_utf16_limits_preserve_the_document() {
        let original=format!("# Hello `request_id`\n\n{} https://example.com/a?q=1&b=2 end.\n```rs\nlet x = 1;\n```\n", "你好🙂 ".repeat(500));
        let protected=text::protect(&original);let mut lines=vec![String::new();protected.lines.len()];
        for fragment in fragments(&protected) {
            if fragment.translate {assert!(fragment.text.encode_utf16().count()<=CHUNK_UNITS);assert!(!fragment.text.contains("__VLXT_"));}
            lines[fragment.line].push_str(&fragment.text);
        }
        assert_eq!(protected.restore(&lines).unwrap().iter().map(|p|p.translation.as_str()).collect::<Vec<_>>().join("\n"),original);
        assert!(parse_auth("unexpected page").is_err());
        assert_eq!(language("zh-TW"),"zh-Hant");assert_eq!(detected_language("zh-Hans").unwrap(),"zh-CN");
    }

    #[test]
    fn requests_refresh_expired_credentials_and_preserve_protected_text() {
        let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr();
        let worker=std::thread::spawn(move || {
            for index in 0..5 {
                let mut req=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
                if index==0 || index==2 {
                    assert!(req.url().starts_with("/translator?_="));assert_eq!(req.method(),&tiny_http::Method::Get);
                    req.respond(tiny_http::Response::from_string(page(if index==0 {"expired"} else {"fresh"}))).unwrap();continue;
                }
                assert!(req.url().starts_with("/ttranslatev3?isVertical=1&IG=fixture-ig&IID=translator.1.1"));
                let mut raw=String::new();req.as_reader().read_to_string(&mut raw).unwrap();
                let fields=url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect::<HashMap<_,_>>();
                assert_eq!(fields["fromLang"],"auto-detect");assert_eq!(fields["to"],"zh-Hans");
                assert!(!fields["text"].contains("request_id"));assert!(!fields["text"].contains("__VLXT_"));
                if index==1 {assert_eq!(fields["token"],"expired");req.respond(tiny_http::Response::empty(401)).unwrap();continue;}
                assert_eq!(fields["token"],"fresh");
                req.respond(tiny_http::Response::from_string(json!([{"detectedLanguage":{"language":"en"},"translations":[{"to":"zh-Hans","text":"你好"}]},{"inputTransliteration":"Hello"}]).to_string())).unwrap();
            }
        });
        let protected=text::protect("Hello `request_id`\n\nHello again.");
        let (lines,detected)=translate_at(&protected,"auto","zh-CN",&format!("http://{address}"),&Client::default()).unwrap();
        assert_eq!(detected,"en");
        assert_eq!(protected.restore(&lines).unwrap().iter().map(|p|p.translation.as_str()).collect::<Vec<_>>().join("\n"),"你好 `request_id`\n\n你好");
        worker.join().unwrap();
    }

    #[test]
    fn rate_limits_block_follow_up_requests_and_malformed_responses_fail() {
        for (body,status,expected) in [("",429,"translation:http:429"),("{\"statusCode\":429}",200,"translation:http:429"),("[]",200,"translation:structure"),("<html>Unavailable</html>",200,"translation:structure")] {
            let server=tiny_http::Server::http("127.0.0.1:0").unwrap();let address=server.server_addr();
            let worker=std::thread::spawn(move || {
                let req=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
                req.respond(tiny_http::Response::from_string(page("fresh"))).unwrap();
                let req=server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
                req.respond(tiny_http::Response::from_string(body).with_status_code(status)).unwrap();
                if expected=="translation:http:429" {assert!(server.recv_timeout(Duration::from_millis(300)).unwrap().is_none());}
            });
            let client=Client::default();let protected=text::protect("Hello.");let base=format!("http://{address}");
            assert_eq!(translate_at(&protected,"en","de",&base,&client).unwrap_err(),expected);
            if expected=="translation:http:429" {assert_eq!(translate_at(&protected,"en","de",&base,&client).unwrap_err(),expected);}
            worker.join().unwrap();
        }
    }

    #[test]
    #[ignore = "Run explicitly against a local Postman simulation with VLX_BING_TEST_BASE set."]
    fn bing_http_simulation() {
        let base=std::env::var("VLX_BING_TEST_BASE").expect("Set VLX_BING_TEST_BASE to the local simulation route");
        let protected=text::protect("Hello `request_id`\n\nHello again.");
        let (lines,detected)=translate_at(&protected,"auto","zh-CN",&base,&Client::default()).unwrap();
        assert_eq!(detected,"en");
        assert_eq!(protected.restore(&lines).unwrap().iter().map(|p|p.translation.as_str()).collect::<Vec<_>>().join("\n"),"你好 `request_id`\n\n你好");
    }
}
