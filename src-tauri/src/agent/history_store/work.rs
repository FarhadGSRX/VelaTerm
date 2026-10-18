//! All large history consumers share two read slots. Preparation waits for source
//! jobs before taking a slot, so exports/searches cannot block the work they await.

use std::cell::Cell;
use std::sync::{Condvar,Mutex};
use std::sync::atomic::{AtomicBool,Ordering};
static ACTIVE:Mutex<usize>=Mutex::new(0);
static READY:Condvar=Condvar::new();
thread_local! {static DEPTH:Cell<usize>=const {Cell::new(0)};}
pub struct Work {root:bool,_thread:std::marker::PhantomData<std::rc::Rc<()>>}
impl Work {
    pub fn acquire(cancel:&AtomicBool) -> Result<Self,String> {
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        if DEPTH.with(|depth|depth.get()>0) {DEPTH.with(|depth|depth.set(depth.get()+1));return Ok(Self {root:false,_thread:std::marker::PhantomData});}
        let mut active=ACTIVE.lock().unwrap();
        while *active>=2 {
            if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
            active=READY.wait_timeout(active,std::time::Duration::from_millis(100)).unwrap().0;
        }
        if cancel.load(Ordering::Relaxed) {return Err("read_canceled".into());}
        *active+=1;DEPTH.with(|depth|depth.set(1));Ok(Self {root:true,_thread:std::marker::PhantomData})
    }
}
impl Drop for Work {
    fn drop(&mut self) {
        DEPTH.with(|depth|depth.set(depth.get()-1));
        if self.root {let mut active=ACTIVE.lock().unwrap();*active-=1;READY.notify_one();}
    }
}
