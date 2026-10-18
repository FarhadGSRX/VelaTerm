//! Cancellable content searches over the persistent search index ([`super::corpus`]).
//!
//! A job first brings the sessions in its scope up to date, which for an unchanged session costs one
//! file stat, then ranks matching sessions from the index. Snippets are read only for the page a
//! client asks for, so a common term never loads every matching fragment.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc,Mutex,OnceLock};
use std::sync::atomic::{AtomicBool,AtomicU64,Ordering};
use std::time::{Duration,Instant};
use serde_json::{json,Value};
use crate::{host::AppCtx,models::SessionKind};
use super::{SearchScope,SessionSearchHit,corpus::{self,Query,Ranked,Refreshed}};

const PAGE_BYTES:usize=512*1024;
const JOBS_LIMIT:usize=8;
struct Ready {query:Option<Query>,ranked:Vec<Ranked>,names:HashMap<String,(String,SessionKind,bool)>}
enum State {Pending,Ready(Box<Ready>),Failed(String)}
struct Job {cancel:Arc<AtomicBool>,state:Mutex<State>,touched:Mutex<Instant>,sessions:AtomicU64,bytes:AtomicU64,skipped:AtomicU64}
struct Task {app:AppCtx,job:Arc<Job>,query:String,scope:SearchScope,session:Option<String>}
static JOBS:OnceLock<Mutex<HashMap<(PathBuf,String),Arc<Job>>>>=OnceLock::new();
static QUEUE:OnceLock<std::sync::mpsc::SyncSender<Task>>=OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<(PathBuf,String),Arc<Job>>> {
    JOBS.get_or_init(|| {
        std::thread::spawn(||loop {
            std::thread::park_timeout(Duration::from_secs(30));
            if let Some(jobs)=JOBS.get() {jobs.lock().unwrap().retain(|_,job| {
                if job.touched.lock().unwrap().elapsed()>Duration::from_secs(600) {job.cancel.store(true,Ordering::Relaxed);false} else {true}
            });}
        });Mutex::new(HashMap::new())
    })
}

