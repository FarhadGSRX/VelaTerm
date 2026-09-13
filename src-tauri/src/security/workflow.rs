//! Legacy result compatibility and shared audit logging. New scans execute the upstream workflow.
use super::Run;
#[cfg(test)]
use super::{scope, Result};
use crate::host::AppCtx;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub path: String,
    pub line: usize,
    pub end_line: usize,
    pub evidence: String,
    pub source: String,
    pub sink: String,
    pub preconditions: String,
    pub impact: String,
    pub recommendation: String,
    pub verdict: String,
}
#[cfg(test)]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reply {
    pub stage_id: String,
    pub summary: String,
    pub reviewed_files: Vec<String>,
    pub gaps: Vec<String>,
    pub findings: Vec<Finding>,
}
#[cfg(test)]
pub fn parse(text: &str, stage: &str) -> Result<Reply> {
    if text.len() > 512 * 1024 {
        return Err("security_invalid_output".into());
    }
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|s| s.trim_end().strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let reply: Reply = serde_json::from_str(text).map_err(|_| "security_invalid_output")?;
    if reply.stage_id != stage
        || reply.summary.len() > 30000
        || reply.gaps.len() > 200
        || reply.findings.len() > 100
        || reply.reviewed_files.len() > 20000
        || reply.gaps.iter().any(|s| s.len() > 4000)
    {
        return Err("security_invalid_output".into());
    }
    Ok(reply)
}
#[cfg(test)]
pub fn check_finding(root: &str, files: &[scope::File], finding: &Finding) -> Result<()> {
    if !["critical", "high", "medium", "low"].contains(&finding.severity.as_str())
        || !["candidate", "validated", "rejected"].contains(&finding.verdict.as_str())
        || finding.line == 0
        || finding.end_line < finding.line
        || finding.end_line - finding.line > 100
        || [
            &finding.id,
            &finding.title,
            &finding.evidence,
            &finding.source,
            &finding.sink,
            &finding.preconditions,
            &finding.impact,
            &finding.recommendation,
        ]
        .iter()
        .any(|s| s.trim().is_empty() || s.len() > 16000)
    {
        return Err("security_invalid_finding".into());
    }
    let file = files
        .iter()
        .find(|f| f.path == finding.path)
        .ok_or("security_outside_scope")?;
    let source = scope::source(root, &finding.path)?;
    if scope::digest(source.as_bytes()) != file.sha256 {
        return Err("security_source_changed".into());
    }
    let lines: Vec<_> = source.lines().collect();
    if finding.end_line > lines.len()
        || lines[finding.line - 1..finding.end_line].join("\n").trim() != finding.evidence.trim()
    {
        return Err("security_evidence_mismatch".into());
    }
    Ok(())
}

pub fn report(run: &Run) -> String {
    if let Some(report) = run.upstream.as_ref().and_then(|v| v["report"].as_str()) {
        if run.upstream.as_ref().is_some_and(|v| v["evidenceError"].is_string()) {
            return format!("# Audit validation failed\n\nStatus: {}\n\nSource evidence did not pass host verification ({}). The upstream report below is retained for inspection; it is not an accepted audit result.\n\n---\n\n{}", run.status, run.error, report);
        }
        return report.to_owned();
    }
    if let Some(upstream) = &run.upstream {
        return format!("# Codex Security scan receipt\n\nStatus: {}\n\nScan: {}\n\nEngine: {}\n\nModel: {}\n\nEffort: {}\n\nNo sealed report is available. This receipt is not a completed security assessment. Open the audit conversation for progress and any pending requests.\n",
            run.status, upstream["scan"]["scanId"].as_str().unwrap_or("unknown"), run.agent,
            run.model.as_deref().unwrap_or("native default"), run.effort.as_deref().unwrap_or("auto"));
    }
    let mut out = format!(
        "# Security audit\n\nStatus: {}\n\nEngine: {} (local CLI)\n\nModel: {}\n\nEffort: {}\n\nScope: {} {}\n\nFiles reported reviewed: {} / {}\n\nThis is a static review. Coverage is reported by the agent; it is not proof that all vulnerabilities were found. Dynamic exploitation was not performed.\n\n## Threat model\n\n{}\n\n## Findings\n",
        run.status, run.agent, run.model.as_deref().unwrap_or("native default"), run.effort.as_deref().unwrap_or("auto"), run.scope, run.path, run.reviewed.len(), run.files.len(), run.threat_model
    );
    for f in &run.findings {
        out.push_str(&format!("\n### {} · {} · {}\n\nLocation: {}:{}–{}\n\nSource: {}\n\nSink: {}\n\nPreconditions: {}\n\nImpact: {}\n\nRecommendation: {}\n\nEvidence:\n\n",f.severity,f.verdict,f.title,f.path,f.line,f.end_line,f.source,f.sink,f.preconditions,f.impact,f.recommendation));
        for line in f.evidence.lines() {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out.push_str("\n## Coverage gaps\n\n");
    for gap in &run.gaps {
        out.push_str(&format!("- {gap}\n"));
    }
    for file in &run.excluded {
        out.push_str(&format!("- Excluded or unavailable: {file}\n"));
    }
    for file in &run.files {
        if !run.reviewed.contains(&file.path) {
            out.push_str(&format!("- No reported review: {}\n", file.path));
        }
    }
    if !run.error.is_empty() {
        out.push_str(&format!("\nExecution error: {}\n", run.error));
    }
    out
}
pub fn log(app: &AppCtx, run: &Run, event: &str, data: &Value) {
    let _ = app;
    let level=if data["status"]=="failed" {"ERROR"} else if data["status"]=="invalid" {"WARN"} else {"INFO"};
    if !crate::diagnostics::enabled(&std::env::var("VLX_SECURITY_LOG_LEVEL").unwrap_or_else(|_|"info".into()),level) {return;}
    let mut fields=data.clone(); fields["jobId"]=serde_json::json!(run.id); fields["step"]=serde_json::json!(event);
    crate::diagnostics::record(level,"security",fields);
}
