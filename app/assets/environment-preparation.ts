import type { IslandInstance } from "hypergraft/browser";

const RETRY_MS = 1000;

export function initEnvironmentPreparation(root: HTMLElement): IslandInstance {
    let timer: ReturnType<typeof setTimeout> | undefined;

    const form = () =>
        root.querySelector<HTMLFormElement>("form[method='get']");

    const submit = () => {
        if (root.dataset.preparationActive !== "true") {
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

    schedule(0);

    return {
        reconcile(context) {
            if (context.cause !== "patch") {
                return;
            }
            if (root.dataset.preparationActive !== "true") {
                return;
            }
            if (context.detail.outcome === "applied-patch") {
                if (
                    !context.detail.targetIds.includes(
                        "environment-preparation",
                    )
                ) {
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
