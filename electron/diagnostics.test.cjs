const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { createDiagnostics } = require("./diagnostics.cjs");

test("bootstrap diagnostics persist only approved numeric metadata", async () => {
  const dir=await fs.mkdtemp(path.join(os.tmpdir(),"vlx-log-test-"));
  try {
    const record=createDiagnostics(dir);
    record("startup_failed",{password:"synthetic-secret",bytes:8,exitCode:"user@example.test"});
    record("unknown synthetic-secret",{});
    await record.flush();
    const files=await fs.readdir(dir);
    assert.equal(files.length,1);
    const text=await fs.readFile(path.join(dir,files[0]),"utf8");
    assert.match(text,/event=startup_failed/);
    assert.match(text,/"bytes":8/);
    assert.doesNotMatch(text,/synthetic-secret|example\.test|password/);
  } finally { await fs.rm(dir,{recursive:true,force:true}); }
});
