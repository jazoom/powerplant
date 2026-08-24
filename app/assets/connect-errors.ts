import type { IslandInstance } from "hypergraft/browser/islands";

export function initConnectErrors(root: HTMLElement): IslandInstance {
    const focusSummary = () => {
        root.querySelector<HTMLElement>("#connect-errors")?.focus();
    };

    focusSummary();

    return {
        reconcile(context) {
            if (context.cause !== "patch") {
                return;
            }
            if (context.detail.outcome !== "applied-patch") {
                return;
            }
            if (context.detail.status === 200) {
                return;
            }
            if (!context.detail.targetIds.includes(root.id)) {
                return;
            }
            focusSummary();
        },
        destroy() {},
    };
}
