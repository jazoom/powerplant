export const PIN_THRESHOLD_PX = 96;

export function leftoverPx(input: {
    scrollHeight: number;
    scrollY: number;
    viewportHeight: number;
}): number {
    return input.scrollHeight - input.scrollY - input.viewportHeight;
}

export function isPinned(
    leftover: number,
    threshold = PIN_THRESHOLD_PX,
): boolean {
    return leftover <= threshold;
}

export function initTranscript(root: HTMLElement): () => void {
    // A large append increases leftover. Pin must come from the last user scroll.
    let pinned = true;

    const measure = () =>
        leftoverPx({
            scrollHeight: document.documentElement.scrollHeight,
            scrollY: window.scrollY,
            viewportHeight: window.innerHeight,
        });

    const onScroll = () => {
        pinned = isPinned(measure());
    };

    const stick = () => {
        if (!pinned) {
            return;
        }
        root.lastElementChild?.scrollIntoView({ block: "end" });
    };

    const observer = new MutationObserver(stick);
    observer.observe(root, { childList: true, subtree: true });
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
        observer.disconnect();
        window.removeEventListener("scroll", onScroll);
    };
}
