import type { IslandInstance } from "hypergraft/browser/islands";

const RETRY_MS = 1000;

export function initJobObserve(root: HTMLElement): IslandInstance {
    let timer: ReturnType<typeof setTimeout> | undefined;

    const form = () =>
        root.querySelector<HTMLFormElement>("form[method='get']");

    const submit = () => {
        if (root.dataset.jobActive !== "true") {
            return;
        }
        form()?.requestSubmit();
    };

    const clearTimer = () => {
        if (timer === undefined) {
            return;
        }
        clearTimeout(timer);
        timer = undefined;
    };

    const schedule = (delay: number) => {
        clearTimer();
        timer = setTimeout(() => {
            timer = undefined;
            submit();
        }, delay);
    };

    // The send POST is still pending when this root is first inserted.
    // A synchronous submit is dropped by the document unsafe guard.
    schedule(0);

    return {
        reconcile(context) {
            if (context.cause !== "patch") {
                return;
            }
            if (root.dataset.jobActive !== "true") {
                return;
            }
            // A later segment must start after this one settles. Morph may
            // keep this root, so mount will not run again.
            if (context.detail.outcome === "applied-patch") {
                if (!context.detail.targetIds.includes("composer")) {
                    return;
                }
                clearTimer();
                submit();
                return;
            }
            if (context.detail.form !== form()) {
                return;
            }
            if (context.detail.outcome !== "safe-failure") {
                return;
            }
            schedule(RETRY_MS);
        },
        destroy() {
            clearTimer();
        },
    };
}