fn check(cancel:&AtomicBool) -> Result<(),String> {if cancel.load(Ordering::Relaxed) {Err("read_canceled".into())} else {Ok(())}}
fn queue() -> &'static std::sync::mpsc::SyncSender<Task> {
    QUEUE.get_or_init(|| {
        let (sender,receiver)=std::sync::mpsc::sync_channel::<Task>(JOBS_LIMIT);let receiver=Arc::new(Mutex::new(receiver));
        for _ in 0..2 {let receiver=receiver.clone();std::thread::spawn(move||loop {
            let task=match receiver.lock().unwrap().recv() {Ok(task)=>task,Err(_)=>return};
            let result=run(&task);*task.job.state.lock().unwrap()=match result {Ok(state)=>state,Err(error)=>State::Failed(error)};
        });}sender
    })
}
fn new_job() -> Arc<Job> {
    Arc::new(Job {cancel:Arc::new(AtomicBool::new(false)),state:Mutex::new(State::Pending),touched:Mutex::new(Instant::now()),
        sessions:AtomicU64::new(0),bytes:AtomicU64::new(0),skipped:AtomicU64::new(0)})
}
pub fn start(app:&AppCtx,query:&str,scope:SearchScope,session:Option<&str>) -> Result<Value,String> {
    if query.len()>512 || session.is_some_and(|id|id.len()>128) {return Err("Search query exceeds its byte budget".into());}
    let directory=app.data_dir()?;let mut jobs=jobs().lock().unwrap();
    jobs.retain(|_,job| {
        if job.touched.lock().unwrap().elapsed()>Duration::from_secs(600) {job.cancel.store(true,Ordering::Relaxed);false} else {true}
    });
    if jobs.len()>=JOBS_LIMIT {return Err("index_busy: too many active content searches".into());}
    let id=format!("search:{}",uuid::Uuid::new_v4());let job=new_job();
    queue().try_send(Task {app:app.clone(),job:job.clone(),query:query.into(),scope,session:session.map(str::to_owned)}).map_err(|_|"index_busy: search queue is full")?;
    jobs.insert((directory,id.clone()),job);Ok(json!({"jobId":id,"status":"pending","scannedSessions":0,"scannedBytes":0}))
}
fn find(app:&AppCtx,id:&str) -> Result<Arc<Job>,String> {
    let job=jobs().lock().unwrap().get(&(app.data_dir()?,id.into())).cloned().ok_or("cursor_expired: content search expired")?;
    if job.touched.lock().unwrap().elapsed()>Duration::from_secs(600) {job.cancel.store(true,Ordering::Relaxed);return Err("cursor_expired: content search expired".into());}
    *job.touched.lock().unwrap()=Instant::now();Ok(job)
}
pub fn cancel(app:&AppCtx,id:&str) -> Result<(),String> {
    let job=find(app,id)?;job.cancel.store(true,Ordering::Relaxed);
    let mut state=job.state.lock().unwrap();if matches!(*state,State::Ready(_)) {*state=State::Failed("read_canceled".into());}
    JOBS.get().unwrap().lock().unwrap().remove(&(app.data_dir()?,id.into()));Ok(())
}
pub fn page(app:&AppCtx,id:&str,after:Option<u64>,limit:Option<u64>,transcripts_only:bool) -> Result<Value,String> {
    page_window(app,id,after,None,limit,transcripts_only)
}
/// Cursors are one-based positions in the job's ranking; `after` and `before` are exclusive.
pub fn page_window(app:&AppCtx,id:&str,after:Option<u64>,before:Option<u64>,limit:Option<u64>,transcripts_only:bool) -> Result<Value,String> {
    let job=find(app,id)?;check(&job.cancel)?;let state=job.state.lock().unwrap();
    let progress=json!({"scannedSessions":job.sessions.load(Ordering::Relaxed),"scannedBytes":job.bytes.load(Ordering::Relaxed),"skippedSessions":job.skipped.load(Ordering::Relaxed)});
    let ready=match &*state {
        State::Pending=>{let mut value=json!({"jobId":id,"status":"pending"});value.as_object_mut().unwrap().extend(progress.as_object().unwrap().clone());return Ok(value);},
        State::Failed(error)=>return Err(error.clone()),State::Ready(ready)=>ready,
    };
    if before.is_some() && after.is_some() {return Err("cursor_expired: invalid content search cursor".into());}
    let limit=limit.unwrap_or(10).clamp(1,50) as usize;
    let visible:Vec<(u64,&Ranked)>=ready.ranked.iter().filter(|ranked|!transcripts_only || ranked.source=="transcript")
        .enumerate().map(|(index,ranked)|(index as u64+1,ranked)).collect();
    let selected:Vec<&(u64,&Ranked)>=if let Some(before)=before {
        let earlier:Vec<_>=visible.iter().filter(|(position,_)|*position<before).collect();earlier[earlier.len().saturating_sub(limit)..].to_vec()
    } else {let from=after.unwrap_or(0);visible.iter().filter(|(position,_)|*position>from).take(limit).collect()};
    let mut hits=Vec::new();let mut bytes=2048;let mut last=None;
    if let Some(query)=&ready.query {
        let conn=corpus::open(app)?;
        for (position,ranked) in &selected {
            let Some(hit)=corpus::hit(&conn,query,ranked,&ready.names)? else {continue;};
            let value=serde_json::to_value(&hit).map_err(|e|e.to_string())?;let size=value.to_string().len();
            if bytes+size>PAGE_BYTES {if hits.is_empty() {return Err("unsupported_record: search result exceeds its response budget".into());}break;}
            bytes+=size;hits.push(value);last=Some(*position);
        }
    }
    let first=selected.first().map(|item|item.0);
    let total=visible.len() as u64;let recordings=ready.ranked.iter().filter(|ranked|ranked.source!="transcript").count() as u64;
    let matches:u64=ready.ranked.iter().filter(|ranked|!transcripts_only || ranked.source=="transcript").map(|ranked|ranked.count as u64).sum();
    let mut value=json!({"jobId":id,"status":"complete","hits":hits,"totalSessions":total,"totalMatches":matches,"recordingOnlySessions":recordings,
        "nextCursor":last.filter(|last|visible.iter().any(|(position,_)|position>last)),
        "previousCursor":first.filter(|first|*first>1),"answer":ready.query.as_ref().map_or(0,|query|query.answered.code())});
    value.as_object_mut().unwrap().extend(progress.as_object().unwrap().clone());Ok(value)
}

