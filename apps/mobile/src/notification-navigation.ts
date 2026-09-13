export interface NotificationTarget {id: string; sessionId?: string}

/** A newer tap supersedes a pending reconnect; the native callbacks can arrive during cold start. */
export class NotificationNavigation {
  private generation = 0;
  cancel() { this.generation++ }
  async open(target: NotificationTarget, actions: {
    cancelConnection(): void;
    disconnect(): Promise<void>;
    showConnections(): Promise<void>;
    connect(target: NotificationTarget): Promise<void>;
  }) {
    const epoch = ++this.generation;
    actions.cancelConnection();
    await actions.disconnect();
    if (epoch !== this.generation) return;
    await actions.showConnections();
    if (epoch !== this.generation) return;
    await actions.connect(target);
  }
}
