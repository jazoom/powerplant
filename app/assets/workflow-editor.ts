import type { IslandInstance, IslandMountContext } from "hypergraft/browser";

export function initWorkflowEditor(
    root: HTMLElement,
    { signal }: IslandMountContext,
): IslandInstance | void {
    if (!(root instanceof HTMLFormElement)) return;

    let selected = "0";
    const tabs = () =>
        Array.from(
            root.querySelectorAll<HTMLButtonElement>(
                "[data-workflow-phase-tab]",
            ),
        );
    const panels = () =>
        Array.from(
            root.querySelectorAll<HTMLElement>("[data-workflow-phase-panel]"),
        );

    const sync = () => {
        const available = panels();
        if (
            !available.some(
                (panel) => panel.dataset.workflowPhasePanel === selected,
            )
        ) {
            selected = available[0]?.dataset.workflowPhasePanel ?? "0";
        }
        for (const tab of tabs()) {
            const active = tab.dataset.workflowPhaseTab === selected;
            tab.setAttribute("aria-selected", String(active));
            tab.tabIndex = active ? 0 : -1;
        }
        for (const panel of available) {
            panel.hidden = panel.dataset.workflowPhasePanel !== selected;
        }
    };

    const reveal = (element: HTMLElement) => {
        const panel = element.closest<HTMLElement>(
            "[data-workflow-phase-panel]",
        );
        if (panel) selected = panel.dataset.workflowPhasePanel ?? "0";
        for (
            let parent = element.parentElement;
            parent && parent !== root;
            parent = parent.parentElement
        ) {
            if (parent instanceof HTMLDetailsElement) parent.open = true;
        }
        sync();
    };

    const reconcile = () => {
        const errorPanel = root.querySelector<HTMLElement>(
            '[data-workflow-phase-error="true"]',
        );
        if (errorPanel) {
            selected = errorPanel.dataset.workflowPhasePanel ?? "0";
            for (const details of errorPanel.querySelectorAll("details"))
                details.open = true;
        }
        sync();
        errorPanel?.focus();
    };

    root.addEventListener(
        "click",
        (event) => {
            if (!(event.target instanceof Element)) return;
            const tab = event.target.closest<HTMLButtonElement>(
                "[data-workflow-phase-tab]",
            );
            if (!tab) return;
            selected = tab.dataset.workflowPhaseTab ?? "0";
            sync();
        },
        { signal },
    );

    root.addEventListener(
        "keydown",
        (event) => {
            if (!(event.target instanceof HTMLButtonElement)) return;
            const available = tabs();
            const index = available.indexOf(event.target);
            if (index < 0) return;
            let next: number;
            switch (event.key) {
                case "ArrowRight":
                    next = (index + 1) % available.length;
                    break;
                case "ArrowLeft":
                    next = (index + available.length - 1) % available.length;
                    break;
                case "Home":
                    next = 0;
                    break;
                case "End":
                    next = available.length - 1;
                    break;
                default:
                    return;
            }
            event.preventDefault();
            available[next]?.click();
            available[next]?.focus();
        },
        { signal },
    );

    // Native validation must reveal hidden controls before the browser moves focus.
    let invalidControl: HTMLElement | undefined;
    root.addEventListener(
        "invalid",
        (event) => {
            if (!(event.target instanceof HTMLElement)) return;
            if (!invalidControl) {
                invalidControl = event.target;
                reveal(invalidControl);
                // One validation pass can emit several native invalid events.
                setTimeout(() => {
                    invalidControl = undefined;
                }, 0);
            } else {
                event.preventDefault();
            }
        },
        { capture: true, signal },
    );

    reconcile();
    return {
        reconcile,
        destroy() {
            for (const panel of panels()) panel.hidden = false;
            for (const tab of tabs()) tab.tabIndex = 0;
        },
    };
}