fn in_scope(session:&corpus::Session,task:&Task) -> bool {
    task.session.as_ref().is_none_or(|id|id==&session.id) && match task.scope {SearchScope::All=>true,SearchScope::Live=>!session.archived,SearchScope::Archived=>session.archived}
}

fn run(task:&Task) -> Result<State,String> {
    check(&task.job.cancel)?;
    let conn=corpus::open(&task.app)?;
    let keys=task.query.split_whitespace().count();
    let mut after=None;let mut names=HashMap::new();
    loop {
        let sessions=corpus::sessions(&task.app,after.as_deref())?;if sessions.is_empty() {break;}
        for session in sessions {
            check(&task.job.cancel)?;after=Some(session.id.clone());
            names.insert(session.id.clone(),(session.name.clone(),session.kind,session.archived));
            if keys==0 || !in_scope(&session,task) {continue;}
            // One conversation that cannot be read must not discard every other result. Only
            // cancellation and storage exhaustion end the whole search.
            match corpus::refresh_session(&task.app,&conn,&session,&task.job.cancel) {
                Ok(Refreshed::Indexed(bytes))=>{task.job.bytes.fetch_add(bytes,Ordering::Relaxed);},
                Ok(_)=>{},
                Err(error) if error=="read_canceled" || error.starts_with("storage_pressure:")=>return Err(error),
                Err(error)=> {
                    task.job.skipped.fetch_add(1,Ordering::Relaxed);
                    let code=error.split_once(':').map(|(code,_)|code).filter(|code|code.chars().all(|ch|ch.is_ascii_lowercase() || ch=='_')).unwrap_or("operation_failed");
                    crate::diagnostics::record("WARN","search_session_skipped",json!({"sessionId":session.id,"errorCode":code}));
                },
            }
            task.job.sessions.fetch_add(1,Ordering::Relaxed);
        }
    }
    check(&task.job.cancel)?;
    let (query,ranked)=corpus::rank(&conn,&task.query,task.scope,task.session.as_deref())?;
    let ranked=ranked.into_iter().filter(|ranked|names.contains_key(&ranked.session)).collect();
    Ok(State::Ready(Box::new(Ready {query,ranked,names})))
}

