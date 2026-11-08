//! Bounded concurrency and Retry-After cooldowns for anonymous translation providers.

use super::*;
use std::sync::Condvar;

#[derive(Default)]
struct Throttle { active:usize, next:Option<Instant>, cooldown:Option<Instant> }
#[derive(Default)]
pub(super) struct Limiter { state:Mutex<Throttle>, changed:Condvar }
pub(super) struct Permit<'a>(&'a Limiter);
impl Drop for Permit<'_> {
    fn drop(&mut self) { self.0.state.lock().unwrap().active-=1;self.0.changed.notify_all(); }
}
impl Limiter {
    pub(super) fn acquire(&self,deadline:Instant) -> Result<Permit<'_>,String> {
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
    pub(super) fn rate_limited(&self,retry_after:Option<&str>) {
        let seconds=retry_after.and_then(|s|s.parse::<u64>().ok()).filter(|n|*n>0).unwrap_or(60).min(300);
        let mut state=self.state.lock().unwrap();let until=Instant::now()+Duration::from_secs(seconds);
        state.cooldown=Some(state.cooldown.map_or(until,|old|old.max(until)));
        drop(state);self.changed.notify_all();
    }
}
