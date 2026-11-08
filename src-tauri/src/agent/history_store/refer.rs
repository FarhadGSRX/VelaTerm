//! Markdown export of a history branch with bounded backpressure and cancellation when the reader drops.

use std::io::{Read,Write};
use std::sync::{Arc,mpsc::{self,Receiver,SyncSender}};
use std::sync::atomic::{AtomicBool,Ordering};

pub struct Stream {receiver:Receiver<Result<Vec<u8>,String>>,pending:std::io::Cursor<Vec<u8>>,cancel:Arc<AtomicBool>}
impl Read for Stream {
    fn read(&mut self,output:&mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {return Ok(0);}
        loop {
            let count=self.pending.read(output)?;if count>0 {return Ok(count);}
            match self.receiver.recv() {
                Ok(Ok(bytes))=>{self.pending=std::io::Cursor::new(bytes);},
                Ok(Err(error))=>return Err(std::io::Error::other(error)),
                Err(_)=>return Ok(0),
            }
        }
    }
}
impl Drop for Stream {fn drop(&mut self) {self.cancel.store(true,Ordering::Relaxed);}}

pub fn markdown_stream(branch:super::api::Branch,name:String,at:Option<String>) -> Stream {
    let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));let worker_cancel=cancel.clone();
    std::thread::spawn(move|| {
        let mut writer=std::io::BufWriter::with_capacity(32*1024,Output {sender:sender.clone(),cancel:worker_cancel.clone()});
        let result=super::markdown::write_markdown(branch,&name,at.as_deref(),&worker_cancel,&mut writer)
            .and_then(|_|writer.flush().map_err(|e|e.to_string()));
        if let Err(error)=result {let _=sender.send(Err(error));}
    });
    Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel}
}

struct Output {sender:SyncSender<Result<Vec<u8>,String>>,cancel:Arc<AtomicBool>}
impl Write for Output {
    fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
        for chunk in bytes.chunks(32*1024) {
            if self.cancel.load(Ordering::Relaxed) {return Err(std::io::Error::other("read_canceled"));}
            self.sender.send(Ok(chunk.to_vec())).map_err(|_|std::io::Error::other("read_canceled"))?;
        }Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {Ok(())}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_failure_reaches_the_reader_after_the_bytes_before_it() {
        let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));
        sender.send(Ok(b"# prefix".to_vec())).unwrap();sender.send(Err("source_changed: original was replaced".into())).unwrap();drop(sender);
        let mut stream=Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel};
        let mut output=Vec::new();assert!(stream.read_to_end(&mut output).is_err());assert_eq!(output,b"# prefix");
    }

    #[test]
    fn disconnect_cancels_a_backpressured_stream_and_releases_its_worker() {
        let (sender,receiver)=mpsc::sync_channel(2);let cancel=Arc::new(AtomicBool::new(false));let worker_cancel=cancel.clone();
        let worker=std::thread::spawn(move|| {
            let mut output=Output {sender,cancel:worker_cancel};output.write_all(&vec![b'x';256*1024]).unwrap_err().to_string()
        });
        let stream=Stream {receiver,pending:std::io::Cursor::new(Vec::new()),cancel:cancel.clone()};drop(stream);
        assert!(cancel.load(Ordering::Relaxed));assert!(worker.join().unwrap().contains("read_canceled"));
    }
}
