export function initTranscript(root: HTMLElement): () => void {
    const stick = () => {
        const leftover =
            document.documentElement.scrollHeight -
            window.scrollY -
            window.innerHeight;
        if (leftover > 96) {
            return;
        }
        root.lastElementChild?.scrollIntoView({ block: "end" });
    };
    const observer = new MutationObserver(stick);
    observer.observe(root, { childList: true, subtree: true });
    return () => observer.disconnect();
}
