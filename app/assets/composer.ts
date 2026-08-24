import type { IslandMountContext } from "hypergraft/browser";

function usesCommandKey(): boolean {
    const nav = navigator as Navigator & {
        userAgentData?: { platform?: string };
    };
    const platform = nav.userAgentData?.platform ?? nav.platform;
    return /Mac|iPhone|iPad|iPod/.test(platform);
}

function sendShortcutHint(): string {
    const key = usesCommandKey() ? "⌘" : "Ctrl";
    return `${key} + Enter to send.`;
}

export function initShortcutHint(root: HTMLElement): void {
    root.textContent = sendShortcutHint();
}

export function initComposer(
    root: HTMLElement,
    { signal }: IslandMountContext,
): void {
    if (!(root instanceof HTMLFormElement)) {
        return;
    }

    const onKeyDown = (event: KeyboardEvent) => {
        if (event.isComposing) {
            return;
        }
        if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey)) {
            return;
        }
        if (!(event.target instanceof HTMLTextAreaElement)) {
            return;
        }
        event.preventDefault();
        root.requestSubmit();
    };

    root.addEventListener("keydown", onKeyDown, { signal });
}
