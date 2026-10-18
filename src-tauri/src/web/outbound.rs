//! Socket staging has byte and item quotas, including encrypted/writer copies still in flight.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool,AtomicUsize,Ordering};
use axum::extract::ws::Message;
use tokio::sync::{mpsc,Notify};

const CONTROL_BYTES:usize=3*1024*1024;
const PRESENTATION_BYTES:usize=5*1024*1024;
const ATTACHMENT_BYTES:usize=32*1024*1024;
const GLOBAL_BYTES:usize=64*1024*1024;
static GLOBAL:AtomicUsize=AtomicUsize::new(0);

#[derive(Clone,Copy)]
enum Class {Control,Presentation,Attachment}
struct Budget {control:AtomicUsize,presentation:AtomicUsize,attachment:AtomicUsize,closed:AtomicBool,pressure:Notify}
#[derive(Clone)]
pub(super) struct Sender {control:mpsc::Sender<Queued>,presentation:mpsc::Sender<Queued>,attachment:mpsc::Sender<Queued>,budget:Arc<Budget>}
pub(super) struct Receiver {control:mpsc::Receiver<Queued>,presentation:mpsc::Receiver<Queued>,attachment:mpsc::Receiver<Queued>,budget:Arc<Budget>,control_closed:bool,presentation_closed:bool,attachment_closed:bool}
pub(super) struct Queued {pub message:Message,_credit:Credit}
struct Credit {budget:Arc<Budget>,class:Class,bytes:usize}
#[derive(Debug)]
pub(super) struct Pressure;

impl Drop for Credit {
    fn drop(&mut self) {
        self.budget.counter(self.class).fetch_sub(self.bytes,Ordering::AcqRel);
        GLOBAL.fetch_sub(self.bytes,Ordering::AcqRel);
    }
}
impl Budget {
    fn counter(&self,class:Class) -> &AtomicUsize {match class {Class::Control=>&self.control,Class::Presentation=>&self.presentation,Class::Attachment=>&self.attachment}}
    fn fail(&self) {self.closed.store(true,Ordering::Release);self.pressure.notify_one();}
    fn reserve(self:&Arc<Self>,class:Class,bytes:usize) -> Result<Credit,Pressure> {
        if self.closed.load(Ordering::Acquire) {return Err(Pressure);}
        if GLOBAL.fetch_update(Ordering::AcqRel,Ordering::Acquire,|used|used.checked_add(bytes).filter(|next|*next<=GLOBAL_BYTES)).is_err() {
            self.fail();return Err(Pressure);
        }
        let limit=match class {Class::Control=>CONTROL_BYTES,Class::Presentation=>PRESENTATION_BYTES,Class::Attachment=>ATTACHMENT_BYTES};
        if self.counter(class).fetch_update(Ordering::AcqRel,Ordering::Acquire,|used|used.checked_add(bytes).filter(|next|*next<=limit)).is_err() {
            GLOBAL.fetch_sub(bytes,Ordering::AcqRel);self.fail();return Err(Pressure);
        }
        Ok(Credit {budget:self.clone(),class,bytes})
    }
}

pub(super) fn channel() -> (Sender,Receiver) {
    let budget=Arc::new(Budget {control:AtomicUsize::new(0),presentation:AtomicUsize::new(0),attachment:AtomicUsize::new(0),closed:AtomicBool::new(false),pressure:Notify::new()});
    let (control,control_rx)=mpsc::channel(64);let (presentation,presentation_rx)=mpsc::channel(128);
    let (attachment,attachment_rx)=mpsc::channel(1);
    (Sender {control,presentation,attachment,budget:budget.clone()},Receiver {control:control_rx,presentation:presentation_rx,attachment:attachment_rx,budget,control_closed:false,presentation_closed:false,attachment_closed:false})
}

