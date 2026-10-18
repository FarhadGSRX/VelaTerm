//! A fixed worker pool keeps queued source scans from creating one operating-system thread per file.

use std::collections::VecDeque;
use std::sync::{Arc,Condvar,Mutex,OnceLock};
use std::sync::atomic::{AtomicBool,Ordering};
#[cfg(test)] use std::sync::atomic::AtomicUsize;

use crate::models::SessionKind;
use super::store::{RegisteredSource,Store};

const WORKERS:usize=2;
const QUEUED_SOURCES:usize=64;
struct Job {store:Arc<Store>,kind:SessionKind,source:RegisteredSource,key:String,cancel:Arc<AtomicBool>}
struct Queue {jobs:Mutex<VecDeque<Job>>,ready:Condvar}
static QUEUE:OnceLock<Arc<Queue>>=OnceLock::new();
#[cfg(test)] static ACTIVE:AtomicUsize=AtomicUsize::new(0);
#[cfg(test)] static PEAK:AtomicUsize=AtomicUsize::new(0);

pub(super) fn schedule(store:Arc<Store>,kind:SessionKind,source:RegisteredSource,key:String,cancel:Arc<AtomicBool>) -> bool {
    let queue=QUEUE.get_or_init(|| {
        let queue=Arc::new(Queue {jobs:Mutex::new(VecDeque::<Job>::new()),ready:Condvar::new()});
        for _ in 0..WORKERS {
            let queue=queue.clone();
            std::thread::spawn(move ||loop {
                let job={
                    let mut jobs=queue.jobs.lock().unwrap();
                    while jobs.is_empty() {jobs=queue.ready.wait(jobs).unwrap();}
                    jobs.pop_front().unwrap()
                };
                if !job.cancel.load(Ordering::Relaxed) {
                    #[cfg(test)] {let count=ACTIVE.fetch_add(1,Ordering::SeqCst)+1;PEAK.fetch_max(count,Ordering::SeqCst);}
                    if let Err(error)=job.store.scan(job.kind,&job.source,&job.cancel,false) {
                        if error!="read_canceled" && !error.ends_with("read_canceled") {
                            if let Ok(conn)=job.store.connect() {
                                let _=conn.execute("UPDATE sources SET status='error',error=?3 WHERE id=?1 AND generation=?2",
                                    rusqlite::params![job.source.id,job.source.generation,error]);
                            }
                            // Provider bodies, paths and projections never enter diagnostics.
                            crate::diagnostic_warn!("history index failed source={} generation={}",job.source.id,job.source.generation);
                        }
                    }
                    #[cfg(test)] {ACTIVE.fetch_sub(1,Ordering::SeqCst);}
                }
                job.store.scan_finished(&job.key);
            });
        }
        queue
    });
    let mut jobs=queue.jobs.lock().unwrap();
    if jobs.len()>=QUEUED_SOURCES {return false;}
    jobs.push_back(Job {store,kind,source,key,cancel});
    queue.ready.notify_one();true
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::source::Segment;

    #[test]
    fn queued_sources_complete_in_the_fixed_pool_and_preserve_each_recording() {
        let temp=super::super::tests::Temp::new();let store=Store::at(&temp.0).unwrap();let mut sources=Vec::new();
        for index in 0..20 {
            let path=temp.0.join(format!("native-{index}.jsonl"));
            std::fs::write(&path,serde_json::json!({"type":"response_item","payload":{"type":"message","id":format!("message-{index}"),
                "role":"user","content":[{"type":"input_text","text":"hello"}]}}).to_string()+"\n").unwrap();
            let source=store.register(SessionKind::Codex,Segment {path,native_id:format!("native-{index}"),end_byte:None,end_ordinal:None}).unwrap();
            store.schedule(SessionKind::Codex,source.clone());sources.push(source);
        }
        let began=std::time::Instant::now();
        while !store.jobs.lock().unwrap().is_empty() && began.elapsed()<std::time::Duration::from_secs(20) {std::thread::sleep(std::time::Duration::from_millis(10));}
        assert!(store.jobs.lock().unwrap().is_empty(),"Every queued read must settle before removing the fixture");
        assert!(PEAK.load(Ordering::SeqCst)>0 && PEAK.load(Ordering::SeqCst)<=WORKERS);
        let conn=store.connect().unwrap();
        assert_eq!(conn.query_row("SELECT count(*) FROM sources WHERE status='complete'",[],|r|r.get::<_,u64>(0)).unwrap(),20);
        assert_eq!(conn.query_row("SELECT count(*) FROM rows",[],|r|r.get::<_,u64>(0)).unwrap(),20);
        assert!(sources.iter().all(|source|source.segment.path.exists()));
    }
}
