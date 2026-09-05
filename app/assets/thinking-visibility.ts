import type { IslandInstance, IslandMountContext } from "hypergraft/browser";

function sync(root: HTMLFormElement): void {
    const toggle = root.elements.namedItem("show_thinking");
    const transcript = document.querySelector<HTMLElement>("#transcript");
    if (!(toggle instanceof HTMLInputElement) || !transcript) {
        return;
    }
    transcript.dataset.showThinking = String(toggle.checked);
}

export function initThinkingVisibility(
    root: HTMLElement,
    { signal }: IslandMountContext,
): IslandInstance | void {
    if (!(root instanceof HTMLFormElement)) {
        return;
    }
    root.addEventListener(
        "change",
        (event) => {
            if (
                event.target instanceof HTMLInputElement &&
                event.target.name === "show_thinking"
            ) {
                sync(root);
                root.requestSubmit();
            }
        },
        { signal },
    );
    sync(root);
    return {
        reconcile(context) {
            if (
                context.cause === "patch" &&
                context.detail.outcome === "applied-patch" &&
                context.detail.targetIds.includes("thinking-visibility")
            ) {
                sync(root);
            }
        },
        destroy() {},
    };
}
