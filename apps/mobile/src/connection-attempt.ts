/** Ignore completions from a connection that the user has already left. */
export class ConnectionAttempt {
  private generation = 0;
  async run(work: () => Promise<unknown>, error: (value: unknown) => void, finish: () => void) {
    const epoch = ++this.generation;
    try { await work() } catch (value) { if (epoch === this.generation) error(value) }
    finally { if (epoch === this.generation) finish() }
  }
  cancel() { this.generation++ }
}
