//! Session-authorized subscriptions keep cancellation local to a reader of a shared physical index.

use std::collections::HashMap;
use std::time::{Duration,Instant};
use std::sync::atomic::Ordering;

use super::store::{RegisteredSource,Store};

pub(super) struct Reader {scope:String,pub(super) sources:Vec<String>,pub(super) touched:Instant}
pub(super) type Readers=HashMap<String,Reader>;

impl Store {
    pub fn subscribe(&self,scope:&str,sources:&[RegisteredSource],existing:Option<&str>) -> Result<String,String> {
        let keys=sources.iter().map(|source|format!("{}:{}",source.id,source.generation)).collect::<Vec<_>>();
        let mut readers=self.readers.lock().unwrap();
        readers.retain(|_,reader|reader.touched.elapsed()<Duration::from_secs(600));
        if let Some(id)=existing {
            let reader=readers.get_mut(id).ok_or("cursor_expired: index reader expired")?;
            if reader.scope!=scope {return Err("cursor_expired: index reader belongs to another session".into());}
            reader.sources=keys;reader.touched=Instant::now();return Ok(id.to_owned());
        }
        if readers.len()>=128 {return Err("index_busy: too many active history readers".into());}
        let id=format!("index:{}",uuid::Uuid::new_v4());
        readers.insert(id.clone(),Reader {scope:scope.into(),sources:keys,touched:Instant::now()});Ok(id)
    }

    pub fn cancel_reader(&self,scope:&str,id:&str) -> Result<bool,String> {
        let mut readers=self.readers.lock().unwrap();
        let Some(reader)=readers.get(id) else {return Ok(false);};
        if reader.scope!=scope {return Err("cursor_expired: index reader belongs to another session".into());}
        let reader=readers.remove(id).unwrap();
        readers.retain(|_,reader|reader.touched.elapsed()<Duration::from_secs(600));
        let jobs=self.jobs.lock().unwrap();
        for key in reader.sources {
            if !readers.values().any(|other|other.sources.contains(&key)) {
                if let Some(cancel)=jobs.get(&key) {cancel.store(true,Ordering::Relaxed);}
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc,atomic::AtomicBool};

    #[test]
    fn canceling_one_window_preserves_another_window_and_checks_session_scope() {
        let temp=super::super::tests::Temp::new();
        let branch=super::super::tests::indexed(&temp,"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"hello\"}]}}\n");
        let source=&branch.sources[0];let key=format!("{}:{}",source.id,source.generation);
        let cancel=Arc::new(AtomicBool::new(false));branch.store.jobs.lock().unwrap().insert(key.clone(),cancel.clone());
        let first=branch.store.subscribe("session",&branch.sources,None).unwrap();
        let second=branch.store.subscribe("session",&branch.sources,None).unwrap();
        assert!(branch.store.cancel_reader("foreign",&first).is_err());assert!(!cancel.load(Ordering::Relaxed));
        assert!(branch.store.cancel_reader("session",&first).unwrap());assert!(!cancel.load(Ordering::Relaxed));
        assert_eq!(branch.store.subscribe("session",&branch.sources,Some(&second)).unwrap(),second);
        assert!(branch.store.cancel_reader("session",&second).unwrap());assert!(cancel.load(Ordering::Relaxed));
        assert!(!branch.store.cancel_reader("session",&second).unwrap());branch.store.scan_finished(&key);
        assert_eq!(branch.page(None,None,None).unwrap()["knownRows"],1);
        assert!(source.segment.path.exists());
    }
}
