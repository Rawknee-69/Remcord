export interface RpcBlacklistEntry {
    name: string;
    id: number;
}

export function createRpcBlacklistCache(
    getInitialValue: () => RpcBlacklistEntry[],
    subscribe: (listener: (entries: RpcBlacklistEntry[]) => void) => () => void,
) {
    let entries: RpcBlacklistEntry[] = [];
    let disposed = false;
    let initialized = false;
    const unsubscribe = subscribe((updatedEntries) => {
        if (!disposed) {
            entries = updatedEntries;
            initialized = true;
        }
    });

    const initialEntries = getInitialValue();
    if (!initialized) entries = initialEntries;

    return {
        get(): RpcBlacklistEntry[] {
            return entries.map(({ name, id }) => ({ name, id }));
        },
        update(updatedEntries: RpcBlacklistEntry[]): void {
            if (!disposed) entries = updatedEntries;
        },
        dispose(): void {
            if (disposed) return;
            disposed = true;
            unsubscribe();
        },
    };
}