impl Sender {
    pub(super) fn send(&self,message:Message) -> Result<(),Pressure> {
        let class=if matches!(message,Message::Binary(_)) {Class::Presentation} else {Class::Control};self.enqueue(message,class)
    }
    pub(super) fn presentation(&self,message:Message) -> Result<(),Pressure> {self.enqueue(message,Class::Presentation)}
    pub(super) fn attachment(&self,message:Message) -> Result<(),Pressure> {self.enqueue(message,Class::Attachment)}
    fn enqueue(&self,message:Message,class:Class) -> Result<(),Pressure> {
        let bytes=match &message {Message::Text(text)=>text.capacity(),Message::Binary(bytes)|Message::Ping(bytes)|Message::Pong(bytes)=>bytes.capacity(),
            Message::Close(frame)=>frame.as_ref().map_or(0,|frame|frame.reason.len())};
        // Original staging, JSON/encryption temporaries, and the socket writer each own a copy.
        // A minimum charge also bounds an adversarial stream of empty frames.
        let credit=self.budget.reserve(class,bytes.saturating_add(256).saturating_mul(4))?;
        let queued=Queued {message,_credit:credit};
        let sender=match class {Class::Control=>&self.control,Class::Presentation=>&self.presentation,Class::Attachment=>&self.attachment};
        if sender.try_send(queued).is_err() {self.budget.fail();return Err(Pressure);}Ok(())
    }
}
impl Receiver {
    pub(super) fn pressured(&self) -> bool {self.budget.closed.load(Ordering::Acquire)}
    pub(super) async fn recv(&mut self) -> Option<Queued> {
        loop {
            if self.pressured() || self.control_closed && self.presentation_closed && self.attachment_closed {return None;}
            tokio::select! {biased;
                _=self.budget.pressure.notified()=>return None,
                message=self.control.recv(),if !self.control_closed=>match message {Some(message)=>return Some(message),None=>self.control_closed=true},
                message=self.attachment.recv(),if !self.attachment_closed=>match message {Some(message)=>return Some(message),None=>self.attachment_closed=true},
                message=self.presentation.recv(),if !self.presentation_closed=>match message {Some(message)=>return Some(message),None=>self.presentation_closed=true},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_GATE:std::sync::Mutex<()>=std::sync::Mutex::new(());
    #[tokio::test]
    async fn control_has_reserved_bytes_and_priority_while_socket_copies_remain_charged() {
        let _gate=TEST_GATE.lock().unwrap();
        let (sender,mut receiver)=channel();
        sender.presentation(Message::Text("x".repeat(512*1024))).unwrap();
        sender.presentation(Message::Text("y".repeat(512*1024))).unwrap();
        sender.send(Message::Text("permission".into())).unwrap();
        let control=receiver.recv().await.unwrap();assert!(matches!(&control.message,Message::Text(text) if text=="permission"));
        assert!(sender.budget.control.load(Ordering::Acquire)>0);drop(control);assert_eq!(sender.budget.control.load(Ordering::Acquire),0);
        let in_flight=receiver.recv().await.unwrap();
        assert!(sender.presentation(Message::Text("z".repeat(512*1024))).is_err());
        assert!(receiver.pressured());assert!(receiver.recv().await.is_none());
        assert!(sender.budget.presentation.load(Ordering::Acquire)>0);
        drop(in_flight);drop(receiver);assert_eq!(sender.budget.presentation.load(Ordering::Acquire),0);
    }
    #[tokio::test]
    async fn empty_frames_and_oversized_controls_cannot_accumulate_unbounded_items() {
        let _gate=TEST_GATE.lock().unwrap();
        let (sender,mut receiver)=channel();for _ in 0..64 {sender.send(Message::Text(String::new())).unwrap();}
        assert!(sender.send(Message::Text(String::new())).is_err());assert!(receiver.recv().await.is_none());
        drop(receiver);assert_eq!(sender.budget.control.load(Ordering::Acquire),0);
        let (sender,mut receiver)=channel();assert!(sender.send(Message::Text("x".repeat(CONTROL_BYTES))).is_err());
        assert!(receiver.pressured());assert!(receiver.recv().await.is_none());assert_eq!(sender.budget.control.load(Ordering::Acquire),0);
    }
    #[tokio::test]
    async fn independent_connections_share_a_process_byte_quota_and_release_on_disconnect() {
        let _gate=TEST_GATE.lock().unwrap();
        let mut receivers=Vec::new();let mut senders=Vec::new();let mut pressured=false;
        for _ in 0..40 {let (sender,receiver)=channel();
            for _ in 0..2 {if sender.presentation(Message::Text("x".repeat(512*1024))).is_err() {pressured=true;break;}}
            senders.push(sender);receivers.push(receiver);if pressured {break;}
        }
        assert!(pressured);assert!(GLOBAL.load(Ordering::Acquire)<=GLOBAL_BYTES);
        drop(receivers);for sender in senders {assert_eq!(sender.budget.presentation.load(Ordering::Acquire),0);}
    }
    #[tokio::test]
    async fn explicit_original_image_uses_one_separate_attachment_slot_without_consuming_control_reserve() {
        let _gate=TEST_GATE.lock().unwrap();let (sender,mut receiver)=channel();
        sender.attachment(Message::Text("x".repeat(7*1024*1024))).unwrap();
        sender.send(Message::Text("permission".into())).unwrap();let control=receiver.recv().await.unwrap();
        assert!(matches!(&control.message,Message::Text(text) if text=="permission"));drop(control);
        let image=receiver.recv().await.unwrap();assert!(sender.budget.attachment.load(Ordering::Acquire)>=28*1024*1024);
        assert!(sender.attachment(Message::Text("x".repeat(7*1024*1024))).is_err());assert!(receiver.pressured());
        drop(image);drop(receiver);assert_eq!(sender.budget.attachment.load(Ordering::Acquire),0);
    }
}
