export type SettingsScrollRestorationAdapter<Scroller, FocusTarget> = {
    findScroller(): Scroller | null;
    resolveScroller(cached: Scroller | null): Scroller | null;
    getScrollTop(scroller: Scroller): number;
    setScrollTop(scroller: Scroller, top: number): void;
    getActiveElement(): FocusTarget | null;
    contains(element: FocusTarget): boolean;
    isActive(element: FocusTarget): boolean;
    focus(element: FocusTarget): void;
    requestFrame(callback: () => void): number;
    cancelFrame(id: number): void;
};

type Snapshot<FocusTarget> = {
    top: number;
    active: FocusTarget | null;
};

export function createSettingsScrollRestoration<Scroller, FocusTarget>(
    adapter: SettingsScrollRestorationAdapter<Scroller, FocusTarget>,
) {
    let mounted = false;
    let cachedScroller: Scroller | null = null;
    let mountGeneration = 0;
    let frameId: number | null = null;
    let requestGeneration = 0;
    let pendingSnapshot: Snapshot<FocusTarget> | null = null;
    let pendingScroller: Scroller | null = null;

    function cancelPending() {
        requestGeneration++;
        if (frameId !== null) {
            adapter.cancelFrame(frameId);
            frameId = null;
        }
        pendingSnapshot = null;
        pendingScroller = null;
    }

    function schedule(scroller: Scroller, snapshot: Snapshot<FocusTarget>) {
        pendingScroller = scroller;
        if (pendingSnapshot) return;

        pendingSnapshot = snapshot;
        requestGeneration++;
        const generation = requestGeneration;
        const ownerGeneration = mountGeneration;

        frameId = adapter.requestFrame(() => {
            if (generation !== requestGeneration || ownerGeneration !== mountGeneration) return;

            frameId = null;
            const target = pendingScroller;
            const saved = pendingSnapshot;
            pendingScroller = null;
            pendingSnapshot = null;
            if (!target || !saved) return;

            if (adapter.getScrollTop(target) !== saved.top) {
                adapter.setScrollTop(target, saved.top);
            }
            if (saved.active && adapter.contains(saved.active) && !adapter.isActive(saved.active)) {
                try {
                    adapter.focus(saved.active);
                } catch {
                    /* Preserve the existing behavior when preventScroll focus is unavailable. */
                }
            }
        });
    }

    return {
        mount() {
            cancelPending();
            mounted = true;
            const generation = ++mountGeneration;
            cachedScroller = adapter.findScroller();

            return () => {
                if (!mounted || generation !== mountGeneration) return;
                mounted = false;
                cachedScroller = null;
                mountGeneration++;
                cancelPending();
            };
        },
        invalidate() {
            if (!mounted) return;
            cachedScroller = adapter.findScroller();
            if (frameId !== null && pendingSnapshot) {
                pendingScroller = adapter.resolveScroller(cachedScroller);
            }
        },
        preserve<T>(run: () => T): T {
            const generationBeforeRun = mountGeneration;
            const scroller = mounted ? adapter.resolveScroller(cachedScroller) : adapter.findScroller();
            const snapshot =
                pendingSnapshot ??
                (scroller
                    ? {
                          top: adapter.getScrollTop(scroller),
                          active: adapter.getActiveElement(),
                      }
                    : null);
            const result = run();
            if (!scroller || !snapshot) return result;

            const target = mounted
                ? adapter.resolveScroller(cachedScroller)
                : generationBeforeRun === mountGeneration
                  ? scroller
                  : null;
            if (!target) {
                cancelPending();
                return result;
            }
            schedule(target, snapshot);
            return result;
        },
        dispose() {
            mounted = false;
            cachedScroller = null;
            mountGeneration++;
            cancelPending();
        },
    };
}
