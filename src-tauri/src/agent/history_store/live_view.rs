//! History pages in front of a running conversation's live timeline.
//!
//! The timeline holds only rows its process produced. Rows recorded before the launch are read as
//! history pages from a branch cut at the sizes captured before the agent started, so the two never
//! overlap and no row needs to be matched by content. Live rows the timeline already dropped from
//! memory are paged from the full branch through their published aliases.
//!
//! Positions share one axis: history rows count backwards from the launch (negative), live rows
//! count from the first row the process produced. Clients order merged rows by these positions.

use serde_json::{json, Value};

use crate::host::AppCtx;
use super::api::{self, Branch, ProcessCut};
use super::limits::PAGE_BYTES;

/// Move a history page's positions onto the live axis, where the launch is position zero.
/// History rows also carry their generation: rewinding one goes through the index, while the
/// process's own rows are addressed in its timeline.
fn shift(page:&mut Value,base:i64,generation:&str) {
    tag(&mut page["rows"],2);
    for row in page["rows"].as_array_mut().into_iter().flatten() {row["historyGeneration"]=json!(generation);}
    if let Some(positions)=page["positions"].as_object_mut() {
        for value in positions.values_mut() {if let Some(position)=value.as_i64() {*value=json!(position-base);}}
    }
}

fn rows(value:&Value) -> Vec<Value> {value["rows"].as_array().cloned().unwrap_or_default()}

/// What the engine reports about a running conversation whose earlier rows are history pages.
pub struct Live {
    pub cut:ProcessCut,
    /// Live rows the timeline already released from memory; they remain in the index.
    pub dropped:usize,
    /// How the process's own rows read their details: 2 with original references, 1 from memory.
    pub row_protocol:u8,
}

/// Mark how every row reads its details. History rows always use the index; the process's own
/// rows follow its receive mode, which may differ when the recording was only too large to replay.
fn tag(rows:&mut Value,protocol:u8) {
    for row in rows.as_array_mut().into_iter().flatten() {
        if row.get("detailProtocol").is_none() {row["detailProtocol"]=json!(protocol);}
    }
}

struct Pages {full:Branch,capped:Branch}

impl Pages {
    /// Rows before the launch, counted on the cut branch; the base that shifts every history page.
    fn base(&self) -> Result<i64,String> {
        Ok(self.capped.page_with(None,None,None,true)?["knownRows"].as_i64().unwrap_or(0))
    }

    /// The history page ending just before `before`. A live row the timeline dropped resolves only
    /// through the full branch; any other row, or none, is read from the cut branch.
    fn older(&self,before:Option<&str>,live_row:bool,dropped:usize) -> Result<Value,String> {
        if dropped>0 && before.is_some() {
            match self.full.page(before,None,None) {
                Ok(mut page) if page["pendingLocation"].is_null() => {let base=self.base()?;shift(&mut page,base,&self.full.generation);return Ok(page);},
                Ok(_) => return Err("index_busy: earlier live rows are still being indexed".into()),
                Err(error) if live_row => return Err(error),
                Err(_) => {},
            }
        }
        let mut page=self.capped.page_with(if live_row {None} else {before},None,None,true)?;
        let base=page["knownRows"].as_i64().unwrap_or(0);shift(&mut page,base,&self.full.generation);Ok(page)
    }
}

fn facts(value:&mut Value,page:&Value) {
    for key in ["coverage","indexing","indexedBytes","sourceBytes","historyError","pendingLocation"] {
        if let Some(fact)=page.get(key) {value[key]=fact.clone();}
    }
}

/// Fold history into a running conversation's snapshot. `live` is the timeline window the engine
/// returned for the same request. History failures leave the live rows in place and report the
/// error, because the running agent stays usable without its earlier rows.
pub fn merge(app:&AppCtx,session:&str,args:&Value,live:&mut Value,state:&Live) {
    if let Err(error)=fold(app,session,args,live,&state.cut,state.dropped) {
        live["coverage"]=json!("error");live["historyError"]=json!(error);
        live["hasMore"]=json!(state.dropped>0);
    }
    // History rows were tagged when their page was shifted; the rest came from the timeline.
    tag(&mut live["rows"],state.row_protocol);
    live["protocolVersion"]=json!(2);
}

