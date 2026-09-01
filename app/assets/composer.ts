import type { IslandInstance, IslandMountContext } from "hypergraft/browser";

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

function messageField(root: HTMLElement): HTMLTextAreaElement | null {
    return root.querySelector("#composer-message");
}

export function initComposer(
    root: HTMLElement,
    { signal }: IslandMountContext,
): IslandInstance | void {
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
        const submitter = root.querySelector<HTMLButtonElement>(
            'button[type="submit"][name="mode"][value="quick"]',
        );
        if (!submitter || submitter.disabled) {
            return;
        }
        root.requestSubmit(submitter);
    };

    root.addEventListener("keydown", onKeyDown, { signal });

    return {
        reconcile(context) {
            if (context.cause !== "patch") {
                return;
            }
            if (context.detail.form !== root) {
                return;
            }
            if (context.detail.outcome !== "applied-patch") {
                return;
            }
            if (context.detail.status !== 200) {
                return;
            }
            if (!context.detail.targetIds.includes("transcript")) {
                return;
            }
            const message = messageField(root);
            if (!message) {
                return;
            }
            message.value = "";
            message.focus();
        },
        destroy() {},
    };
}
