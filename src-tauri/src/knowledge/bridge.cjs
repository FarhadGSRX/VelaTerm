// Application-owned SDK worker. Source and results travel only through the private stdio channel.
const path = require('node:path');
const readline = require('node:readline');
const sdkPath = process.argv[1];
const root = process.argv[2];
const { default: CodeGraph } = require(sdkPath);
const { ToolHandler } = require(path.join(path.dirname(sdkPath), 'mcp/tools.js'));
const write = process.stdout.write.bind(process.stdout);
const send = value => write(`VLX_KNOWLEDGE ${JSON.stringify(value)}\n`);
// Upstream diagnostics must not share the result protocol or expose source in application logs.
console.log = console.info = console.warn = console.error = () => {};
let cg, handler, lastUsed = Date.now(), queue = Promise.resolve(), watching = false, executing = false;
const parent = process.ppid;
const stop = () => { try { cg?.destroy(); } finally { process.exit(0); } };
process.stdin.on('end', stop);
process.on('SIGTERM', stop);
setInterval(() => { if (process.ppid !== parent || (!executing && Date.now() - lastUsed > 300000)) stop(); }, 10000).unref();
const stats = () => ({ ...cg.getStats(), watching });
const progress = p => send({ event: 'progress', phase: p.phase, current: p.current, total: p.total });
async function open() {
  if (cg) { cg.reopenIfReplaced?.(); return; }
  cg = await CodeGraph.open(root);
  handler = new ToolHandler(cg);
  handler.setDefaultProjectHint(root);
  watching = cg.watch({
    onSyncComplete: () => { watching = true; send({ event: 'updated', stats: stats() }); },
    onSyncError: () => { watching = false; send({ event: 'watchError' }); },
    onDegraded: () => { watching = false; send({ event: 'watchError' }); },
  });
}
const graph = value => {
  const nodes = [...value.nodes.values()];
  const shown = nodes.slice(0, 500); const ids = new Set(shown.map(node => node.id));
  const edges = value.edges.filter(edge => ids.has(edge.source) && ids.has(edge.target)).slice(0, 2000);
  return { nodes: shown, edges, roots: value.roots, nodeCount: nodes.length, fileCount: new Set(nodes.map(node => node.filePath)).size, edgeCount: value.edges.length, truncated: shown.length < nodes.length || edges.length < value.edges.length };
};
async function query(args) {
  await open();
  if (args.action === 'sync') {
    const result = cg.getIndexState() === 'complete' ? await cg.sync({ onProgress: progress }) : await cg.indexAll({ onProgress: progress });
    if (result.success === false || cg.getIndexState() !== 'complete') throw new Error('knowledge_partial');
    return stats();
  }
  if (args.action === 'status') return stats();
  if (args.action === 'search') {
    const results = cg.searchNodes(args.query, { limit: args.limit + 1, offset: args.page * args.limit });
    return { nodes: results.slice(0, args.limit).map(r => ({ ...r.node, score: r.score })), hasMore: results.length > args.limit, pageSize: args.limit };
  }
  if (args.action === 'files') {
    const files = cg.getFiles().filter(f => !args.query || f.path.toLowerCase().includes(args.query.toLowerCase())).sort((a,b) => a.path.localeCompare(b.path));
    return { files: files.slice(args.page * args.limit, (args.page + 1) * args.limit), total: files.length, pageSize: args.limit };
  }
  if (args.action === 'impact') return { ...graph(cg.getImpactRadius(args.nodeId, args.depth)), depth: args.depth, maxDepth: 6 };
  if (args.action === 'callers' || args.action === 'callees') {
    const relations = args.action === 'callers' ? cg.getCallers(args.nodeId, args.depth) : cg.getCallees(args.nodeId, args.depth);
    return { relations: relations.slice(0, 500), total: relations.length, truncated: relations.length > 500, depth: args.depth, maxDepth: 6 };
  }
  if (args.action === 'path') return { path: cg.findPath(args.nodeId, args.targetId, ['calls']) };
  if (args.action === 'explore') {
    const result = await handler.execute('codegraph_explore', { query: args.query, maxFiles: args.maxFiles });
    if (result.isError) throw new Error('knowledge_query_failed');
    return { content: result.content };
  }
  throw new Error('knowledge_invalid');
}
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on('line', line => {
  queue = queue.then(async () => {
    lastUsed = Date.now(); executing = true;
    let request;
    try {
      request = JSON.parse(line);
      const result = await query(request);
      const serialized = JSON.stringify(result);
      if (Buffer.byteLength(serialized) > 8 * 1024 * 1024) throw new Error('knowledge_result_large');
      send({ result });
    } catch (error) {
      const code = /^knowledge_[a-z_]+$/.test(error.message) ? error.message : 'knowledge_query_failed';
      send({ error: code });
    } finally { lastUsed = Date.now(); executing = false; }
  });
});