fn fold(app:&AppCtx,session:&str,args:&Value,live:&mut Value,cut:&ProcessCut,dropped:usize) -> Result<(),String> {
    let before=args.pointer("/window/before").and_then(Value::as_str);
    let after=args.get("after").and_then(Value::as_str);
    let anchor=args.get("anchor").and_then(Value::as_str);
    let full=api::prepare(app,session)?;
    live["historyGeneration"]=json!(full.generation);
    let read_job=full.store.subscribe(session,&full.sources,args.get("readJobId").and_then(Value::as_str))?;
    live["readJobId"]=json!(read_job);
    let pages=Pages {capped:full.before_process(cut)?,full};
    let timeline=rows(live);
    let kind=live["pageKind"].as_str().unwrap_or("recent").to_owned();

    if let Some(after)=after {
        // Paging forward: within the timeline, or out of history and into the first live rows.
        let next=if let Some(page)=app.chat().timeline_after(session,Some(after)) {page} else {
            let mut page=pages.capped.page_with(None,Some(after),None,true)?;
            let base=page["knownRows"].as_i64().unwrap_or(0);shift(&mut page,base,&pages.full.generation);facts(live,&page);
            if page["hasNewer"]!=true {
                if let Some(first)=app.chat().timeline_after(session,None) {
                    let mut merged=rows(&page);merged.extend(rows(&first));
                    for (id,position) in first["positions"].as_object().into_iter().flatten() {page["positions"][id]=position.clone();}
                    page["rows"]=json!(merged);page["hasNewer"]=first["hasNewer"].clone();
                }
            }
            page
        };
        live["rows"]=next["rows"].clone();live["positions"]=next["positions"].clone();
        live["hasNewer"]=next["hasNewer"].clone();live["pageKind"]=json!("history");
        return fit(live,false);
    }

    if let Some(anchor)=anchor.filter(|id|app.chat().timeline_after(session,Some(id)).is_none()) {
        // A located history row: show its page; the live rows follow as newer content.
        let mut page=match pages.capped.page_with(None,None,Some(anchor),true) {
            Ok(page)=>page,
            Err(_) if dropped>0 => pages.full.page(None,None,Some(anchor))?,
            Err(error)=>return Err(error),
        };
        let base=pages.base()?;shift(&mut page,base,&pages.full.generation);facts(live,&page);
        live["rows"]=page["rows"].clone();live["positions"]=page["positions"].clone();
        live["hasMore"]=page["hasMore"].clone();live["hasNewer"]=json!(true);
        if let Some(located)=page.get("locatedRowId") {live["locatedRowId"]=located.clone();}
        live["pageKind"]=json!("recent");
        return fit(live,false);
    }

    if let Some(before)=before {
        if kind=="history" && !timeline.is_empty() {
            // Still inside the timeline; earlier history remains once its first row is reached.
            if live["hasMore"]!=true {live["hasMore"]=json!(dropped>0 || cut.has_history());}
            return Ok(());
        }
        let page=pages.older(Some(before),kind=="history",dropped)?;
        facts(live,&page);
        live["rows"]=page["rows"].clone();live["positions"]=page["positions"].clone();
        live["hasMore"]=page["hasMore"].clone();live["pageKind"]=json!("history");
        return fit(live,true);
    }

    // The most recent window. History is prepended only when the timeline window reaches the
    // first live row; otherwise earlier live rows come first when the reader pages back.
    if live["hasMore"]==true || dropped>0 {live["hasMore"]=json!(true);return fit(live,true);}
    let page=pages.older(None,false,0)?;
    facts(live,&page);
    let mut merged=rows(&page);merged.extend(timeline);
    let mut positions=page["positions"].as_object().cloned().unwrap_or_default();
    for (id,position) in live["positions"].as_object().into_iter().flatten() {positions.insert(id.clone(),position.clone());}
    live["rows"]=json!(merged);live["positions"]=Value::Object(positions);
    live["hasMore"]=page["hasMore"].clone();live["pageKind"]=json!("recent");
    fit(live,true)
}

/// The latest history page before the launch, ready to be put in front of the timeline, or None
/// while the index has not yet passed the cut.
pub fn head(app:&AppCtx,session:&str,state:&Live) -> Result<Option<Value>,String> {
    let full=api::prepare(app,session)?;
    let mut page=full.before_process(&state.cut)?.page_with(None,None,None,true)?;
    if page["indexing"]==true && page["coverage"]!="error" {return Ok(None);}
    let base=page["knownRows"].as_i64().unwrap_or(0);shift(&mut page,base,&full.generation);
    page["historyGeneration"]=json!(full.generation);
    Ok(Some(page))
}

/// Put a history page in front of the timeline's newest window, within one page of bytes. The
/// history rows go only when that window already reaches the first live row.
pub fn compose(head:&Value,live_rows:Vec<Value>,live_positions:serde_json::Map<String,Value>,timeline_complete:bool,row_protocol:u8) -> Result<Value,String> {
    let mut value=json!({"rows":live_rows,"positions":live_positions,"hasMore":true});
    tag(&mut value["rows"],row_protocol);
    if timeline_complete {
        let mut merged=rows(head);merged.extend(rows(&value));
        let mut positions=head["positions"].as_object().cloned().unwrap_or_default();
        positions.extend(live_positions_of(&value));
        value=json!({"rows":merged,"positions":positions,"hasMore":head["hasMore"]});
    }
    fit(&mut value,true)?;Ok(value)
}

fn live_positions_of(value:&Value) -> serde_json::Map<String,Value> {value["positions"].as_object().cloned().unwrap_or_default()}

/// Keep the merged window within one page, removing history from the far end first.
fn fit(value:&mut Value,older_end:bool) -> Result<(),String> {
    while serde_json::to_vec(&*value).map_err(|e|e.to_string())?.len()>PAGE_BYTES {
        let rows=value["rows"].as_array_mut().ok_or("Chat snapshot has no rows")?;
        if rows.len()<=1 {return Err("History row exceeds its page byte budget".into());}
        let removed=rows.remove(if older_end {0} else {rows.len()-1});
        if let Some(id)=removed["id"].as_str() {if let Some(positions)=value["positions"].as_object_mut() {positions.remove(id);}}
        value[if older_end {"hasMore"} else {"hasNewer"}]=json!(true);
    }
    Ok(())
}