/// Old clients receive one bounded result batch and a clear upgrade response when
/// they need pagination. CLI callers pass their limit before decoding any hits.
pub fn blocking(app:&AppCtx,query:&str,scope:SearchScope,session:Option<&str>,limit:u64,transcripts_only:bool) -> Result<Value,String> {
    let started=start(app,query,scope,session)?;let id=started["jobId"].as_str().unwrap();
    let result=(||loop {
        let result=page(app,id,None,Some(limit),transcripts_only)?;
        if result["status"]=="complete" {return Ok(result);}
        std::thread::park_timeout(Duration::from_millis(100));
    })();let _=cancel(app,id);result
}
pub fn legacy(app:&AppCtx,query:&str,scope:SearchScope,session:Option<&str>) -> Result<Vec<SessionSearchHit>,String> {
    let result=blocking(app,query,scope,session,50,false)?;
    if !result["nextCursor"].is_null() {return Err(crate::agent::history_store::compat::UPGRADE.into());}
    result["hits"].as_array().ok_or("Invalid search results")?.iter().map(|hit|Ok(SessionSearchHit {
        session_id:hit["sessionId"].as_str().ok_or("Invalid search session")?.into(),name:hit["name"].as_str().ok_or("Invalid search name")?.into(),
        kind:SessionKind::from_db(hit["kind"].as_str().ok_or("Invalid search kind")?),archived:hit["archived"]==true,
        source:if hit["source"]=="transcript" {"transcript"} else {"recording"},match_count:hit["matchCount"].as_u64().ok_or("Invalid search count")? as u32,
        matches:serde_json::from_value(hit["matches"].clone()).map_err(|e|e.to_string())?,
    })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use crate::{agent::history_store as history,db::Db,host::HeadlessHost};
    struct Fixture {path:PathBuf,app:AppCtx}
    impl Fixture {
        fn new() -> Self {
            let path=std::env::temp_dir().join(format!("vlx-search-jobs-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(path.join("recordings")).unwrap();
            let db=Db::open(&path.join("business.sqlite")).unwrap();
            db.conn.lock().unwrap().execute("INSERT INTO projects(id,name,root_path,sort_order,created_at) VALUES('p','Project','/p',0,0)",[]).unwrap();
            let app=AppCtx::Headless(Arc::new(HeadlessHost::new(path.clone(),db)));Self {path,app}
        }
        fn session(&self,id:&str,text:&str,archived:bool) {
            self.app.db().conn.lock().unwrap().execute("INSERT INTO sessions(id,project_id,name,kind,sort_order,created_at,archived_at) VALUES(?1,'p',?1,'terminal',0,0,?2)",params![id,archived.then_some(1)]).unwrap();
            self.session_text(id,text);
        }
        fn session_text(&self,id:&str,text:&str) {
            let path=self.path.join("recordings").join(format!("{id}.log"));std::fs::write(&path,text).unwrap();
            // Size and second-resolution modification time identify a change; make it visible to both.
            let later=std::time::SystemTime::now()+Duration::from_secs(2);std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        }
        fn task(&self,query:&str,scope:SearchScope,session:Option<&str>) -> Task {
            Task {app:self.app.clone(),query:query.into(),scope,session:session.map(str::to_owned),job:new_job()}
        }
        fn hits(&self,task:&Task) -> Vec<Value> {
            let State::Ready(ready)=run(task).unwrap() else {panic!("Search must complete");};
            let Some(query)=&ready.query else {return Vec::new();};let conn=corpus::open(&self.app).unwrap();
            ready.ranked.iter().filter_map(|ranked|corpus::hit(&conn,query,ranked,&ready.names).unwrap()).map(|hit|serde_json::to_value(hit).unwrap()).collect()
        }
    }
    impl Drop for Fixture {fn drop(&mut self) {history::store::release_test_directory(&self.path);let _=std::fs::remove_dir_all(&self.path);}}
    #[cfg(unix)]
    #[test]
    fn an_unreadable_conversation_is_skipped_without_discarding_other_results() {
        let fixture=Fixture::new();
        for id in ["a-first","b-broken","c-last"] {fixture.session(id,"search needle\n",false);}
        // A recording that exists but cannot be read.
        use std::os::unix::fs::PermissionsExt;
        let broken=fixture.path.join("recordings").join("b-broken.log");std::fs::set_permissions(&broken,std::fs::Permissions::from_mode(0o000)).unwrap();
        let task=fixture.task("needle",SearchScope::All,None);
        let hits=fixture.hits(&task);
        let sessions=hits.iter().map(|hit|hit["sessionId"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert!(sessions.contains(&"a-first".to_owned()) && sessions.contains(&"c-last".to_owned()),"{sessions:?}");
        assert!(!sessions.contains(&"b-broken".to_owned()));
        assert_eq!(task.job.skipped.load(Ordering::Relaxed),1);assert_eq!(task.job.sessions.load(Ordering::Relaxed),3);
        std::fs::set_permissions(&broken,std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[test]
    fn jobs_index_sessions_once_and_follow_word_fallback_scope_and_ranking() {
        let fixture=Fixture::new();fixture.session("a","The lead agent spawned two children.\n我买了一件和服，很好看。\n",false);
        fixture.session("b","Run vspawn to create a child session.\n总和服务的费用还没有结清。\nlist every worktree before merging\n",false);
        fixture.session("c","The lead agent spawned two children.\n",true);
        fixture.session("d",&"deploy line\n".repeat(72),false);
        let sessions=|hits:Vec<Value>| hits.iter().map(|hit|hit["sessionId"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        let first=fixture.task("spawn",SearchScope::All,None);assert_eq!(sessions(fixture.hits(&first)),["a","c"]);
        assert_eq!(first.job.bytes.load(Ordering::Relaxed)>0,true);
        // A second search finds every session unchanged and reads nothing.
        let again=fixture.task("和服",SearchScope::Live,None);assert_eq!(sessions(fixture.hits(&again)),["a"]);assert_eq!(again.job.bytes.load(Ordering::Relaxed),0);
        assert_eq!(sessions(fixture.hits(&fixture.task("worktre",SearchScope::Live,None))),["b"]);
        assert_eq!(fixture.hits(&fixture.task("deploy",SearchScope::Live,None))[0]["matchCount"],72);
        assert_eq!(sessions(fixture.hits(&fixture.task("children",SearchScope::Archived,None))),["c"]);
        assert_eq!(sessions(fixture.hits(&fixture.task("spawn",SearchScope::All,Some("a")))),["a"]);
        // A changed recording is indexed again.
        fixture.session_text("a","Nothing about that anymore.\n");
        assert!(!sessions(fixture.hits(&fixture.task("和服",SearchScope::Live,None))).contains(&"a".to_owned()));
        assert_eq!(sessions(fixture.hits(&fixture.task("anymore",SearchScope::Live,None))),["a"]);
    }
    #[test]
    fn giant_logical_lines_match_across_buffers_and_like_uses_sqlite_ascii_case_rules() {
        let fixture=Fixture::new();let line=format!("alpha {} omega\n","中😀,. ".repeat(30000));fixture.session("a",&line,false);
        // A line longer than one fragment is indexed in pieces; each term is found, AND holds within a piece.
        assert_eq!(fixture.hits(&fixture.task("alpha",SearchScope::All,None))[0]["matchCount"],1);
        assert_eq!(fixture.hits(&fixture.task("omega",SearchScope::All,None))[0]["matchCount"],1);
        fixture.session("b","zzÄzz\nzzäzz\nzzAzZ\n",false);
        let hits=fixture.hits(&fixture.task("zÄ",SearchScope::All,Some("b")));assert_eq!(hits.len(),1);assert_eq!(hits[0]["matchCount"],1);
        let hits=fixture.hits(&fixture.task("zA",SearchScope::All,Some("b")));assert_eq!(hits[0]["matchCount"],1);
        assert!(fixture.hits(&fixture.task("alpha absent",SearchScope::All,None)).is_empty());
    }
    #[test]
    fn jobs_page_before_materialization_cancel_and_authorize_the_data_directory() {
        let fixture=Fixture::new();for index in 0..15 {fixture.session(&format!("s-{index:02}"),"search needle\n",false);}
        let other=Fixture::new();let started=start(&fixture.app,"needle",SearchScope::All,None).unwrap();let id=started["jobId"].as_str().unwrap();
        let began=Instant::now();let first=loop {let page=page(&fixture.app,id,None,Some(3),false).unwrap();if page["status"]=="complete" {break page;}
            assert!(began.elapsed()<Duration::from_secs(10));std::thread::sleep(Duration::from_millis(10));};
        assert_eq!(first["hits"].as_array().unwrap().len(),3);assert_eq!(first["totalSessions"],15);assert_eq!(first["recordingOnlySessions"],15);
        assert!(first.to_string().len()<=PAGE_BYTES);assert!(page(&other.app,id,None,None,false).unwrap_err().starts_with("cursor_expired:"));
        let second=page(&fixture.app,id,first["nextCursor"].as_u64(),Some(3),false).unwrap();assert_ne!(first["hits"][0]["sessionId"],second["hits"][0]["sessionId"]);
        assert!(page(&fixture.app,id,None,Some(3),true).unwrap()["hits"].as_array().unwrap().is_empty());
        cancel(&fixture.app,id).unwrap();assert!(page(&fixture.app,id,None,None,false).unwrap_err().starts_with("cursor_expired:"));
        assert!(start(&fixture.app,&"x".repeat(513),SearchScope::All,None).is_err());
        let task=fixture.task("needle",SearchScope::All,None);task.job.cancel.store(true,Ordering::Relaxed);assert_eq!(run(&task).err().unwrap(),"read_canceled");
        assert!(corpus::path(&fixture.path).is_file());
    }
}
